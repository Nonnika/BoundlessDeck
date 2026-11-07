//! The app's colors, light and dark, as the window's appearance says.

use gpui::{Hsla, Rgba, Window, WindowAppearance, rgb, rgba};

#[derive(Clone, Copy)]
pub struct Theme {
    pub dark: bool,
    pub background: Hsla,
    pub surface: Hsla,
    pub surface_hover: Hsla,
    pub border: Hsla,
    pub text: Hsla,
    pub muted: Hsla,
    pub accent: Hsla,
    pub accent_text: Hsla,
    pub danger: Hsla,
    pub shade: Hsla,
}

fn c(v: u32) -> Hsla {
    Hsla::from(rgb(v))
}
fn ca(v: u32) -> Hsla {
    Hsla::from(rgba(v))
}

impl Theme {
    pub fn of(window: &Window) -> Theme {
        match window.appearance() {
            WindowAppearance::Dark | WindowAppearance::VibrantDark => Theme::dark(),
            _ => Theme::light(),
        }
    }

    pub fn light() -> Theme {
        Theme {
            dark: false,
            background: c(0xf6f7f9),
            surface: c(0xffffff),
            surface_hover: c(0xeff1f5),
            border: c(0xdcdfe5),
            text: c(0x1d1f24),
            muted: c(0x6b7080),
            accent: c(0x3b6cf6),
            accent_text: c(0xffffff),
            danger: c(0xd9363e),
            shade: ca(0x00000014),
        }
    }

    pub fn dark() -> Theme {
        Theme {
            dark: true,
            background: c(0x0f1115),
            surface: c(0x1a1c22),
            surface_hover: c(0x262932),
            border: c(0x2f323b),
            text: c(0xe8e9ed),
            muted: c(0x8b90a0),
            accent: c(0x5a86fa),
            accent_text: c(0xffffff),
            danger: c(0xf0565e),
            shade: ca(0x00000066),
        }
    }
}

/// The palette of a sticky note (QuickLook), light or dark.
#[derive(Clone, Copy)]
pub struct Paper {
    pub top: Hsla,
    pub bottom: Hsla,
    pub header: Hsla,
    pub flap: Hsla,
    pub text: Hsla,
    pub muted: Hsla,
    pub link: Hsla,
    pub code: Hsla,
    pub shadow: Hsla,
}

impl Paper {
    pub fn of(t: &Theme) -> Paper {
        if t.dark {
            Paper {
                top: c(0x5e5220),
                bottom: c(0x544919),
                header: c(0x4b4115),
                flap: c(0x6f6229),
                text: c(0xf7ecbf),
                muted: c(0xcdbd80),
                link: c(0xffd27a),
                code: ca(0x00000040),
                shadow: ca(0x00000073),
            }
        } else {
            Paper {
                top: c(0xfff5a8),
                bottom: c(0xfdeb85),
                header: c(0xf8e06c),
                flap: c(0xeacd4f),
                text: c(0x4a3b10),
                muted: c(0x8a7332),
                link: c(0x9a5b00),
                code: ca(0x785a001f),
                shadow: ca(0x785a0038),
            }
        }
    }
}

/// The colors Markdown renders in.
#[derive(Clone, Copy)]
pub struct MdColors {
    pub text: Hsla,
    pub muted: Hsla,
    pub link: Hsla,
    pub code_bg: Hsla,
    pub rule: Hsla,
}

impl MdColors {
    pub fn of_theme(t: &Theme) -> MdColors {
        MdColors { text: t.text, muted: t.muted, link: t.accent, code_bg: t.surface_hover, rule: t.border }
    }
    pub fn of_paper(p: &Paper) -> MdColors {
        MdColors { text: p.text, muted: p.muted, link: p.link, code_bg: p.code, rule: p.flap }
    }
}

pub fn rgba_u8(r: u8, g: u8, b: u8, a: f32) -> Hsla {
    Hsla::from(Rgba { r: r as f32 / 255., g: g as f32 / 255., b: b as f32 / 255., a })
}
