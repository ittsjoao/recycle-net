//! Who deleted a file: Windows object-access auditing (global SACL, DELETE only)
//! plus a subscription to Security events 4663 and 4659.
pub mod event;
pub mod setup;
pub mod subscribe;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::Duration;

use crate::config::Config;
use crate::index::Index;

const FILL_EVERY: Duration = Duration::from_secs(5);
const PRUNE_EVERY_SECS: i64 = 3600;
const KEEP_EVENTS_SECS: i64 = 24 * 3600;

fn now_secs() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}

/// Start the subscription and attribution threads; the subscription thread first
/// configures auditing when `configure` (off the start path, so STOP is never delayed).
/// Never fails: without auditing the bin keeps working and users show as unknown.
pub fn spawn(cfg: &Config, stop: Arc<AtomicBool>, configure: bool) -> Vec<JoinHandle<()>> {
    if !configure {
        tracing::warn!("audit: --console without --takeover does not configure auditing; using what is already on");
    }
    if let Some(parent) = cfg.db_path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let idx = match Index::open(&cfg.db_path) {
        Ok(i) => Arc::new(i),
        Err(e) => {
            tracing::warn!("audit: cannot open index: {e}");
            return vec![];
        }
    };
    let roots: Vec<String> = cfg.folders.iter().map(|f| f.path.display().to_string()).collect();
    let (sub_idx, sub_stop) = (idx.clone(), stop.clone());
    let subscription = std::thread::spawn(move || {
        if configure {
            if let Err(e) = setup::ensure() {
                tracing::warn!("audit: not configured ({e}); deleted-by will show as unknown");
            }
        }
        if let Err(e) = subscribe::run(sub_idx, roots, sub_stop) {
            tracing::warn!("audit: subscription stopped: {e}");
        }
    });
    let attribution = std::thread::spawn(move || {
        // After a long restart rows older than PENDING_SECS may still have their event stored.
        if let Err(e) = crate::attribution::fill_since(&idx, now_secs() - KEEP_EVENTS_SECS) {
            tracing::warn!("audit: startup attribution failed: {e}");
        }
        let mut last_prune = 0;
        while !stop.load(Ordering::Relaxed) {
            let now = now_secs();
            if let Err(e) = crate::attribution::fill_pending(&idx, now) {
                tracing::warn!("audit: attribution failed: {e}");
            }
            if now - last_prune >= PRUNE_EVERY_SECS {
                if let Err(e) = idx.prune_delete_events(now - KEEP_EVENTS_SECS) {
                    tracing::warn!("audit: prune failed: {e}");
                }
                last_prune = now;
            }
            crate::service::sleep_interruptible(FILL_EVERY, &stop);
        }
    });
    vec![subscription, attribution]
}
