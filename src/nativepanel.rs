//! The system's Fonts and Colors panels, for 文字 and 图形: the panel opens
//! on the block's font or color, and what is picked in it comes back through
//! a channel, to be applied on the UI thread. The panels talk to a small
//! object of ours set as their target (`changeFont:`, `changeColor:`).

/// What was picked in a panel.
#[derive(Clone, Debug, PartialEq)]
pub enum Pick {
    /// A font: its family (empty for the system font), size in points, CSS weight, italic.
    Font { family: String, size: f32, weight: u16, italic: bool },
    /// A color, as `#rrggbb`.
    Color(String),
}

/// AppKit's font weights (0–15) as CSS weights, and back.
pub fn css_weight(ns: isize) -> u16 {
    match ns {
        ..=2 => 100,
        3 => 200,
        4 => 300,
        5 => 400,
        6 => 500,
        7 | 8 => 600,
        9 => 700,
        10 => 800,
        _ => 900,
    }
}

pub fn ns_weight(css: u16) -> isize {
    match css {
        0..=150 => 2,
        151..=250 => 3,
        251..=350 => 4,
        351..=450 => 5,
        451..=550 => 6,
        551..=650 => 8,
        651..=750 => 9,
        751..=850 => 10,
        _ => 11,
    }
}

#[cfg(target_os = "macos")]
mod mac {
    use std::cell::RefCell;

    use objc2::rc::Retained;
    use objc2::runtime::{AnyClass, AnyObject, NSObject};
    use objc2::{DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send, sel};
    use objc2_foundation::NSString;

    use super::{Pick, css_weight, ns_weight};

    pub struct Ivars {
        tx: smol::channel::Sender<Pick>,
        /// The font the panel shows, which its changes are made to.
        font: RefCell<Option<Retained<AnyObject>>>,
    }

    define_class!(
        // SAFETY: NSObject has no subclassing requirements; Target has no Drop.
        #[unsafe(super(NSObject))]
        #[thread_kind = MainThreadOnly]
        #[name = "BoundlessDeckPanelTarget"]
        #[ivars = Ivars]
        pub struct Target;

        impl Target {
            #[unsafe(method(changeFont:))]
            fn change_font(&self, sender: &AnyObject) {
                let Some(base) = self.ivars().font.borrow().clone() else { return };
                let font: Option<Retained<AnyObject>> = unsafe { msg_send![sender, convertFont: &*base] };
                let Some(font) = font else { return };
                let family: Retained<NSString> = unsafe { msg_send![&*font, familyName] };
                let size: f64 = unsafe { msg_send![&*font, pointSize] };
                let weight: isize = unsafe { msg_send![sender, weightOfFont: &*font] };
                let traits: usize = unsafe { msg_send![sender, traitsOfFont: &*font] };
                let family = family.to_string();
                // The system font's family is a private name: kept as "the system font".
                let family = if family.starts_with('.') { String::new() } else { family };
                *self.ivars().font.borrow_mut() = Some(font);
                let _ = self.ivars().tx.try_send(Pick::Font {
                    family,
                    size: size as f32,
                    weight: css_weight(weight),
                    italic: traits & 1 != 0,
                });
            }

            #[unsafe(method(changeColor:))]
            fn change_color(&self, sender: &AnyObject) {
                let color: Retained<AnyObject> = unsafe { msg_send![sender, color] };
                let Some(space) = AnyClass::get(c"NSColorSpace") else { return };
                let srgb: Retained<AnyObject> = unsafe { msg_send![space, sRGBColorSpace] };
                let rgb: Option<Retained<AnyObject>> = unsafe { msg_send![&*color, colorUsingColorSpace: &*srgb] };
                let Some(rgb) = rgb else { return };
                let ch = |v: f64| (v.clamp(0., 1.) * 255.).round() as u8;
                let (r, g, b): (f64, f64, f64) =
                    unsafe { (msg_send![&*rgb, redComponent], msg_send![&*rgb, greenComponent], msg_send![&*rgb, blueComponent]) };
                let _ = self.ivars().tx.try_send(Pick::Color(format!("#{:02x}{:02x}{:02x}", ch(r), ch(g), ch(b))));
            }
        }
    );

    thread_local! {
        /// The panels' target: they do not keep it themselves.
        static TARGET: RefCell<Option<Retained<Target>>> = const { RefCell::new(None) };
    }

