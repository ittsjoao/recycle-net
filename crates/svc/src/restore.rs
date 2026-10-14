use std::path::{Path, PathBuf};
use crate::index::Index;

#[derive(thiserror::Error, Debug)]
pub enum RestoreError {
    #[error("db: {0}")] Db(#[from] rusqlite::Error),
    #[error("io: {0}")] Io(#[from] std::io::Error),
    #[error("not found")] NotFound,
    #[error("target exists")] Exists,
    #[error("already restored")] AlreadyRestored,
    #[error("original path is not absolute: {0}")] BadOriginalPath(String),
}

/// Copies the backup to the original path (or override), marks the row restored
/// (row kept as history) and deletes the backup file so it doesn't leak disk.
/// The default target is created with create_new (never clobbers).
pub fn restore(idx: &Index, id: i64, dest_override: Option<&Path>) -> Result<PathBuf, RestoreError> {
    let (orig, backup, restored) = match idx.row_info(id) {
        Ok(r) => r,
        Err(rusqlite::Error::QueryReturnedNoRows) => return Err(RestoreError::NotFound),
        Err(e) => return Err(e.into()),
    };
    if restored { return Err(RestoreError::AlreadyRestored); }
    let dest = match dest_override {
        Some(p) => p.to_path_buf(),
        None => {
            let p = PathBuf::from(&orig);
            if !p.is_absolute() { return Err(RestoreError::BadOriginalPath(orig)); }
            p
        }
    };
    if let Some(parent) = dest.parent() { std::fs::create_dir_all(parent)?; }
    if dest_override.is_some() {
        std::fs::copy(&backup, &dest)?;
    } else {
        let mut out = match std::fs::OpenOptions::new().write(true).create_new(true).open(&dest) {
            Ok(f) => f,
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => return Err(RestoreError::Exists),
            Err(e) => return Err(e.into()),
        };
        let mut src = std::fs::File::open(&backup)?;
        if let Err(e) = std::io::copy(&mut src, &mut out) {
            drop(out);
            let _ = std::fs::remove_file(&dest);
            return Err(e.into());
        }
    }
    idx.mark_restored(id)?;
    if let Err(e) = std::fs::remove_file(&backup) {
        tracing::warn!("restored but could not delete backup {backup}: {e}");
    }
    Ok(dest)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use crate::index::DeletedFile;
    fn add(idx: &Index, orig: &str, bp: &Path) -> i64 {
        idx.insert(&DeletedFile{
            original_path: orig.into(), backup_path: bp.to_string_lossy().into(),
            volume: "D:".into(), size: 4, deleted_utc: 1, user_sid: None, user_name: None }).unwrap()
    }
    #[test]
    fn restores_to_override_marks_row_and_deletes_backup() {
        let dir = tempfile::tempdir().unwrap();
        let bp = dir.path().join("b.bin");
        fs::write(&bp, b"data").unwrap();
        let idx = Index::open_memory().unwrap();
        let id = add(&idx, "D:\\orig\\b.bin", &bp);
        let dest = dir.path().join("restored.bin");
        let out = restore(&idx, id, Some(&dest)).unwrap();
        assert_eq!(fs::read(&out).unwrap(), b"data");
        assert_eq!(idx.total_size().unwrap(), 0);
        assert!(!bp.exists());
        assert!(matches!(restore(&idx, id, Some(&dest)), Err(RestoreError::AlreadyRestored)));
    }
    #[test]
    fn default_target_is_original_and_never_clobbers() {
        let dir = tempfile::tempdir().unwrap();
        let bp = dir.path().join("b.bin");
        fs::write(&bp, b"data").unwrap();
        let orig = dir.path().join("sub").join("o.bin");
        let idx = Index::open_memory().unwrap();
        let id = add(&idx, &orig.to_string_lossy(), &bp);
        fs::create_dir_all(orig.parent().unwrap()).unwrap();
        fs::write(&orig, b"existing").unwrap();
        assert!(matches!(restore(&idx, id, None), Err(RestoreError::Exists)));
        assert_eq!(fs::read(&orig).unwrap(), b"existing");
        assert!(bp.exists());
        fs::remove_file(&orig).unwrap();
        assert_eq!(restore(&idx, id, None).unwrap(), orig);
        assert_eq!(fs::read(&orig).unwrap(), b"data");
    }
    #[test]
    fn relative_original_without_override_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let bp = dir.path().join("b.bin");
        fs::write(&bp, b"data").unwrap();
        let idx = Index::open_memory().unwrap();
        let id = add(&idx, "rel\\b.bin", &bp);
        assert!(matches!(restore(&idx, id, None), Err(RestoreError::BadOriginalPath(_))));
    }
}
