use widestring::U16CString;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Command { AddInclude, AddExclude, Clear, Setup }

pub fn cmd_code(c: Command) -> i32 {
    match c { Command::AddInclude => 0, Command::AddExclude => 1, Command::Clear => 2, Command::Setup => 3 }
}

pub fn encode_mask(mask: &str) -> Vec<u8> {
    let w = U16CString::from_str(mask).expect("no interior nul");
    // units + terminating nul, little-endian bytes
    let mut out = Vec::with_capacity((w.len() + 1) * 2);
    for u in w.as_slice_with_nul() { out.extend_from_slice(&u.to_le_bytes()); }
    out
}

pub fn encode_setup(enabled: bool, log_dir: &str) -> Vec<u8> {
    // UTF-16 of the path WITHOUT prefix and WITHOUT nul; cbLogDir = byte length
    let units: Vec<u16> = log_dir.encode_utf16().collect();
    let cb = (units.len() * 2) as u16;
    let mut out = Vec::with_capacity(4 + cb as usize);
    out.push(if enabled { 1 } else { 0 }); // bEnabled
    out.push(0);                            // pad
    out.extend_from_slice(&cb.to_le_bytes());
    for u in units { out.extend_from_slice(&u.to_le_bytes()); }
    out
}

pub struct Message { pub command: i32, pub payload: Vec<u8> }

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mask_is_nul_terminated_utf16() {
        let b = encode_mask("*.*");
        assert_eq!(b, vec![0x2a,0,0x2e,0,0x2a,0,0,0]);
    }

    #[test]
    fn setup_header_layout_is_exact() {
        let b = encode_setup(true, "D:\Bin");
        assert_eq!(b[0], 1);
        assert_eq!(b[1], 0);
        let cb = u16::from_le_bytes([b[2], b[3]]);
        assert_eq!(cb, 12);
        assert_eq!(&b[4..6], &[0x44, 0x00]);
        assert_eq!(b.len(), 4 + 12);
    }

    #[test]
    fn setup_disabled_has_zero_flag() {
        let b = encode_setup(false, "D:\Bin");
        assert_eq!(b[0], 0);
    }

    #[test]
    fn cmd_codes_match_driver() {
        assert_eq!(cmd_code(Command::AddInclude), 0);
        assert_eq!(cmd_code(Command::AddExclude), 1);
        assert_eq!(cmd_code(Command::Clear), 2);
        assert_eq!(cmd_code(Command::Setup), 3);
    }
}
