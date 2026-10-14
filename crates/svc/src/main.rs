use std::ffi::OsString;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use rnetrbin_svc::config::{Config, CONFIG_PATH};
use rnetrbin_svc::{install, service};
use windows_service::service::{
    ServiceControl, ServiceControlAccept, ServiceExitCode, ServiceState, ServiceStatus, ServiceType,
};
use windows_service::service_control_handler::{self, ServiceControlHandlerResult, ServiceStatusHandle};
use windows_service::{define_windows_service, service_dispatcher};

const SERVICE_NAME: &str = "rnetrbin-svc";
const LOG_PATH: &str = r"C:\ProgramData\rnetrbin\rnetrbin-svc.log";

define_windows_service!(ffi_service_main, service_main);

fn service_main(_args: Vec<OsString>) {
    init_file_logging();
    if let Err(e) = run_as_service() {
        tracing::error!("service failed: {e}");
    }
}

fn init_file_logging() {
    if let Some(parent) = Path::new(LOG_PATH).parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    match std::fs::OpenOptions::new().create(true).append(true).open(LOG_PATH) {
        Ok(f) => {
            let _ = tracing_subscriber::fmt()
                .with_writer(std::sync::Mutex::new(f))
                .with_ansi(false)
                .try_init();
        }
        Err(e) => eprintln!("cannot open log file {LOG_PATH}: {e}"),
    }
}

/// StopPending wait hint; must exceed the worst-case handback duration.
const STOP_WAIT_HINT: Duration = Duration::from_secs(90);

fn status(state: ServiceState, accept: ServiceControlAccept, code: u32, wait_hint: Duration) -> ServiceStatus {
    ServiceStatus {
        service_type: ServiceType::OWN_PROCESS,
        current_state: state,
        controls_accepted: accept,
        exit_code: if code == 0 { ServiceExitCode::Win32(0) } else { ServiceExitCode::ServiceSpecific(code) },
        checkpoint: if state == ServiceState::StopPending { 1 } else { 0 },
        wait_hint,
        process_id: None,
    }
}

fn run_as_service() -> Result<(), Box<dyn std::error::Error>> {
    let stop = Arc::new(AtomicBool::new(false));
    let h_stop = stop.clone();
    let handback = Arc::new(AtomicBool::new(false));
    let h_handback = handback.clone();
    let status_handle: Arc<OnceLock<ServiceStatusHandle>> = Arc::new(OnceLock::new());
    let h_status = status_handle.clone();
    let handle = service_control_handler::register(SERVICE_NAME, move |ctl| match ctl {
        ServiceControl::Stop | ServiceControl::Shutdown => {
            // Only an explicit STOP hands SNetRBin back; SHUTDOWN keeps the takeover.
            if matches!(ctl, ServiceControl::Stop) {
                h_handback.store(true, Ordering::Relaxed);
            }
            h_stop.store(true, Ordering::Relaxed);
            if let Some(h) = h_status.get() {
                let _ = h.set_service_status(status(
                    ServiceState::StopPending,
                    ServiceControlAccept::empty(),
                    0,
                    // Worst case handback: ~30s waiting for SNetRBin to stop
                    // plus ~30s for StartServiceW, plus the watcher join.
                    STOP_WAIT_HINT,
                ));
            }
            ServiceControlHandlerResult::NoError
        }
        ServiceControl::Interrogate => ServiceControlHandlerResult::NoError,
        _ => ServiceControlHandlerResult::NotImplemented,
    })?;
    let _ = status_handle.set(handle);
    let accept = ServiceControlAccept::STOP | ServiceControlAccept::SHUTDOWN;
    handle.set_service_status(status(ServiceState::Running, accept, 0, Duration::default()))?;

    // A bad/missing config exits before any takeover. If a previous takeover left a
    // state file, SNetRBin is handed back first; otherwise the SCM is never touched.
    let result = match Config::load(Path::new(CONFIG_PATH)) {
        Ok(cfg) => service::run(cfg, stop, handback, true).map_err(|e| e.to_string()),
        Err(e) => {
            service::early_exit_restore_service();
            Err(e.to_string())
        }
    };
    let code = match &result {
        Ok(()) => 0,
        Err(e) => {
            tracing::error!("{e}");
            1
        }
    };
    handle.set_service_status(status(ServiceState::Stopped, ServiceControlAccept::empty(), code, Duration::default()))?;
    Ok(())
}

fn run_console(takeover: bool) -> Result<(), Box<dyn std::error::Error>> {
    let _ = tracing_subscriber::fmt().with_writer(std::io::stderr).try_init();
    let cfg = match Config::load(Path::new(CONFIG_PATH)) {
        Ok(c) => c,
        Err(e) => {
            // Same early-exit rule as service mode, but only when this run is
            // allowed to touch the SCM at all.
            if takeover {
                service::early_exit_restore_service();
            }
            return Err(e.into());
        }
    };
    let stop = Arc::new(AtomicBool::new(false));
    let handback = Arc::new(AtomicBool::new(false));
    let (c_stop, c_hand) = (stop.clone(), handback.clone());
    // Ctrl+C acts like STOP; it hands back only when this run took over.
    ctrlc::set_handler(move || {
        if takeover {
            c_hand.store(true, Ordering::Relaxed);
        }
        c_stop.store(true, Ordering::Relaxed);
    })?;
    if takeover {
        tracing::warn!("--takeover: SNetRBin will be stopped and disabled");
    }
    service::run(cfg, stop, handback, takeover)?;
    Ok(())
}

