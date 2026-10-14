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

/// Pure buffer placement: (InBuffer, OutBuffer) = (command LE bytes, owned mutable payload copy).
/// The driver reads the command from InBuffer and the payload from OutBuffer.
pub fn build_buffers(command: i32, payload: &[u8]) -> (Vec<u8>, Vec<u8>) {
    (command.to_le_bytes().to_vec(), payload.to_vec())
}


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
        let (inb, mut outb) = build_buffers(command, payload);
        let mut returned: u32 = 0;
        unsafe {
            FilterSendMessage(
                self.handle,
                inb.as_ptr() as *const _,
                inb.len() as u32,
                if outb.is_empty() { None } else { Some(outb.as_mut_ptr() as *mut _) },
                outb.len() as u32,
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

    /// Heartbeat: re-assert the monitor (SetupMonitor only). It must NOT send
    /// Clear + masks — that would reopen a recurring window with no include
    /// masks, and re-adding masks without Clear would grow the driver's list.
    /// If the driver reloaded, the port is closed and this send fails, which
    /// makes the caller drop the session and run a full `connect_and_configure`.
    pub fn reassert(&self, log_dir: &str) -> Result<(), PortError> {
        self.send(cmd_code(Command::Setup), &encode_setup(true, log_dir))
    }
}

impl Drop for PortClient {
    fn drop(&mut self) {
        unsafe { let _ = CloseHandle(self.handle); }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn command_in_inbuffer_payload_in_outbuffer() {
        let (i, o) = build_buffers(3, &[9, 8, 7]);
        assert_eq!(i, 3i32.to_le_bytes().to_vec());
        assert_eq!(o, vec![9, 8, 7]);
        let (i, o) = build_buffers(2, &[]);
        assert_eq!(i, vec![2, 0, 0, 0]);
        assert!(o.is_empty());
    }
}
