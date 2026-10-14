use std::path::{Path, PathBuf};
use crate::index::Index;

#[derive(thiserror::Error, Debug)]
pub enum RestoreError {
    #[error("db: {0}")] Db(#[from] rusqlite::Error),
    #[error("io: {0}")] Io(#[from] std::io::Error),
    #[error("not found")] NotFound,
    #[error("target exists")] Exists,
}

pub fn restore(idx: &Index, id: i64, dest_override: Option<&Path>) -> Result<PathBuf, RestoreError> {
    let (orig, backup): (String, String) = match idx.conn_row(id) {
        Ok(r) => r,
        Err(rusqlite::Error::QueryReturnedNoRows) => return Err(RestoreError::NotFound),
        Err(e) => return Err(e.into()),
    };
    let dest = dest_override.map(|p| p.to_path_buf()).unwrap_or_else(|| PathBuf::from(&orig));
    if dest_override.is_none() && dest.exists() { return Err(RestoreError::Exists); }
    if let Some(parent) = dest.parent() { std::fs::create_dir_all(parent)?; }
    std::fs::copy(&backup, &dest)?;
    idx.mark_restored(id)?;
    Ok(dest)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    #[test]
    fn restores_to_override_and_marks_row() {
        let dir = tempfile::tempdir().unwrap();
        let bp = dir.path().join("b.bin");
        fs::write(&bp, b"data").unwrap();
        let idx = crate::index::Index::open_memory().unwrap();
        let id = idx.insert(&crate::index::DeletedFile{
            original_path: "D:\\orig\\b.bin".into(), backup_path: bp.to_string_lossy().into(),
            volume: "D:".into(), size: 4, deleted_utc: 1, user_sid: None, user_name: None }).unwrap();
        let dest = dir.path().join("restored.bin");
        let out = restore(&idx, id, Some(&dest)).unwrap();
        assert_eq!(fs::read(&out).unwrap(), b"data");
        assert_eq!(idx.total_size().unwrap(), 0);
    }
}
