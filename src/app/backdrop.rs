//! A blur over the whole window but a hole: a panel drawn in the window
//! (the welcome) stands sharp in the hole, and all behind it, web pages too,
//! is blurred. GPUI draws no blur, so it is native views over GPUI's, whose
//! layers blur what lies behind them (`backgroundFilters`, a Core Image
//! Gaussian blur). They are four bands around the hole rather than one view
//! with the hole cut out: a view's blur takes in all it covers, so one over
//! the panel too would smear the panel's colors out past its edge. The
//! hole's rounded corners are four small views more, each masked to the
//! part of its square outside the corner's arc. The views take no clicks
//! (`hitTest:` is nil): input goes on to GPUI.

/// Blur over parts of a window: its views are made as needed, and taken off
/// the window when dropped.
#[derive(Default)]
pub struct Backdrop {
    #[cfg(target_os = "macos")]
    mac: mac::Views,
}

/// A rounded rect in the window left clear: x, y, w, h and the corner radius, in points.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Hole {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
    pub r: f32,
}

/// A part of the window to blur: x, y, w, h, and maybe a disc left out of
/// it (its center, from the rect's top-left, and its radius).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Patch {
    pub rect: (f32, f32, f32, f32),
    pub cut: Option<(f32, f32, f32)>,
}

/// What blurs all of a window `vw` by `vh` but `hole`: the four bands
/// around its rect (above, below, left, right; the sides run its height),
/// and its four corners' squares less the rounded hole's arcs.
pub fn around(hole: Hole, vw: f32, vh: f32) -> Vec<Patch> {
    let (x0, y0) = (hole.x.clamp(0., vw), hole.y.clamp(0., vh));
    let (x1, y1) = ((hole.x + hole.w).clamp(x0, vw), (hole.y + hole.h).clamp(y0, vh));
    let bands = [(0., 0., vw, y0), (0., y1, vw, vh - y1), (0., y0, x0, y1 - y0), (x1, y0, vw - x1, y1 - y0)];
    let mut out: Vec<Patch> = bands.into_iter().map(|rect| Patch { rect, cut: None }).collect();
    let r = hole.r.min(hole.w / 2.).min(hole.h / 2.);
    if r >= 1. {
        let (x, y, w, h) = (hole.x, hole.y, hole.w, hole.h);
        for (cx, cy, ox, oy) in [(x, y, r, r), (x + w - r, y, 0., r), (x, y + h - r, r, 0.), (x + w - r, y + h - r, 0., 0.)] {
            out.push(Patch { rect: (cx, cy, r, r), cut: Some((ox, oy, r)) });
        }
    }
    out
}

impl Backdrop {
    /// Blurs the window by `radius` points, all but `hole`; 0 takes it off.
    pub fn around(&mut self, window: &gpui::Window, radius: f64, hole: Hole) {
        let vs = window.viewport_size();
        self.patches(window, radius, &around(hole, f32::from(vs.width), f32::from(vs.height)));
    }

    /// Blurs `rects` of the window (x, y, w, h) by `radius` points; 0 takes it off.
    pub fn over(&mut self, window: &gpui::Window, radius: f64, rects: &[(f32, f32, f32, f32)]) {
        let patches: Vec<Patch> = rects.iter().map(|&rect| Patch { rect, cut: None }).collect();
        self.patches(window, radius, &patches);
    }

    fn patches(&mut self, window: &gpui::Window, radius: f64, patches: &[Patch]) {
        #[cfg(target_os = "macos")]
        self.mac.place(window, if radius < 0.05 { &[] } else { patches }, radius);
        #[cfg(not(target_os = "macos"))]
        let _ = (window, radius, patches);
    }
}

#[cfg(target_os = "macos")]
mod mac {
    use std::ffi::c_void;

