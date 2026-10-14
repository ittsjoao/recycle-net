use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

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
    #[error("config: {0}")]
    Config(#[from] crate::config::ConfigError),
}

const BACKOFF_MAX_SECS: u64 = 60;
/// Total backoff budget for connect+configure before handing back to SNetRBin.
pub const CONNECT_BUDGET: Duration = Duration::from_secs(300);
/// Wait before retrying a failed takeover (first cycles).
pub const RETRY_WAIT: Duration = Duration::from_secs(15 * 60);
/// After this many consecutive failed cycles, retry only every LONG_RETRY_WAIT.
pub const MAX_FAST_FAILED_CYCLES: u32 = 3;
pub const LONG_RETRY_WAIT: Duration = Duration::from_secs(6 * 3600);
/// Re-send configure this often (driver reload recovery).
pub const HEARTBEAT_EVERY: Duration = Duration::from_secs(300);
const TICK: Duration = Duration::from_secs(60);
/// A watcher run longer than this counts as healthy (backoff reset).
const WATCHER_HEALTHY: Duration = Duration::from_secs(60);

pub const START_AUTO: u32 = 2;
pub const START_DISABLED: u32 = 4;
pub const STATE_PATH: &str = r"C:\ProgramData\rnetrbin\takeover.json";

/// Capped exponential backoff: 1s, 2s, 4s, ... capped at 60s.
pub fn backoff_delay(attempt: u32) -> Duration {
    let secs = 1u64.checked_shl(attempt).unwrap_or(u64::MAX).min(BACKOFF_MAX_SECS);
    Duration::from_secs(secs)
}

/// Wait between failed takeover cycles: 15 min, then 6 h after repeated failures.
pub fn retry_wait(failed_cycles: u32) -> Duration {
    if failed_cycles >= MAX_FAST_FAILED_CYCLES {
        LONG_RETRY_WAIT
    } else {
        RETRY_WAIT
    }
}

pub fn retention_tick(idx: &Index, max_bin_bytes: u64, max_age_days: Option<u32>) -> Result<u64, crate::retention::RetentionError> {
    retention::enforce(idx, max_bin_bytes, max_age_days)
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
    /// Current start type (None if the service does not exist).
    fn query_start_type(&self) -> Result<Option<u32>, String>;
    /// Persistently disable the original.
    fn disable_original(&self) -> Result<(), String>;
    /// Handback: set the start type (None = leave unchanged) and start the original.
    fn restore_start(&self, start_type: Option<u32>) -> Result<(), String>;
}

/// The driver communication-port session.
pub trait Driver {
    fn connect_and_configure(&mut self) -> Result<(), String>;
    /// Re-send configuration over the existing session.
    fn heartbeat(&mut self) -> Result<(), String>;
    fn disconnect(&mut self);
}

/// Content of the takeover state file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Marker {
    None,
    Corrupt,
    Saved(u32),
}

/// Persistent record that WE disabled SNetRBin (and what it was before).
pub trait StateStore {
    fn read(&self) -> Marker;
    fn write(&self, start_type: u32) -> Result<(), String>;
    fn clear(&self);
}

pub fn encode_marker(start_type: u32) -> String {
    format!("{{ \"original_start_type\": {start_type} }}\n")
}

pub fn decode_marker(s: &str) -> Option<u32> {
    let rest = &s[s.find("\"original_start_type\"")? + "\"original_start_type\"".len()..];
    let rest = rest.trim_start().strip_prefix(':')?.trim_start();
    let digits: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
    digits.parse().ok()
}

pub struct FileStore {
    path: PathBuf,
}

impl FileStore {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        FileStore { path: path.into() }
    }
}

impl StateStore for FileStore {
    fn read(&self) -> Marker {
        match std::fs::read_to_string(&self.path) {
            Ok(s) => decode_marker(&s).map_or(Marker::Corrupt, Marker::Saved),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Marker::None,
            Err(_) => Marker::Corrupt,
        }
    }
    fn write(&self, start_type: u32) -> Result<(), String> {
        use std::io::Write;
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        let mut tmp = self.path.clone().into_os_string();
        tmp.push(".tmp");
        let tmp = PathBuf::from(tmp);
        let mut f = std::fs::File::create(&tmp).map_err(|e| e.to_string())?;
        f.write_all(encode_marker(start_type).as_bytes()).map_err(|e| e.to_string())?;
        f.sync_all().map_err(|e| e.to_string())?;
        drop(f);
        std::fs::rename(&tmp, &self.path).map_err(|e| e.to_string())
    }
    fn clear(&self) {
        if let Err(e) = std::fs::remove_file(&self.path) {
            if e.kind() != std::io::ErrorKind::NotFound {
                tracing::error!("cannot remove takeover state file: {e}");
            }
        }
    }
}

