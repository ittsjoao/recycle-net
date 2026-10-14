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
    #[error("prepare: {0}")]
    Prepare(#[from] std::io::Error),
}

const BACKOFF_MAX_SECS: u64 = 60;
/// Total backoff budget for connect+configure before handing back to SNetRBin.
pub const CONNECT_BUDGET: Duration = Duration::from_secs(300);
/// Wait before retrying a failed takeover.
pub const RETRY_WAIT: Duration = Duration::from_secs(15 * 60);
/// Re-send configure this often (driver reload recovery).
pub const HEARTBEAT_EVERY: Duration = Duration::from_secs(300);
const TICK: Duration = Duration::from_secs(60);

/// Capped exponential backoff: 1s, 2s, 4s, ... capped at 60s.
pub fn backoff_delay(attempt: u32) -> Duration {
    let secs = 1u64.checked_shl(attempt).unwrap_or(u64::MAX).min(BACKOFF_MAX_SECS);
    Duration::from_secs(secs)
}

pub fn retention_tick(idx: &Index, max_bin_bytes: u64) -> Result<u64, crate::retention::RetentionError> {
    retention::enforce(idx, max_bin_bytes)
}

/// Sleep up to `total` in 1-second slices, returning early when `stop` is set.
pub fn sleep_interruptible(total: Duration, stop: &AtomicBool) {
    let mut left = total;
    while !left.is_zero() && !stop.load(Ordering::Relaxed) {
        let slice = left.min(Duration::from_secs(1));
        std::thread::sleep(slice);
        left -= slice;
    }
}

/// Control of the original SNetRBin service.
pub trait Scm {
    /// Request stop and wait (bounded) until STOPPED. Absent service is Ok.
    fn stop_original(&self) -> Result<(), String>;
    /// Persistently disable the original (takeover persisted).
    fn disable_original(&self) -> Result<(), String>;
    /// Handback: re-enable (auto start) and start the original.
    fn restore_original(&self) -> Result<(), String>;
}

/// The driver communication-port session.
pub trait Driver {
    fn connect_and_configure(&mut self) -> Result<(), String>;
    /// Re-send configuration over the existing session.
    fn heartbeat(&mut self) -> Result<(), String>;
    fn disconnect(&mut self);
}

/// Transactional takeover. Returns true when we hold the driver port and the
/// original is disabled; false if stop was requested (original restored if we
/// had stopped it). Never leaves both services down: every non-success exit
/// after stopping the original restores it.
pub fn takeover<S: Scm, D: Driver>(
    scm: &S,
    driver: &mut D,
    stop: &AtomicBool,
    sleep: &dyn Fn(Duration),
) -> bool {
    enum Outcome {
        Connected,
        Aborted,
        Exhausted,
    }
    loop {
        if stop.load(Ordering::Relaxed) {
            return false;
        }
        if let Err(e) = scm.stop_original() {
            tracing::warn!("stopping SNetRBin: {e}");
        }
        let mut attempt: u32 = 0;
        let mut spent = Duration::ZERO;
        let outcome = loop {
            if stop.load(Ordering::Relaxed) {
                break Outcome::Aborted;
            }
            match driver.connect_and_configure() {
                Ok(()) => break Outcome::Connected,
                Err(e) => {
                    let d = backoff_delay(attempt);
                    attempt = attempt.saturating_add(1);
                    spent += d;
                    tracing::warn!("driver port unavailable ({e}); attempt {attempt}");
                    if spent >= CONNECT_BUDGET {
                        break Outcome::Exhausted;
                    }
                    sleep(d);
                }
            }
        };
        match outcome {
            Outcome::Connected => {
                tracing::info!("driver port connected and configured");
                if let Err(e) = scm.disable_original() {
                    tracing::error!("could not disable SNetRBin: {e}");
                }
                return true;
            }
            Outcome::Aborted => {
                if let Err(e) = scm.restore_original() {
                    tracing::error!("could not restore SNetRBin: {e}");
                }
                return false;
            }
            Outcome::Exhausted => {
                if let Err(e) = scm.restore_original() {
                    tracing::error!("could not restore SNetRBin: {e}");
                }
                tracing::error!("takeover failed; original SNetRBin restored; will retry in 15 min");
                sleep(RETRY_WAIT);
            }
        }
    }
}

