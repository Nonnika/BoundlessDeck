//! The app's colors, light and dark, as the window's appearance says.

use std::sync::atomic::{AtomicUsize, Ordering};

use gpui::{Hsla, Rgba, Window, WindowAppearance, rgb, rgba};

/// An accent the user can pick in 设置: its id (kept in settings.json), its
/// name, its color in light and in dark mode, and the color of text on it.
pub struct Accent {
    pub id: &'static str,
    pub name: &'static str,
    pub light: u32,
    pub dark: u32,
    pub text: u32,
}

/// The accents to pick from, the classic ones of macOS; the first is the default.
pub const ACCENTS: &[Accent] = &[
    Accent { id: "blue", name: "蓝色", light: 0x3b6cf6, dark: 0x5a86fa, text: 0xffffff },
    Accent { id: "purple", name: "紫色", light: 0x8a4fd8, dark: 0xa275ec, text: 0xffffff },
    Accent { id: "pink", name: "粉色", light: 0xd9407f, dark: 0xeb659d, text: 0xffffff },
    Accent { id: "red", name: "红色", light: 0xd9363e, dark: 0xef5a60, text: 0xffffff },
    Accent { id: "orange", name: "橙色", light: 0xe0731c, dark: 0xe77210, text: 0xffffff },
    Accent { id: "yellow", name: "黄色", light: 0xd9a400, dark: 0xf2c230, text: 0x1d1f24 },
    Accent { id: "green", name: "绿色", light: 0x2f9e4f, dark: 0x3aa658, text: 0xffffff },
    Accent { id: "teal", name: "青色", light: 0x1b93a4, dark: 0x34a1b0, text: 0xffffff },
    Accent { id: "graphite", name: "石墨", light: 0x6b7080, dark: 0x8c93a3, text: 0xffffff },
];

/// Which of `ACCENTS` the app draws with.
static ACCENT: AtomicUsize = AtomicUsize::new(0);

/// Draws with the accent of this id from now on; an unknown id or none is the default.
pub fn set_accent(id: Option<&str>) {
    ACCENT.store(accent_index(id), Ordering::Relaxed);
}

/// Where the accent of this id is in `ACCENTS`; the default for none or one unknown.
fn accent_index(id: Option<&str>) -> usize {
    id.and_then(|id| ACCENTS.iter().position(|a| a.id == id)).unwrap_or(0)
}

/// The accent the app draws with.
pub fn accent() -> &'static Accent {
    &ACCENTS[ACCENT.load(Ordering::Relaxed).min(ACCENTS.len() - 1)]
}

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
            accent: c(accent().light),
            accent_text: c(accent().text),
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
            accent: c(accent().dark),
            accent_text: c(accent().text),
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

#[cfg(test)]
mod accent_tests {
    use super::*;

    #[test]
    fn accents_are_found_by_id_and_default_to_blue() {
        assert_eq!(accent_index(None), 0);
        assert_eq!(accent_index(Some("nope")), 0);
        assert_eq!(ACCENTS[accent_index(Some("orange"))].id, "orange");
        let mut ids: Vec<_> = ACCENTS.iter().map(|a| a.id).collect();
        ids.dedup();
        assert_eq!(ids.len(), ACCENTS.len());
    }

    #[test]
    fn text_on_each_accent_reads() {
        // WCAG relative luminance and contrast; buttons want at least 3:1.
        let lum = |v: u32| {
            let ch = |s: u32| {
                let c = ((v >> s) & 0xff) as f64 / 255.;
                if c <= 0.03928 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) }
            };
            0.2126 * ch(16) + 0.7152 * ch(8) + 0.0722 * ch(0)
        };
        let contrast = |a: u32, b: u32| {
            let (x, y) = (lum(a), lum(b));
            (x.max(y) + 0.05) / (x.min(y) + 0.05)
        };
        for a in ACCENTS {
            for bg in [a.light, a.dark] {
                assert!(contrast(bg, a.text) >= 3., "{} {bg:06x}: {:.2}", a.id, contrast(bg, a.text));
            }
        }
    }
}
