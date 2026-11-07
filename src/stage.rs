//! Going from one screen to another (opening a folder, going home), the
//! whole window blurs out; at its blurriest the other screen takes its place,
//! and comes clear. GPUI draws no blur, so it is a Core Image filter on the
//! window's view: it blurs the web pages over it too.

use gpui::Window;

/// How blurred the window gets, and how long it takes to blur out and to
/// come clear again (as the Go build has them).
pub const BLUR: f64 = 28.;
pub const OUT_MS: f64 = 260.;
pub const CLEAR_MS: f64 = 420.;

/// Ease in and out, f from 0 to 1.
pub fn ease(f: f64) -> f64 {
    let f = f.clamp(0., 1.);
    if f < 0.5 { 4. * f * f * f } else { 1. - (-2. * f + 2.).powi(3) / 2. }
}

/// The f at which `ease` reaches v: where a blur under way starts from.
pub fn uneased(v: f64) -> f64 {
    let (mut lo, mut hi) = (0., 1.);
    for _ in 0..24 {
        let mid = (lo + hi) / 2.;
        if ease(mid) < v { lo = mid } else { hi = mid }
    }
    (lo + hi) / 2.
}

/// Blurs the window's content by radius points; 0 takes the blur off.
#[cfg(target_os = "macos")]
pub fn set_blur(window: &Window, radius: f64) {
    use objc2::rc::Retained;
    use objc2_app_kit::NSView;
    use objc2_core_image::CIFilter;
    use objc2_foundation::{NSArray, NSNumber, NSObjectNSKeyValueCoding, NSString};
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};

    let Ok(handle) = HasWindowHandle::window_handle(window) else { return };
    let RawWindowHandle::AppKit(h) = handle.as_raw() else { return };
    let view: &NSView = unsafe { h.ns_view.cast().as_ref() };
    if radius <= 0.01 {
        view.setContentFilters(&NSArray::new());
        return;
    }
    let Some(blur) = (unsafe { CIFilter::filterWithName(&NSString::from_str("CIGaussianBlur")) }) else { return };
    unsafe {
        let r = NSNumber::new_f64(radius);
        blur.setValue_forKey(Some(&r), &NSString::from_str("inputRadius"));
    }
    view.setLayerUsesCoreImageFilters(true);
    let filters: Retained<NSArray<CIFilter>> = NSArray::from_retained_slice(&[blur]);
    view.setContentFilters(&filters);
}

#[cfg(not(target_os = "macos"))]
pub fn set_blur(_: &Window, _: f64) {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uneased_undoes_ease() {
        for v in [0., 0.1, 0.5, 0.9, 1.] {
            assert!((ease(uneased(v)) - v).abs() < 1e-4, "{v}");
        }
    }
}
