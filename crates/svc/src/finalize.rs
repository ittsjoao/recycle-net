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

/// Derive (volume, original_path) from the recycle-relative path of the backup
/// (with the `.nrbnew` suffix already stripped).
///
/// ASSUMPTION (pending VM confirmation): the driver mirrors the volume-relative
/// path under the recycle root as `<recycle_root>\<DriveLetter>\<rest of path>`.
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
/// `orig_base` is the path (under the recycle root, without unique suffix or
/// `.nrbnew`) used to derive volume/original_path.
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

/// Adopt a non-flag, non-.nrbnew recycle file that has no row (e.g. a crash
/// between rename and commit). Zero-byte files (unused reservations) are skipped.
/// NOTE: the original path is derived from the file's own name, so a unique
/// suffix (`a.1.txt`) cannot be told apart from a real name here.
pub fn adopt_unindexed(idx: &Index, file: &Path, recycle_root: &Path)
    -> Result<Option<i64>, FinalizeError>
{
    let meta = std::fs::metadata(file)?;
    if meta.len() == 0 { return Ok(None); }
    let t = secs(meta.modified()).unwrap_or_else(now_secs);
    adopt(idx, file, file, recycle_root, t)
}

pub fn finalize_one(idx: &Index, nrbnew: &Path, recycle_root: &Path)
    -> Result<Option<i64>, FinalizeError>
{
    finalize_with(idx, nrbnew, recycle_root, "COMMIT")
}

