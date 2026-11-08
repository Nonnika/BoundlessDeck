//! The monospace font of terminals, code and the chart table: JetBrains Mono's
//! Nerd Font when it is installed (the Mono variant, whose icons fit one cell),
//! else Menlo, which every Mac has. Looked up once at launch.

use std::sync::OnceLock;

use gpui::{App, SharedString};

const MONO_FAMILIES: &[&str] = &["JetBrainsMono Nerd Font Mono", "Menlo"];
const FALLBACK: &str = "Menlo";

static MONO: OnceLock<SharedString> = OnceLock::new();

/// Picks the monospace font from those installed.
pub fn init(cx: &App) {
    MONO.get_or_init(|| {
        let names = cx.text_system().all_font_names();
        let found = MONO_FAMILIES.iter().find(|f| names.iter().any(|n| n == *f));
        SharedString::new_static(found.copied().unwrap_or(FALLBACK))
    });
}

/// The monospace font family.
pub fn mono() -> SharedString {
    MONO.get().cloned().unwrap_or(SharedString::new_static(FALLBACK))
}
