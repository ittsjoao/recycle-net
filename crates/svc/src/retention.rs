use crate::index::Index;

#[derive(thiserror::Error, Debug)]
pub enum RetentionError {
    #[error("db: {0}")] Db(#[from] rusqlite::Error),
    #[error("io: {0}")] Io(#[from] std::io::Error),
}

/// Evict oldest non-restored backups (file + row) until under the limit.
/// A row is only dropped if its file was removed or is already gone; otherwise
/// it is logged and skipped for this pass.
pub fn enforce(idx: &Index, max_bin_bytes: u64) -> Result<u64, RetentionError> {
    let mut freed = 0u64;
    let mut total = idx.total_size()? as u64;
    if total <= max_bin_bytes { return Ok(0); }
    for (id, backup_path, size) in idx.oldest_unrestored(1_000_000)? {
        if total <= max_bin_bytes { break; }
        match std::fs::remove_file(&backup_path) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => {
                tracing::warn!("retention: cannot delete {backup_path}: {e}; skipping this pass");
                continue;
            }
        }
        idx.delete_row(id)?;
        freed += size as u64;
        total = total.saturating_sub(size as u64);
    }
    Ok(freed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    fn add(idx: &Index, dir: &std::path::Path, i: i64) -> i64 {
        let bp = dir.join(format!("{i}.bin"));
        fs::write(&bp, vec![0u8; 100]).unwrap();
        idx.insert(&crate::index::DeletedFile{
            original_path: format!("o{i}"), backup_path: bp.to_string_lossy().into(),
            volume: "D:".into(), size: 100, deleted_utc: i,
            user_sid: None, user_name: None }).unwrap()
    }
    #[test]
    fn evicts_oldest_until_under_limit() {
        let dir = tempfile::tempdir().unwrap();
        let idx = Index::open_memory().unwrap();
        for i in 1..=3i64 { add(&idx, dir.path(), i); }
        let freed = enforce(&idx, 150).unwrap();
        assert_eq!(freed, 200);
        assert_eq!(idx.total_size().unwrap(), 100);
        assert!(!dir.path().join("1.bin").exists());
        assert!(dir.path().join("3.bin").exists());
    }
    #[test]
    fn restored_rows_are_neither_evicted_nor_counted() {
        let dir = tempfile::tempdir().unwrap();
        let idx = Index::open_memory().unwrap();
        let restored = add(&idx, dir.path(), 1); // oldest
        idx.mark_restored(restored).unwrap();
        add(&idx, dir.path(), 2);
        add(&idx, dir.path(), 3);
        // non-restored total is 200; limit 150 evicts exactly one (id 2), not the restored one
        let freed = enforce(&idx, 150).unwrap();
        assert_eq!(freed, 100);
        assert!(dir.path().join("1.bin").exists());
        assert!(!dir.path().join("2.bin").exists());
        assert!(dir.path().join("3.bin").exists());
        assert_eq!(idx.conn_row(restored).unwrap().1, dir.path().join("1.bin").to_string_lossy());
    }
    #[test]
    fn locked_file_is_skipped_not_dropped() {
        use std::os::windows::fs::OpenOptionsExt;
        let dir = tempfile::tempdir().unwrap();
        let idx = Index::open_memory().unwrap();
        let locked = add(&idx, dir.path(), 1);
        add(&idx, dir.path(), 2);
        let _held = fs::OpenOptions::new().read(true).share_mode(1)
            .open(dir.path().join("1.bin")).unwrap();
        let freed = enforce(&idx, 100).unwrap();
        assert_eq!(freed, 100);
        assert!(idx.conn_row(locked).is_ok()); // row kept
        assert!(!dir.path().join("2.bin").exists());
    }
}
