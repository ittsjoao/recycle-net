//! Material Icons (Google, Apache 2.0) embedded in the exe. Names are resolved
//! through the official `.codepoints` file, so no codepoint is typed by hand.
use std::collections::HashMap;
use std::sync::OnceLock;

use iced::widget::{text, Text};
use iced::{Color, Font};

pub const FONT_BYTES: &[u8] = include_bytes!("../../assets/MaterialIcons-Regular.ttf");
const CODEPOINTS: &str = include_str!("../../assets/MaterialIcons-Regular.codepoints");
/// Family name inside the TTF.
pub const FONT: Font = Font::with_name("Material Icons");

/// Every icon the window uses (checked against the font by a test).
pub const USED: &[&str] = &[
    "restore", "drive_file_move", "delete_forever", "history", "refresh", "search",
    "create_new_folder", "rule", "folder_off", "save", "install_desktop", "play_circle",
    "stop_circle", "delete", "folder_special", "folder", "folder_open", "description",
    "arrow_upward", "check_circle", "error", "info", "close",
];

fn map() -> &'static HashMap<&'static str, char> {
    static MAP: OnceLock<HashMap<&'static str, char>> = OnceLock::new();
    MAP.get_or_init(|| {
        CODEPOINTS
            .lines()
            .filter_map(|l| {
                let (name, hex) = l.split_once(' ')?;
                Some((name, char::from_u32(u32::from_str_radix(hex.trim(), 16).ok()?)?))
            })
            .collect()
    })
}

/// The font character for a Material icon name. Panics on an unknown name,
/// which the `every_used_icon_exists_in_the_font` test rules out.
pub fn glyph(name: &str) -> char {
    *map().get(name).unwrap_or_else(|| panic!("unknown Material icon {name:?}"))
}

pub fn icon<'a>(name: &str, size: f32, color: Color) -> Text<'a> {
    text(glyph(name).to_string()).font(FONT).size(size).color(color)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_used_icon_exists_in_the_font() {
        for name in USED {
            let _ = glyph(name);
        }
        assert_eq!(glyph("delete"), '\u{e872}');
        assert_eq!(glyph("restore"), '\u{e8b3}');
    }

    #[test]
    fn font_is_a_truetype_file() {
        assert_eq!(&FONT_BYTES[..4], &[0, 1, 0, 0]);
    }
}
