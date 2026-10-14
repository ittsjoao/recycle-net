use crate::index::Index;

#[derive(thiserror::Error, Debug)]
pub enum RetentionError {
    #[error("db: {0}")] Db(#[from] rusqlite::Error),
    #[error("io: {0}")] Io(#[from] std::io::Error),
}

/// Delete a backup file; returns true if the row should be dropped (file gone),
/// false if it is locked and must be skipped this pass.
fn remove_backup(backup_path: &str) -> bool {
    match std::fs::remove_file(backup_path) {
        Ok(()) => true,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => true,
        Err(e) => {
            tracing::warn!("retention: cannot delete {backup_path}: {e}; skipping this pass");
            false
        }
    }
}

/// Enforce size quota and optional age limit. `now` is the reference time
/// (unix seconds); `enforce` uses the real clock.
pub fn enforce_at(idx: &Index, max_bin_bytes: u64, max_age_days: Option<u32>, now: i64)
    -> Result<u64, RetentionError>
{
    let mut freed = 0u64;
    // 1. Age: evict anything deleted before the cutoff, regardless of size.
    if let Some(days) = max_age_days {
        let cutoff = now - (days as i64) * 86_400;
        for (id, backup_path, size) in idx.older_than_unrestored(cutoff)? {
            if remove_backup(&backup_path) {
                idx.delete_row(id)?;
                freed += size as u64;
            }
        }
    }
    // 2. Size: evict oldest until under the byte limit.
    let mut total = idx.total_size()? as u64;
    if total > max_bin_bytes {
        for (id, backup_path, size) in idx.oldest_unrestored(1_000_000)? {
            if total <= max_bin_bytes { break; }
            if remove_backup(&backup_path) {
                idx.delete_row(id)?;
                freed += size as u64;
                total = total.saturating_sub(size as u64);
            }
        }
    }
    Ok(freed)
}

/// Evict restored-excluded backups over the size quota or past the age limit.
pub fn enforce(idx: &Index, max_bin_bytes: u64, max_age_days: Option<u32>) -> Result<u64, RetentionError> {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0);
    enforce_at(idx, max_bin_bytes, max_age_days, now)
}

/// Permanently delete one backup (file, then row). A locked file is an error
/// and the row stays, so nothing is lost from the index.
pub fn purge(idx: &Index, id: i64) -> Result<(), RetentionError> {
    let (_, backup_path) = idx.conn_row(id)?;
    match std::fs::remove_file(&backup_path) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(e.into()),
    }
    idx.delete_row(id)?;
    Ok(())
}

/// Empty the bin: purge every non-restored backup; locked files are skipped.
/// Returns (purged, skipped).
pub fn purge_all(idx: &Index) -> Result<(u64, u64), RetentionError> {
    let (mut purged, mut skipped) = (0, 0);
    for (id, backup_path, _) in idx.oldest_unrestored(usize::MAX >> 1)? {
        if remove_backup(&backup_path) {
            idx.delete_row(id)?;
            purged += 1;
        } else {
            skipped += 1;
        }
    }
    Ok((purged, skipped))
}

#[cfg(test)]
mod tests {
    #[test]
    fn purge_removes_file_and_row_and_purge_all_empties() {
        let dir = tempfile::tempdir().unwrap();
        let idx = Index::open_memory().unwrap();
        let a = add(&idx, dir.path(), 1);
        add(&idx, dir.path(), 2);
        add(&idx, dir.path(), 3);
        purge(&idx, a).unwrap();
        assert!(!dir.path().join("1.bin").exists());
        assert!(idx.conn_row(a).is_err());
        assert_eq!(purge_all(&idx).unwrap(), (2, 0));
        assert_eq!(idx.total_size().unwrap(), 0);
        assert!(!dir.path().join("3.bin").exists());
    }
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
        let freed = enforce(&idx, 150, None).unwrap();
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
        let freed = enforce(&idx, 150, None).unwrap();
        assert_eq!(freed, 100);
        assert!(dir.path().join("1.bin").exists());
        assert!(!dir.path().join("2.bin").exists());
        assert!(dir.path().join("3.bin").exists());
        assert_eq!(idx.conn_row(restored).unwrap().1, dir.path().join("1.bin").to_string_lossy());
    }
    #[test]
    fn age_evicts_rows_older_than_cutoff_regardless_of_size() {
        let dir = tempfile::tempdir().unwrap();
        let idx = Index::open_memory().unwrap();
        let now = 1_000_000i64;
        let day = 86_400i64;
        // deleted_utc from add() is the loop index; set explicit ages instead.
        let old = {
            let bp = dir.path().join("old.bin");
            fs::write(&bp, vec![0u8; 100]).unwrap();
            idx.insert(&crate::index::DeletedFile{ original_path: "o".into(),
                backup_path: bp.to_string_lossy().into(), volume: "D:".into(), size: 100,
                deleted_utc: now - 10 * day, user_sid: None, user_name: None }).unwrap()
        };
        let fresh = {
            let bp = dir.path().join("fresh.bin");
            fs::write(&bp, vec![0u8; 100]).unwrap();
            idx.insert(&crate::index::DeletedFile{ original_path: "n".into(),
                backup_path: bp.to_string_lossy().into(), volume: "D:".into(), size: 100,
                deleted_utc: now - day, user_sid: None, user_name: None }).unwrap()
        };
        // quota huge (no size eviction); age cutoff 7 days
        let freed = enforce_at(&idx, u64::MAX, Some(7), now).unwrap();
        assert_eq!(freed, 100);
        assert!(!dir.path().join("old.bin").exists());
        assert!(dir.path().join("fresh.bin").exists());
        assert!(idx.conn_row(old).is_err());
        assert!(idx.conn_row(fresh).is_ok());
    }
    #[test]
    fn no_max_age_keeps_everything_under_quota() {
        let dir = tempfile::tempdir().unwrap();
        let idx = Index::open_memory().unwrap();
        for i in 1..=3i64 { add(&idx, dir.path(), i); }
        assert_eq!(enforce_at(&idx, u64::MAX, None, 10_000_000).unwrap(), 0);
        assert_eq!(idx.total_size().unwrap(), 300);
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
        let freed = enforce(&idx, 100, None).unwrap();
        assert_eq!(freed, 100);
        assert!(idx.conn_row(locked).is_ok()); // row kept
        assert!(!dir.path().join("2.bin").exists());
    }
}
