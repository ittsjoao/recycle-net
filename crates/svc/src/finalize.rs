use std::path::{Path, PathBuf};
use crate::index::{Index, DeletedFile};

#[derive(thiserror::Error, Debug)]
pub enum FinalizeError {
    #[error("io: {0}")] Io(#[from] std::io::Error),
    #[error("db: {0}")] Db(#[from] rusqlite::Error),
}

/// Finalized backups live here, out of the driver's way: the driver only writes
/// under `<recycle_root>\<DriveLetter>\`, and its copy fails (silently) when the
/// name is taken, so nothing of ours may stay at a name it can write.
pub const STORE_DIR: &str = ".store";

/// Driver contract (read from netrbin.sys, see docs/notes): it copies the deleted
/// file to `<recycle_root>\<DriveLetter>\<path>\<name>`, then creates an empty
/// `<name>.nrbnew` flag once the copy is complete. Data path of a flag:
fn data_path(flag: &Path) -> PathBuf { flag.with_extension("") }

/// Derive (volume, original_path) from the recycle-relative path of the copy.
/// The driver mirrors the path as `<recycle_root>\<DriveLetter>\<rest of path>`.
/// A first component that is not a single ASCII letter yields an empty volume and
/// the relative path as-is (non-absolute; restore without override will refuse it).
pub fn derive_original(rel: &Path) -> (String, String) {
    let mut comps = rel.components();
    let first = comps.next().map(|c| c.as_os_str().to_string_lossy().into_owned());
    match first {
        Some(l) if l.len() == 1 && l.as_bytes()[0].is_ascii_alphabetic() => {
            let rest: Vec<String> = comps.map(|c| c.as_os_str().to_string_lossy().into_owned()).collect();
            let letter = l.to_ascii_uppercase();
            (format!("{letter}:"), format!("{letter}:\\{}", rest.join("\\")))
        }
        _ => (String::new(), rel.to_string_lossy().into_owned()),
    }
}

/// Candidate final name number `n` (0 = base itself): `a.txt`, `a.1.txt`, ...
fn unique_candidate(base: &Path, n: u32) -> PathBuf {
    if n == 0 { return base.to_path_buf(); }
    let stem = base.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let name = match base.extension() {
        Some(e) => format!("{stem}.{n}.{}", e.to_string_lossy()),
        None => format!("{stem}.{n}"),
    };
    base.with_file_name(name)
}

/// Pick a final path that does not exist yet and reserve it atomically
/// (create_new), so a later rename can never replace an earlier backup.
fn reserve_unique(base: &Path) -> std::io::Result<PathBuf> {
    use std::fs::OpenOptions;
    let mut n = 0u32;
    loop {
        let cand = unique_candidate(base, n);
        match OpenOptions::new().write(true).create_new(true).open(&cand) {
            Ok(_) => return Ok(cand),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => n += 1,
            Err(e) => return Err(e),
        }
    }
}

fn secs(t: std::io::Result<std::time::SystemTime>) -> Option<i64> {
    t.ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map(|d| d.as_secs() as i64)
}

fn now_secs() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs() as i64
}

/// Create an index row for an already-finalized backup file that has none.
/// `orig_base` is the driver's copy path (under the recycle root) used to
/// derive volume/original_path.
fn adopt(idx: &Index, file: &Path, orig_base: &Path, recycle_root: &Path, deleted_utc: i64)
    -> Result<Option<i64>, FinalizeError>
{
    let key = file.to_string_lossy().into_owned();
    if idx.count_by_backup_path(&key)? > 0 { return Ok(None); }
    let meta = std::fs::metadata(file)?;
    let rel = orig_base.strip_prefix(recycle_root).unwrap_or(orig_base);
    let (volume, original_path) = derive_original(rel);
    let df = DeletedFile {
        original_path, volume, backup_path: key,
        size: meta.len() as i64, deleted_utc, user_sid: None, user_name: None,
    };
    Ok(Some(idx.insert(&df)?))
}

