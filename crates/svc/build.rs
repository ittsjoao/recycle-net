//! Embeds `netrbin.sys` into the binary so `install` needs no external file.
//! Source: `$RNETRBIN_DRIVER`, else `<workspace>/driver/netrbin.sys` (gitignored,
//! third-party binary). The build fails without it: an exe that cannot install
//! the driver is not shippable.
use std::path::PathBuf;

const IMAGE_FILE_MACHINE_AMD64: u16 = 0x8664;

fn main() {
    println!("cargo:rerun-if-env-changed=RNETRBIN_DRIVER");
    let src = match std::env::var_os("RNETRBIN_DRIVER") {
        Some(p) => PathBuf::from(p),
        None => PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").unwrap()).join("../../driver/netrbin.sys"),
    };
    println!("cargo:rerun-if-changed={}", src.display());
    let bytes = std::fs::read(&src).unwrap_or_else(|e| {
        panic!(
            "netrbin.sys not found at {} ({e}). Copy it from a licensed install \
             (C:\\Windows\\System32\\drivers\\netrbin.sys) to driver/netrbin.sys \
             or set RNETRBIN_DRIVER.",
            src.display()
        )
    });
    if let Err(why) = check_x64_pe(&bytes) {
        panic!("{}: {why}", src.display());
    }
    let out = PathBuf::from(std::env::var_os("OUT_DIR").unwrap()).join("netrbin.sys");
    std::fs::write(&out, &bytes).expect("write embedded driver to OUT_DIR");
}

fn check_x64_pe(b: &[u8]) -> Result<(), &'static str> {
    if !b.starts_with(b"MZ") || b.len() < 0x40 {
        return Err("not a PE image (missing MZ header)");
    }
    let pe = u32::from_le_bytes(b[0x3c..0x40].try_into().unwrap()) as usize;
    if b.get(pe..pe + 4) != Some(b"PE\0\0") {
        return Err("not a PE image (missing PE signature)");
    }
    let machine = b.get(pe + 4..pe + 6).map(|m| u16::from_le_bytes([m[0], m[1]]));
    if machine != Some(IMAGE_FILE_MACHINE_AMD64) {
        return Err("not an x64 driver");
    }
    Ok(())
}