/// Heartbeat: re-send configure. On error the session is dropped and false is
/// returned so the caller re-enters `takeover`.
pub fn heartbeat<D: Driver>(driver: &mut D) -> bool {
    match driver.heartbeat() {
        Ok(()) => true,
        Err(e) => {
            tracing::warn!("driver heartbeat failed ({e}); reconnecting");
            driver.disconnect();
            false
        }
    }
}

/// Local fallible setup done BEFORE any SCM action.
fn prepare(cfg: &Config) -> Result<Arc<Index>, ServiceError> {
    std::fs::create_dir_all(&cfg.recycle_dir)?;
    if let Some(parent) = cfg.db_path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)?;
        }
    }
    let idx = Arc::new(Index::open(&cfg.db_path)?);
    match watcher::reconcile(&idx, &cfg.recycle_dir) {
        Ok(n) => tracing::info!("initial reconcile adopted {n} item(s)"),
        Err(e) => tracing::error!("initial reconcile failed: {e}"),
    }
    Ok(idx)
}

fn spawn_watcher_supervisor(
    idx: Arc<Index>,
    root: std::path::PathBuf,
    stop: Arc<AtomicBool>,
) -> std::thread::JoinHandle<()> {
    std::thread::spawn(move || {
        let mut attempt: u32 = 0;
        while !stop.load(Ordering::Relaxed) {
            let (i, r, s) = (idx.clone(), root.clone(), stop.clone());
            let res = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| watcher::watch(i, r, s)));
            if stop.load(Ordering::Relaxed) {
                break;
            }
            match res {
                Ok(Ok(())) => tracing::error!("watcher exited unexpectedly; restarting"),
                Ok(Err(e)) => tracing::error!("watcher failed: {e}; restarting"),
                Err(_) => tracing::error!("watcher thread panicked; restarting"),
            }
            sleep_interruptible(backoff_delay(attempt), &stop);
            attempt = attempt.saturating_add(1);
        }
    })
}

/// Orchestration with injected SCM/driver (production uses `run`).
pub fn run_with<S: Scm, D: Driver>(
    cfg: Config,
    stop: Arc<AtomicBool>,
    scm: &S,
    driver: &mut D,
) -> Result<(), ServiceError> {
    // 1. PREPARE: on failure SNetRBin is never touched.
    let idx = prepare(&cfg)?;
    let handle = spawn_watcher_supervisor(idx.clone(), cfg.recycle_dir.clone(), stop.clone());
    let sleep = |d: Duration| sleep_interruptible(d, &stop);

    let mut protected = false;
    let mut since_hb = Duration::ZERO;
    while !stop.load(Ordering::Relaxed) {
        if !protected {
            protected = takeover(scm, driver, &stop, &sleep);
            since_hb = Duration::ZERO;
            if !protected {
                break;
            }
        }
        match retention_tick(&idx, cfg.max_bin_bytes) {
            Ok(freed) if freed > 0 => tracing::info!("retention freed {freed} bytes"),
            Ok(_) => {}
            Err(e) => tracing::warn!("retention tick failed: {e}"),
        }
        if let Err(e) = watcher::reconcile(&idx, &cfg.recycle_dir) {
            tracing::warn!("reconcile failed: {e}");
        }
        sleep(TICK);
        since_hb += TICK;
        if !stop.load(Ordering::Relaxed) && since_hb >= HEARTBEAT_EVERY {
            since_hb = Duration::ZERO;
            if !heartbeat(driver) {
                protected = false;
            }
        }
    }
    if handle.join().is_err() {
        tracing::error!("watcher supervisor thread panicked");
    }
    Ok(())
}

