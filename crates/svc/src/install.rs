//! Standalone install: ship our own copy of `netrbin.sys`, register it as a
//! minifilter exactly as the original installer does, and register
//! `rnetrbin-svc`. The original product (SNetRBin) is not required; when it is
//! present, uninstall leaves the shared driver in place for it.
use std::path::{Path, PathBuf};
use std::time::Duration;

pub const DRIVER_SERVICE: &str = "netrbin";
pub const DRIVER_FILE: &str = "netrbin.sys";
/// Registry values written by the original installer (v6.1.1.3).
pub const DRIVER_DISPLAY_NAME: &str = "Network Recycle Bin Mini-Filter";
pub const DRIVER_GROUP: &str = "Filter";
pub const DRIVER_IMAGE_PATH: &str = r"system32\Drivers\netrbin.sys";
pub const INSTANCE_NAME: &str = "netrbin Instance";
pub const ALTITUDE: &str = "370150";
pub const INSTANCE_FLAGS: u32 = 0;

pub const OWN_SERVICE: &str = "rnetrbin-svc";
pub const OWN_DISPLAY_NAME: &str = "rnetrbin network recycle bin";
pub const ORIGINAL_SERVICE: &str = "SNetRBin";

/// Must exceed the service's worst-case handback on STOP (see main.rs STOP_WAIT_HINT).
const OWN_STOP_TIMEOUT: Duration = Duration::from_secs(120);
const DRIVER_STOP_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(thiserror::Error, Debug)]
pub enum InstallError {
    #[error("driver source {0}: {1}")]
    Source(PathBuf, String),
    #[error("{step}: {msg}")]
    Step { step: &'static str, msg: String },
}

fn step(step: &'static str) -> impl FnOnce(String) -> InstallError {
    move |msg| InstallError::Step { step, msg }
}

/// System operations used by install/uninstall. Every method is idempotent.
pub trait Host {
    /// Copy `src` over `%SystemRoot%\System32\drivers\netrbin.sys` unless identical.
    fn copy_driver(&self, src: &Path) -> Result<(), String>;
    /// Create the minifilter service and its default instance (altitude) keys.
    fn ensure_driver_service(&self) -> Result<(), String>;
    /// Load the minifilter (already running is Ok).
    fn start_driver(&self) -> Result<(), String>;
    /// Create or update `rnetrbin-svc` (auto start) pointing at `exe`.
    fn ensure_own_service(&self, exe: &Path) -> Result<(), String>;
    /// Stop (waiting for the handback) and delete `rnetrbin-svc`; absent is Ok.
    fn remove_own_service(&self) -> Result<(), String>;
    /// Whether the original SNetRBin service is installed.
    fn original_present(&self) -> Result<bool, String>;
    /// Unload the minifilter; not running is Ok.
    fn stop_driver(&self) -> Result<(), String>;
    fn delete_driver_service(&self) -> Result<(), String>;
    /// Ok(true) if removed now, Ok(false) if scheduled for deletion at reboot.
    fn remove_driver_file(&self) -> Result<bool, String>;
}

/// Reject anything that is obviously not a driver image before touching the system.
pub fn validate_driver_source(src: &Path) -> Result<(), InstallError> {
    let bad = |why: &str| InstallError::Source(src.to_path_buf(), why.into());
    let is_sys = src.extension().is_some_and(|e| e.eq_ignore_ascii_case("sys"));
    if !is_sys {
        return Err(bad("expected a .sys file"));
    }
    let bytes = std::fs::read(src).map_err(|e| bad(&e.to_string()))?;
    if !bytes.starts_with(b"MZ") {
        return Err(bad("not a PE image (missing MZ header)"));
    }
    Ok(())
}

/// Install order: driver file, driver service, load it, then our service. The
/// service is not started here; `sc start rnetrbin-svc` is left to the operator.
pub fn install<H: Host>(host: &H, driver_src: &Path, exe: &Path) -> Result<(), InstallError> {
    validate_driver_source(driver_src)?;
    host.copy_driver(driver_src).map_err(step("copy driver"))?;
    host.ensure_driver_service().map_err(step("register driver"))?;
    host.start_driver().map_err(step("load driver"))?;
    host.ensure_own_service(exe).map_err(step("register service"))?;
    Ok(())
}

#[derive(Debug, PartialEq, Eq)]
pub enum Uninstalled {
    /// Driver left in place (asked to, or SNetRBin still uses it).
    DriverKept,
    DriverRemoved,
    /// Driver could not be unloaded or its file deleted now; gone after reboot.
    RebootRequired,
}

/// Our service goes first so its STOP handback runs while the driver is still
/// loaded. The driver is removed only when nothing else (SNetRBin) needs it.
pub fn uninstall<H: Host>(host: &H, keep_driver: bool) -> Result<Uninstalled, InstallError> {
    host.remove_own_service().map_err(step("remove service"))?;
    if keep_driver {
        return Ok(Uninstalled::DriverKept);
    }
    if host.original_present().map_err(step("query SNetRBin"))? {
        tracing::info!("SNetRBin is installed; keeping the shared netrbin driver");
        return Ok(Uninstalled::DriverKept);
    }
    let unloaded = match host.stop_driver() {
        Ok(()) => true,
        Err(e) => {
            tracing::warn!("driver unload failed, removal completes at reboot: {e}");
            false
        }
    };
    host.delete_driver_service().map_err(step("delete driver service"))?;
    let removed_now = host.remove_driver_file().map_err(step("remove driver file"))?;
    Ok(if unloaded && removed_now { Uninstalled::DriverRemoved } else { Uninstalled::RebootRequired })
}

pub fn drivers_dir() -> PathBuf {
    let root = std::env::var_os("SystemRoot").unwrap_or_else(|| r"C:\Windows".into());
    PathBuf::from(root).join("System32").join("drivers")
}

/// Real Win32 implementation (requires an elevated process).
pub struct WinHost;

mod win {
    use std::time::{Duration, Instant};
    use widestring::U16CString;
    use windows::core::PCWSTR;
    use windows::Win32::Foundation::*;
    use windows::Win32::System::Services::*;

