use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use crate::{config::Config, index::Index, port::PortClient, retention, watcher};

#[derive(thiserror::Error, Debug)]
pub enum ServiceError {
    #[error("port: {0}")]
    Port(#[from] crate::port::PortError),
    #[error("watch: {0}")]
    Watch(#[from] crate::watcher::WatchError),
    #[error("retention: {0}")]
    Retention(#[from] crate::retention::RetentionError),
    #[error("db: {0}")]
    Db(#[from] rusqlite::Error),
    #[error("scm: {0}")]
    Scm(windows::core::Error),
    #[error("finalize: {0}")]
    Finalize(#[from] crate::finalize::FinalizeError),
}

const BACKOFF_MAX_SECS: u64 = 60;

/// Capped exponential backoff: 1s, 2s, 4s, ... capped at 60s.
pub fn backoff_delay(attempt: u32) -> Duration {
    let secs = 1u64.checked_shl(attempt).unwrap_or(u64::MAX).min(BACKOFF_MAX_SECS);
    Duration::from_secs(secs)
}

pub fn retention_tick(idx: &Index, max_bin_bytes: u64) -> Result<u64, crate::retention::RetentionError> {
    retention::enforce(idx, max_bin_bytes)
}

/// Sleep up to `total` in 1-second slices, returning early when `stop` is set.
fn sleep_interruptible(total: Duration, stop: &AtomicBool) {
    let mut left = total;
    while !left.is_zero() && !stop.load(Ordering::Relaxed) {
        let slice = left.min(Duration::from_secs(1));
        std::thread::sleep(slice);
        left -= slice;
    }
}

/// Connect to the driver port and push the configuration, retrying with
/// capped exponential backoff. Returns None if stop was requested first.
fn connect_and_configure(cfg: &Config, stop: &AtomicBool) -> Option<PortClient> {
    let mut attempt: u32 = 0;
    while !stop.load(Ordering::Relaxed) {
        let res = PortClient::connect().and_then(|p| {
            p.configure(&cfg.includes, &cfg.excludes, &cfg.recycle_dir.to_string_lossy())?;
            Ok(p)
        });
        match res {
            Ok(p) => return Some(p),
            Err(e) => {
                let d = backoff_delay(attempt);
                tracing::warn!("driver port unavailable ({e}); retrying in {}s", d.as_secs());
                attempt = attempt.saturating_add(1);
                sleep_interruptible(d, stop);
            }
        }
    }
    None
}

pub fn run(cfg: Config, stop: Arc<AtomicBool>) -> Result<(), ServiceError> {
    if let Err(e) = stop_and_disable_original() {
        tracing::warn!("could not stop/disable original service: {e}");
    }
    let idx = Arc::new(Index::open(&cfg.db_path)?);
    // Keep the port handle alive for the whole run.
    let _port = match connect_and_configure(&cfg, &stop) {
        Some(p) => {
            tracing::info!("driver port connected and configured");
            Some(p)
        }
        None => return Ok(()),
    };
    match watcher::reconcile(&idx, &cfg.recycle_dir) {
        Ok(n) => tracing::info!("initial reconcile adopted {n} item(s)"),
        Err(e) => tracing::error!("initial reconcile failed: {e}"),
    }

    let w_idx = idx.clone();
    let w_root = cfg.recycle_dir.clone();
    let w_stop = stop.clone();
    let handle = std::thread::spawn(move || {
        if let Err(e) = watcher::watch(w_idx, w_root, w_stop) {
            tracing::error!("watcher exited: {e}");
        }
    });

    while !stop.load(Ordering::Relaxed) {
        match retention_tick(&idx, cfg.max_bin_bytes) {
            Ok(freed) if freed > 0 => tracing::info!("retention freed {freed} bytes"),
            Ok(_) => {}
            Err(e) => tracing::warn!("retention tick failed: {e}"),
        }
        sleep_interruptible(Duration::from_secs(60), &stop);
    }
    let _ = handle.join();
    Ok(())
}

/// Stop and disable the original `SNetRBin` service. Idempotent; a missing or
/// already-stopped service is not an error.
pub fn stop_and_disable_original() -> Result<(), ServiceError> {
    use windows::core::w;
    use windows::Win32::System::Services::*;
    unsafe {
        let scm = OpenSCManagerW(None, None, SC_MANAGER_CONNECT).map_err(ServiceError::Scm)?;
        match OpenServiceW(scm, w!("SNetRBin"), SERVICE_STOP | SERVICE_CHANGE_CONFIG) {
            Ok(svc) => {
                let mut status = SERVICE_STATUS::default();
                if let Err(e) = ControlService(svc, SERVICE_CONTROL_STOP, &mut status) {
                    tracing::info!("SNetRBin stop request: {e} (probably already stopped)");
                }
                if let Err(e) = ChangeServiceConfigW(
                    svc,
                    ENUM_SERVICE_TYPE(SERVICE_NO_CHANGE),
                    SERVICE_DISABLED,
                    SERVICE_ERROR(SERVICE_NO_CHANGE),
                    None, None, None, None, None, None, None,
                ) {
                    tracing::warn!("SNetRBin disable failed: {e}");
                } else {
                    tracing::info!("SNetRBin stopped and disabled");
                }
                let _ = CloseServiceHandle(svc);
            }
            Err(e) => tracing::info!("SNetRBin service not available: {e}"),
        }
        let _ = CloseServiceHandle(scm);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn retention_tick_never_over_max_frees_nothing() {
        let idx = crate::index::Index::open_memory().unwrap();
        assert_eq!(retention_tick(&idx, u64::MAX).unwrap(), 0);
    }

    #[test]
    fn backoff_doubles_and_caps() {
        assert_eq!(backoff_delay(0), Duration::from_secs(1));
        assert_eq!(backoff_delay(1), Duration::from_secs(2));
        assert_eq!(backoff_delay(2), Duration::from_secs(4));
        assert_eq!(backoff_delay(5), Duration::from_secs(32));
        assert_eq!(backoff_delay(6), Duration::from_secs(60));
        assert_eq!(backoff_delay(7), Duration::from_secs(60));
        assert_eq!(backoff_delay(1000), Duration::from_secs(60));
    }
}