    fn target(tx: &smol::channel::Sender<Pick>, mtm: MainThreadMarker) -> Retained<Target> {
        TARGET.with(|t| {
            let mut t = t.borrow_mut();
            if let Some(old) = t.as_ref().filter(|o| o.ivars().tx.same_channel(tx)) {
                return old.clone();
            }
            let this = Target::alloc(mtm).set_ivars(Ivars { tx: tx.clone(), font: RefCell::new(None) });
            let new: Retained<Target> = unsafe { msg_send![super(this), init] };
            *t = Some(new.clone());
            new
        })
    }

    pub fn font_panel(family: &str, size: f32, weight: u16, italic: bool, tx: &smol::channel::Sender<Pick>) {
        let Some(mtm) = MainThreadMarker::new() else { return };
        let (Some(fm_class), Some(font_class)) = (AnyClass::get(c"NSFontManager"), AnyClass::get(c"NSFont")) else {
            return;
        };
        let target = target(tx, mtm);
        unsafe {
            let fm: Retained<AnyObject> = msg_send![fm_class, sharedFontManager];
            let size = size as f64;
            let font: Option<Retained<AnyObject>> = if family.is_empty() {
                None
            } else {
                let traits: usize = if italic { 1 } else { 0 };
                msg_send![&*fm, fontWithFamily: &*NSString::from_str(family), traits: traits, weight: ns_weight(weight), size: size]
            };
            let font = match font {
                Some(f) => f,
                None => {
                    let sys: Retained<AnyObject> = msg_send![font_class, systemFontOfSize: size];
                    sys
                }
            };
            *target.ivars().font.borrow_mut() = Some(font.clone());
            let _: () = msg_send![&*fm, setTarget: &*target];
            let _: () = msg_send![&*fm, setAction: sel!(changeFont:)];
            let _: () = msg_send![&*fm, setSelectedFont: &*font, isMultiple: false];
            let none: Option<&AnyObject> = None;
            let _: () = msg_send![&*fm, orderFrontFontPanel: none];
        }
    }

    pub fn color_panel(hex: &str, tx: &smol::channel::Sender<Pick>) {
        let Some(mtm) = MainThreadMarker::new() else { return };
        let (Some(panel_class), Some(color_class)) = (AnyClass::get(c"NSColorPanel"), AnyClass::get(c"NSColor")) else {
            return;
        };
        let target = target(tx, mtm);
        let v = hex.trim_start_matches('#');
        let rgb = u32::from_str_radix(v, 16).ok().filter(|_| v.len() == 6).unwrap_or(0x3e63dd);
        let ch = |s: u32| ((rgb >> s) & 0xff) as f64 / 255.;
        unsafe {
            let panel: Retained<AnyObject> = msg_send![panel_class, sharedColorPanel];
            let color: Retained<AnyObject> =
                msg_send![color_class, colorWithSRGBRed: ch(16), green: ch(8), blue: ch(0), alpha: 1.0f64];
            // Set before the target, so showing the block's color is not taken as a pick.
            let none: Option<&AnyObject> = None;
            let _: () = msg_send![&*panel, setTarget: none];
            let _: () = msg_send![&*panel, setColor: &*color];
            let _: () = msg_send![&*panel, setContinuous: true];
            let _: () = msg_send![&*panel, setTarget: &*target];
            let _: () = msg_send![&*panel, setAction: sel!(changeColor:)];
            let _: () = msg_send![&*panel, orderFront: none];
        }
    }
}

/// Opens the Fonts panel on this font; what is picked comes through `tx`.
pub fn font_panel(family: &str, size: f32, weight: u16, italic: bool, tx: &smol::channel::Sender<Pick>) {
    #[cfg(target_os = "macos")]
    mac::font_panel(family, size, weight, italic, tx);
    #[cfg(not(target_os = "macos"))]
    let _ = (family, size, weight, italic, tx);
}

/// Opens the Colors panel on this color (`#rrggbb`); what is picked comes through `tx`.
pub fn color_panel(hex: &str, tx: &smol::channel::Sender<Pick>) {
    #[cfg(target_os = "macos")]
    mac::color_panel(hex, tx);
    #[cfg(not(target_os = "macos"))]
    let _ = (hex, tx);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn weights_go_both_ways() {
        for css in [100, 200, 300, 400, 500, 600, 700, 800, 900] {
            assert_eq!(css_weight(ns_weight(css)), css, "{css}");
        }
        assert_eq!(css_weight(5), 400, "AppKit's regular is CSS's");
        assert_eq!(css_weight(9), 700, "and bold, bold");
    }
}
