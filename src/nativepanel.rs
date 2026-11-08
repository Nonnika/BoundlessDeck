//! The system's Colors panel, for 文字 and 图形: it opens on the block's
//! color, and what is picked in it comes back through a channel, to be
//! applied on the UI thread. The panel talks to a small object of ours set as
//! its target (`changeColor:`). Fonts are picked in the style panel itself.

/// What was picked in a panel.
#[derive(Clone, Debug, PartialEq)]
pub enum Pick {
    /// A color, as `#rrggbb`.
    Color(String),
}

#[cfg(target_os = "macos")]
mod mac {
    use std::cell::RefCell;

    use objc2::rc::Retained;
    use objc2::runtime::{AnyClass, AnyObject, NSObject};
    use objc2::{DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send, sel};

    use super::Pick;

    pub struct Ivars {
        tx: smol::channel::Sender<Pick>,
    }

    define_class!(
        // SAFETY: NSObject has no subclassing requirements; Target has no Drop.
        #[unsafe(super(NSObject))]
        #[thread_kind = MainThreadOnly]
        #[name = "BoundlessDeckPanelTarget"]
        #[ivars = Ivars]
        pub struct Target;

        impl Target {
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
        /// The panel's target: it does not keep it itself.
        static TARGET: RefCell<Option<Retained<Target>>> = const { RefCell::new(None) };
    }

    fn target(tx: &smol::channel::Sender<Pick>, mtm: MainThreadMarker) -> Retained<Target> {
        TARGET.with(|t| {
            let mut t = t.borrow_mut();
            if let Some(old) = t.as_ref().filter(|o| o.ivars().tx.same_channel(tx)) {
                return old.clone();
            }
            let this = Target::alloc(mtm).set_ivars(Ivars { tx: tx.clone() });
            let new: Retained<Target> = unsafe { msg_send![super(this), init] };
            *t = Some(new.clone());
            new
        })
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

/// Opens the Colors panel on this color (`#rrggbb`); what is picked comes through `tx`.
pub fn color_panel(hex: &str, tx: &smol::channel::Sender<Pick>) {
    #[cfg(target_os = "macos")]
    mac::color_panel(hex, tx);
    #[cfg(not(target_os = "macos"))]
    let _ = (hex, tx);
}