    pub fn wide(s: &str) -> U16CString {
        U16CString::from_str(s).expect("no interior NUL")
    }

    /// Closes the SCM and service handles on drop.
    pub struct Svc {
        pub scm: SC_HANDLE,
        pub svc: SC_HANDLE,
    }
    impl Drop for Svc {
        fn drop(&mut self) {
            unsafe {
                let _ = CloseServiceHandle(self.svc);
                let _ = CloseServiceHandle(self.scm);
            }
        }
    }

    pub fn open_scm(access: u32) -> Result<SC_HANDLE, String> {
        unsafe { OpenSCManagerW(None, None, access) }.map_err(|e| format!("open SCM: {e} (run as Administrator)"))
    }

    /// Ok(None) if the service does not exist.
    pub fn open(name: &str, access: u32) -> Result<Option<Svc>, String> {
        let scm = open_scm(SC_MANAGER_CONNECT)?;
        let n = wide(name);
        match unsafe { OpenServiceW(scm, PCWSTR(n.as_ptr()), access) } {
            Ok(svc) => Ok(Some(Svc { scm, svc })),
            Err(e) => {
                unsafe {
                    let _ = CloseServiceHandle(scm);
                }
                if e.code() == ERROR_SERVICE_DOES_NOT_EXIST.to_hresult() {
                    Ok(None)
                } else {
                    Err(format!("open {name}: {e}"))
                }
            }
        }
    }

    pub fn wait_stopped(svc: SC_HANDLE, timeout: Duration) -> bool {
        let deadline = Instant::now() + timeout;
        let mut st = SERVICE_STATUS::default();
        loop {
            if unsafe { QueryServiceStatus(svc, &mut st) }.is_ok() && st.dwCurrentState == SERVICE_STOPPED {
                return true;
            }
            if Instant::now() >= deadline {
                return false;
            }
            std::thread::sleep(Duration::from_millis(500));
        }
    }

    /// Request STOP and wait; a service that is not running counts as stopped.
    pub fn stop(name: &str, timeout: Duration) -> Result<(), String> {
        let Some(s) = open(name, SERVICE_STOP | SERVICE_QUERY_STATUS)? else {
            return Ok(());
        };
        let mut st = SERVICE_STATUS::default();
        match unsafe { ControlService(s.svc, SERVICE_CONTROL_STOP, &mut st) } {
            Ok(()) => {}
            Err(e) if e.code() == ERROR_SERVICE_NOT_ACTIVE.to_hresult() => return Ok(()),
            Err(e) => return Err(format!("stop {name}: {e}")),
        }
        if wait_stopped(s.svc, timeout) {
            Ok(())
        } else {
            Err(format!("timed out waiting for {name} to stop"))
        }
    }

