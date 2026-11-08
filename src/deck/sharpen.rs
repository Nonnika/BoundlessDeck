//! Sharpening: when a block's content is replaced by a sharper version of
//! itself, the new one fades in over the old instead of swapping in a single
//! frame. The deck owns the timing and the curve; a block kind only has to
//! say how its content gets sharp, which is one of three ways:
//!
//! - **Vector** (text and paths: note, memo, file, terminal, chart): drawn at
//!   any scale in the same frame, sharp at once. The only change to fade is
//!   coming back from LOD, where the body is a mask of the block's own
//!   background: the content is drawn whole at once and the mask over it
//!   fades off, slower and eased in and out (`REVEAL_TIME`, `ease_in_out`);
//!   `block_view` does that for every kind through `Runtime::appear`.
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

/// How long the mask takes to fade off a body coming back from LOD.
pub(super) const REVEAL_TIME: Duration = Duration::from_millis(450);

/// An easing curve, f from 0 to 1.
type Curve = fn(f32) -> f32;

/// One fade in of a sharper version; idle when none is under way.
#[derive(Clone, Copy, Debug, Default)]
pub(super) struct Sharpen {
    /// When it began, how long it takes, and its curve.
    at: Option<(Instant, Duration, Curve)>,
}

impl Sharpen {
    /// Starts fading the new version in now, unless the system asks for less motion.
    pub fn begin(&mut self, now: Instant) {
        self.start(now, SHARPEN_TIME, ease, !reduce_motion());
    }

    /// Starts taking the mask off a body back from LOD now (`REVEAL_TIME`, eased in and out).
    pub fn reveal(&mut self, now: Instant) {
        self.start(now, REVEAL_TIME, ease_in_out, !reduce_motion());
    }

    fn start(&mut self, now: Instant, len: Duration, curve: Curve, animate: bool) {
        self.at = animate.then_some((now, len, curve));
    }

    /// How far the new version has come in, eased: 0 just begun, 1 done or idle.
    pub fn alpha(&self, now: Instant) -> f32 {
        match self.at {
            None => 1.0,
            Some((at, len, curve)) => curve(now.saturating_duration_since(at).as_secs_f32() / len.as_secs_f32()),
        }
    }

    /// Whether a fade is under way at now; a finished one is cleared.
    pub fn tick(&mut self, now: Instant) -> bool {
        if self.at.is_some_and(|(at, len, _)| now < at + len) {
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

/// Ease in and out (cubic), f from 0 to 1: starts and ends gently.
pub(super) fn ease_in_out(f: f32) -> f32 {
    let f = f.clamp(0.0, 1.0);
    if f < 0.5 { 4.0 * f * f * f } else { 1.0 - (-2.0 * f + 2.0).powi(3) / 2.0 }
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

pub(super) fn reduce_motion() -> bool {
    crate::stage::reduce_motion()
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
        f.start(now, SHARPEN_TIME, ease, true);
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
        f.start(now, SHARPEN_TIME, ease, false);
        assert!(!f.tick(now));
        assert_eq!(f.alpha(now), 1.0);
    }

    #[test]
    fn a_reveal_takes_its_own_time() {
        let now = Instant::now();
        let mut f = Sharpen::default();
        f.start(now, REVEAL_TIME, ease_in_out, true);
        assert!(f.tick(now + SHARPEN_TIME), "still under way past the sharpen time");
        assert!(f.alpha(now + REVEAL_TIME / 4) < 0.25, "starts gently");
        assert!(!f.tick(now + REVEAL_TIME));
        assert_eq!(f.alpha(now + REVEAL_TIME), 1.0);
    }

    #[test]
    fn eases_are_clamped_and_monotonic() {
        for curve in [ease as Curve, ease_in_out] {
            assert_eq!(curve(-1.0), 0.0);
            assert_eq!(curve(2.0), 1.0);
            let mut last = 0.0;
            for i in 0..=20 {
                let v = curve(i as f32 / 20.0);
                assert!(v >= last);
                last = v;
            }
        }
    }
}