pub fn run(cfg: Config, stop: Arc<AtomicBool>, take_over_original: bool) -> Result<(), ServiceError> {
    let mut driver = WinDriver::new(&cfg);
    if take_over_original {
        run_with(cfg, stop, &WinScm, &mut driver)
    } else {
        run_with(cfg, stop, &NoScm, &mut driver)
    }
}

/// SCM stand-in for `--console` without `--takeover`: touches nothing.
pub struct NoScm;
impl Scm for NoScm {
    fn stop_original(&self) -> Result<(), String> {
        Ok(())
    }
    fn disable_original(&self) -> Result<(), String> {
        Ok(())
    }
    fn restore_original(&self) -> Result<(), String> {
        Ok(())
    }
}

pub struct WinDriver {
    includes: Vec<String>,
    excludes: Vec<String>,
    log_dir: String,
    port: Option<PortClient>,
}

impl WinDriver {
    pub fn new(cfg: &Config) -> Self {
        WinDriver {
            includes: cfg.includes.clone(),
            excludes: cfg.excludes.clone(),
            log_dir: cfg.recycle_dir.to_string_lossy().into_owned(),
            port: None,
        }
    }
}

impl Driver for WinDriver {
    fn connect_and_configure(&mut self) -> Result<(), String> {
        self.port = None;
        let p = PortClient::connect().map_err(|e| e.to_string())?;
        p.configure(&self.includes, &self.excludes, &self.log_dir).map_err(|e| e.to_string())?;
        self.port = Some(p);
        Ok(())
    }
    fn heartbeat(&mut self) -> Result<(), String> {
        match &self.port {
            Some(p) => p.configure(&self.includes, &self.excludes, &self.log_dir).map_err(|e| e.to_string()),
            None => Err("no port session".into()),
        }
    }
    fn disconnect(&mut self) {
        self.port = None;
    }
}

/// Real SCM access to the original `SNetRBin` service.
pub struct WinScm;

type ScHandles = (
    windows::Win32::System::Services::SC_HANDLE,
    windows::Win32::System::Services::SC_HANDLE,
);

impl WinScm {
    /// Open SNetRBin with `access`; Ok(None) if it does not exist.
    unsafe fn open(access: u32) -> Result<Option<ScHandles>, String> {
        use windows::core::w;
        use windows::Win32::Foundation::ERROR_SERVICE_DOES_NOT_EXIST;
        use windows::Win32::System::Services::*;
        let scm = OpenSCManagerW(None, None, SC_MANAGER_CONNECT).map_err(|e| format!("open SCM: {e}"))?;
        match OpenServiceW(scm, w!("SNetRBin"), access) {
            Ok(svc) => Ok(Some((scm, svc))),
            Err(e) => {
                let _ = CloseServiceHandle(scm);
                if e.code() == ERROR_SERVICE_DOES_NOT_EXIST.to_hresult() {
                    tracing::info!("SNetRBin service not present");
                    Ok(None)
                } else {
                    Err(format!("open SNetRBin: {e}"))
                }
            }
        }
    }

    fn change_start(
        start: windows::Win32::System::Services::SERVICE_START_TYPE,
        then_start: bool,
    ) -> Result<(), String> {
        use windows::Win32::Foundation::ERROR_SERVICE_ALREADY_RUNNING;
        use windows::Win32::System::Services::*;
        unsafe {
            let Some((scm, svc)) = Self::open(SERVICE_CHANGE_CONFIG | SERVICE_START)? else {
                return Ok(());
            };
            let mut result = ChangeServiceConfigW(
                svc,
                ENUM_SERVICE_TYPE(SERVICE_NO_CHANGE),
                start,
                SERVICE_ERROR(SERVICE_NO_CHANGE),
                None,
                None,
                None,
                None,
                None,
                None,
                None,
            )
            .map_err(|e| format!("change start type: {e}"));
            if then_start {
                match StartServiceW(svc, None) {
                    Ok(()) => tracing::info!("SNetRBin started"),
                    Err(e) if e.code() == ERROR_SERVICE_ALREADY_RUNNING.to_hresult() => {}
                    Err(e) => {
                        if result.is_ok() {
                            result = Err(format!("start: {e}"));
                        }
                    }
                }
            }
            let _ = CloseServiceHandle(svc);
            let _ = CloseServiceHandle(scm);
            result
        }
    }
}

