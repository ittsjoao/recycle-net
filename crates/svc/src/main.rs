use std::ffi::OsString;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use rnetrbin_svc::{config::Config, service};
use windows_service::service::{
    ServiceControl, ServiceControlAccept, ServiceExitCode, ServiceState, ServiceStatus, ServiceType,
};
use windows_service::service_control_handler::{self, ServiceControlHandlerResult};
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
    let file = std::fs::OpenOptions::new().create(true).append(true).open(LOG_PATH);
    if let Ok(f) = file {
        let _ = tracing_subscriber::fmt()
            .with_writer(std::sync::Mutex::new(f))
            .with_ansi(false)
            .try_init();
    }
}

fn status(state: ServiceState, accept: ServiceControlAccept, code: u32) -> ServiceStatus {
    ServiceStatus {
        service_type: ServiceType::OWN_PROCESS,
        current_state: state,
        controls_accepted: accept,
        exit_code: ServiceExitCode::Win32(code),
        checkpoint: 0,
        wait_hint: Duration::default(),
        process_id: None,
    }
}

fn run_as_service() -> Result<(), Box<dyn std::error::Error>> {
    let stop = Arc::new(AtomicBool::new(false));
    let h_stop = stop.clone();
    let handle = service_control_handler::register(SERVICE_NAME, move |ctl| match ctl {
        ServiceControl::Stop => {
            h_stop.store(true, Ordering::Relaxed);
            ServiceControlHandlerResult::NoError
        }
        ServiceControl::Interrogate => ServiceControlHandlerResult::NoError,
        _ => ServiceControlHandlerResult::NotImplemented,
    })?;
    handle.set_service_status(status(ServiceState::Running, ServiceControlAccept::STOP, 0))?;

    let result = Config::load(Path::new(CONFIG_PATH))
        .map_err(|e| e.to_string())
        .and_then(|cfg| service::run(cfg, stop).map_err(|e| e.to_string()));
    let code = match &result {
        Ok(()) => 0,
        Err(e) => {
            tracing::error!("{e}");
            1
        }
    };
    handle.set_service_status(status(ServiceState::Stopped, ServiceControlAccept::empty(), code))?;
    Ok(())
}

fn run_console() -> Result<(), Box<dyn std::error::Error>> {
    let _ = tracing_subscriber::fmt().with_writer(std::io::stderr).try_init();
    let cfg = Config::load(Path::new(CONFIG_PATH))?;
    let stop = Arc::new(AtomicBool::new(false));
    let c_stop = stop.clone();
    ctrlc::set_handler(move || c_stop.store(true, Ordering::Relaxed))?;
    service::run(cfg, stop)?;
    Ok(())
}

fn main() {
    if std::env::args().any(|a| a == "--console") {
        if let Err(e) = run_console() {
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
