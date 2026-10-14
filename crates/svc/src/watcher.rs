use std::path::{Path, PathBuf};
use std::sync::{Arc, atomic::{AtomicBool, Ordering}};
use std::sync::mpsc::channel;
use std::time::{Duration, Instant};
use notify::{RecursiveMode, Watcher, EventKind};
use crate::index::Index;
use crate::finalize::{finalize_one, FinalizeError};

#[derive(thiserror::Error, Debug)]
pub enum WatchError {
    #[error("notify: {0}")] Notify(#[from] notify::Error),
    #[error("finalize: {0}")] Finalize(#[from] FinalizeError),
}

const RECONCILE_INTERVAL: Duration = Duration::from_secs(60);

fn is_db_file(p: &Path) -> bool {
    ["db", "db-wal", "db-shm", "db-journal", "sqlite"].iter().any(|e| ext_is(p, e))
}

fn ext_is(p: &Path, want: &str) -> bool {
    p.extension().is_some_and(|x| x.to_string_lossy().eq_ignore_ascii_case(want))
}

/// Finalize every complete `.nrbnew` (and clean stale flags) under the root.
/// Per-file errors are logged and skipped; returns the number of successes.
pub fn reconcile(idx: &Index, recycle_root: &Path) -> Result<usize, FinalizeError> {
    let mut count = 0;
    let (candidates, others) = walk_candidates(recycle_root);
    for entry in candidates {
        match finalize_one(idx, &entry, recycle_root) {
            Ok(Some(_)) => count += 1,
            Ok(None) => {}
            Err(e) => tracing::warn!("reconcile: failed to finalize {}: {e}", entry.display()),
        }
    }
    // sweep: adopt finalized backup files that have no row (crash between rename and commit)
    for f in others {
        match crate::finalize::adopt_unindexed(idx, &f, recycle_root) {
            Ok(Some(_)) => count += 1,
            Ok(None) => {}
            Err(e) => tracing::warn!("reconcile: failed to adopt {}: {e}", f.display()),
        }
    }
    Ok(count)
}

/// `.nrbnew` paths to try; stale `.nrbflag` files map to their `.nrbnew` path.
/// Symlinks/junctions are not followed.
fn walk_candidates(root: &Path) -> (Vec<PathBuf>, Vec<PathBuf>) {
    let mut others: Vec<PathBuf> = Vec::new();
    let mut out: Vec<PathBuf> = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(d) = stack.pop() {
        if let Ok(rd) = std::fs::read_dir(&d) {
            for e in rd.flatten() {
                let Ok(ft) = e.file_type() else { continue };
                if ft.is_symlink() { continue; }
                let p = e.path();
                if ft.is_dir() { stack.push(p); }
                else if ext_is(&p, "nrbnew") { out.push(p); }
                else if ext_is(&p, "nrbflag") { out.push(p.with_extension("")); }
                else if !is_db_file(&p) { others.push(p); }
            }
        }
    }
    out.sort();
    out.dedup();
    others.sort();
    (out, others)
}

pub fn watch(idx: Arc<Index>, recycle_root: PathBuf, stop: Arc<AtomicBool>) -> Result<(), WatchError> {
    let (tx, rx) = channel();
    let mut w = notify::recommended_watcher(move |res| { let _ = tx.send(res); })?;
    w.watch(&recycle_root, RecursiveMode::Recursive)?;
    let mut last_reconcile = Instant::now();
    while !stop.load(Ordering::Relaxed) {
        let mut need_reconcile = false;
        match rx.recv_timeout(Duration::from_millis(500)) {
            Ok(Ok(ev)) => {
                let ev: notify::Event = ev;
                if matches!(ev.kind, EventKind::Create(_) | EventKind::Modify(_)) {
                    for p in ev.paths {
                        // react to the flag file; derive the .nrbnew path
                        if ext_is(&p, "nrbflag") {
                            let nrb = p.with_extension(""); // strip .nrbflag
                            if let Err(e) = finalize_one(&idx, &nrb, &recycle_root) {
                                tracing::warn!("finalize {} failed: {e}", nrb.display());
                                need_reconcile = true;
                            }
                        }
                    }
                }
            }
            Ok(Err(e)) => {
                tracing::warn!("notify error (possible overflow): {e}");
                need_reconcile = true;
            }
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
            Err(_) => break,
        }
        if need_reconcile || last_reconcile.elapsed() >= RECONCILE_INTERVAL {
            if need_reconcile { std::thread::sleep(Duration::from_millis(250)); }
            if let Err(e) = reconcile(&idx, &recycle_root) {
                tracing::warn!("periodic reconcile failed: {e}");
            }
            last_reconcile = Instant::now();
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    #[test]
    fn reconcile_finalizes_only_complete_orphans() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("a.txt.nrbnew"), b"aa").unwrap();
        fs::write(dir.path().join("a.txt.nrbnew.nrbflag"), b"").unwrap();
        fs::write(dir.path().join("b.txt.nrbnew"), b"bbbb").unwrap();
        let idx = crate::index::Index::open_memory().unwrap();
        let n = reconcile(&idx, dir.path()).unwrap();
        assert_eq!(n, 1);
        assert_eq!(idx.total_size().unwrap(), 2);
    }
    #[test]
    fn sweep_adopts_unindexed_files_but_not_db_or_empty() {
        let dir = tempfile::tempdir().unwrap();
        let sub = dir.path().join("D");
        fs::create_dir_all(&sub).unwrap();
        fs::write(sub.join("lost.txt"), b"abc").unwrap();
        fs::write(sub.join("empty.txt"), b"").unwrap();
        fs::write(dir.path().join("index.db"), b"sqlite").unwrap();
        let idx = crate::index::Index::open_memory().unwrap();
        assert_eq!(reconcile(&idx, dir.path()).unwrap(), 1);
        assert_eq!(idx.total_size().unwrap(), 3);
        assert_eq!(reconcile(&idx, dir.path()).unwrap(), 0); // idempotent
    }
    #[test]
    fn reconcile_continues_after_a_bad_orphan() {
        use std::os::windows::fs::OpenOptionsExt;
        let dir = tempfile::tempdir().unwrap();
        // bad: held open without share-delete so rename fails
        let bad = dir.path().join("a.txt.nrbnew");
        fs::write(&bad, b"x").unwrap();
        fs::write(dir.path().join("a.txt.nrbnew.nrbflag"), b"").unwrap();
        let _held = fs::OpenOptions::new().read(true).share_mode(1).open(&bad).unwrap();
        fs::write(dir.path().join("b.txt.nrbnew"), b"yy").unwrap();
        fs::write(dir.path().join("b.txt.nrbnew.nrbflag"), b"").unwrap();
        let idx = crate::index::Index::open_memory().unwrap();
        assert_eq!(reconcile(&idx, dir.path()).unwrap(), 1);
        assert_eq!(idx.total_size().unwrap(), 2);
    }
    #[test]
    fn reconcile_matches_extensions_case_insensitively_and_cleans_stale_flags() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("a.txt.NRBNEW"), b"aa").unwrap();
        fs::write(dir.path().join("a.txt.NRBNEW.NRBFLAG"), b"").unwrap();
        fs::write(dir.path().join("gone.txt.nrbnew.nrbflag"), b"").unwrap();
        let idx = crate::index::Index::open_memory().unwrap();
        assert_eq!(reconcile(&idx, dir.path()).unwrap(), 1);
        assert!(!dir.path().join("gone.txt.nrbnew.nrbflag").exists());
    }
}