pub(crate) fn finalize_with(idx: &Index, nrbnew: &Path, recycle_root: &Path, commit_sql: &str)
    -> Result<Option<i64>, FinalizeError>
{
    let flag = flag_path(nrbnew);
    let base = nrbnew.with_extension(""); // strip .nrbnew
    if !nrbnew.exists() {
        if flag.exists() {
            // Stale flag. A crash between rename and commit leaves a final file
            // with no row: adopt it before discarding the flag (the only clue).
            let t = secs(std::fs::metadata(&flag).and_then(|m| m.modified())).unwrap_or_else(now_secs);
            let mut n = 0u32;
            loop {
                let cand = unique_candidate(&base, n);
                if !cand.exists() { if n > 0 { break; } else { n += 1; continue; } }
                if std::fs::metadata(&cand).map(|m| m.len() > 0).unwrap_or(false) {
                    adopt(idx, &cand, &base, recycle_root, t)?;
                }
                n += 1;
            }
            let _ = std::fs::remove_file(&flag);
        }
        return Ok(None);
    }
    if !flag.exists() { return Ok(None); }

    let meta = std::fs::metadata(nrbnew)?;
    let rel = base.strip_prefix(recycle_root).unwrap_or(&base);
    let (volume, original_path) = derive_original(rel);
    // flag is written at copy completion ~ deletion time; the .nrbnew mtime may
    // be the source's preserved mtime.
    let deleted_utc = secs(std::fs::metadata(&flag).and_then(|m| m.modified())).unwrap_or_else(now_secs);

    let final_path = reserve_unique(&base)?;
    let df = DeletedFile {
        original_path, volume,
        backup_path: final_path.to_string_lossy().into_owned(),
        size: meta.len() as i64, deleted_utc,
        user_sid: None, user_name: None,
    };
    // row insert + rename in one transaction; rollback if the rename fails
    let renamed = std::cell::Cell::new(false);
    let res = idx.insert_with_commit(&df, commit_sql, || {
        std::fs::rename(nrbnew, &final_path).map_err(FinalizeError::Io)?;
        renamed.set(true);
        Ok(())
    });
    match res {
        Ok(id) => {
            let _ = std::fs::remove_file(&flag); // LAST
            Ok(Some(id))
        }
        Err(e) => {
            if renamed.get() {
                // commit failed after the rename: put the backup back so the next
                // reconcile retries; never delete it. The flag is kept.
                if std::fs::rename(&final_path, nrbnew).is_err() {
                    tracing::warn!("commit failed and could not restore {}; backup kept at {}",
                        nrbnew.display(), final_path.display());
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
        assert!(dir.path().join("a.txt").exists());
        assert!(!flag_path(&nrb).exists());
        let id2 = finalize_one(&idx, &nrb, dir.path()).unwrap();
        assert_eq!(id2, None);
        assert_eq!(idx.total_size().unwrap(), 5);
    }
    #[test]
    fn same_name_twice_keeps_both_backups() {
        let dir = tempfile::tempdir().unwrap();
        let idx = crate::index::Index::open_memory().unwrap();
        let nrb = dir.path().join("a.txt.nrbnew");
        fs::write(&nrb, b"first").unwrap();
        fs::write(flag_path(&nrb), b"").unwrap();
        let id1 = finalize_one(&idx, &nrb, dir.path()).unwrap().unwrap();
        fs::write(&nrb, b"second!").unwrap();
        fs::write(flag_path(&nrb), b"").unwrap();
        let id2 = finalize_one(&idx, &nrb, dir.path()).unwrap().unwrap();
        let (_, b1) = idx.conn_row(id1).unwrap();
        let (_, b2) = idx.conn_row(id2).unwrap();
        assert_ne!(b1, b2);
        assert_eq!(fs::read(&b1).unwrap(), b"first");
        assert_eq!(fs::read(&b2).unwrap(), b"second!");
        assert_eq!(idx.count_by_backup_path(&b1).unwrap(), 1);
    }
    #[test]
    fn commit_failure_keeps_backup_and_flag_and_retry_succeeds() {
        let dir = tempfile::tempdir().unwrap();
        let nrb = dir.path().join("a.txt.nrbnew");
        fs::write(&nrb, b"precious").unwrap();
        fs::write(flag_path(&nrb), b"").unwrap();
        let idx = crate::index::Index::open_memory().unwrap();
        // forced commit failure (syntax error) AFTER the rename succeeded
        assert!(finalize_with(&idx, &nrb, dir.path(), "COMMIT BOGUS").is_err());
        assert_eq!(fs::read(&nrb).unwrap(), b"precious"); // put back as .nrbnew
        assert!(flag_path(&nrb).exists());
        assert_eq!(idx.total_size().unwrap(), 0);
        // same connection is not stuck in a transaction: retry succeeds
        let id = finalize_one(&idx, &nrb, dir.path()).unwrap();
        assert!(id.is_some());
        assert_eq!(idx.total_size().unwrap(), 8);
        assert_eq!(fs::read(dir.path().join("a.txt")).unwrap(), b"precious");
    }
    #[test]
    fn unindexed_final_file_with_stale_flag_is_adopted_and_evictable() {
        let dir = tempfile::tempdir().unwrap();
        let sub = dir.path().join("D");
        fs::create_dir_all(&sub).unwrap();
        let fin = sub.join("a.txt");
        fs::write(&fin, vec![1u8; 10]).unwrap();
        let nrb = sub.join("a.txt.nrbnew");
        fs::write(flag_path(&nrb), b"").unwrap(); // stale flag, no .nrbnew, no row
        let idx = crate::index::Index::open_memory().unwrap();
        crate::watcher::reconcile(&idx, dir.path()).unwrap();
        assert!(!flag_path(&nrb).exists());
        assert_eq!(idx.total_size().unwrap(), 10);
        let freed = crate::retention::enforce(&idx, 0).unwrap();
        assert_eq!(freed, 10);
        assert!(!fin.exists());
    }
    #[test]
    fn deleted_utc_comes_from_flag_mtime() {
        let dir = tempfile::tempdir().unwrap();
        let nrb = dir.path().join("a.txt.nrbnew");
        fs::write(&nrb, b"x").unwrap();
        let fl = flag_path(&nrb);
        fs::write(&fl, b"").unwrap();
        let t = std::time::UNIX_EPOCH + std::time::Duration::from_secs(1_000_000_000);
        fs::OpenOptions::new().write(true).open(&fl).unwrap().set_modified(t).unwrap();
        let idx = crate::index::Index::open_memory().unwrap();
        let id = finalize_one(&idx, &nrb, dir.path()).unwrap().unwrap();
        assert_eq!(idx.deleted_utc(id).unwrap(), 1_000_000_000);
    }
    #[test]
    fn stale_flag_without_backup_is_cleaned() {
        let dir = tempfile::tempdir().unwrap();
        let nrb = dir.path().join("a.txt.nrbnew");
        fs::write(flag_path(&nrb), b"").unwrap();
        let idx = crate::index::Index::open_memory().unwrap();
        assert_eq!(finalize_one(&idx, &nrb, dir.path()).unwrap(), None);
        assert!(!flag_path(&nrb).exists());
    }
    #[test]
    fn rename_failure_rolls_back_row_and_keeps_flag() {
        let dir = tempfile::tempdir().unwrap();
        let nrb = dir.path().join("a.txt.nrbnew");
        fs::write(&nrb, b"x").unwrap();
        fs::write(flag_path(&nrb), b"").unwrap();
        // hold the source open without share-delete so rename fails on Windows
        use std::os::windows::fs::OpenOptionsExt;
        let _held = fs::OpenOptions::new().read(true).share_mode(1).open(&nrb).unwrap();
        let idx = crate::index::Index::open_memory().unwrap();
        assert!(finalize_one(&idx, &nrb, dir.path()).is_err());
        assert_eq!(idx.total_size().unwrap(), 0);
        assert!(flag_path(&nrb).exists());
        assert!(!dir.path().join("a.txt").exists());
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
        let sub = dir.path().join("D").join("data");
        fs::create_dir_all(&sub).unwrap();
        let idx = crate::index::Index::open_memory().unwrap();
        for _ in 0..2 {
            let nrb = sub.join("a.txt.nrbnew");
            fs::write(&nrb, b"z").unwrap();
            fs::write(flag_path(&nrb), b"").unwrap();
            finalize_one(&idx, &nrb, dir.path()).unwrap().unwrap();
        }
        let (orig, backup, _) = idx.row_info(2).unwrap();
        assert_eq!(orig, "D:\\data\\a.txt");
        assert!(backup.ends_with("a.1.txt"));
    }
}