impl Scm for WinScm {
    fn stop_original(&self) -> Result<(), String> {
        use windows::Win32::Foundation::ERROR_SERVICE_NOT_ACTIVE;
        use windows::Win32::System::Services::*;
        unsafe {
            let Some((scm, svc)) = Self::open(SERVICE_STOP | SERVICE_QUERY_STATUS)? else {
                return Ok(());
            };
            let mut status = SERVICE_STATUS::default();
            let mut result = Ok(());
            match ControlService(svc, SERVICE_CONTROL_STOP, &mut status) {
                Ok(()) => {}
                Err(e) if e.code() == ERROR_SERVICE_NOT_ACTIVE.to_hresult() => {
                    tracing::info!("SNetRBin already stopped");
                }
                Err(e) => result = Err(format!("stop request: {e}")),
            }
            let mut stopped = false;
            for _ in 0..60 {
                if QueryServiceStatus(svc, &mut status).is_ok() && status.dwCurrentState == SERVICE_STOPPED {
                    stopped = true;
                    break;
                }
                std::thread::sleep(Duration::from_millis(500));
            }
            if !stopped && result.is_ok() {
                result = Err("timed out waiting for SNetRBin to stop".into());
            }
            let _ = CloseServiceHandle(svc);
            let _ = CloseServiceHandle(scm);
            result
        }
    }

    fn disable_original(&self) -> Result<(), String> {
        Self::change_start(windows::Win32::System::Services::SERVICE_DISABLED, false)
    }

    fn restore_original(&self) -> Result<(), String> {
        Self::change_start(windows::Win32::System::Services::SERVICE_AUTO_START, true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    #[derive(Default)]
    struct FakeScm {
        calls: RefCell<Vec<&'static str>>,
    }
    impl Scm for FakeScm {
        fn stop_original(&self) -> Result<(), String> {
            self.calls.borrow_mut().push("stop");
            Ok(())
        }
        fn disable_original(&self) -> Result<(), String> {
            self.calls.borrow_mut().push("disable");
            Ok(())
        }
        fn restore_original(&self) -> Result<(), String> {
            self.calls.borrow_mut().push("restore");
            Ok(())
        }
    }

    /// Fails the first `fail_first` connects (u32::MAX = always).
    struct FakeDriver<'a> {
        scm: &'a FakeScm,
        fail_first: u32,
        connects: u32,
        hb_err: bool,
        disconnects: u32,
    }
    impl<'a> FakeDriver<'a> {
        fn new(scm: &'a FakeScm, fail_first: u32) -> Self {
            FakeDriver { scm, fail_first, connects: 0, hb_err: false, disconnects: 0 }
        }
    }
    impl Driver for FakeDriver<'_> {
        fn connect_and_configure(&mut self) -> Result<(), String> {
            self.connects += 1;
            self.scm.calls.borrow_mut().push("configure");
            if self.connects <= self.fail_first {
                Err("down".into())
            } else {
                Ok(())
            }
        }
        fn heartbeat(&mut self) -> Result<(), String> {
            if self.hb_err {
                Err("gone".into())
            } else {
                Ok(())
            }
        }
        fn disconnect(&mut self) {
            self.disconnects += 1;
        }
    }

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