    use objc2::encode::{Encoding, RefEncode};
    use objc2::rc::Retained;
    use objc2::runtime::{AnyClass, AnyObject};
    use objc2::{MainThreadMarker, MainThreadOnly, define_class, msg_send};
    use objc2_app_kit::NSView;
    use objc2_core_image::CIFilter;
    use objc2_foundation::{NSArray, NSNumber, NSObjectNSKeyValueCoding, NSPoint, NSRect, NSSize, NSString};
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};

    use super::Patch;

    /// CoreGraphics' path, which a shape layer takes.
    #[repr(C)]
    struct CGPath {
        _private: [u8; 0],
    }

    // SAFETY: a CGPathRef is a pointer to the opaque struct CGPath.
    unsafe impl RefEncode for CGPath {
        const ENCODING_REF: Encoding = Encoding::Pointer(&Encoding::Struct("CGPath", &[]));
    }

    #[link(name = "CoreGraphics", kind = "framework")]
    unsafe extern "C" {
        fn CGPathCreateMutable() -> *mut CGPath;
        fn CGPathAddRect(path: *mut CGPath, m: *const c_void, rect: NSRect);
        fn CGPathAddEllipseInRect(path: *mut CGPath, m: *const c_void, rect: NSRect);
        fn CGPathRelease(path: *mut CGPath);
    }

    /// A layer showing only `w` by `h` less a disc at (cx, cy) of radius r, in
    /// the layer's own coordinates; none if Core Animation has no shape layers.
    fn mask_outside(w: f64, h: f64, cx: f64, cy: f64, r: f64) -> Option<Retained<AnyObject>> {
        let cls = AnyClass::get(c"CAShapeLayer")?;
        let mask: Retained<AnyObject> = unsafe { msg_send![cls, layer] };
        unsafe {
            let path = CGPathCreateMutable();
            CGPathAddRect(path, std::ptr::null(), NSRect::new(NSPoint::new(0., 0.), NSSize::new(w, h)));
            CGPathAddEllipseInRect(path, std::ptr::null(), NSRect::new(NSPoint::new(cx - r, cy - r), NSSize::new(2. * r, 2. * r)));
            // Even-odd: the square filled, the disc where it overlaps it not.
            let _: () = msg_send![&*mask, setFillRule: &*NSString::from_str("even-odd")];
            let _: () = msg_send![&*mask, setFrame: NSRect::new(NSPoint::new(0., 0.), NSSize::new(w, h))];
            let _: () = msg_send![&*mask, setPath: path as *const CGPath];
            CGPathRelease(path);
        }
        Some(mask)
    }

    define_class!(
        // SAFETY: NSView has no subclassing requirements; BlurView has no Drop.
        #[unsafe(super(NSView))]
        #[thread_kind = MainThreadOnly]
        #[name = "BoundlessDeckBackdropView"]
        struct BlurView;

        impl BlurView {
            /// Never the view hit: clicks and wheels go on to GPUI's view under it.
            #[unsafe(method(hitTest:))]
            fn hit_test(&self, _point: NSPoint) -> *mut NSView {
                std::ptr::null_mut()
            }
        }
    );

    /// The blur views, made as needed and kept.
    #[derive(Default)]
    pub struct Views {
        views: Vec<Retained<BlurView>>,
        /// What each view was last placed as.
        placed: Vec<Option<Patch>>,
        radius: f64,
        shown: usize,
    }

    /// GPUI's view of the window.
    fn canvas(window: &gpui::Window) -> Option<&NSView> {
        let handle = HasWindowHandle::window_handle(window).ok()?;
        let RawWindowHandle::AppKit(h) = handle.as_raw() else { return None };
        Some(unsafe { h.ns_view.cast().as_ref() })
    }

    impl Views {
        pub fn place(&mut self, window: &gpui::Window, patches: &[Patch], radius: f64) {
            if patches.is_empty() && self.shown == 0 {
                return;
            }
            let Some(mtm) = MainThreadMarker::new() else { return };
            let Some(canvas) = canvas(window) else { return };
            while self.views.len() < patches.len() {
                let v: Retained<BlurView> = unsafe { msg_send![BlurView::alloc(mtm), initWithFrame: NSRect::ZERO] };
                v.setWantsLayer(true);
                v.setLayerUsesCoreImageFilters(true);
                // The blur stays in its rect: unclipped, it spreads over the whole window.
                if let Some(layer) = v.layer() {
                    let _: () = unsafe { msg_send![&*layer, setMasksToBounds: true] };
                }
                v.setHidden(true);
                // Over GPUI's view and the web pages laid on it.
                canvas.addSubview(&v);
                self.views.push(v);
                self.placed.push(None);
            }
            let size = canvas.bounds().size;
            let flipped = canvas.isFlipped();
            let renew = (radius - self.radius).abs() > 0.01;
            let filters = (!patches.is_empty()).then(|| {
                let blur = unsafe { CIFilter::filterWithName(&NSString::from_str("CIGaussianBlur")) }?;
                unsafe {
                    blur.setValue_forKey(Some(&NSNumber::new_f64(radius)), &NSString::from_str("inputRadius"));
                }
                Some(NSArray::from_retained_slice(&[blur]))
            });
            for (i, v) in self.views.iter().enumerate() {
                match patches.get(i).filter(|p| p.rect.2 >= 1. && p.rect.3 >= 1.) {
                    Some(&patch) => {
                        let (x, y, w, h) = patch.rect;
                        let (x, y, w, h) = (x as f64, y as f64, w as f64, h as f64);
                        let y = if flipped { y } else { size.height - y - h };
                        let frame = NSRect::new(NSPoint::new(x, y), NSSize::new(w, h));
                        if v.frame() != frame {
                            v.setFrame(frame);
                        }
                        if self.placed[i] != Some(patch)
                            && let Some(layer) = v.layer()
                        {
                            // The view is not flipped: its layer's y runs up.
                            let mask = patch.cut.and_then(|(cx, cy, r)| {
                                let cy = if v.isFlipped() { cy as f64 } else { h - cy as f64 };
                                mask_outside(w, h, cx as f64, cy, r as f64)
                            });
                            let mask: *const AnyObject = mask.as_deref().map_or(std::ptr::null(), |m| m as *const _);
                            let _: () = unsafe { msg_send![&*layer, setMask: mask] };
                            self.placed[i] = Some(patch);
                        }
                        if (renew || v.isHidden())
                            && let Some(Some(f)) = &filters
                        {
                            v.setBackgroundFilters(f);
                        }
                        v.setHidden(false);
                    }
                    None => {
                        if !v.isHidden() {
                            v.setBackgroundFilters(&NSArray::new());
                            v.setHidden(true);
                        }
                    }
                }
            }
            self.radius = if patches.is_empty() { 0. } else { radius };
            self.shown = patches.len();
        }
    }

    impl Drop for Views {
        fn drop(&mut self) {
            for v in &self.views {
                v.removeFromSuperview();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_blur_covers_all_but_the_rounded_hole() {
        let hole = Hole { x: 100., y: 50., w: 200., h: 120., r: 16. };
        let patches = around(hole, 500., 300.);
        // Inside a patch, and not in its cut.
        let covers = |p: &Patch, x: f32, y: f32| {
            let (px, py, pw, ph) = p.rect;
            let inside = x >= px && x < px + pw && y >= py && y < py + ph;
            inside && p.cut.is_none_or(|(cx, cy, r)| (x - px - cx).powi(2) + (y - py - cy).powi(2) > r * r)
        };
        let count = |x: f32, y: f32| patches.iter().filter(|p| covers(p, x, y)).count();
        // Outside the hole: blurred once. Inside it, even in a corner's square: not at all.
        for (x, y) in [(10., 10.), (200., 10.), (200., 290.), (50., 110.), (450., 110.), (101., 51.), (299., 169.)] {
            assert_eq!(count(x, y), 1, "{x},{y}");
        }
        for (x, y) in [(200., 110.), (110., 60.), (290., 160.), (115., 51.)] {
            assert_eq!(count(x, y), 0, "{x},{y}");
        }
    }
}