    pub fn delete(name: &str) -> Result<(), String> {
        let Some(s) = open(name, 0x0001_0000 /* DELETE */)? else {
            return Ok(());
        };
        match unsafe { DeleteService(s.svc) } {
            Ok(()) => Ok(()),
            Err(e) if e.code() == ERROR_SERVICE_MARKED_FOR_DELETE.to_hresult() => Ok(()),
            Err(e) => Err(format!("delete {name}: {e}")),
        }
    }
}

impl WinHost {
    fn set_instance_keys() -> Result<(), String> {
        use windows::core::PCWSTR;
        use windows::Win32::System::Registry::*;
        let set = |sub: &str, vals: &[(&str, REG_VALUE_TYPE, Vec<u8>)]| -> Result<(), String> {
            let path = win::wide(&format!(r"SYSTEM\CurrentControlSet\Services\{DRIVER_SERVICE}\{sub}"));
            let mut key = HKEY::default();
            let rc = unsafe {
                RegCreateKeyExW(
                    HKEY_LOCAL_MACHINE,
                    PCWSTR(path.as_ptr()),
                    0,
                    None,
                    REG_OPTION_NON_VOLATILE,
                    KEY_SET_VALUE,
                    None,
                    &mut key,
                    None,
                )
            };
            if rc.is_err() {
                return Err(format!("create key {sub}: {rc:?}"));
            }
            let mut out = Ok(());
            for (name, ty, data) in vals {
                let n = win::wide(name);
                let rc = unsafe { RegSetValueExW(key, PCWSTR(n.as_ptr()), 0, *ty, Some(data)) };
                if rc.is_err() {
                    out = Err(format!("set {sub}\\{name}: {rc:?}"));
                    break;
                }
            }
            unsafe {
                let _ = RegCloseKey(key);
            }
            out
        };
        let sz = |s: &str| -> Vec<u8> { win::wide(s).as_slice_with_nul().iter().flat_map(|u| u.to_le_bytes()).collect() };
        set("Instances", &[("DefaultInstance", REG_SZ, sz(INSTANCE_NAME))])?;
        set(
            &format!(r"Instances\{INSTANCE_NAME}"),
            &[("Altitude", REG_SZ, sz(ALTITUDE)), ("Flags", REG_DWORD, INSTANCE_FLAGS.to_le_bytes().to_vec())],
        )
    }
}

impl Host for WinHost {
    fn copy_driver(&self, src: &Path) -> Result<(), String> {
        let dst = drivers_dir().join(DRIVER_FILE);
        let new = std::fs::read(src).map_err(|e| format!("read {}: {e}", src.display()))?;
        if std::fs::read(&dst).is_ok_and(|cur| cur == new) {
            tracing::info!("{} already up to date", dst.display());
            return Ok(());
        }
        std::fs::write(&dst, &new).map_err(|e| {
            format!("write {}: {e} (if the driver is loaded, run uninstall or unload it first)", dst.display())
        })
    }

    fn ensure_driver_service(&self) -> Result<(), String> {
        use windows::core::PCWSTR;
        use windows::Win32::System::Services::*;
        if win::open(DRIVER_SERVICE, SERVICE_QUERY_STATUS)?.is_none() {
            let scm = win::open_scm(SC_MANAGER_CREATE_SERVICE)?;
            let (name, disp, path, group) = (
                win::wide(DRIVER_SERVICE),
                win::wide(DRIVER_DISPLAY_NAME),
                win::wide(DRIVER_IMAGE_PATH),
                win::wide(DRIVER_GROUP),
            );
            let res = unsafe {
                CreateServiceW(
                    scm,
                    PCWSTR(name.as_ptr()),
                    PCWSTR(disp.as_ptr()),
                    SERVICE_ALL_ACCESS,
                    SERVICE_FILE_SYSTEM_DRIVER,
                    SERVICE_BOOT_START,
                    SERVICE_ERROR_IGNORE,
                    PCWSTR(path.as_ptr()),
                    PCWSTR(group.as_ptr()),
                    None,
                    None,
                    None,
                    None,
                )
            };
            unsafe {
                if let Ok(h) = &res {
                    let _ = CloseServiceHandle(*h);
                }
                let _ = CloseServiceHandle(scm);
            }
            res.map_err(|e| format!("create {DRIVER_SERVICE}: {e}"))?;
            tracing::info!("created minifilter service {DRIVER_SERVICE}");
        }
        Self::set_instance_keys()
    }

