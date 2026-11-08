//! A blur over the whole window but a hole: a panel drawn in the window
//! (the welcome) stands sharp in the hole, and all behind it, web pages too,
//! is blurred. GPUI draws no blur, so it is native views over GPUI's, whose
//! layers blur what lies behind them (`backgroundFilters`, a Core Image
//! Gaussian blur). They are four bands around the hole rather than one view
//! with the hole cut out: a view's blur takes in all it covers, so one over
//! the panel too would smear the panel's colors out past its edge. The views
//! take no clicks (`hitTest:` is nil): input goes on to GPUI.

/// Blur over parts of a window: its views are made as needed, and taken off
/// the window when dropped.
#[derive(Default)]
pub struct Backdrop {
    #[cfg(target_os = "macos")]
    mac: mac::Views,
}

/// A rect in the window left clear: x, y, w, h, in points.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Hole {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

/// The four bands around `hole` in a window `vw` by `vh`: above, below, left,
/// right (x, y, w, h); the sides run the hole's height.
pub fn bands(hole: Hole, vw: f32, vh: f32) -> [(f32, f32, f32, f32); 4] {
    let (x0, y0) = (hole.x.clamp(0., vw), hole.y.clamp(0., vh));
    let (x1, y1) = ((hole.x + hole.w).clamp(x0, vw), (hole.y + hole.h).clamp(y0, vh));
    [(0., 0., vw, y0), (0., y1, vw, vh - y1), (0., y0, x0, y1 - y0), (x1, y0, vw - x1, y1 - y0)]
}

impl Backdrop {
    /// Blurs the window by `radius` points, all but `hole`; 0 takes it off.
    pub fn around(&mut self, window: &gpui::Window, radius: f64, hole: Hole) {
        let vs = window.viewport_size();
        self.over(window, radius, &bands(hole, f32::from(vs.width), f32::from(vs.height)));
    }

    /// Blurs `rects` of the window (x, y, w, h) by `radius` points; 0 takes it off.
    pub fn over(&mut self, window: &gpui::Window, radius: f64, rects: &[(f32, f32, f32, f32)]) {
        #[cfg(target_os = "macos")]
        self.mac.place(window, if radius < 0.05 { &[] } else { rects }, radius);
        #[cfg(not(target_os = "macos"))]
        let _ = (window, radius, rects);
    }
}

#[cfg(target_os = "macos")]
mod mac {
    use objc2::rc::Retained;
    use objc2::{MainThreadMarker, MainThreadOnly, define_class, msg_send};
    use objc2_app_kit::NSView;
    use objc2_core_image::CIFilter;
    use objc2_foundation::{NSArray, NSNumber, NSObjectNSKeyValueCoding, NSPoint, NSRect, NSSize, NSString};
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};

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
        pub fn place(&mut self, window: &gpui::Window, rects: &[(f32, f32, f32, f32)], radius: f64) {
            if rects.is_empty() && self.shown == 0 {
                return;
            }
            let Some(mtm) = MainThreadMarker::new() else { return };
            let Some(canvas) = canvas(window) else { return };
            while self.views.len() < rects.len() {
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
            }
            let size = canvas.bounds().size;
            let flipped = canvas.isFlipped();
            let renew = (radius - self.radius).abs() > 0.01;
            let filters = (!rects.is_empty()).then(|| {
                let blur = unsafe { CIFilter::filterWithName(&NSString::from_str("CIGaussianBlur")) }?;
                unsafe {
                    blur.setValue_forKey(Some(&NSNumber::new_f64(radius)), &NSString::from_str("inputRadius"));
                }
                Some(NSArray::from_retained_slice(&[blur]))
            });
            for (i, v) in self.views.iter().enumerate() {
                match rects.get(i).filter(|r| r.2 >= 1. && r.3 >= 1.) {
                    Some(&(x, y, w, h)) => {
                        let (x, y, w, h) = (x as f64, y as f64, w as f64, h as f64);
                        let y = if flipped { y } else { size.height - y - h };
                        let frame = NSRect::new(NSPoint::new(x, y), NSSize::new(w, h));
                        if v.frame() != frame {
                            v.setFrame(frame);
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
            self.radius = if rects.is_empty() { 0. } else { radius };
            self.shown = rects.len();
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
    fn bands_cover_all_but_the_hole() {
        let hole = Hole { x: 100., y: 50., w: 200., h: 120. };
        let bands = bands(hole, 500., 300.);
        let area: f32 = bands.iter().map(|b| b.2 * b.3).sum();
        assert!((area + 200. * 120. - 500. * 300.).abs() < 1e-3);
        let inside = |b: &(f32, f32, f32, f32), x: f32, y: f32| x >= b.0 && x < b.0 + b.2 && y >= b.1 && y < b.1 + b.3;
        // The middle of the hole is in none; points around it are each in one.
        assert!(!bands.iter().any(|b| inside(b, 200., 110.)));
        for (x, y) in [(10., 10.), (200., 10.), (200., 290.), (50., 110.), (450., 110.)] {
            assert_eq!(bands.iter().filter(|b| inside(b, x, y)).count(), 1, "{x},{y}");
        }
    }
}
