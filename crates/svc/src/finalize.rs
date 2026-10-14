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

/// Pick a final path that does not exist yet and reserve it atomically
/// (create_new), so a later rename can never replace an earlier backup.
/// Collisions get a counter inserted before the original extension: `a.1.txt`.
fn reserve_unique(base: &Path) -> std::io::Result<PathBuf> {
    use std::fs::OpenOptions;
    let mut n = 0u32;
    loop {
        let cand = if n == 0 { base.to_path_buf() } else {
            let stem = base.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
            let name = match base.extension() {
                Some(e) => format!("{stem}.{n}.{}", e.to_string_lossy()),
                None => format!("{stem}.{n}"),
            };
            base.with_file_name(name)
        };
        match OpenOptions::new().write(true).create_new(true).open(&cand) {
            Ok(_) => return Ok(cand),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => n += 1,
            Err(e) => return Err(e),
        }
    }
}

pub fn finalize_one(idx: &Index, nrbnew: &Path, recycle_root: &Path)
    -> Result<Option<i64>, FinalizeError>
{
    let flag = flag_path(nrbnew);
    if !nrbnew.exists() {
        // stale flag left by a crash after the rename: clean it up, idempotent no-op
        if flag.exists() { let _ = std::fs::remove_file(&flag); }
        return Ok(None);
    }
    if !flag.exists() { return Ok(None); }

    let meta = std::fs::metadata(nrbnew)?;
    let base = nrbnew.with_extension(""); // strip .nrbnew
    let rel = base.strip_prefix(recycle_root).unwrap_or(&base);
    let (volume, original_path) = derive_original(rel);
    let deleted_utc = meta.modified().ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs() as i64)
        .unwrap_or_else(|| std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH).unwrap().as_secs() as i64);

    let final_path = reserve_unique(&base)?;
    let df = DeletedFile {
        original_path, volume,
        backup_path: final_path.to_string_lossy().into_owned(),
        size: meta.len() as i64, deleted_utc,
        user_sid: None, user_name: None,
    };
    // row insert + rename in one transaction; rollback if the rename fails
    let res = idx.insert_with(&df, || {
        std::fs::rename(nrbnew, &final_path).map_err(FinalizeError::Io)
    });
    match res {
        Ok(id) => {
            let _ = std::fs::remove_file(&flag); // LAST
            Ok(Some(id))
        }
        Err(e) => {
            let _ = std::fs::remove_file(&final_path); // drop the reservation
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
