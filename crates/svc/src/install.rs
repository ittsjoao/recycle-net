//! Standalone install: ship our own copy of `netrbin.sys` (embedded at build
//! time, see build.rs), register it as a minifilter exactly as the original installer does, and register
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

/// The driver image packaged into this binary by build.rs.
pub static EMBEDDED_DRIVER: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/netrbin.sys"));

/// Must exceed the service's worst-case handback on STOP (see main.rs STOP_WAIT_HINT).
const OWN_STOP_TIMEOUT: Duration = Duration::from_secs(120);
const DRIVER_STOP_TIMEOUT: Duration = Duration::from_secs(30);
/// How long a pending netrbin state may take to settle before install gives up.
const DRIVER_PENDING_GRACE: Duration = Duration::from_secs(10);

#[derive(thiserror::Error, Debug)]
pub enum InstallError {
    #[error("driver source {0}: {1}")]
    Source(PathBuf, String),
    #[error("{step}: {msg}")]
    Step { step: &'static str, msg: String },
    /// The netrbin driver service is stop/start pending and does not settle.
    #[error("netrbin driver is stuck in a pending state")]
    DriverPending,
    /// The driver was started but is not running afterwards.
    #[error("netrbin driver is not running after start")]
    DriverNotRunning,
    #[error("--driver given while the netrbin driver is loaded")]
    OverrideWhileLoaded,
}

/// SCM start type the original installer gives the minifilter (SERVICE_BOOT_START).
pub const BOOT_START: u32 = 0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DriverSvc {
    pub start_type: u32,
    pub state: SvcState,
}

/// The netrbin minifilter as found on the machine before install touches it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DriverStatus {
    /// None when the `netrbin` service does not exist.
    pub service: Option<DriverSvc>,
    /// `System32\drivers\netrbin.sys` exists.
    pub file_exists: bool,
}

#[derive(Debug, PartialEq, Eq)]
pub struct DriverPlan {
    pub write_file: bool,
    /// Put the start type back to boot (0).
    pub fix_start: bool,
}

/// What to do with the driver. An installed driver is reused (SNetRBin's, or one
/// already registered): writing over a loaded `netrbin.sys` fails, and the
/// coexisting topology must share the original file. The file is written only
/// when missing, when nothing owns it, or on an explicit `--driver` while unloaded.
pub fn plan_driver(original: bool, st: &DriverStatus, explicit: bool) -> Result<DriverPlan, InstallError> {
    let state = st.service.map(|s| s.state);
    if state == Some(SvcState::Busy) {
        return Err(InstallError::DriverPending);
    }
    let loaded = state == Some(SvcState::Running);
    if explicit && loaded {
        return Err(InstallError::OverrideWhileLoaded);
    }
    let owned = original || st.service.is_some();
    let write_file = !st.file_exists || explicit || !owned;
    let fix_start = st.service.is_some_and(|s| s.start_type != BOOT_START);
    Ok(DriverPlan { write_file, fix_start })
}

/// What `install` did, for the operator.
#[derive(Debug, PartialEq, Eq)]
pub struct Installed {
    /// The existing `netrbin.sys` was kept (not rewritten).
    pub reused_driver: bool,
    /// The `netrbin` start type was set back to boot.
    pub reenabled_driver: bool,
}

/// Win32 error code at the end of a host message: the `(0x8007XXXX)` HRESULT
/// windows-rs appends, or Rust's `(os error N)`. The first code found wins.
pub fn win32_code(msg: &str) -> Option<u32> {
    if let Some(i) = msg.find("(0x") {
        let hex = msg.get(i + 3..i + 11)?;
        let h = u32::from_str_radix(hex, 16).ok()?;
        return (h & 0xFFFF_0000 == 0x8007_0000).then_some(h & 0xFFFF);
    }
    let i = msg.find("(os error ")?;
    msg[i + 10..].split(')').next()?.parse().ok()
}

