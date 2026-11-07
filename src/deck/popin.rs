//! A block's entrance, as a window opens in a film's computer: a bright line
//! grows out from the middle across the block's width, the box opens up and
//! down from it, then a scan line sweeps down and the content is printed in
//! behind it, while the glow round the box fades.
//!
//! Only blocks just made (`on_create`) enter so: not those an undo brings
//! back, nor those the deck opens with. The clock starts the first frame the
//! block is drawn, so one made off screen enters when the view reaches it, if
//! that is soon. With "Reduce motion" on, blocks just appear.

use std::time::{Duration, Instant};

/// How long the entrance takes.
pub(super) const POP_TIME: Duration = Duration::from_millis(560);
/// How long a block made off screen still enters when it comes into view.
const POP_WAIT: Duration = Duration::from_secs(3);

/// A block's entrance; idle when none is due.
#[derive(Clone, Copy, Debug, Default)]
pub(super) struct PopIn {
    /// When the block was made, while its entrance is due but not begun.
    due: Option<Instant>,
    /// When it began.
    at: Option<Instant>,
}

/// The entrance at a moment. Fractions are of the block's box.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Frame {
    /// How much of the width and height is open, about the middle.
    pub width: f32,
    pub height: f32,
    /// How far down the content is printed in (the scan line), 0 to 1.
    pub scan: f32,
    /// The accent wash over the box while it is a line, 0 to 1.
    pub tint: f32,
    /// The glow round the box and its edge, 0 to 1.
    pub glow: f32,
}

impl PopIn {
    /// The block was made now: it enters when first drawn.
    pub fn arm(&mut self, now: Instant) {
        if !super::sharpen::reduce_motion() {
            *self = PopIn { due: Some(now), at: None };
        }
    }

    /// Whether the entrance is under way: the block's native view waits.
    pub fn active(&self) -> bool {
        self.at.is_some()
    }

    /// The entrance as drawn at now, starting it if due; None once done or
    /// when none is due.
    pub fn frame(&mut self, now: Instant) -> Option<Frame> {
        if let Some(due) = self.due.take() {
            if now.saturating_duration_since(due) > POP_WAIT {
                return None;
            }
            self.at = Some(now);
        }
        let at = self.at?;
        let f = now.saturating_duration_since(at).as_secs_f32() / POP_TIME.as_secs_f32();
        if f >= 1.0 {
            self.at = None;
            return None;
        }
        Some(frame_at(f))
    }
}

/// f of the way (0 to 1) through the span [a, b], clamped.
fn span(f: f32, a: f32, b: f32) -> f32 {
    ((f - a) / (b - a)).clamp(0.0, 1.0)
}

fn ease_out(f: f32) -> f32 {
    1.0 - (1.0 - f).powi(3)
}

fn ease_in_out(f: f32) -> f32 {
    if f < 0.5 { 4.0 * f * f * f } else { 1.0 - (-2.0 * f + 2.0).powi(3) / 2.0 }
}

/// The entrance f of the way through: the line (to 0.3), the opening (0.22
/// to 0.6, overlapping so it does not stop), the printing (0.5 to 1).
pub(super) fn frame_at(f: f32) -> Frame {
    let width = ease_out(span(f, 0.0, 0.3));
    let height = ease_in_out(span(f, 0.22, 0.6));
    let scan = ease_out(span(f, 0.5, 1.0));
    let tint = 0.85 * (1.0 - height);
    let glow = 1.0 - ease_in_out(span(f, 0.6, 1.0));
    Frame { width, height, scan, tint, glow }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn it_opens_as_a_line_then_a_box_then_prints() {
        let start = frame_at(0.0);
        assert_eq!((start.width, start.height, start.scan), (0.0, 0.0, 0.0));
        let line = frame_at(0.25);
        assert!(line.width > 0.9 && line.height < 0.1 && line.tint > 0.7, "{line:?}");
        let open = frame_at(0.52);
        assert_eq!(open.width, 1.0);
        assert!(open.height > 0.9 && open.scan < 0.2, "{open:?}");
        let end = frame_at(1.0);
        assert_eq!((end.width, end.height, end.scan, end.glow), (1.0, 1.0, 1.0, 0.0));
        assert!(end.tint.abs() < 1e-6);
    }

    #[test]
    fn every_part_only_grows() {
        let mut last = frame_at(0.0);
        for i in 1..=100 {
            let f = frame_at(i as f32 / 100.0);
            assert!(f.width >= last.width && f.height >= last.height && f.scan >= last.scan);
            assert!(f.glow <= last.glow && f.tint <= last.tint + 1e-6);
            last = f;
        }
    }

    #[test]
    fn the_clock_starts_when_first_drawn_and_ends() {
        let made = Instant::now();
        let mut p = PopIn { due: Some(made), at: None };
        let shown = made + Duration::from_millis(800);
        assert_eq!(p.frame(shown).map(|f| f.width), Some(0.0), "begins where it is first drawn");
        assert!(p.active());
        assert!(p.frame(shown + POP_TIME).is_none());
        assert!(!p.active());
        // Made long before it comes into view: it just shows.
        let mut late = PopIn { due: Some(made), at: None };
        assert!(late.frame(made + POP_WAIT + Duration::from_millis(1)).is_none());
    }
}