/// Finalize the backup announced by `flag` (`<name>.nrbnew`): move the copy into
/// the store and index it. A missing flag means the copy is still in progress.
pub fn finalize_one(idx: &Index, flag: &Path, recycle_root: &Path)
    -> Result<Option<i64>, FinalizeError>
{
    finalize_with(idx, flag, recycle_root, "COMMIT")
}

pub(crate) fn finalize_with(idx: &Index, flag: &Path, recycle_root: &Path, commit_sql: &str)
    -> Result<Option<i64>, FinalizeError>
{
    // The driver's flags are always empty; a non-empty `x.nrbnew` is a deleted
    // user file copied as data, finalized through its own flag.
    match std::fs::metadata(flag) {
        Ok(m) if m.len() == 0 => {}
        _ => return Ok(None),
    }
    let data = data_path(flag);
    let rel = data.strip_prefix(recycle_root).unwrap_or(&data).to_path_buf();
    let base = recycle_root.join(STORE_DIR).join(&rel);
    // the flag is written at copy completion ~ deletion time; the copy's mtime
    // may be the source's preserved mtime.
    let deleted_utc = secs(std::fs::metadata(flag).and_then(|m| m.modified())).unwrap_or_else(now_secs);
    if !data.exists() {
        // Flag without copy. Either a crash between the move and the commit
        // (a stored file with no row: adopt it, the flag is the only clue) or
        // the driver's copy hit an existing name and was dropped.
        let mut n = 0u32;
        loop {
            let cand = unique_candidate(&base, n);
            if !cand.exists() { if n > 0 { break; } else { n += 1; continue; } }
            if std::fs::metadata(&cand).map(|m| m.len() > 0).unwrap_or(false) {
                adopt(idx, &cand, &data, recycle_root, deleted_utc)?;
            }
            n += 1;
        }
        tracing::warn!("flag {} has no copy next to it; removed", flag.display());
        let _ = std::fs::remove_file(flag);
        return Ok(None);
    }

    let meta = std::fs::metadata(&data)?;
    let (volume, original_path) = derive_original(&rel);
    if let Some(dir) = base.parent() { std::fs::create_dir_all(dir)?; }
    let final_path = reserve_unique(&base)?;
    let df = DeletedFile {
        original_path, volume,
        backup_path: final_path.to_string_lossy().into_owned(),
        size: meta.len() as i64, deleted_utc,
        user_sid: None, user_name: None,
    };
    // row insert + move in one transaction; rollback if the move fails
    let renamed = std::cell::Cell::new(false);
    let res = idx.insert_with_commit(&df, commit_sql, || {
        std::fs::rename(&data, &final_path).map_err(FinalizeError::Io)?;
        renamed.set(true);
        Ok(())
    });
    match res {
        Ok(id) => {
            let _ = std::fs::remove_file(flag); // LAST
            Ok(Some(id))
        }
        Err(e) => {
            if renamed.get() {
                // commit failed after the move: put the copy back so the next
                // reconcile retries; never delete it. The flag is kept.
                if std::fs::rename(&final_path, &data).is_err() {
                    tracing::warn!("commit failed and could not restore {}; backup kept at {}",
                        data.display(), final_path.display());
                }
            } else {
                let _ = std::fs::remove_file(&final_path); // drop the reservation only
            }
            Err(e)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    /// What the driver leaves for one deleted file: the copy under its own name
    /// at `rel`, then an empty `<name>.nrbnew` completion flag. Returns the flag.
    fn drop_backup(root: &Path, rel: &str, bytes: &[u8]) -> PathBuf {
        let data = root.join(rel);
        fs::create_dir_all(data.parent().unwrap()).unwrap();
        fs::write(&data, bytes).unwrap();
        let flag = root.join(format!("{rel}.nrbnew"));
        fs::write(&flag, b"").unwrap();
        flag
    }
    #[test]
    fn skips_copy_still_in_progress() {
        let dir = tempfile::tempdir().unwrap();
        let data = dir.path().join("a.txt");
        fs::write(&data, b"partial").unwrap(); // no flag yet
        let idx = crate::index::Index::open_memory().unwrap();
        assert_eq!(finalize_one(&idx, &dir.path().join("a.txt.nrbnew"), dir.path()).unwrap(), None);
        assert_eq!(idx.total_size().unwrap(), 0);
        assert!(data.exists());
    }
    #[test]
    fn finalizes_into_the_store_and_frees_the_driver_slot() {
        let dir = tempfile::tempdir().unwrap();
        let flag = drop_backup(dir.path(), r"D\a.txt", b"hello");
        let idx = crate::index::Index::open_memory().unwrap();
        let id = finalize_one(&idx, &flag, dir.path()).unwrap().unwrap();
        assert!(!dir.path().join(r"D\a.txt").exists());
        assert!(!flag.exists());
        let (_, backup) = idx.conn_row(id).unwrap();
        assert_eq!(Path::new(&backup), dir.path().join(r".store\D\a.txt"));
        assert_eq!(fs::read(&backup).unwrap(), b"hello");
        assert_eq!(finalize_one(&idx, &flag, dir.path()).unwrap(), None);
        assert_eq!(idx.total_size().unwrap(), 5);
    }
    #[test]
    fn same_name_twice_keeps_both_backups() {
        let dir = tempfile::tempdir().unwrap();
        let idx = crate::index::Index::open_memory().unwrap();
        let flag = drop_backup(dir.path(), r"D\a.txt", b"first");
        let id1 = finalize_one(&idx, &flag, dir.path()).unwrap().unwrap();
        drop_backup(dir.path(), r"D\a.txt", b"second!");
        let id2 = finalize_one(&idx, &flag, dir.path()).unwrap().unwrap();
        let (_, b1) = idx.conn_row(id1).unwrap();
        let (_, b2) = idx.conn_row(id2).unwrap();
        assert_ne!(b1, b2);
        assert_eq!(fs::read(&b1).unwrap(), b"first");
        assert_eq!(fs::read(&b2).unwrap(), b"second!");
        assert!(b2.ends_with(r".store\D\a.1.txt"));
    }
    #[test]
    fn commit_failure_keeps_backup_and_flag_and_retry_succeeds() {
        let dir = tempfile::tempdir().unwrap();
        let flag = drop_backup(dir.path(), r"D\a.txt", b"precious");
        let idx = crate::index::Index::open_memory().unwrap();
        // forced commit failure (syntax error) AFTER the move succeeded
        assert!(finalize_with(&idx, &flag, dir.path(), "COMMIT BOGUS").is_err());
        assert_eq!(fs::read(dir.path().join(r"D\a.txt")).unwrap(), b"precious"); // put back
        assert!(flag.exists());
        assert_eq!(idx.total_size().unwrap(), 0);
        // same connection is not stuck in a transaction: retry succeeds
        assert!(finalize_one(&idx, &flag, dir.path()).unwrap().is_some());
        assert_eq!(fs::read(dir.path().join(r".store\D\a.txt")).unwrap(), b"precious");
    }
    #[test]
    fn unindexed_stored_file_with_stale_flag_is_adopted_and_evictable() {
        // crash between the move into the store and the commit
        let dir = tempfile::tempdir().unwrap();
        let fin = dir.path().join(r".store\D\a.txt");
        fs::create_dir_all(fin.parent().unwrap()).unwrap();
        fs::write(&fin, vec![1u8; 10]).unwrap();
        let flag = dir.path().join(r"D\a.txt.nrbnew");
        fs::create_dir_all(flag.parent().unwrap()).unwrap();
        fs::write(&flag, b"").unwrap();
        let idx = crate::index::Index::open_memory().unwrap();
        crate::watcher::reconcile(&idx, dir.path()).unwrap();
        assert!(!flag.exists());
        assert_eq!(idx.total_size().unwrap(), 10);
        assert_eq!(crate::retention::enforce(&idx, 0, None).unwrap(), 10);
        assert!(!fin.exists());
    }
    #[test]
    fn deleted_utc_comes_from_flag_mtime() {
        let dir = tempfile::tempdir().unwrap();
        let flag = drop_backup(dir.path(), "a.txt", b"x");
        let t = std::time::UNIX_EPOCH + std::time::Duration::from_secs(1_000_000_000);
        fs::OpenOptions::new().write(true).open(&flag).unwrap().set_modified(t).unwrap();
        let idx = crate::index::Index::open_memory().unwrap();
        let id = finalize_one(&idx, &flag, dir.path()).unwrap().unwrap();
        assert_eq!(idx.deleted_utc(id).unwrap(), 1_000_000_000);
    }
    #[test]
    fn flag_without_copy_is_cleaned_and_indexed_backups_are_not_readopted() {
        // The driver's copy hit an existing name (FILE_CREATE) but it still
        // wrote the flag: drop the flag, never index the old file again.
        let dir = tempfile::tempdir().unwrap();
        let idx = crate::index::Index::open_memory().unwrap();
        let flag = drop_backup(dir.path(), r"D\a.txt", b"old");
        finalize_one(&idx, &flag, dir.path()).unwrap().unwrap();
        fs::write(&flag, b"").unwrap(); // flag alone
        assert_eq!(finalize_one(&idx, &flag, dir.path()).unwrap(), None);
        assert!(!flag.exists());
        assert_eq!(idx.total_size().unwrap(), 3);
    }
    #[test]
    fn a_non_empty_file_named_nrbnew_is_data_not_a_flag() {
        // A deleted user file called `x.nrbnew` is copied as data; with no `x`
        // next to it, it must not be mistaken for a stale flag and removed.
        let dir = tempfile::tempdir().unwrap();
        let flag = drop_backup(dir.path(), "x.nrbnew", b"user data");
        let idx = crate::index::Index::open_memory().unwrap();
        let data = dir.path().join("x.nrbnew");
        assert_eq!(finalize_one(&idx, &data, dir.path()).unwrap(), None);
        assert!(data.exists());
        assert!(finalize_one(&idx, &flag, dir.path()).unwrap().is_some());
        assert_eq!(idx.total_size().unwrap(), 9);
    }
    #[test]
    fn rename_failure_rolls_back_row_and_keeps_flag() {
        let dir = tempfile::tempdir().unwrap();
        let flag = drop_backup(dir.path(), "a.txt", b"x");
        // hold the copy open without share-delete so the move fails on Windows
        use std::os::windows::fs::OpenOptionsExt;
        let _held = fs::OpenOptions::new().read(true).share_mode(1).open(dir.path().join("a.txt")).unwrap();
        let idx = crate::index::Index::open_memory().unwrap();
        assert!(finalize_one(&idx, &flag, dir.path()).is_err());
        assert_eq!(idx.total_size().unwrap(), 0);
        assert!(flag.exists());
        assert!(!dir.path().join(r".store\a.txt").exists());
    }
    #[test]
    fn derives_volume_and_original_from_mirror_layout() {
        let (v, o) = derive_original(Path::new("d\\data\\sub\\a.txt"));
        assert_eq!(v, "D:");
        assert_eq!(o, "D:\\data\\sub\\a.txt");
        let (v, o) = derive_original(Path::new("a.txt"));
        assert_eq!(v, "");
        assert_eq!(o, "a.txt");
    }
    #[test]
    fn finalize_records_derived_paths_without_unique_suffix() {
        let dir = tempfile::tempdir().unwrap();
        let idx = crate::index::Index::open_memory().unwrap();
        for _ in 0..2 {
            let flag = drop_backup(dir.path(), r"D\data\a.txt", b"z");
            finalize_one(&idx, &flag, dir.path()).unwrap().unwrap();
        }
        let (orig, backup, _) = idx.row_info(2).unwrap();
        assert_eq!(orig, "D:\\data\\a.txt");
        assert!(backup.ends_with(r".store\D\data\a.1.txt"));
    }
}