/// The sentence Windows gave, without the code tail: the text between the last
/// `": "` and the first `" ("`.
fn windows_text(msg: &str) -> Option<&str> {
    let end = msg.find(" (0x").or_else(|| msg.find(" (os error "))?;
    let head = &msg[..end];
    let start = head.rfind(": ").map_or(0, |i| i + 2);
    let t = head[start..].trim().trim_end_matches('.');
    (!t.is_empty()).then_some(t)
}

const HVCI_HINT: &str = "O Windows bloqueou o driver (assinatura de 2015). Desligue a Integridade de Memória em \
                         Segurança do Windows → Isolamento de núcleo, reinicie e instale de novo.";

/// Readable pt-BR text for an install/uninstall error: what failed, the cause
/// Windows reported (text and code), a hint when there is one, and the raw
/// message last so a support report has the exact detail.
pub fn describe(e: &InstallError) -> String {
    let (step, msg) = match e {
        InstallError::Source(p, m) => return format!("O driver indicado em --driver ({}) é inválido: {m}.", p.display()),
        InstallError::DriverPending => {
            return "O driver netrbin está parando e não responde. Reinicie o servidor e rode o setup de novo.".into()
        }
        InstallError::DriverNotRunning => {
            return "O Windows aceitou iniciar o driver netrbin, mas ele não ficou carregado. \
                    Reinicie o servidor e rode o setup de novo."
                .into()
        }
        InstallError::OverrideWhileLoaded => {
            return "O driver netrbin está carregado; para trocá-lo com --driver, rode \
                    \"rnetrbin-svc.exe uninstall\" (ou reinicie) antes."
                .into()
        }
        InstallError::Step { step, msg } => (*step, msg.as_str()),
    };
    let what = match step {
        "copy driver" => r"copiar o driver para C:\Windows\System32\drivers",
        "register driver" => "registrar o driver netrbin",
        "query driver" => "consultar o driver netrbin",
        "enable driver" => "reativar o driver netrbin (início no boot)",
        "load driver" => "carregar o driver netrbin.sys",
        "register service" => "registrar o serviço rnetrbin-svc",
        "remove service" => "remover o serviço rnetrbin-svc",
        "query SNetRBin" => "consultar o serviço SNetRBin",
        "delete driver service" => "remover o registro do driver netrbin",
        "remove driver file" => "apagar o arquivo netrbin.sys",
        s => s,
    };
    let mut out = format!("Não foi possível {what}.\n");
    let code = win32_code(msg);
    if let Some(c) = code {
        let text = windows_text(msg).unwrap_or("erro do Windows");
        out += &format!("Causa informada pelo Windows: {text} (código {c}).\n");
    }
    let hint = match (step, code) {
        ("load driver", Some(577 | 1275)) => Some(HVCI_HINT),
        (_, Some(5)) => Some("Rode o setup (ou o comando) como administrador."),
        ("copy driver", Some(32)) => {
            Some("O netrbin.sys em uso já está carregado; rode \"rnetrbin-svc.exe uninstall\" ou reinicie e tente de novo.")
        }
        _ => None,
    };
    if let Some(h) = hint {
        out += h;
        out.push('\n');
    }
    out + &format!("Detalhe técnico: {step}: {msg}")
}

fn step(step: &'static str) -> impl FnOnce(String) -> InstallError {
    move |msg| InstallError::Step { step, msg }
}