/// State store for modes that never take over (console without --takeover).
pub struct NoStore;
impl StateStore for NoStore {
    fn read(&self) -> Marker {
        Marker::None
    }
    fn write(&self, _start_type: u32) -> Result<(), String> {
        Ok(())
    }
    fn clear(&self) {}
}

/// Re-enable (saved start type, fallback AUTO_START if corrupt) and start the
/// original, then delete the state file. Without a marker the start type is
/// left unchanged (we never disabled it), and it is just started.
pub fn restore<S: Scm, T: StateStore>(scm: &S, store: &T) {
    let marker = store.read();
    let start = match marker {
        Marker::None => None,
        Marker::Corrupt => Some(START_AUTO),
        Marker::Saved(t) => Some(t),
    };
    match scm.restore_start(start) {
        Ok(()) => {
            if marker != Marker::None {
                store.clear();
            }
        }
        Err(e) => tracing::error!("could not restore SNetRBin: {e}"),
    }
}

/// Early exit (bad config, prepare failure): if we previously disabled SNetRBin
/// give it back; otherwise do not touch the SCM at all.
pub fn early_exit_restore<S: Scm, T: StateStore>(scm: &S, store: &T) {
    if store.read() != Marker::None {
        tracing::error!("exiting early after a previous takeover; restoring SNetRBin");
        restore(scm, store);
    }
}

/// Early-exit helper for the real service (used by main on config failure).
pub fn early_exit_restore_service() {
    early_exit_restore(&WinScm, &FileStore::new(STATE_PATH));
}

/// Close the port (single-client) first, then give SNetRBin back.
fn handback<S: Scm, T: StateStore, D: Driver>(scm: &S, store: &T, driver: &mut D) {
    driver.disconnect();
    restore(scm, store);
}

/// Record the original start type, then disable. Never disables without a
/// durable record; never overrides an administrator's own disable.
fn disable_with_state<S: Scm, T: StateStore>(scm: &S, store: &T) {
    match scm.query_start_type() {
        Ok(Some(t)) if t != START_DISABLED => {
            if let Err(e) = store.write(t) {
                tracing::error!("cannot write takeover state ({e}); NOT disabling SNetRBin");
                return;
            }
            if let Err(e) = scm.disable_original() {
                tracing::error!("could not disable SNetRBin: {e}");
            }
        }
        Ok(Some(_)) => tracing::info!("SNetRBin already disabled; leaving as is"),
        Ok(None) => tracing::info!("SNetRBin not present; nothing to disable"),
        Err(e) => tracing::error!("cannot query SNetRBin start type ({e}); NOT disabling"),
    }
}

