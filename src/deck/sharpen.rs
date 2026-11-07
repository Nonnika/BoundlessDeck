//! Sharpening: when a block's content is replaced by a sharper version of
//! itself, the new one fades in over the old instead of swapping in a single
//! frame. The deck owns the timing and the curve; a block kind only has to
//! say how its content gets sharp, which is one of three ways:
//!
//! - **Vector** (text and paths: note, memo, file, terminal, chart): drawn at
//!   any scale in the same frame, sharp at once. The only change to fade is
//!   coming back from LOD, where the body was not drawn at all; `block_view`
//!   does that for every kind through `Runtime::appear`.
//! - **Raster** (a PDF page): the sharper bitmap is made in the background.
//!   The old one stays under, stretched, and the new one fades in over it
//!   when it arrives (`Runtime::sharpen` and `crossfade`).
//! - **Native view** (a browser's WKWebView): while zooming or covered a
//!   picture of the page stands in for it. Coming back into sight, the picture
//!   stays under and the view's alpha goes from 0 to 1 (`Runtime::sharpen`).
//!
//! With "Reduce motion" on in the system, changes happen at once.

use std::time::{Duration, Instant};

use gpui::{AnyElement, Div, ParentElement, Styled, div};

/// How long a sharper version takes to fade in.
pub(super) const SHARPEN_TIME: Duration = Duration::from_millis(180);

/// One fade in of a sharper version; idle when none is under way.
#[derive(Clone, Copy, Debug, Default)]
pub(super) struct Sharpen {
    at: Option<Instant>,
}

impl Sharpen {
    /// Starts fading the new version in now, unless the system asks for less motion.
    pub fn begin(&mut self, now: Instant) {
        self.start(now, !reduce_motion());
    }

    fn start(&mut self, now: Instant, animate: bool) {
        self.at = animate.then_some(now);
    }

    /// How far the new version has come in, eased: 0 just begun, 1 done or idle.
    pub fn alpha(&self, now: Instant) -> f32 {
        match self.at {
            None => 1.0,
            Some(at) => ease(now.saturating_duration_since(at).as_secs_f32() / SHARPEN_TIME.as_secs_f32()),
        }
    }

    /// Whether a fade is under way at now; a finished one is cleared.
    pub fn tick(&mut self, now: Instant) -> bool {
        if self.at.is_some_and(|at| now < at + SHARPEN_TIME) {
            return true;
        }
        self.at = None;
        false
    }
}

/// Ease out (cubic), f from 0 to 1.
pub(super) fn ease(f: f32) -> f32 {
    let f = f.clamp(0.0, 1.0);
    1.0 - (1.0 - f).powi(3)
}

/// The new version at alpha over the old one, both filling the box.
pub(super) fn crossfade(under: Option<AnyElement>, over: AnyElement, alpha: f32) -> Div {
    let layer = || div().absolute().top_0().left_0().size_full();
    div()
        .relative()
        .size_full()
        .children(under.map(|u| layer().child(u)))
        .child(layer().opacity(alpha).child(over))
}

#[cfg(target_os = "macos")]
pub(super) fn reduce_motion() -> bool {
    objc2_app_kit::NSWorkspace::sharedWorkspace().accessibilityDisplayShouldReduceMotion()
}

#[cfg(not(target_os = "macos"))]
pub(super) fn reduce_motion() -> bool {
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_fade_runs_from_zero_to_one_and_clears() {
        let now = Instant::now();
        let mut f = Sharpen::default();
        assert!(!f.tick(now));
        assert_eq!(f.alpha(now), 1.0, "idle shows the content fully");
        f.start(now, true);
        assert!(f.tick(now));
        assert!(f.alpha(now) < 0.01);
        let mid = now + SHARPEN_TIME / 2;
        assert!(f.alpha(mid) > 0.5 && f.alpha(mid) < 1.0, "ease out is past half at half time");
        let end = now + SHARPEN_TIME;
        assert!(!f.tick(end));
        assert_eq!(f.alpha(end), 1.0);
    }

    #[test]
    fn less_motion_skips_the_fade() {
        let now = Instant::now();
        let mut f = Sharpen::default();
        f.start(now, false);
        assert!(!f.tick(now));
        assert_eq!(f.alpha(now), 1.0);
    }

    #[test]
    fn ease_is_clamped_and_monotonic() {
        assert_eq!(ease(-1.0), 0.0);
        assert_eq!(ease(2.0), 1.0);
        let mut last = 0.0;
        for i in 0..=20 {
            let v = ease(i as f32 / 20.0);
            assert!(v >= last);
            last = v;
        }
    }
}