/// `install [--driver <netrbin.sys>]`: installs the embedded driver unless
/// overridden. The config must already be valid, so the service never starts
/// against a config it would reject.
fn run_install(args: &[String]) -> Result<(), Box<dyn std::error::Error>> {
    let _ = tracing_subscriber::fmt().with_writer(std::io::stderr).try_init();
    let driver = match args.iter().position(|a| a == "--driver") {
        Some(i) => {
            let p = args.get(i + 1).ok_or("usage: rnetrbin-svc install [--driver <path\\to\\netrbin.sys>]")?;
            Some(install::read_driver_override(Path::new(p))?)
        }
        None => None,
    };
    Config::load(Path::new(CONFIG_PATH)).map_err(|e| format!("config {CONFIG_PATH}: {e}"))?;
    let exe = std::env::current_exe()?;
    install::install(&install::WinHost, driver.as_deref().unwrap_or(install::EMBEDDED_DRIVER), &exe)?;
    println!("installed; start with: sc.exe start {SERVICE_NAME}");
    Ok(())
}

fn run_uninstall(args: &[String]) -> Result<(), Box<dyn std::error::Error>> {
    let _ = tracing_subscriber::fmt().with_writer(std::io::stderr).try_init();
    let keep = args.iter().any(|a| a == "--keep-driver");
    match install::uninstall(&install::WinHost, keep)? {
        install::Uninstalled::DriverKept => println!("service removed; netrbin driver kept"),
        install::Uninstalled::DriverRemoved => println!("service and netrbin driver removed"),
        install::Uninstalled::RebootRequired => println!("service removed; reboot to finish removing the driver"),
    }
    if let Err(e) = rnetrbin_svc::audit::setup::undo() {
        eprintln!("warning: could not undo audit settings: {e}");
    }
    Ok(())
}

/// Console exe (so CLI output and exit codes work in cmd/PowerShell): when
/// opened by double click the console exists only for us, so drop it.
fn detach_own_console() {
    use windows::Win32::System::Console::{FreeConsole, GetConsoleProcessList};
    let mut ids = [0u32; 2];
    if unsafe { GetConsoleProcessList(&mut ids) } == 1 {
        let _ = unsafe { FreeConsole() };
    }
}

fn is_elevated() -> bool {
    use windows::Win32::Foundation::{CloseHandle, HANDLE};
    use windows::Win32::Security::{GetTokenInformation, TokenElevation, TOKEN_ELEVATION, TOKEN_QUERY};
    use windows::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};
    let mut token = HANDLE::default();
    if unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) }.is_err() {
        return false;
    }
    let mut el = TOKEN_ELEVATION::default();
    let mut len = 0u32;
    let ok = unsafe {
        GetTokenInformation(
            token,
            TokenElevation,
            Some(&mut el as *mut _ as *mut _),
            std::mem::size_of::<TOKEN_ELEVATION>() as u32,
            &mut len,
        )
    }
    .is_ok();
    unsafe {
        let _ = CloseHandle(token);
    }
    ok && el.TokenIsElevated != 0
}

/// Relaunch this exe through UAC; Ok(false) if the user declined.
fn relaunch_elevated() -> Result<bool, Box<dyn std::error::Error>> {
    use windows::core::{w, PCWSTR};
    use windows::Win32::UI::Shell::ShellExecuteW;
    use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
    let exe = widestring::U16CString::from_os_str(std::env::current_exe()?)?;
    let h = unsafe { ShellExecuteW(None, w!("runas"), PCWSTR(exe.as_ptr()), None, None, SW_SHOWNORMAL) };
    // ShellExecute reports success as a pseudo-handle value > 32.
    Ok(h.0 as isize > 32)
}

fn run_gui() -> Result<(), Box<dyn std::error::Error>> {
    detach_own_console();
    if !is_elevated() {
        relaunch_elevated()?;
        return Ok(());
    }
    rnetrbin_svc::gui::run()?;
    Ok(())
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.get(1).map(String::as_str) == Some(install::SERVICE_ARG) {
        if let Err(e) = service_dispatcher::start(SERVICE_NAME, ffi_service_main) {
            init_file_logging();
            tracing::error!("service dispatcher failed: {e}");
            std::process::exit(1);
        }
        return;
    }
    let cli = matches!(args.get(1).map(String::as_str), Some("install" | "uninstall" | "--console"));
    if !cli {
        if let Err(e) = run_gui() {
            init_file_logging();
            tracing::error!("gui failed: {e}");
            std::process::exit(1);
        }
        return;
    }
    let sub = match args.get(1).map(String::as_str) {
        Some("install") => Some(run_install(&args)),
        Some("uninstall") => Some(run_uninstall(&args)),
        _ => None,
    };
    if let Some(res) = sub {
        if let Err(e) = res {
            eprintln!("error: {e}");
            std::process::exit(1);
        }
        return;
    }
    let takeover = args.iter().any(|a| a == "--takeover");
    if let Err(e) = run_console(takeover) {
        eprintln!("error: {e}");
        std::process::exit(1);
    }
}