/// Transactional takeover. Returns true when we hold the driver port (and the
/// original is disabled with a state record); false if stop was requested.
/// Every non-success exit after stopping the original hands it back.
///
/// `failed_cycles` is owned by the caller and survives across calls: a
/// successful connect does NOT reset it (a session that dies right after
/// connecting must still escalate). The caller resets it once a heartbeat
/// proves the session is healthy.
pub fn takeover<S: Scm, T: StateStore, D: Driver>(
    scm: &S,
    store: &T,
    driver: &mut D,
    stop: &AtomicBool,
    failed_cycles: &mut u32,
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
                disable_with_state(scm, store);
                return true;
            }
            Outcome::Aborted => {
                handback(scm, store, driver);
                return false;
            }
            Outcome::Exhausted => {
                handback(scm, store, driver);
                *failed_cycles = failed_cycles.saturating_add(1);
                let wait = retry_wait(*failed_cycles);
                tracing::error!(
                    "takeover failed; original SNetRBin restored; will retry in {} min",
                    wait.as_secs() / 60
                );
                sleep(wait);
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
    // Refuse a populated directory we did not create (drive/share root, another
    // tool's bin) before anything is indexed or any SCM action happens.
    crate::config::ensure_dedicated_dir(&cfg.recycle_dir)?;
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
    root: PathBuf,
    stop: Arc<AtomicBool>,
) -> std::thread::JoinHandle<()> {
    std::thread::spawn(move || {
        let mut attempt: u32 = 0;
        while !stop.load(Ordering::Relaxed) {
            let (i, r, s) = (idx.clone(), root.clone(), stop.clone());
            let started = Instant::now();
            let res = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| watcher::watch(i, r, s)));
            if stop.load(Ordering::Relaxed) {
                break;
            }
            if started.elapsed() > WATCHER_HEALTHY {
                attempt = 0;
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

/// Orchestration with injected SCM/state/driver/sleeper (production uses `run`).
/// `handback_on_exit` is set only by an explicit STOP (not SHUTDOWN).
pub fn run_with<S: Scm, T: StateStore, D: Driver>(
    cfg: Config,
    stop: Arc<AtomicBool>,
    handback_on_exit: &AtomicBool,
    scm: &S,
    store: &T,
    driver: &mut D,
    sleep: &dyn Fn(Duration),
) -> Result<(), ServiceError> {
    // 1. PREPARE: on failure the SCM is only touched if we had taken over before.
    let idx = match prepare(&cfg) {
        Ok(i) => i,
        Err(e) => {
            early_exit_restore(scm, store);
            return Err(e);
        }
    };
    let handle = spawn_watcher_supervisor(idx.clone(), cfg.recycle_dir.clone(), stop.clone());

    let mut protected = false;
    let mut failed_cycles: u32 = 0;
    let mut since_hb = Duration::ZERO;
    while !stop.load(Ordering::Relaxed) {
        if !protected {
            protected = takeover(scm, store, driver, &stop, &mut failed_cycles, sleep);
            since_hb = Duration::ZERO;
            if !protected {
                break;
            }
        }
        match retention_tick(&idx, cfg.max_bin_bytes, cfg.max_age_days) {
            Ok(freed) if freed > 0 => tracing::info!("retention freed {freed} bytes"),
            Ok(_) => {}
            Err(e) => tracing::warn!("retention tick failed: {e}"),
        }
        if let Err(e) = watcher::reconcile_until(&idx, &cfg.recycle_dir, &stop) {
            tracing::warn!("reconcile failed: {e}");
        }
        sleep(TICK);
        since_hb += TICK;
        if !stop.load(Ordering::Relaxed) && since_hb >= HEARTBEAT_EVERY {
            since_hb = Duration::ZERO;
            if heartbeat(driver) {
                // A session that survives a heartbeat is healthy.
                failed_cycles = 0;
            } else {
                protected = false;
            }
        }
    }
    // Hand back on STOP whenever WE are the reason SNetRBin is down: either we
    // hold the port now, or the state marker proves we disabled it earlier
    // (STOP during prepare / before the first loop pass after a SHUTDOWN or
    // crash, or right after a heartbeat failure dropped the session).
    if handback_on_exit.load(Ordering::Relaxed) && (protected || store.read() != Marker::None) {
        tracing::info!("stop requested: handing protection back to SNetRBin");
        handback(scm, store, driver);
    }
    if handle.join().is_err() {
        tracing::error!("watcher supervisor thread panicked");
    }
    Ok(())
}

pub fn run(
    cfg: Config,
    stop: Arc<AtomicBool>,
    handback_on_exit: Arc<AtomicBool>,
    take_over_original: bool,
) -> Result<(), ServiceError> {
    let mut driver = WinDriver::new(&cfg);
    let s2 = stop.clone();
    let sleep = move |d: Duration| sleep_interruptible(d, &s2);
    if take_over_original {
        run_with(cfg, stop, &handback_on_exit, &WinScm, &FileStore::new(STATE_PATH), &mut driver, &sleep)
    } else {
        run_with(cfg, stop, &handback_on_exit, &NoScm, &NoStore, &mut driver, &sleep)
    }
}

/// SCM stand-in for `--console` without `--takeover`: touches nothing.
pub struct NoScm;
impl Scm for NoScm {
    fn stop_original(&self) -> Result<(), String> {
        Ok(())
    }
    fn query_start_type(&self) -> Result<Option<u32>, String> {
        Ok(None)
    }
    fn disable_original(&self) -> Result<(), String> {
        Ok(())
    }
    fn restore_start(&self, _start_type: Option<u32>) -> Result<(), String> {
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
            Some(p) => p.reassert(&self.log_dir).map_err(|e| e.to_string()),
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

    fn change_start(access: u32, start: Option<u32>, then_start: bool) -> Result<(), String> {
        use windows::Win32::Foundation::ERROR_SERVICE_ALREADY_RUNNING;
        use windows::Win32::System::Services::*;
        unsafe {
            let Some((scm, svc)) = Self::open(access)? else {
                return Ok(());
            };
            let mut result = ChangeServiceConfigW(
                svc,
                ENUM_SERVICE_TYPE(SERVICE_NO_CHANGE),
                SERVICE_START_TYPE(start.unwrap_or(SERVICE_NO_CHANGE)),
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
            let mut ctl_err = None;
            match ControlService(svc, SERVICE_CONTROL_STOP, &mut status) {
                Ok(()) => {}
                Err(e) if e.code() == ERROR_SERVICE_NOT_ACTIVE.to_hresult() => {
                    tracing::info!("SNetRBin already stopped");
                }
                Err(e) => ctl_err = Some(format!("stop request: {e}")),
            }
            let mut stopped = false;
            for _ in 0..60 {
                if QueryServiceStatus(svc, &mut status).is_ok() && status.dwCurrentState == SERVICE_STOPPED {
                    stopped = true;
                    break;
                }
                std::thread::sleep(Duration::from_millis(500));
            }
            let _ = CloseServiceHandle(svc);
            let _ = CloseServiceHandle(scm);
            if stopped {
                // Reaching STOPPED is what matters, even if the request errored.
                Ok(())
            } else {
                Err(ctl_err.unwrap_or_else(|| "timed out waiting for SNetRBin to stop".into()))
            }
        }
    }

    fn query_start_type(&self) -> Result<Option<u32>, String> {
        use windows::Win32::System::Services::*;
        unsafe {
            let Some((scm, svc)) = Self::open(SERVICE_QUERY_CONFIG)? else {
                return Ok(None);
            };
            let mut needed = 0u32;
            let _ = QueryServiceConfigW(svc, None, 0, &mut needed); // expected: insufficient buffer
            let mut buf = vec![0u64; (needed as usize).div_ceil(8).max(1)];
            let res = QueryServiceConfigW(
                svc,
                Some(buf.as_mut_ptr() as *mut QUERY_SERVICE_CONFIGW),
                (buf.len() * 8) as u32,
                &mut needed,
            );
            let out = match res {
                Ok(()) => Ok(Some((*(buf.as_ptr() as *const QUERY_SERVICE_CONFIGW)).dwStartType.0)),
                Err(e) => Err(format!("query config: {e}")),
            };
            let _ = CloseServiceHandle(svc);
            let _ = CloseServiceHandle(scm);
            out
        }
    }

    fn disable_original(&self) -> Result<(), String> {
        use windows::Win32::System::Services::*;
        Self::change_start(SERVICE_CHANGE_CONFIG, Some(SERVICE_DISABLED.0), false)
    }

    fn restore_start(&self, start_type: Option<u32>) -> Result<(), String> {
        use windows::Win32::System::Services::*;
        Self::change_start(SERVICE_CHANGE_CONFIG | SERVICE_START, start_type, true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::{Cell, RefCell};

    /// One fake world implementing Scm + StateStore, logging every call in order.
    struct World {
        calls: RefCell<Vec<&'static str>>,
        marker: RefCell<Marker>,
        start_type: Cell<u32>,
        restored_with: Cell<Option<Option<u32>>>,
        stop_err: Cell<bool>,
    }
    impl World {
        fn new() -> Self {
            World {
                calls: RefCell::new(vec![]),
                marker: RefCell::new(Marker::None),
                start_type: Cell::new(2),
                restored_with: Cell::new(None),
                stop_err: Cell::new(false),
            }
        }
        fn log(&self, c: &'static str) {
            self.calls.borrow_mut().push(c);
        }
        fn calls(&self) -> Vec<&'static str> {
            self.calls.borrow().clone()
        }
    }
    impl Scm for World {
        fn stop_original(&self) -> Result<(), String> {
            self.log("stop");
            if self.stop_err.get() { Err("access denied".into()) } else { Ok(()) }
        }
        fn query_start_type(&self) -> Result<Option<u32>, String> {
            self.log("query");
            Ok(Some(self.start_type.get()))
        }
        fn disable_original(&self) -> Result<(), String> {
            self.log("disable");
            self.start_type.set(START_DISABLED);
            Ok(())
        }
        fn restore_start(&self, start_type: Option<u32>) -> Result<(), String> {
            self.log("restore");
            self.restored_with.set(Some(start_type));
            Ok(())
        }
    }
    impl StateStore for World {
        fn read(&self) -> Marker {
            *self.marker.borrow()
        }
        fn write(&self, start_type: u32) -> Result<(), String> {
            self.log("write_state");
            *self.marker.borrow_mut() = Marker::Saved(start_type);
            Ok(())
        }
        fn clear(&self) {
            self.log("clear_state");
            *self.marker.borrow_mut() = Marker::None;
        }
    }

    struct FakeDriver<'a> {
        w: &'a World,
        fail_first: u32,
        fail_after: u32,
        connects: u32,
        hb_err: bool,
        stop_on_hb_err: Option<Arc<AtomicBool>>,
    }
    impl<'a> FakeDriver<'a> {
        fn new(w: &'a World, fail_first: u32) -> Self {
            FakeDriver { w, fail_first, fail_after: u32::MAX, connects: 0, hb_err: false, stop_on_hb_err: None }
        }
    }
    impl Driver for FakeDriver<'_> {
        fn connect_and_configure(&mut self) -> Result<(), String> {
            self.connects += 1;
            self.w.log("configure");
            if self.connects <= self.fail_first || self.connects > self.fail_after {
                Err("down".into())
            } else {
                Ok(())
            }
        }
        fn heartbeat(&mut self) -> Result<(), String> {
            if self.hb_err {
                if let Some(s) = &self.stop_on_hb_err {
                    s.store(true, Ordering::Relaxed);
                }
                Err("gone".into())
            } else {
                Ok(())
            }
        }
        fn disconnect(&mut self) {
            self.w.log("disconnect");
        }
    }

    fn tmp_cfg(dir: &std::path::Path, db_parent_is_file: bool) -> Config {
        let db = if db_parent_is_file {
            let blocker = dir.join("blocker");
            std::fs::write(&blocker, b"x").unwrap();
            blocker.join("sub").join("db.sqlite")
        } else {
            dir.join("db.sqlite")
        };
        Config {
            recycle_dir: dir.join("recycle"),
            db_path: db,
            includes: vec!["*".into()],
            excludes: vec![],
            max_bin_bytes: u64::MAX,
            max_age_days: None,
        }
    }

    fn stop_on_tick(stop: &Arc<AtomicBool>) -> impl Fn(Duration) {
        let s = stop.clone();
        move |d: Duration| {
            if d == TICK {
                s.store(true, Ordering::Relaxed);
            }
        }
    }

    #[test]
    fn retention_tick_never_over_max_frees_nothing() {
        let idx = crate::index::Index::open_memory().unwrap();
        assert_eq!(retention_tick(&idx, u64::MAX, None).unwrap(), 0);
    }

    #[test]
    fn backoff_doubles_and_caps() {
        assert_eq!(backoff_delay(0), Duration::from_secs(1));
        assert_eq!(backoff_delay(1), Duration::from_secs(2));
        assert_eq!(backoff_delay(5), Duration::from_secs(32));
        assert_eq!(backoff_delay(6), Duration::from_secs(60));
        assert_eq!(backoff_delay(1000), Duration::from_secs(60));
    }

    #[test]
    fn retry_wait_escalates_after_three_failed_cycles() {
        assert_eq!(retry_wait(1), RETRY_WAIT);
        assert_eq!(retry_wait(2), RETRY_WAIT);
        assert_eq!(retry_wait(3), LONG_RETRY_WAIT);
        assert_eq!(retry_wait(10), LONG_RETRY_WAIT);
        assert_eq!(LONG_RETRY_WAIT, Duration::from_secs(6 * 3600));
    }

    #[test]
    fn marker_encode_decode_roundtrip() {
        assert_eq!(decode_marker(&encode_marker(2)), Some(2));
        assert_eq!(decode_marker(&encode_marker(3)), Some(3));
        assert_eq!(decode_marker("garbage"), None);
        assert_eq!(decode_marker("{ \"original_start_type\": }"), None);
    }

    #[test]
    fn file_store_roundtrip_missing_and_corrupt() {
        let dir = tempfile::tempdir().unwrap();
        let st = FileStore::new(dir.path().join("takeover.json"));
        assert_eq!(st.read(), Marker::None);
        st.write(3).unwrap();
        assert_eq!(st.read(), Marker::Saved(3));
        assert!(!dir.path().join("takeover.json.tmp").exists());
        std::fs::write(dir.path().join("takeover.json"), "{{{").unwrap();
        assert_eq!(st.read(), Marker::Corrupt);
        st.clear();
        assert_eq!(st.read(), Marker::None);
    }

    #[test]
    fn takeover_writes_state_before_disable_and_disables_after_configure() {
        let w = World::new();
        let mut drv = FakeDriver::new(&w, 2);
        let stop = AtomicBool::new(false);
        assert!(takeover(&w, &w, &mut drv, &stop, &mut 0, &|_| {}));
        assert_eq!(
            w.calls(),
            vec!["stop", "configure", "configure", "configure", "query", "write_state", "disable"]
        );
        assert_eq!(w.read(), Marker::Saved(2));
    }

    #[test]
    fn takeover_does_not_record_or_disable_when_admin_already_disabled() {
        let w = World::new();
        w.start_type.set(START_DISABLED);
        let mut drv = FakeDriver::new(&w, 0);
        let stop = AtomicBool::new(false);
        assert!(takeover(&w, &w, &mut drv, &stop, &mut 0, &|_| {}));
        assert!(!w.calls().contains(&"write_state"));
        assert_eq!(w.read(), Marker::None);
    }

    #[test]
    fn takeover_failure_restores_never_disables_and_never_leaves_both_down() {
        let w = World::new();
        let mut drv = FakeDriver::new(&w, u32::MAX);
        let stop = AtomicBool::new(false);
        let sleeps = RefCell::new(Vec::new());
        let sleep = |d: Duration| {
            sleeps.borrow_mut().push(d);
            if d == RETRY_WAIT {
                stop.store(true, Ordering::Relaxed);
            }
        };
        assert!(!takeover(&w, &w, &mut drv, &stop, &mut 0, &sleep));
        let calls = w.calls();
        assert!(!calls.contains(&"disable"));
        assert_eq!(calls.first(), Some(&"stop"));
        assert_eq!(calls.last(), Some(&"restore"));
        assert!(sleeps.borrow().contains(&RETRY_WAIT));
        let backoff_total: Duration = sleeps.borrow().iter().filter(|d| **d != RETRY_WAIT).sum();
        assert!(backoff_total <= CONNECT_BUDGET);
        // never disabled -> nothing recorded, restore keeps the start type
        assert_eq!(w.restored_with.get(), Some(None));
    }

    #[test]
    fn takeover_after_three_failed_cycles_waits_six_hours() {
        let w = World::new();
        let mut drv = FakeDriver::new(&w, u32::MAX);
        let stop = AtomicBool::new(false);
        let waits = RefCell::new(Vec::new());
        let sleep = |d: Duration| {
            if d == RETRY_WAIT || d == LONG_RETRY_WAIT {
                waits.borrow_mut().push(d);
                if waits.borrow().len() == 5 {
                    stop.store(true, Ordering::Relaxed);
                }
            }
        };
        assert!(!takeover(&w, &w, &mut drv, &stop, &mut 0, &sleep));
        assert_eq!(
            *waits.borrow(),
            vec![RETRY_WAIT, RETRY_WAIT, LONG_RETRY_WAIT, LONG_RETRY_WAIT, LONG_RETRY_WAIT]
        );
    }

    #[test]
    fn takeover_stop_requested_midway_restores_original() {
        let w = World::new();
        let mut drv = FakeDriver::new(&w, u32::MAX);
        let stop = AtomicBool::new(false);
        let sleep = |_d: Duration| stop.store(true, Ordering::Relaxed);
        assert!(!takeover(&w, &w, &mut drv, &stop, &mut 0, &sleep));
        let calls = w.calls();
        assert_eq!(calls.last(), Some(&"restore"));
        assert!(!calls.contains(&"disable"));
    }

    #[test]
    fn takeover_not_started_when_stop_already_set() {
        let w = World::new();
        let mut drv = FakeDriver::new(&w, 0);
        let stop = AtomicBool::new(true);
        assert!(!takeover(&w, &w, &mut drv, &stop, &mut 0, &|_| {}));
        assert!(w.calls().is_empty());
    }

    #[test]
    fn takeover_proceeds_when_stop_original_fails() {
        let w = World::new();
        w.stop_err.set(true);
        let mut drv = FakeDriver::new(&w, 0);
        let stop = AtomicBool::new(false);
        assert!(takeover(&w, &w, &mut drv, &stop, &mut 0, &|_| {}));
        assert_eq!(w.calls()[..2], ["stop", "configure"]);
        assert!(w.calls().contains(&"disable"));
    }

    #[test]
    fn restore_uses_saved_start_type_then_clears_marker() {
        let w = World::new();
        *w.marker.borrow_mut() = Marker::Saved(3);
        restore(&w, &w);
        assert_eq!(w.restored_with.get(), Some(Some(3)));
        assert_eq!(w.read(), Marker::None);
        assert_eq!(w.calls(), vec!["restore", "clear_state"]);
    }

    #[test]
    fn restore_with_corrupt_marker_falls_back_to_auto_start() {
        let w = World::new();
        *w.marker.borrow_mut() = Marker::Corrupt;
        restore(&w, &w);
        assert_eq!(w.restored_with.get(), Some(Some(START_AUTO)));
        assert_eq!(w.read(), Marker::None);
    }

    #[test]
    fn heartbeat_error_drops_session_and_takeover_reenters() {
        let w = World::new();
        let mut drv = FakeDriver::new(&w, 0);
        let stop = AtomicBool::new(false);
        assert!(takeover(&w, &w, &mut drv, &stop, &mut 0, &|_| {}));
        assert!(heartbeat(&mut drv));
        drv.hb_err = true;
        assert!(!heartbeat(&mut drv));
        assert_eq!(w.calls().iter().filter(|c| **c == "disconnect").count(), 1);
        drv.hb_err = false;
        assert!(takeover(&w, &w, &mut drv, &stop, &mut 0, &|_| {}));
        assert_eq!(drv.connects, 2);
    }

    #[test]
    fn prepare_failure_without_marker_never_touches_scm() {
        let dir = tempfile::tempdir().unwrap();
        let cfg = tmp_cfg(dir.path(), true);
        let w = World::new();
        let mut drv = FakeDriver::new(&w, 0);
        let r = run_with(cfg, Arc::new(AtomicBool::new(false)), &AtomicBool::new(true), &w, &w, &mut drv, &|_| {});
        assert!(r.is_err());
        assert!(w.calls().is_empty());
        assert_eq!(drv.connects, 0);
    }

    #[test]
    fn populated_unmarked_recycle_dir_refuses_start_before_scm() {
        let dir = tempfile::tempdir().unwrap();
        let cfg = tmp_cfg(dir.path(), false);
        std::fs::create_dir_all(&cfg.recycle_dir).unwrap();
        std::fs::write(cfg.recycle_dir.join("user-data.xlsx"), b"x").unwrap();
        let w = World::new();
        let mut drv = FakeDriver::new(&w, 0);
        let stop = Arc::new(AtomicBool::new(false));
        let s2 = stop.clone();
        // If prepare wrongly succeeds, stop on the first sleep so the test fails instead of hanging.
        let sleep = move |_d: Duration| s2.store(true, Ordering::Relaxed);
        let r = run_with(cfg, stop, &AtomicBool::new(true), &w, &w, &mut drv, &sleep);
        assert!(r.is_err());
        assert!(w.calls().is_empty(), "SCM touched: {:?}", w.calls());
        assert_eq!(drv.connects, 0);
    }

    #[test]
    fn prepare_failure_with_marker_restores_original() {
        let dir = tempfile::tempdir().unwrap();
        let cfg = tmp_cfg(dir.path(), true);
        let w = World::new();
        *w.marker.borrow_mut() = Marker::Saved(2);
        let mut drv = FakeDriver::new(&w, 0);
        let r = run_with(cfg, Arc::new(AtomicBool::new(false)), &AtomicBool::new(true), &w, &w, &mut drv, &|_| {});
        assert!(r.is_err());
        assert_eq!(w.calls(), vec!["restore", "clear_state"]);
        assert_eq!(w.restored_with.get(), Some(Some(2)));
    }

    #[test]
    fn early_exit_restore_without_marker_touches_nothing() {
        let w = World::new();
        early_exit_restore(&w, &w);
        assert!(w.calls().is_empty());
    }

    #[test]
    fn run_with_stop_hands_back_disconnect_before_restore() {
        let dir = tempfile::tempdir().unwrap();
        let cfg = tmp_cfg(dir.path(), false);
        let w = World::new();
        let mut drv = FakeDriver::new(&w, 0);
        let stop = Arc::new(AtomicBool::new(false));
        let sleep = stop_on_tick(&stop);
        run_with(cfg, stop, &AtomicBool::new(true), &w, &w, &mut drv, &sleep).unwrap();
        let calls = w.calls();
        let n = calls.len();
        assert_eq!(calls[n - 3..], ["disconnect", "restore", "clear_state"]);
        assert_eq!(w.restored_with.get(), Some(Some(2)));
    }

    #[test]
    fn run_with_shutdown_keeps_takeover_persisted() {
        let dir = tempfile::tempdir().unwrap();
        let cfg = tmp_cfg(dir.path(), false);
        let w = World::new();
        let mut drv = FakeDriver::new(&w, 0);
        let stop = Arc::new(AtomicBool::new(false));
        let sleep = stop_on_tick(&stop);
        run_with(cfg, stop, &AtomicBool::new(false), &w, &w, &mut drv, &sleep).unwrap();
        assert!(!w.calls().contains(&"restore"));
        assert!(w.calls().contains(&"disable"));
        assert_eq!(w.read(), Marker::Saved(2));
    }

    #[test]
    fn run_with_heartbeat_fail_then_budget_exhausted_restores_saved_type() {
        let dir = tempfile::tempdir().unwrap();
        let cfg = tmp_cfg(dir.path(), false);
        let w = World::new();
        let mut drv = FakeDriver::new(&w, 0);
        drv.fail_after = 1; // first connect ok, every later one fails
        drv.hb_err = true;
        let stop = Arc::new(AtomicBool::new(false));
        let s2 = stop.clone();
        let sleep = move |d: Duration| {
            if d == RETRY_WAIT {
                s2.store(true, Ordering::Relaxed);
            }
        };
        run_with(cfg, stop, &AtomicBool::new(false), &w, &w, &mut drv, &sleep).unwrap();
        let calls = w.calls();
        assert!(calls.contains(&"disable"));
        let d = calls.iter().position(|c| *c == "disable").unwrap();
        assert!(calls[d..].contains(&"restore"));
        assert_eq!(w.restored_with.get(), Some(Some(2)));
        assert_eq!(w.read(), Marker::None);
    }
}

#[cfg(test)]
mod tests_round3 {
    use super::*;
    use std::cell::{Cell, RefCell};

    // Minimal local fakes (independent of the main test module).
    struct W {
        calls: RefCell<Vec<&'static str>>,
        marker: RefCell<Marker>,
        connects: Cell<u32>,
        stop_on_hb: Option<Arc<AtomicBool>>,
    }
    impl W {
        fn new(marker: Marker) -> Self {
            W { calls: RefCell::new(vec![]), marker: RefCell::new(marker), connects: Cell::new(0), stop_on_hb: None }
        }
        fn log(&self, c: &'static str) {
            self.calls.borrow_mut().push(c);
        }
    }
    impl Scm for W {
        fn stop_original(&self) -> Result<(), String> { self.log("stop"); Ok(()) }
        fn query_start_type(&self) -> Result<Option<u32>, String> { Ok(Some(2)) }
        fn disable_original(&self) -> Result<(), String> { self.log("disable"); Ok(()) }
        fn restore_start(&self, _t: Option<u32>) -> Result<(), String> { self.log("restore"); Ok(()) }
    }
    impl StateStore for W {
        fn read(&self) -> Marker { *self.marker.borrow() }
        fn write(&self, t: u32) -> Result<(), String> { *self.marker.borrow_mut() = Marker::Saved(t); Ok(()) }
        fn clear(&self) { self.log("clear_state"); *self.marker.borrow_mut() = Marker::None; }
    }
    struct D<'a> { w: &'a W, ok_connects: u32 }
    impl Driver for D<'_> {
        fn connect_and_configure(&mut self) -> Result<(), String> {
            self.w.connects.set(self.w.connects.get() + 1);
            if self.w.connects.get() <= self.ok_connects { Ok(()) } else { Err("down".into()) }
        }
        fn heartbeat(&mut self) -> Result<(), String> {
            if let Some(s) = &self.w.stop_on_hb { s.store(true, Ordering::Relaxed); }
            Err("gone".into())
        }
        fn disconnect(&mut self) { self.w.log("disconnect"); }
    }
    fn cfg(dir: &std::path::Path) -> Config {
        Config {
            recycle_dir: dir.join("recycle"),
            db_path: dir.join("db.sqlite"),
            includes: vec!["*".into()],
            excludes: vec![],
            max_bin_bytes: u64::MAX,
            max_age_days: None,
        }
    }

    #[test]
    fn stop_before_loop_with_marker_still_hands_back() {
        let dir = tempfile::tempdir().unwrap();
        let w = W::new(Marker::Saved(2));
        let mut d = D { w: &w, ok_connects: 0 };
        let stop = Arc::new(AtomicBool::new(true));
        run_with(cfg(dir.path()), stop, &AtomicBool::new(true), &w, &w, &mut d, &|_| {}).unwrap();
        assert_eq!(*w.calls.borrow(), vec!["disconnect", "restore", "clear_state"]);
    }

    #[test]
    fn stop_before_loop_without_marker_or_handback_touches_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let w = W::new(Marker::None);
        let mut d = D { w: &w, ok_connects: 0 };
        let stop = Arc::new(AtomicBool::new(true));
        run_with(cfg(dir.path()), stop.clone(), &AtomicBool::new(true), &w, &w, &mut d, &|_| {}).unwrap();
        assert!(w.calls.borrow().is_empty(), "no marker: SCM must not be touched");
        let w2 = W::new(Marker::Saved(2));
        let mut d2 = D { w: &w2, ok_connects: 0 };
        run_with(cfg(dir.path()), stop, &AtomicBool::new(false), &w2, &w2, &mut d2, &|_| {}).unwrap();
        assert!(w2.calls.borrow().is_empty(), "SHUTDOWN must not restore");
    }

    #[test]
    fn stop_after_heartbeat_failure_before_retakeover_restores() {
        let dir = tempfile::tempdir().unwrap();
        let stop = Arc::new(AtomicBool::new(false));
        let mut w = W::new(Marker::None);
        w.stop_on_hb = Some(stop.clone());
        let mut d = D { w: &w, ok_connects: 1 };
        run_with(cfg(dir.path()), stop, &AtomicBool::new(true), &w, &w, &mut d, &|_| {}).unwrap();
        let calls = w.calls.borrow();
        let n = calls.len();
        assert_eq!(calls[n - 3..], ["disconnect", "restore", "clear_state"]);
        assert_eq!(w.connects.get(), 1, "no re-takeover attempted after STOP");
    }

    #[test]
    fn takeover_success_does_not_reset_failure_counter() {
        let w = W::new(Marker::None);
        let mut d = D { w: &w, ok_connects: 1 };
        let mut failed = 2u32;
        assert!(takeover(&w, &w, &mut d, &AtomicBool::new(false), &mut failed, &|_| {}));
        assert_eq!(failed, 2);
    }

    #[test]
    fn carried_failure_counter_escalates_first_exhausted_cycle() {
        let w = W::new(Marker::None);
        let mut d = D { w: &w, ok_connects: 0 };
        let stop = AtomicBool::new(false);
        let waits = RefCell::new(vec![]);
        let mut failed = 2u32;
        let sleep = |dur: Duration| {
            if dur >= RETRY_WAIT {
                waits.borrow_mut().push(dur);
                stop.store(true, Ordering::Relaxed);
            }
        };
        assert!(!takeover(&w, &w, &mut d, &stop, &mut failed, &sleep));
        assert_eq!(*waits.borrow(), vec![LONG_RETRY_WAIT]);
        assert_eq!(failed, 3);
    }
}
