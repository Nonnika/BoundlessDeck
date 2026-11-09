//! The monospace font of terminals, code and the chart table: JetBrains Mono's
//! Nerd Font when it is installed (the Mono variant, whose icons fit one cell),
//! else Menlo, which every Mac has. Looked up once at launch.
//!
//! And the index of installed families the style panel's font picker lists:
//! built off the UI thread at launch, since asking Core Text for every
//! family's localized name (楷体-简 for Kaiti SC) takes a moment.

use std::sync::OnceLock;

use gpui::{App, SharedString};

const MONO_FAMILIES: &[&str] = &["JetBrainsMono Nerd Font Mono", "Menlo"];
const FALLBACK: &str = "Menlo";

static MONO: OnceLock<SharedString> = OnceLock::new();

/// Picks the monospace font from those installed, and starts the index.
pub fn init(cx: &App) {
    #[cfg(target_os = "macos")]
    std::thread::spawn(|| {
        INDEX.get_or_init(mac::families);
    });
    #[cfg(not(target_os = "macos"))]
    {
        let mut names = cx.text_system().all_font_names();
        names.sort();
        names.dedup();
        INDEX.get_or_init(|| names.into_iter().map(|n| Family::new(n.clone(), n)).collect());
    }
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

/// A font family as the picker shows it.
#[derive(Clone, Debug, PartialEq)]
pub struct Family {
    /// The name styles keep and GPUI looks fonts up by.
    pub name: String,
    /// The name in the system's language, when it has one.
    pub shown: String,
    /// What a search matches: both names, lowercase.
    key: String,
}

impl Family {
    pub fn new(name: String, shown: String) -> Family {
        let key = format!("{} {}", name.to_lowercase(), shown.to_lowercase());
        Family { name, shown, key }
    }
}

static INDEX: OnceLock<Vec<Family>> = OnceLock::new();

/// The installed families, once indexed.
pub fn families() -> Option<&'static [Family]> {
    INDEX.get().map(Vec::as_slice)
}

/// The families whose names hold every word of the query, in index order.
pub fn search<'a>(all: &'a [Family], query: &str) -> Vec<&'a Family> {
    let words: Vec<String> = query.split_whitespace().map(str::to_lowercase).collect();
    all.iter().filter(|f| words.iter().all(|w| f.key.contains(w.as_str()))).collect()
}

/// Families sorted for the picker: Latin names first, then the rest, each by
/// the name shown; private system families (a leading dot) left out.
fn sorted(mut v: Vec<Family>) -> Vec<Family> {
    v.retain(|f| !f.name.starts_with('.') && !f.name.is_empty());
    v.sort_by(|a, b| {
        let latin = |f: &Family| f.shown.chars().next().is_some_and(|c| c.is_ascii());
        latin(b).cmp(&latin(a)).then_with(|| a.shown.to_lowercase().cmp(&b.shown.to_lowercase()))
    });
    v.dedup_by(|a, b| a.name == b.name);
    v
}

#[cfg(target_os = "macos")]
mod mac {
    use std::ffi::c_void;

    use objc2::rc::Retained;
    use objc2_foundation::{NSArray, NSDictionary, NSString};

    use super::{Family, sorted};

    #[link(name = "CoreText", kind = "framework")]
    unsafe extern "C" {
        static kCTFontFamilyNameAttribute: *const c_void;
        fn CTFontManagerCopyAvailableFontFamilyNames() -> *mut c_void;
        fn CTFontDescriptorCreateWithAttributes(attributes: *const c_void) -> *mut c_void;
        fn CTFontDescriptorCopyLocalizedAttribute(d: *const c_void, attribute: *const c_void, language: *mut *const c_void) -> *mut c_void;
    }

    #[link(name = "CoreFoundation", kind = "framework")]
    unsafe extern "C" {
        fn CFRelease(cf: *mut c_void);
    }

    /// Every family Core Text makes visible, with its localized name. Core
    /// Text is safe off the main thread, unlike AppKit's font manager.
    pub fn families() -> Vec<Family> {
        // SAFETY: the Copy functions return +1 references, owned here (CFArray
        // and CFString are toll-free bridged to NSArray and NSString); the
        // descriptor is released after use.
        unsafe {
            let raw = CTFontManagerCopyAvailableFontFamilyNames();
            let Some(names) = Retained::<NSArray<NSString>>::from_raw(raw.cast()) else { return Vec::new() };
            let key = &*(kCTFontFamilyNameAttribute as *const NSString);
            let mut out = Vec::with_capacity(names.count());
            for name in names.iter() {
                let attrs = NSDictionary::<NSString, NSString>::from_slices(&[key], &[&*name]);
                let desc = CTFontDescriptorCreateWithAttributes(Retained::as_ptr(&attrs).cast());
                let mut shown = None;
                if !desc.is_null() {
                    let local = CTFontDescriptorCopyLocalizedAttribute(desc, kCTFontFamilyNameAttribute, std::ptr::null_mut());
                    shown = Retained::<NSString>::from_raw(local.cast()).map(|s| s.to_string());
                    CFRelease(desc);
                }
                let name = name.to_string();
                out.push(Family::new(name.clone(), shown.filter(|s| !s.is_empty()).unwrap_or(name)));
            }
            sorted(out)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn search_matches_either_name_by_every_word() {
        let all = sorted(vec![
            Family::new("Kaiti SC".into(), "楷体-简".into()),
            Family::new(".SF NS".into(), ".SF NS".into()),
            Family::new("Avenir Next".into(), "Avenir Next".into()),
            Family::new("Avenir".into(), "Avenir".into()),
        ]);
        let names: Vec<&str> = all.iter().map(|f| f.name.as_str()).collect();
        assert_eq!(names, ["Avenir", "Avenir Next", "Kaiti SC"], "private families out, Latin first");
        let hit = |q: &str| search(&all, q).iter().map(|f| f.name.clone()).collect::<Vec<_>>();
        assert_eq!(hit("楷体"), ["Kaiti SC"]);
        assert_eq!(hit("kaiti"), ["Kaiti SC"]);
        assert_eq!(hit("avenir next"), ["Avenir Next"]);
        assert_eq!(hit(""), names);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn the_index_lists_installed_families() {
        let all = mac::families();
        if all.is_empty() {
            return;
        }
        assert!(all.iter().any(|f| f.name == "Helvetica"), "Helvetica is on every Mac");
        assert!(all.iter().all(|f| !f.name.starts_with('.')));
    }
}
