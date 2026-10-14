use windows::core::PCWSTR;
use windows::Win32::Foundation::{CloseHandle, HANDLE};
use windows::Win32::Storage::InstallableFileSystems::{
    FilterConnectCommunicationPort, FilterSendMessage,
};
use widestring::U16CString;
use crate::proto::{Command, cmd_code, encode_mask, encode_setup};

#[derive(thiserror::Error, Debug)]
pub enum PortError {
    #[error("connect failed: {0}")]
    Connect(windows::core::Error),
    #[error("send failed: {0}")]
    Send(windows::core::Error),
}

pub struct PortClient { handle: HANDLE }

const PORT_NAME: &str = r"\CaptureFileMonitorPort";

impl PortClient {
    pub fn connect() -> Result<Self, PortError> {
        let name = U16CString::from_str(PORT_NAME).unwrap();
        // No connection context required (ConnectCallback ignores it).
        let handle = unsafe {
            FilterConnectCommunicationPort(PCWSTR(name.as_ptr()), 0, None, 0, None)
        }
        .map_err(PortError::Connect)?;
        Ok(Self { handle })
    }

    /// Driver reads the command int from InBuffer and the payload from OutBuffer.
    pub fn send(&self, command: i32, payload: &[u8]) -> Result<(), PortError> {
        let cmd = command.to_le_bytes();
        let mut returned: u32 = 0;
        unsafe {
            FilterSendMessage(
                self.handle,
                cmd.as_ptr() as *const _,
                cmd.len() as u32,
                if payload.is_empty() { None } else { Some(payload.as_ptr() as *mut _) },
                payload.len() as u32,
                &mut returned,
            )
        }
        .map_err(PortError::Send)?;
        Ok(())
    }

    pub fn configure(&self, includes: &[String], excludes: &[String], log_dir: &str)
        -> Result<(), PortError>
    {
        self.send(cmd_code(Command::Clear), &[])?;
        for m in includes { self.send(cmd_code(Command::AddInclude), &encode_mask(m))?; }
        for m in excludes { self.send(cmd_code(Command::AddExclude), &encode_mask(m))?; }
        self.send(cmd_code(Command::Setup), &encode_setup(true, log_dir))?;
        Ok(())
    }
}

impl Drop for PortClient {
    fn drop(&mut self) {
        unsafe { let _ = CloseHandle(self.handle); }
    }
}
