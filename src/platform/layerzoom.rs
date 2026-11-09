//! Zooming the window's picture as a photo is zoomed: Core Animation
//! scales and moves the layer GPUI drew into, on the GPU, while GPUI draws
//! nothing new. The window's look is set as a map from where things were
//! drawn to where they are seen: p' = k·p + t, in points from the top left.
//!
//! Taking the map off must happen in the very frame the sharp picture
//! arrives, or the old picture shows unscaled (or the new one twice scaled)
//! for a frame. `present` does both in one Core Animation transaction: it
//! asks the layer to draw now, which makes GPUI draw a frame and present it
//! with the transaction (GPUI's `displayLayer:`), and sets the map with it.
//! As that runs a GPUI frame, it must be called where nothing of GPUI is
//! borrowed: from a task, outside any `update`.
//!
//! Scaled down, the picture leaves the window's edges bare: the window's
//! own background shows there, so it is set to the canvas's (`attach`).

/// The map from where things were drawn to where they are seen.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Map {
    pub k: f32,
    pub tx: f32,
    pub ty: f32,
}

impl Map {
    pub const NONE: Map = Map { k: 1., tx: 0., ty: 0. };
}

#[cfg(target_os = "macos")]
mod mac {
    use std::cell::RefCell;

    use objc2::encode::{Encode, Encoding};
    use objc2::rc::Retained;
    use objc2::runtime::{AnyClass, AnyObject, Bool};
    use objc2::msg_send;
    use objc2_foundation::{NSPoint, NSRect};
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};

    use super::Map;

    #[repr(C)]
    #[derive(Clone, Copy)]
    struct CGAffineTransform {
        a: f64,
        b: f64,
        c: f64,
        d: f64,
        tx: f64,
        ty: f64,
    }

    unsafe impl Encode for CGAffineTransform {
        const ENCODING: Encoding = Encoding::Struct("CGAffineTransform", &[Encoding::Double, Encoding::Double, Encoding::Double, Encoding::Double, Encoding::Double, Encoding::Double]);
    }

    thread_local! {
        /// The layer GPUI draws the deck's window into.
        static LAYER: RefCell<Option<Retained<AnyObject>>> = const { RefCell::new(None) };
        /// The window's background as last set, `#rrggbb`.
        static BACKDROP: RefCell<u32> = const { RefCell::new(u32::MAX) };
    }

    pub fn attach(window: &gpui::Window, backdrop: u32) {
        let Ok(handle) = HasWindowHandle::window_handle(window) else { return };
        let RawWindowHandle::AppKit(h) = handle.as_raw() else { return };
        let view: &AnyObject = unsafe { h.ns_view.cast().as_ref() };
        let layer: Option<Retained<AnyObject>> = unsafe { msg_send![view, layer] };
        LAYER.with(|l| {
            let mut l = l.borrow_mut();
            let same = matches!((l.as_ref(), layer.as_ref()), (Some(a), Some(b)) if std::ptr::eq(&**a, &**b));
            if !same {
                *l = layer;
            }
        });
        if BACKDROP.with(|b| b.replace(backdrop)) != backdrop {
            let ns_window: Option<Retained<AnyObject>> = unsafe { msg_send![view, window] };
            let Some(color_class) = AnyClass::get(c"NSColor") else { return };
            if let Some(w) = ns_window {
                let ch = |s: u32| f64::from((backdrop >> s) & 0xff) / 255.;
                unsafe {
                    let color: Retained<AnyObject> =
                        msg_send![color_class, colorWithSRGBRed: ch(16), green: ch(8), blue: ch(0), alpha: 1.0f64];
                    let _: () = msg_send![&*w, setBackgroundColor: &*color];
                }
            }
        }
    }

    pub fn available() -> bool {
        LAYER.with(|l| l.borrow().is_some())
    }

    /// The map as the layer's own transform: about its anchor point, in its
    /// own coordinates (y up unless its geometry is flipped).
    fn apply(layer: &AnyObject, m: Map) {
        let bounds: NSRect = unsafe { msg_send![layer, bounds] };
        let anchor: NSPoint = unsafe { msg_send![layer, anchorPoint] };
        let flipped: Bool = unsafe { msg_send![layer, isGeometryFlipped] };
        let (w, h) = (bounds.size.width, bounds.size.height);
        let k = f64::from(m.k);
        // The anchor, in points from the top left.
        let ax = anchor.x * w;
        let ay = if flipped.as_bool() { anchor.y * h } else { h - anchor.y * h };
        // Scaled about the anchor, then moved: p' = a + k(p - a) + d = k·p + t.
        let dx = f64::from(m.tx) - ax * (1. - k);
        let dy = f64::from(m.ty) - ay * (1. - k);
        let t = CGAffineTransform { a: k, b: 0., c: 0., d: k, tx: dx, ty: if flipped.as_bool() { dy } else { -dy } };
        let _: () = unsafe { msg_send![layer, setAffineTransform: t] };
    }

    fn transaction(f: impl FnOnce()) {
        let Some(tx) = AnyClass::get(c"CATransaction") else { return f() };
        unsafe {
            let _: () = msg_send![tx, begin];
            let _: () = msg_send![tx, setDisableActions: Bool::YES];
        }
        f();
        unsafe {
            let _: () = msg_send![tx, commit];
        }
    }

    pub fn set(m: Map) {
        LAYER.with(|l| {
            if let Some(layer) = l.borrow().as_ref() {
                transaction(|| apply(layer, m));
            }
        });
    }

    pub fn present(m: Map) {
        let layer = LAYER.with(|l| l.borrow().clone());
        let Some(layer) = layer else { return };
        transaction(|| {
            unsafe {
                let _: () = msg_send![&*layer, setNeedsDisplay];
                let _: () = msg_send![&*layer, displayIfNeeded];
            }
            apply(&layer, m);
        });
    }
}

