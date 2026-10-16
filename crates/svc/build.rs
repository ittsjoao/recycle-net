//! Embeds `netrbin.sys` into the binary so `install` needs no external file.
//! Source: `$RNETRBIN_DRIVER`, else `<workspace>/driver/netrbin.sys`. The build
//! fails without it: an exe that cannot install the driver is not shippable.
//! Also embeds the app icon: `icons/rnetrbin.ico` as the exe resource (Explorer,
//! shortcuts) and `icons/rnetrbin.png` as raw RGBA for the window (taskbar).
use std::path::PathBuf;

const IMAGE_FILE_MACHINE_AMD64: u16 = 0x8664;

fn main() {
    embed_icon();
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

fn embed_icon() {
    println!("cargo:rerun-if-changed=rnetrbin.rc");
    println!("cargo:rerun-if-changed=../../icons/rnetrbin.ico");
    embed_resource::compile("rnetrbin.rc", embed_resource::NONE)
        .manifest_optional()
        .expect("compile rnetrbin.rc");

    let png = PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").unwrap()).join("../../icons/rnetrbin.png");
    println!("cargo:rerun-if-changed={}", png.display());
    let file = std::fs::File::open(&png).unwrap_or_else(|e| panic!("{}: {e}", png.display()));
    let mut decoder = png::Decoder::new(file);
    decoder.set_transformations(png::Transformations::EXPAND);
    let mut reader = decoder.read_info().expect("read icon png header");
    let mut rgba = vec![0; reader.output_buffer_size()];
    let info = reader.next_frame(&mut rgba).expect("decode icon png");
    assert!(
        info.color_type == png::ColorType::Rgba && info.bit_depth == png::BitDepth::Eight,
        "{}: expected 8-bit RGBA",
        png.display()
    );
    assert_eq!(info.width, 256, "{}: expected 256x256", png.display());
    assert_eq!(info.height, 256, "{}: expected 256x256", png.display());
    rgba.truncate(info.buffer_size());
    let out = PathBuf::from(std::env::var_os("OUT_DIR").unwrap()).join("icon.rgba");
    std::fs::write(&out, &rgba).expect("write icon to OUT_DIR");
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