    fn start_driver(&self) -> Result<(), String> {
        use windows::Win32::Foundation::ERROR_SERVICE_ALREADY_RUNNING;
        use windows::Win32::System::Services::*;
        let s = win::open(DRIVER_SERVICE, SERVICE_START)?.ok_or("driver service missing")?;
        match unsafe { StartServiceW(s.svc, None) } {
            Ok(()) => Ok(()),
            Err(e) if e.code() == ERROR_SERVICE_ALREADY_RUNNING.to_hresult() => Ok(()),
            Err(e) => Err(format!("start {DRIVER_SERVICE}: {e}")),
        }
    }

    fn ensure_own_service(&self, exe: &Path) -> Result<(), String> {
        use windows::core::PCWSTR;
        use windows::Win32::System::Services::*;
        let bin = win::wide(&format!("\"{}\"", exe.display()));
        if let Some(s) = win::open(OWN_SERVICE, SERVICE_CHANGE_CONFIG)? {
            return unsafe {
                ChangeServiceConfigW(
                    s.svc,
                    SERVICE_WIN32_OWN_PROCESS,
                    SERVICE_AUTO_START,
                    SERVICE_ERROR_NORMAL,
                    PCWSTR(bin.as_ptr()),
                    None,
                    None,
                    None,
                    None,
                    None,
                    None,
                )
            }
            .map_err(|e| format!("update {OWN_SERVICE}: {e}"));
        }
        let scm = win::open_scm(SC_MANAGER_CREATE_SERVICE)?;
        let (name, disp) = (win::wide(OWN_SERVICE), win::wide(OWN_DISPLAY_NAME));
        // No dependency on the driver: if it fails to load, the service still
        // starts and its connect/retry loop keeps trying (and logging).
        let res = unsafe {
            CreateServiceW(
                scm,
                PCWSTR(name.as_ptr()),
                PCWSTR(disp.as_ptr()),
                SERVICE_ALL_ACCESS,
                SERVICE_WIN32_OWN_PROCESS,
                SERVICE_AUTO_START,
                SERVICE_ERROR_NORMAL,
                PCWSTR(bin.as_ptr()),
                None,
                None,
                None,
                None,
                None,
            )
        };
        unsafe {
            if let Ok(h) = &res {
                let _ = CloseServiceHandle(*h);
            }
            let _ = CloseServiceHandle(scm);
        }
        res.map(|_| ()).map_err(|e| format!("create {OWN_SERVICE}: {e}"))
    }

    fn remove_own_service(&self) -> Result<(), String> {
        win::stop(OWN_SERVICE, OWN_STOP_TIMEOUT)?;
        win::delete(OWN_SERVICE)
    }

    fn original_present(&self) -> Result<bool, String> {
        use windows::Win32::System::Services::SERVICE_QUERY_STATUS;
        Ok(win::open(ORIGINAL_SERVICE, SERVICE_QUERY_STATUS)?.is_some())
    }

    fn stop_driver(&self) -> Result<(), String> {
        win::stop(DRIVER_SERVICE, DRIVER_STOP_TIMEOUT)
    }

    fn delete_driver_service(&self) -> Result<(), String> {
        win::delete(DRIVER_SERVICE)
    }