/// Remembers the layer the window draws into, and gives the window the
/// canvas's background (`0xrrggbb`) for the edges a scaled-down picture
/// leaves bare: called each frame, cheap.
pub fn attach(window: &gpui::Window, backdrop: u32) {
    #[cfg(target_os = "macos")]
    mac::attach(window, backdrop);
    #[cfg(not(target_os = "macos"))]
    let _ = (window, backdrop);
}

/// Whether the window's picture can be zoomed as a photo here.
pub fn available() -> bool {
    #[cfg(target_os = "macos")]
    return mac::available();
    #[cfg(not(target_os = "macos"))]
    false
}

/// Sets the map on the picture now. Safe anywhere: it draws nothing.
pub fn set(m: Map) {
    #[cfg(target_os = "macos")]
    mac::set(m);
    #[cfg(not(target_os = "macos"))]
    let _ = m;
}

/// Has GPUI draw a frame now and shows it with this map, in one transaction.
/// Only where nothing of GPUI is borrowed (a task, outside `update`).
pub fn present(m: Map) {
    #[cfg(target_os = "macos")]
    mac::present(m);
    #[cfg(not(target_os = "macos"))]
    let _ = m;
}

/// The map that shows a picture drawn with camera `drawn` as camera `live` sees it.
pub fn map_between(drawn: crate::model::Camera, live: crate::model::Camera) -> Map {
    let k = live.s / drawn.s;
    Map { k, tx: (drawn.x - live.x) * live.s, ty: (drawn.y - live.y) * live.s }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Camera;

    #[test]
    fn the_map_puts_what_was_drawn_where_the_live_camera_sees_it() {
        let drawn = Camera { x: 100., y: 50., s: 1. };
        let live = Camera { x: 140., y: 70., s: 1.6 };
        let m = map_between(drawn, live);
        for w in [(100., 50.), (400., 300.), (-20., 900.)] {
            let (sx, sy) = drawn.to_screen(w.0, w.1);
            let (lx, ly) = live.to_screen(w.0, w.1);
            assert!((m.k * sx + m.tx - lx).abs() < 1e-3 && (m.k * sy + m.ty - ly).abs() < 1e-3, "{w:?}");
        }
        assert_eq!(map_between(live, live), Map::NONE);
    }
}
