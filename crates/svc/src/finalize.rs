use std::path::{Path, PathBuf};
use crate::index::{Index, DeletedFile};

#[derive(thiserror::Error, Debug)]
pub enum FinalizeError {
    #[error("io: {0}")] Io(#[from] std::io::Error),
    #[error("db: {0}")] Db(#[from] rusqlite::Error),
}

/// Flag file naming: "<name>.nrbnew.nrbflag" (confirm at runtime, see docs/notes).
pub fn flag_path(nrbnew: &Path) -> PathBuf {
    let mut s = nrbnew.as_os_str().to_owned();
    s.push(".nrbflag");
    PathBuf::from(s)
}

pub fn is_complete(nrbnew: &Path) -> bool { flag_path(nrbnew).exists() }

pub fn finalize_one(idx: &Index, nrbnew: &Path, recycle_root: &Path)
    -> Result<Option<i64>, FinalizeError>
{
    if !nrbnew.exists() || !is_complete(nrbnew) { return Ok(None); }
    let final_path = nrbnew.with_extension(""); // strip .nrbnew
    std::fs::rename(nrbnew, &final_path)?;
    let _ = std::fs::remove_file(flag_path(nrbnew));
    let meta = std::fs::metadata(&final_path)?;
    // original_path/volume derived from the recycle-relative path (mirror layout)
    let rel = final_path.strip_prefix(recycle_root).unwrap_or(&final_path);
    let df = DeletedFile {
        original_path: rel.to_string_lossy().into_owned(),
        backup_path: final_path.to_string_lossy().into_owned(),
        volume: rel.components().next().map(|c| c.as_os_str().to_string_lossy().into_owned()).unwrap_or_default(),
        size: meta.len() as i64,
        deleted_utc: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH).unwrap().as_secs() as i64,
        user_sid: None, user_name: None,
    };
    Ok(Some(idx.insert(&df)?))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    #[test]
    fn skips_incomplete_backup() {
        let dir = tempfile::tempdir().unwrap();
        let nrb = dir.path().join("a.txt.nrbnew");
        fs::write(&nrb, b"partial").unwrap(); // no flag file
        let idx = crate::index::Index::open_memory().unwrap();
        assert_eq!(finalize_one(&idx, &nrb, dir.path()).unwrap(), None);
        assert_eq!(idx.total_size().unwrap(), 0);
    }
    #[test]
    fn finalizes_complete_backup_once() {
        let dir = tempfile::tempdir().unwrap();
        let nrb = dir.path().join("a.txt.nrbnew");
        fs::write(&nrb, b"hello").unwrap();
        fs::write(flag_path(&nrb), b"").unwrap(); // completion flag
        let idx = crate::index::Index::open_memory().unwrap();
        let id1 = finalize_one(&idx, &nrb, dir.path()).unwrap();
        assert!(id1.is_some());
        assert!(!nrb.exists());
        let id2 = finalize_one(&idx, &nrb, dir.path()).unwrap();
        assert_eq!(id2, None);
        assert_eq!(idx.total_size().unwrap(), 5);
    }
}
