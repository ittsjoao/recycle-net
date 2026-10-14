//! Rust (default), Branco and Preto palettes, copied from the approved mockup.
use iced::theme::Palette;
use iced::widget::{button, container};
use iced::{Background, Border, Color, Theme};

use crate::config::ThemeName;

/// Colors used by our own widgets (iced's palette only covers its defaults).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Pal {
    pub bg: Color,
    pub fg: Color,
    pub muted: Color,
    pub panel: Color,
    pub line: Color,
    pub soft: Color,
    pub acc: Color,
    pub acc_fg: Color,
    pub sel: Color,
    pub ok: Color,
    pub danger: Color,
}

const fn hex(rgb: u32) -> Color {
    Color::from_rgb8((rgb >> 16) as u8, (rgb >> 8) as u8, rgb as u8)
}

pub fn pal(t: ThemeName) -> Pal {
    match t {
        ThemeName::Rust => Pal {
            bg: hex(0xe1e1db), fg: hex(0x262625), muted: hex(0x6e6b5e), panel: hex(0xecebe5),
            line: hex(0xc4c3b8), soft: hex(0xd4d3cb), acc: hex(0xb7410e), acc_fg: Color::WHITE,
            sel: hex(0xf0d5c2), ok: hex(0x3d5a1e), danger: hex(0xa3260f),
        },
        ThemeName::Branco => Pal {
            bg: Color::WHITE, fg: hex(0x1f2328), muted: hex(0x59636e), panel: hex(0xf6f8fa),
            line: hex(0xd1d9e0), soft: hex(0xeaeef2), acc: hex(0x0969da), acc_fg: Color::WHITE,
            sel: hex(0xddf4ff), ok: hex(0x1a7f37), danger: hex(0xcf222e),
        },
        ThemeName::Preto => Pal {
            bg: hex(0x0d1117), fg: hex(0xe6edf3), muted: hex(0x9198a1), panel: hex(0x151b23),
            line: hex(0x30363d), soft: hex(0x21262d), acc: hex(0x2f81f7), acc_fg: Color::WHITE,
            sel: hex(0x1f3a5f), ok: hex(0x3fb950), danger: hex(0xf85149),
        },
    }
}

/// iced theme so default widgets (text inputs, scrollbars, checkboxes) match.
pub fn iced_theme(t: ThemeName) -> Theme {
    let p = pal(t);
    let name = match t {
        ThemeName::Rust => "Rust",
        ThemeName::Branco => "Branco",
        ThemeName::Preto => "Preto",
    };
    Theme::custom(
        name,
        Palette { background: p.bg, text: p.fg, primary: p.acc, success: p.ok, danger: p.danger, ..Palette::LIGHT },
    )
}

/// Borderless button (ribbon, tree, list rows); `selected` paints the row.
pub fn flat_button(p: Pal, selected: bool) -> impl Fn(&Theme, button::Status) -> button::Style {
    move |_, status| {
        let bg = match (selected, status) {
            (true, _) => Some(p.sel),
            (false, button::Status::Hovered | button::Status::Pressed) => Some(p.soft),
            _ => None,
        };
        button::Style {
            background: bg.map(Background::Color),
            text_color: if matches!(status, button::Status::Disabled) { p.muted } else { p.fg },
            border: Border::default(),
            ..Default::default()
        }
    }
}

/// Tab of the ribbon bar. `accent` is the "Arquivo" tab.
pub fn tab_button(p: Pal, active: bool, accent: bool) -> impl Fn(&Theme, button::Status) -> button::Style {
    move |_, _| {
        let (bg, fg) = match (accent, active) {
            (true, _) => (p.acc, p.acc_fg),
            (false, true) => (p.bg, p.fg),
            (false, false) => (p.panel, p.fg),
        };
        button::Style {
            background: Some(Background::Color(bg)),
            text_color: fg,
            border: Border { color: if active { p.line } else { Color::TRANSPARENT }, width: 1.0, radius: 0.0.into() },
            ..Default::default()
        }
    }
}

/// Plain area with a background color and a thin border.
pub fn surface(p: Pal, bg: Color) -> impl Fn(&Theme) -> container::Style {
    move |_| container::Style {
        background: Some(Background::Color(bg)),
        text_color: Some(p.fg),
        border: Border { color: p.line, width: 1.0, radius: 0.0.into() },
        ..Default::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn palettes_match_the_mockup() {
        assert_eq!(pal(ThemeName::Rust).bg, Color::from_rgb8(0xe1, 0xe1, 0xdb));
        assert_eq!(pal(ThemeName::Rust).acc, Color::from_rgb8(0xb7, 0x41, 0x0e));
        assert_eq!(pal(ThemeName::Branco).bg, Color::WHITE);
        assert_eq!(pal(ThemeName::Preto).bg, Color::from_rgb8(0x0d, 0x11, 0x17));
    }
}