    fn remove_driver_file(&self) -> Result<bool, String> {
        use windows::core::PCWSTR;
        use windows::Win32::Storage::FileSystem::{MoveFileExW, MOVEFILE_DELAY_UNTIL_REBOOT};
        let path = drivers_dir().join(DRIVER_FILE);
        match std::fs::remove_file(&path) {
            Ok(()) => Ok(true),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(true),
            Err(_) => {
                let p = win::wide(&path.to_string_lossy());
                unsafe { MoveFileExW(PCWSTR(p.as_ptr()), None, MOVEFILE_DELAY_UNTIL_REBOOT) }
                    .map_err(|e| format!("schedule delete of {}: {e}", path.display()))?;
                Ok(false)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    #[derive(Default)]
    struct Mock {
        log: RefCell<Vec<&'static str>>,
        fail: Option<&'static str>,
        original: bool,
        stop_fails: bool,
        file_busy: bool,
    }
    impl Mock {
        fn call(&self, name: &'static str) -> Result<(), String> {
            self.log.borrow_mut().push(name);
            if self.fail == Some(name) { Err("boom".into()) } else { Ok(()) }
        }
        fn log(&self) -> Vec<&'static str> {
            self.log.borrow().clone()
        }
    }
    impl Host for Mock {
        fn copy_driver(&self, _src: &Path) -> Result<(), String> { self.call("copy") }
        fn ensure_driver_service(&self) -> Result<(), String> { self.call("driver_svc") }
        fn start_driver(&self) -> Result<(), String> { self.call("start_driver") }
        fn ensure_own_service(&self, _exe: &Path) -> Result<(), String> { self.call("own_svc") }
        fn remove_own_service(&self) -> Result<(), String> { self.call("rm_own") }
        fn original_present(&self) -> Result<bool, String> { self.call("query_orig").map(|_| self.original) }
        fn stop_driver(&self) -> Result<(), String> {
            self.call("stop_driver")?;
            if self.stop_fails { Err("busy".into()) } else { Ok(()) }
        }
        fn delete_driver_service(&self) -> Result<(), String> { self.call("del_driver_svc") }
        fn remove_driver_file(&self) -> Result<bool, String> { self.call("rm_file").map(|_| !self.file_busy) }
    }

    fn driver_file(dir: &tempfile::TempDir, name: &str, bytes: &[u8]) -> PathBuf {
        let p = dir.path().join(name);
        std::fs::write(&p, bytes).unwrap();
        p
    }

    #[test]
    fn validate_rejects_non_sys_and_non_pe() {
        let d = tempfile::tempdir().unwrap();
        assert!(validate_driver_source(&driver_file(&d, "a.dll", b"MZ..")).is_err());
        assert!(validate_driver_source(&driver_file(&d, "a.sys", b"PK..")).is_err());
        assert!(validate_driver_source(&d.path().join("missing.sys")).is_err());
        assert!(validate_driver_source(&driver_file(&d, "netrbin.SYS", b"MZ\x90\x00")).is_ok());
    }

    #[test]
    fn install_runs_steps_in_order() {
        let d = tempfile::tempdir().unwrap();
        let m = Mock::default();
        install(&m, &driver_file(&d, "netrbin.sys", b"MZ"), Path::new(r"C:\x\rnetrbin-svc.exe")).unwrap();
        assert_eq!(m.log(), ["copy", "driver_svc", "start_driver", "own_svc"]);
    }

    #[test]
    fn install_bad_source_touches_nothing() {
        let d = tempfile::tempdir().unwrap();
        let m = Mock::default();
        assert!(install(&m, &driver_file(&d, "netrbin.sys", b"xx"), Path::new("x.exe")).is_err());
        assert!(m.log().is_empty());
    }

    #[test]
    fn install_stops_at_first_failure() {
        let d = tempfile::tempdir().unwrap();
        let m = Mock { fail: Some("start_driver"), ..Default::default() };
        let e = install(&m, &driver_file(&d, "netrbin.sys", b"MZ"), Path::new("x.exe")).unwrap_err();
        assert!(e.to_string().starts_with("load driver"));
        assert_eq!(m.log(), ["copy", "driver_svc", "start_driver"]);
    }

    #[test]
    fn uninstall_removes_service_then_driver() {
        let m = Mock::default();
        assert_eq!(uninstall(&m, false).unwrap(), Uninstalled::DriverRemoved);
        assert_eq!(m.log(), ["rm_own", "query_orig", "stop_driver", "del_driver_svc", "rm_file"]);
    }

    #[test]
    fn uninstall_keeps_driver_when_asked_or_original_present() {
        let m = Mock::default();
        assert_eq!(uninstall(&m, true).unwrap(), Uninstalled::DriverKept);
        assert_eq!(m.log(), ["rm_own"]);
        let m = Mock { original: true, ..Default::default() };
        assert_eq!(uninstall(&m, false).unwrap(), Uninstalled::DriverKept);
        assert_eq!(m.log(), ["rm_own", "query_orig"]);
    }

    #[test]
    fn uninstall_service_failure_leaves_driver_alone() {
        let m = Mock { fail: Some("rm_own"), ..Default::default() };
        assert!(uninstall(&m, false).is_err());
        assert_eq!(m.log(), ["rm_own"]);
    }

    #[test]
    fn uninstall_unload_or_file_failure_requires_reboot() {
        let m = Mock { stop_fails: true, ..Default::default() };
        assert_eq!(uninstall(&m, false).unwrap(), Uninstalled::RebootRequired);
        assert_eq!(m.log(), ["rm_own", "query_orig", "stop_driver", "del_driver_svc", "rm_file"]);
        let m = Mock { file_busy: true, ..Default::default() };
        assert_eq!(uninstall(&m, false).unwrap(), Uninstalled::RebootRequired);
    }
}
