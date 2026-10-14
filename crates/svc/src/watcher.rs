use std::path::{Path, PathBuf};
use std::sync::{Arc, atomic::{AtomicBool, Ordering}};
use std::sync::mpsc::channel;
use notify::{RecursiveMode, Watcher, EventKind};
use crate::index::Index;
use crate::finalize::{finalize_one, FinalizeError};

#[derive(thiserror::Error, Debug)]
pub enum WatchError {
    #[error("notify: {0}")] Notify(#[from] notify::Error),
    #[error("finalize: {0}")] Finalize(#[from] FinalizeError),
}

pub fn reconcile(idx: &Index, recycle_root: &Path) -> Result<usize, FinalizeError> {
    let mut count = 0;
    for entry in walkdir_nrbnew(recycle_root) {
        if finalize_one(idx, &entry, recycle_root)?.is_some() { count += 1; }
    }
    Ok(count)
}

fn walkdir_nrbnew(root: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(d) = stack.pop() {
        if let Ok(rd) = std::fs::read_dir(&d) {
            for e in rd.flatten() {
                let p = e.path();
                if p.is_dir() { stack.push(p); }
                else if p.extension().map_or(false, |x| x == "nrbnew") { out.push(p); }
            }
        }
    }
    out
}

pub fn watch(idx: Arc<Index>, recycle_root: PathBuf, stop: Arc<AtomicBool>) -> Result<(), WatchError> {
    let (tx, rx) = channel();
    let mut w = notify::recommended_watcher(move |res| { let _ = tx.send(res); })?;
    w.watch(&recycle_root, RecursiveMode::Recursive)?;
    while !stop.load(Ordering::Relaxed) {
        match rx.recv_timeout(std::time::Duration::from_millis(500)) {
            Ok(Ok(ev)) => {
                let ev: notify::Event = ev;
                if matches!(ev.kind, EventKind::Create(_) | EventKind::Modify(_)) {
                    for p in ev.paths {
                        // react to the flag file; derive the .nrbnew path
                        if p.extension().map_or(false, |x| x == "nrbflag") {
                            let nrb = p.with_extension(""); // strip .nrbflag
                            let _ = finalize_one(&idx, &nrb, &recycle_root);
                        }
                    }
                }
            }
            Ok(Err(_)) | Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
            Err(_) => break,
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
}
