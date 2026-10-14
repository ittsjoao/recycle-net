use std::ffi::OsString;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use rnetrbin_svc::{config::Config, service};
use windows_service::service::{
    ServiceControl, ServiceControlAccept, ServiceExitCode, ServiceState, ServiceStatus, ServiceType,
};
use windows_service::service_control_handler::{self, ServiceControlHandlerResult, ServiceStatusHandle};
use windows_service::{define_windows_service, service_dispatcher};

const SERVICE_NAME: &str = "rnetrbin-svc";
const CONFIG_PATH: &str = r"C:\ProgramData\rnetrbin\config.toml";
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
    let status_handle: Arc<OnceLock<ServiceStatusHandle>> = Arc::new(OnceLock::new());
    let h_status = status_handle.clone();
    let handle = service_control_handler::register(SERVICE_NAME, move |ctl| match ctl {
        ServiceControl::Stop | ServiceControl::Shutdown => {
            h_stop.store(true, Ordering::Relaxed);
            if let Some(h) = h_status.get() {
                let _ = h.set_service_status(status(
                    ServiceState::StopPending,
                    ServiceControlAccept::empty(),
                    0,
                    Duration::from_secs(10),
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

    // A bad/missing config exits here, before any SCM takeover: SNetRBin is untouched.
    let result = Config::load(Path::new(CONFIG_PATH))
        .map_err(|e| e.to_string())
        .and_then(|cfg| service::run(cfg, stop, true).map_err(|e| e.to_string()));
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
    let cfg = Config::load(Path::new(CONFIG_PATH))?;
    let stop = Arc::new(AtomicBool::new(false));
    let c_stop = stop.clone();
    ctrlc::set_handler(move || c_stop.store(true, Ordering::Relaxed))?;
    if takeover {
        tracing::warn!("--takeover: SNetRBin will be stopped and disabled");
    }
    service::run(cfg, stop, takeover)?;
    Ok(())
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.iter().any(|a| a == "--console") {
        let takeover = args.iter().any(|a| a == "--takeover");
        if let Err(e) = run_console(takeover) {
            eprintln!("error: {e}");
            std::process::exit(1);
        }
        return;
    }
    if let Err(e) = service_dispatcher::start(SERVICE_NAME, ffi_service_main) {
        eprintln!("service dispatcher failed (use --console to run interactively): {e}");
        std::process::exit(1);
    }
}