/// System operations used by install/uninstall. Every method is idempotent.
pub trait Host {
    /// Write `image` to `%SystemRoot%\System32\drivers\netrbin.sys` unless identical.
    fn copy_driver(&self, image: &[u8]) -> Result<(), String>;
    /// The `netrbin` service (start type, state) and whether its file exists.
    fn driver_status(&self) -> Result<DriverStatus, String>;
    /// Create the minifilter service and its default instance (altitude) keys.
    fn ensure_driver_service(&self) -> Result<(), String>;
    /// Set the `netrbin` start type back to boot (0); nothing else changes.
    fn set_driver_boot_start(&self) -> Result<(), String>;
    /// Load the minifilter (already running is Ok).
    /// Ok(true) when Windows answered "already running" (0x420), which it also
    /// says for a driver stuck stopping.
    fn start_driver(&self) -> Result<bool, String>;
    /// Create `rnetrbin-svc` (auto start) pointing at `exe`, or repoint an existing
    /// one (its start type is kept).
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

/// Read a `--driver` override, rejecting anything that is obviously not a
/// driver image before touching the system.
pub fn read_driver_override(src: &Path) -> Result<Vec<u8>, InstallError> {
    let bad = |why: &str| InstallError::Source(src.to_path_buf(), why.into());
    let is_sys = src.extension().is_some_and(|e| e.eq_ignore_ascii_case("sys"));
    if !is_sys {
        return Err(bad("expected a .sys file"));
    }
    let bytes = std::fs::read(src).map_err(|e| bad(&e.to_string()))?;
    if !bytes.starts_with(b"MZ") {
        return Err(bad("not a PE image (missing MZ header)"));
    }
    Ok(bytes)
}

/// Install order: look at the driver, write its file only when `plan_driver`
/// says so, register it (boot start), load it and check it is running, then
/// register our service. `explicit` is true for `--driver`. The service is not
/// started here (the setup runs `sc start`).
pub fn install<H: Host>(host: &H, driver: &[u8], explicit: bool, exe: &Path) -> Result<Installed, InstallError> {
    let original = host.original_present().map_err(step("query SNetRBin"))?;
    let before = host.driver_status().map_err(step("query driver"))?;
    let plan = plan_driver(original, &before, explicit)?;
    if plan.write_file {
        host.copy_driver(driver).map_err(step("copy driver"))?;
    } else {
        tracing::info!("reusing the installed {DRIVER_FILE}");
    }
    host.ensure_driver_service().map_err(step("register driver"))?;
    if plan.fix_start {
        host.set_driver_boot_start().map_err(step("enable driver"))?;
    }
    let said_running = host.start_driver().map_err(step("load driver"))?;
    // "Already running" (0x420) while the driver is not running means it is stuck stopping.
    match host.driver_status().map_err(step("query driver"))?.service.map(|s| s.state) {
        Some(SvcState::Running) => {}
        Some(SvcState::Busy) => return Err(InstallError::DriverPending),
        _ if said_running => return Err(InstallError::DriverPending),
        _ => return Err(InstallError::DriverNotRunning),
    }
    host.ensure_own_service(exe).map_err(step("register service"))?;
    Ok(Installed { reused_driver: !plan.write_file, reenabled_driver: plan.fix_start })
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

/// Command-line flag the SCM passes so the exe runs as the service (no flag opens the GUI).
pub const SERVICE_ARG: &str = "--service";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SvcState {
    NotInstalled,
    Stopped,
    Running,
    /// Start/stop pending or paused.
    Busy,
}

pub fn service_state(name: &str) -> Result<SvcState, String> {
    use windows::Win32::System::Services::*;
    let Some(s) = win::open(name, SERVICE_QUERY_STATUS)? else {
        return Ok(SvcState::NotInstalled);
    };
    win::state(&s, name)
}

/// Start a service; already running is Ok.
pub fn start_service(name: &str) -> Result<(), String> {
    start_service_reporting(name).map(|_| ())
}

/// Start a service; Ok(true) when Windows answered "already running" (0x420).
fn start_service_reporting(name: &str) -> Result<bool, String> {
    use windows::Win32::Foundation::ERROR_SERVICE_ALREADY_RUNNING;
    use windows::Win32::System::Services::*;
    let s = win::open(name, SERVICE_START)?.ok_or_else(|| format!("{name} is not installed"))?;
    match unsafe { StartServiceW(s.svc, None) } {
        Ok(()) => Ok(false),
        Err(e) if e.code() == ERROR_SERVICE_ALREADY_RUNNING.to_hresult() => {
            tracing::info!("start {name}: already running (0x420)");
            Ok(true)
        }
        Err(e) => Err(format!("start {name}: {e}")),
    }
}

/// Whether an SCM start type means the service starts with Windows
/// (Automatic; "delayed" is a separate flag on the same start type).
fn is_autostart(start_type: u32) -> bool {
    start_type == windows::Win32::System::Services::SERVICE_AUTO_START.0
}

/// Whether the service starts with Windows (SCM start type is Automatic).
pub fn service_autostart(name: &str) -> Result<bool, String> {
    use windows::Win32::System::Services::*;
    let s = win::open(name, SERVICE_QUERY_CONFIG)?.ok_or_else(|| format!("{name} is not installed"))?;
    Ok(is_autostart(win::start_type(&s, name)?))
}

/// Switch the service between Automatic (`on`) and Manual start; nothing else changes.
pub fn set_service_autostart(name: &str, on: bool) -> Result<(), String> {
    use windows::Win32::System::Services::*;
    let s = win::open(name, SERVICE_CHANGE_CONFIG)?.ok_or_else(|| format!("{name} is not installed"))?;
    let start = if on { SERVICE_AUTO_START } else { SERVICE_DEMAND_START };
    unsafe {
        ChangeServiceConfigW(
            s.svc,
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
    }
    .map_err(|e| format!("change config {name}: {e}"))
}

/// Stop `rnetrbin-svc` and wait for it; not running is Ok.
pub fn stop_own_service() -> Result<(), String> {
    win::stop(OWN_SERVICE, OWN_STOP_TIMEOUT)
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

    pub fn state(s: &Svc, name: &str) -> Result<super::SvcState, String> {
        let mut st = SERVICE_STATUS::default();
        unsafe { QueryServiceStatus(s.svc, &mut st) }.map_err(|e| format!("query {name}: {e}"))?;
        Ok(match st.dwCurrentState {
            SERVICE_STOPPED => super::SvcState::Stopped,
            SERVICE_RUNNING => super::SvcState::Running,
            _ => super::SvcState::Busy,
        })
    }

    /// SCM start type (0 boot ... 4 disabled); needs SERVICE_QUERY_CONFIG.
    pub fn start_type(s: &Svc, name: &str) -> Result<u32, String> {
        let mut needed = 0u32;
        // First call only sizes the buffer (fails with ERROR_INSUFFICIENT_BUFFER).
        let _ = unsafe { QueryServiceConfigW(s.svc, None, 0, &mut needed) };
        // u64 storage keeps the struct (pointers inside) aligned.
        let mut buf = vec![0u64; (needed as usize).div_ceil(8).max(1)];
        let cfg = buf.as_mut_ptr() as *mut QUERY_SERVICE_CONFIGW;
        unsafe { QueryServiceConfigW(s.svc, Some(cfg), (buf.len() * 8) as u32, &mut needed) }
            .map_err(|e| format!("query config {name}: {e}"))?;
        Ok(unsafe { (*cfg).dwStartType }.0)
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
                return Err(format!("create key {sub}: {}", windows::core::Error::from(rc.to_hresult())));
            }
            let mut out = Ok(());
            for (name, ty, data) in vals {
                let n = win::wide(name);
                let rc = unsafe { RegSetValueExW(key, PCWSTR(n.as_ptr()), 0, *ty, Some(data)) };
                if rc.is_err() {
                    out = Err(format!("set {sub}\\{name}: {}", windows::core::Error::from(rc.to_hresult())));
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
    fn copy_driver(&self, image: &[u8]) -> Result<(), String> {
        let dst = drivers_dir().join(DRIVER_FILE);
        if std::fs::read(&dst).is_ok_and(|cur| cur == image) {
            tracing::info!("{} already up to date", dst.display());
            return Ok(());
        }
        std::fs::write(&dst, image).map_err(|e| {
            format!("write {}: {e} (if the driver is loaded, run uninstall or unload it first)", dst.display())
        })
    }

    fn driver_status(&self) -> Result<DriverStatus, String> {
        use windows::Win32::System::Services::*;
        let file_exists = drivers_dir().join(DRIVER_FILE).exists();
        let Some(s) = win::open(DRIVER_SERVICE, SERVICE_QUERY_STATUS | SERVICE_QUERY_CONFIG)? else {
            return Ok(DriverStatus { service: None, file_exists });
        };
        let start_type = win::start_type(&s, DRIVER_SERVICE)?;
        // A pending state may still settle; give it a few seconds before calling it stuck.
        let deadline = std::time::Instant::now() + DRIVER_PENDING_GRACE;
        let state = loop {
            let st = win::state(&s, DRIVER_SERVICE)?;
            if st != SvcState::Busy || std::time::Instant::now() >= deadline {
                break st;
            }
            std::thread::sleep(Duration::from_millis(500));
        };
        tracing::info!("{DRIVER_SERVICE}: start type {start_type}, {state:?}, file present: {file_exists}");
        Ok(DriverStatus { service: Some(DriverSvc { start_type, state }), file_exists })
    }

    fn set_driver_boot_start(&self) -> Result<(), String> {
        use windows::Win32::System::Services::*;
        let s = win::open(DRIVER_SERVICE, SERVICE_CHANGE_CONFIG)?
            .ok_or_else(|| format!("{DRIVER_SERVICE} is not installed"))?;
        unsafe {
            ChangeServiceConfigW(
                s.svc,
                ENUM_SERVICE_TYPE(SERVICE_NO_CHANGE),
                SERVICE_BOOT_START,
                SERVICE_ERROR(SERVICE_NO_CHANGE),
                None,
                None,
                None,
                None,
                None,
                None,
                None,
            )
        }
        .map_err(|e| format!("change config {DRIVER_SERVICE}: {e}"))
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

    fn start_driver(&self) -> Result<bool, String> {
        start_service_reporting(DRIVER_SERVICE)
    }

    fn ensure_own_service(&self, exe: &Path) -> Result<(), String> {
        use windows::core::PCWSTR;
        use windows::Win32::System::Services::*;
        let bin = win::wide(&format!("\"{}\" {SERVICE_ARG}", exe.display()));
        if let Some(s) = win::open(OWN_SERVICE, SERVICE_CHANGE_CONFIG)? {
            return unsafe {
                // Keep the start type: the user may have turned autostart off in the window.
                ChangeServiceConfigW(
                    s.svc,
                    SERVICE_WIN32_OWN_PROCESS,
                    SERVICE_START_TYPE(SERVICE_NO_CHANGE),
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
        drv: RefCell<DriverStatus>,
        /// State start_driver leaves the driver in (default Running).
        start_leaves: Option<SvcState>,
        /// start_driver answers "already running" (0x420).
        says_running: bool,
    }
    impl Mock {
        fn call(&self, name: &'static str) -> Result<(), String> {
            self.log.borrow_mut().push(name);
            if self.fail == Some(name) { Err("boom".into()) } else { Ok(()) }
        }
        fn log(&self) -> Vec<&'static str> {
            self.log.borrow().clone()
        }
        fn with_driver(start_type: u32, state: SvcState, file_exists: bool) -> Mock {
            let drv = DriverStatus { service: Some(DriverSvc { start_type, state }), file_exists };
            Mock { drv: RefCell::new(drv), ..Default::default() }
        }
    }
    impl Host for Mock {
        fn copy_driver(&self, _image: &[u8]) -> Result<(), String> {
            self.call("copy")?;
            self.drv.borrow_mut().file_exists = true;
            Ok(())
        }
        fn driver_status(&self) -> Result<DriverStatus, String> {
            self.call("status")?;
            Ok(self.drv.borrow().clone())
        }
        fn ensure_driver_service(&self) -> Result<(), String> {
            self.call("driver_svc")?;
            let mut d = self.drv.borrow_mut();
            if d.service.is_none() {
                d.service = Some(DriverSvc { start_type: BOOT_START, state: SvcState::Stopped });
            }
            Ok(())
        }
        fn set_driver_boot_start(&self) -> Result<(), String> {
            self.call("boot_start")?;
            if let Some(svc) = self.drv.borrow_mut().service.as_mut() {
                svc.start_type = BOOT_START;
            }
            Ok(())
        }
        fn start_driver(&self) -> Result<bool, String> {
            self.call("start_driver")?;
            if let Some(svc) = self.drv.borrow_mut().service.as_mut() {
                svc.state = self.start_leaves.unwrap_or(SvcState::Running);
            }
            Ok(self.says_running)
        }
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
    fn override_rejects_non_sys_and_non_pe() {
        let d = tempfile::tempdir().unwrap();
        assert!(read_driver_override(&driver_file(&d, "a.dll", b"MZ..")).is_err());
        assert!(read_driver_override(&driver_file(&d, "a.sys", b"PK..")).is_err());
        assert!(read_driver_override(&d.path().join("missing.sys")).is_err());
        assert_eq!(read_driver_override(&driver_file(&d, "netrbin.SYS", b"MZ\x90\x00")).unwrap(), b"MZ\x90\x00");
    }

    #[test]
    fn only_automatic_start_type_is_autostart() {
        use windows::Win32::System::Services::*;
        assert!(is_autostart(SERVICE_AUTO_START.0));
        for t in [SERVICE_DEMAND_START, SERVICE_DISABLED, SERVICE_BOOT_START, SERVICE_SYSTEM_START] {
            assert!(!is_autostart(t.0));
        }
    }

    #[test]
    fn embedded_driver_is_a_pe_image() {
        assert!(EMBEDDED_DRIVER.starts_with(b"MZ"));
    }

    #[test]
    fn win32_code_reads_hresult_and_os_error_tails() {
        assert_eq!(win32_code("start netrbin: Falhou. (0x80070241)"), Some(577));
        assert_eq!(win32_code("x: y (0x800704FB)"), Some(1275));
        assert_eq!(win32_code(r"write C:\a: Acesso negado. (os error 5)"), Some(5));
        assert_eq!(win32_code("open SCM: Acesso negado. (0x80070005) (run as Administrator)"), Some(5));
        assert_eq!(win32_code("boom"), None);
        assert_eq!(win32_code("odd (0x8000FFFF)"), None);
    }

    fn step_err(step: &'static str, msg: &str) -> InstallError {
        InstallError::Step { step, msg: msg.into() }
    }

    #[test]
    fn describe_load_driver_blocked_adds_the_hvci_hint() {
        let raw = "start netrbin: O Windows não pode verificar a assinatura digital deste arquivo. (0x80070241)";
        assert_eq!(
            describe(&step_err("load driver", raw)),
            "Não foi possível carregar o driver netrbin.sys.\n\
             Causa informada pelo Windows: O Windows não pode verificar a assinatura digital deste arquivo (código 577).\n\
             O Windows bloqueou o driver (assinatura de 2015). Desligue a Integridade de Memória em \
             Segurança do Windows → Isolamento de núcleo, reinicie e instale de novo.\n\
             Detalhe técnico: load driver: start netrbin: O Windows não pode verificar a assinatura digital deste arquivo. (0x80070241)"
        );
        assert!(describe(&step_err("load driver", "start netrbin: Bloqueado. (0x800704FB)")).contains("Integridade de Memória"));
        // The hint belongs to the load step only.
        assert!(!describe(&step_err("copy driver", "write: x (0x80070241)")).contains("Integridade"));
    }

    #[test]
    fn describe_names_each_step_in_portuguese() {
        for (step, what) in [
            ("copy driver", "copiar o driver"),
            ("register driver", "registrar o driver"),
            ("load driver", "carregar o driver"),
            ("register service", "registrar o serviço"),
            ("remove service", "remover o serviço"),
        ] {
            let t = describe(&step_err(step, "boom"));
            assert!(t.starts_with(&format!("Não foi possível {what}")), "{t}");
            assert!(t.ends_with(&format!("Detalhe técnico: {step}: boom")), "{t}");
            assert!(!t.contains("Causa informada"), "{t}");
        }
    }

    #[test]
    fn describe_access_denied_and_file_in_use() {
        let t = describe(&step_err("register service", "open SCM: Acesso negado. (0x80070005) (run as Administrator)"));
        assert!(t.contains("Causa informada pelo Windows: Acesso negado (código 5)."), "{t}");
        assert!(t.contains("como administrador"), "{t}");
        let raw = r"write C:\Windows\System32\drivers\netrbin.sys: O arquivo já está sendo usado por outro processo. (os error 32)";
        let t = describe(&step_err("copy driver", raw));
        assert!(t.contains("usado por outro processo (código 32)."), "{t}");
        assert!(t.contains("em uso"), "{t}");
    }

    #[test]
    fn describe_bad_driver_override() {
        let e = InstallError::Source(PathBuf::from(r"C:\x\a.dll"), "expected a .sys file".into());
        assert_eq!(describe(&e), r"O driver indicado em --driver (C:\x\a.dll) é inválido: expected a .sys file.");
    }

    const EXE: &str = r"C:\x\rnetrbin-svc.exe";

    #[test]
    fn fresh_install_writes_driver_and_checks_it_loaded() {
        let m = Mock::default();
        let r = install(&m, EMBEDDED_DRIVER, false, Path::new(EXE)).unwrap();
        assert_eq!(r, Installed { reused_driver: false, reenabled_driver: false });
        assert_eq!(m.log(), ["query_orig", "status", "copy", "driver_svc", "start_driver", "status", "own_svc"]);
    }

    #[test]
    fn coexisting_install_reuses_driver_and_reenables_boot_start() {
        // The human's server: SNetRBin present, netrbin left DISABLED (4) and stopped.
        let m = Mock { original: true, ..Mock::with_driver(4, SvcState::Stopped, true) };
        let r = install(&m, EMBEDDED_DRIVER, false, Path::new(EXE)).unwrap();
        assert_eq!(r, Installed { reused_driver: true, reenabled_driver: true });
        assert_eq!(m.log(), ["query_orig", "status", "driver_svc", "boot_start", "start_driver", "status", "own_svc"]);
    }

    #[test]
    fn loaded_driver_is_reused_even_without_snetrbin() {
        let m = Mock::with_driver(BOOT_START, SvcState::Running, true);
        let r = install(&m, EMBEDDED_DRIVER, false, Path::new(EXE)).unwrap();
        assert_eq!(r, Installed { reused_driver: true, reenabled_driver: false });
        assert!(!m.log().contains(&"copy"));
        assert!(!m.log().contains(&"boot_start"));
    }

    #[test]
    fn plan_driver_decides_file_and_start_type() {
        let svc = |start_type, state| Some(DriverSvc { start_type, state });
        let st = |service, file_exists| DriverStatus { service, file_exists };
        let plan = |orig, s: DriverStatus, explicit| plan_driver(orig, &s, explicit).unwrap();
        // Nothing installed, or a stray file with no service and no SNetRBin: write.
        assert_eq!(plan(false, st(None, false), false), DriverPlan { write_file: true, fix_start: false });
        assert_eq!(plan(false, st(None, true), false), DriverPlan { write_file: true, fix_start: false });
        // A missing file is always written, even with SNetRBin.
        assert!(plan(true, st(svc(0, SvcState::Stopped), false), false).write_file);
        // SNetRBin present, or the service and the file exist: reuse.
        assert!(!plan(true, st(None, true), false).write_file);
        assert!(!plan(false, st(svc(0, SvcState::Stopped), true), false).write_file);
        // Any start type other than boot (0) is put back.
        assert!(plan(false, st(svc(4, SvcState::Stopped), true), false).fix_start);
        assert!(plan(false, st(svc(3, SvcState::Stopped), true), false).fix_start);
        assert!(!plan(false, st(svc(0, SvcState::Running), true), false).fix_start);
        // --driver overwrites only an unloaded driver.
        assert!(plan(true, st(svc(0, SvcState::Stopped), true), true).write_file);
        assert!(matches!(
            plan_driver(true, &st(svc(0, SvcState::Running), true), true),
            Err(InstallError::OverrideWhileLoaded)
        ));
        // A pending driver stops everything.
        assert!(matches!(plan_driver(true, &st(svc(4, SvcState::Busy), true), false), Err(InstallError::DriverPending)));
    }

    #[test]
    fn stop_pending_driver_fails_before_touching_anything() {
        let m = Mock { original: true, ..Mock::with_driver(4, SvcState::Busy, true) };
        let e = install(&m, EMBEDDED_DRIVER, false, Path::new(EXE)).unwrap_err();
        assert_eq!(m.log(), ["query_orig", "status"]);
        assert_eq!(describe(&e), "O driver netrbin está parando e não responde. Reinicie o servidor e rode o setup de novo.");
    }

    #[test]
    fn driver_not_running_after_start_is_an_error() {
        // StartService said OK (or "already running", 0x420) but the driver is not up.
        let m = Mock { start_leaves: Some(SvcState::Busy), ..Default::default() };
        let e = install(&m, EMBEDDED_DRIVER, false, Path::new(EXE)).unwrap_err();
        assert!(matches!(e, InstallError::DriverPending));
        assert!(!m.log().contains(&"own_svc"));
        let m = Mock { start_leaves: Some(SvcState::Stopped), ..Default::default() };
        let e = install(&m, EMBEDDED_DRIVER, false, Path::new(EXE)).unwrap_err();
        assert!(matches!(e, InstallError::DriverNotRunning));
        assert!(describe(&e).contains("Reinicie o servidor"));
    }

    #[test]
    fn already_running_answer_with_driver_not_running_is_pending() {
        // 0x420 from StartService, then the driver settles to Stopped: stuck stopping.
        let m = Mock {
            original: true,
            says_running: true,
            start_leaves: Some(SvcState::Stopped),
            ..Mock::with_driver(BOOT_START, SvcState::Stopped, true)
        };
        let e = install(&m, EMBEDDED_DRIVER, false, Path::new(EXE)).unwrap_err();
        assert!(matches!(e, InstallError::DriverPending));
        assert_eq!(describe(&e), "O driver netrbin está parando e não responde. Reinicie o servidor e rode o setup de novo.");
        assert!(!m.log().contains(&"own_svc"));
        // 0x420 with the driver really running is fine.
        let m = Mock { says_running: true, ..Mock::with_driver(BOOT_START, SvcState::Running, true) };
        install(&m, EMBEDDED_DRIVER, false, Path::new(EXE)).unwrap();
    }

    #[test]
    fn explicit_driver_overwrites_an_unloaded_driver_only() {
        let m = Mock { original: true, ..Mock::with_driver(BOOT_START, SvcState::Stopped, true) };
        install(&m, EMBEDDED_DRIVER, true, Path::new(EXE)).unwrap();
        assert!(m.log().contains(&"copy"));
        let m = Mock::with_driver(BOOT_START, SvcState::Running, true);
        let e = install(&m, EMBEDDED_DRIVER, true, Path::new(EXE)).unwrap_err();
        assert!(describe(&e).contains("uninstall"));
        assert_eq!(m.log(), ["query_orig", "status"]);
    }

    #[test]
    fn install_stops_at_first_failure() {
        let m = Mock { fail: Some("start_driver"), ..Default::default() };
        let e = install(&m, EMBEDDED_DRIVER, false, Path::new("x.exe")).unwrap_err();
        assert!(e.to_string().starts_with("load driver"));
        assert_eq!(m.log(), ["query_orig", "status", "copy", "driver_svc", "start_driver"]);
    }

    #[test]
    fn uninstall_on_coexisting_machine_never_touches_the_driver() {
        let m = Mock { original: true, ..Mock::with_driver(BOOT_START, SvcState::Running, true) };
        assert_eq!(uninstall(&m, false).unwrap(), Uninstalled::DriverKept);
        assert_eq!(m.log(), ["rm_own", "query_orig"]);
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
