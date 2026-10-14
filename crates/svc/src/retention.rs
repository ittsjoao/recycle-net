use crate::index::Index;

#[derive(thiserror::Error, Debug)]
pub enum RetentionError {
    #[error("db: {0}")] Db(#[from] rusqlite::Error),
    #[error("io: {0}")] Io(#[from] std::io::Error),
}

pub fn enforce(idx: &Index, max_bin_bytes: u64) -> Result<u64, RetentionError> {
    let mut freed = 0u64;
    loop {
        let total = idx.total_size()? as u64;
        if total <= max_bin_bytes { break; }
        let batch = idx.oldest_unrestored(64)?;
        if batch.is_empty() { break; }
        for (id, backup_path, size) in batch {
            let _ = std::fs::remove_file(&backup_path); // best-effort; row removed regardless
            idx.delete_row(id)?;
            freed += size as u64;
            if (idx.total_size()? as u64) <= max_bin_bytes { break; }
        }
    }
    Ok(freed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    #[test]
    fn evicts_oldest_until_under_limit() {
        let dir = tempfile::tempdir().unwrap();
        let idx = crate::index::Index::open_memory().unwrap();
        for i in 1..=3i64 {
            let bp = dir.path().join(format!("{i}.bin"));
            fs::write(&bp, vec![0u8; 100]).unwrap();
            idx.insert(&crate::index::DeletedFile{
                original_path: format!("o{i}"), backup_path: bp.to_string_lossy().into(),
                volume: "D:".into(), size: 100, deleted_utc: i,
                user_sid: None, user_name: None }).unwrap();
        }
        let freed = enforce(&idx, 150).unwrap();
        assert_eq!(freed, 200);
        assert_eq!(idx.total_size().unwrap(), 100);
        assert!(!dir.path().join("1.bin").exists());
        assert!(dir.path().join("3.bin").exists());
    }
}