    #[test]
    fn takeover_success_disables_only_after_configure() {
        let scm = FakeScm::default();
        let mut drv = FakeDriver::new(&scm, 2);
        let stop = AtomicBool::new(false);
        assert!(takeover(&scm, &mut drv, &stop, &|_| {}));
        assert_eq!(*scm.calls.borrow(), vec!["stop", "configure", "configure", "configure", "disable"]);
    }

    #[test]
    fn takeover_failure_restores_original_and_retries_after_15_min() {
        let scm = FakeScm::default();
        let mut drv = FakeDriver::new(&scm, u32::MAX);
        let stop = AtomicBool::new(false);
        let sleeps = RefCell::new(Vec::new());
        let sleep = |d: Duration| {
            sleeps.borrow_mut().push(d);
            if d == RETRY_WAIT {
                stop.store(true, Ordering::Relaxed);
            }
        };
        assert!(!takeover(&scm, &mut drv, &stop, &sleep));
        let calls = scm.calls.borrow();
        assert!(!calls.contains(&"disable"));
        assert_eq!(calls.first(), Some(&"stop"));
        assert_eq!(calls.last(), Some(&"restore"));
        assert_eq!(calls.iter().filter(|c| **c == "restore").count(), 1);
        assert!(sleeps.borrow().contains(&RETRY_WAIT));
        let backoff_total: Duration = sleeps.borrow().iter().filter(|d| **d != RETRY_WAIT).sum();
        assert!(backoff_total <= CONNECT_BUDGET);
    }

    #[test]
    fn takeover_stop_requested_midway_restores_original() {
        let scm = FakeScm::default();
        let mut drv = FakeDriver::new(&scm, u32::MAX);
        let stop = AtomicBool::new(false);
        let sleep = |_d: Duration| stop.store(true, Ordering::Relaxed);
        assert!(!takeover(&scm, &mut drv, &stop, &sleep));
        let calls = scm.calls.borrow();
        assert_eq!(calls.last(), Some(&"restore"));
        assert!(!calls.contains(&"disable"));
    }

    #[test]
    fn takeover_not_started_when_stop_already_set() {
        let scm = FakeScm::default();
        let mut drv = FakeDriver::new(&scm, 0);
        let stop = AtomicBool::new(true);
        assert!(!takeover(&scm, &mut drv, &stop, &|_| {}));
        assert!(scm.calls.borrow().is_empty());
    }

    #[test]
    fn heartbeat_error_drops_session_and_takeover_reenters() {
        let scm = FakeScm::default();
        let mut drv = FakeDriver::new(&scm, 0);
        let stop = AtomicBool::new(false);
        assert!(takeover(&scm, &mut drv, &stop, &|_| {}));
        assert!(heartbeat(&mut drv));
        drv.hb_err = true;
        assert!(!heartbeat(&mut drv));
        assert_eq!(drv.disconnects, 1);
        drv.hb_err = false;
        assert!(takeover(&scm, &mut drv, &stop, &|_| {}));
        assert_eq!(drv.connects, 2);
    }

    #[test]
    fn prepare_failure_never_touches_scm() {
        let dir = tempfile::tempdir().unwrap();
        let blocker = dir.path().join("blocker");
        std::fs::write(&blocker, b"x").unwrap();
        let cfg = Config {
            recycle_dir: dir.path().join("recycle"),
            db_path: blocker.join("sub").join("db.sqlite"), // parent is a file
            includes: vec!["*".into()],
            excludes: vec![],
            max_bin_bytes: 1,
            max_age_days: None,
        };
        let scm = FakeScm::default();
        let mut drv = FakeDriver::new(&scm, 0);
        let r = run_with(cfg, Arc::new(AtomicBool::new(false)), &scm, &mut drv);
        assert!(r.is_err());
        assert!(scm.calls.borrow().is_empty());
        assert_eq!(drv.connects, 0);
    }
}
