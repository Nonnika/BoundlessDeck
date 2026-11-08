//! The camera over the canvas, and the discrete zoom levels it rests on.

/// Shows the world point (x, y) at the window's top-left corner, s times as large.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Camera {
    pub x: f32,
    pub y: f32,
    pub s: f32,
}

impl Default for Camera {
    fn default() -> Self {
        Camera { x: 0.0, y: 0.0, s: 1.0 }
    }
}

impl Camera {
    pub fn to_screen(&self, wx: f32, wy: f32) -> (f32, f32) {
        ((wx - self.x) * self.s, (wy - self.y) * self.s)
    }
    pub fn to_world(&self, sx: f32, sy: f32) -> (f32, f32) {
        (sx / self.s + self.x, sy / self.s + self.y)
    }
    pub(super) fn clamp(&mut self) {
        self.s = self.s.clamp(MIN_SCALE, MAX_SCALE);
    }
}

pub const MIN_SCALE: f32 = 0.08;
pub const MAX_SCALE: f32 = 4.0;
/// Below this scale a block shows its frame and title alone.
pub const LOD_SCALE: f32 = 0.3;
/// During a zoom gesture the blocks layer is re-laid out only after the camera crosses this ratio from it.
pub const ZOOM_REBASE: f32 = 0.5;
/// Zoom levels per doubling: the camera rests on one.
pub const ZOOM_LEVELS: f64 = 12.0;
/// How many levels a key press zooms by.
pub const ZOOM_KEY_STEPS: i32 = 3;

/// The zoom level nearest s.
pub fn zoom_level(s: f32) -> f32 {
    zoom_level_of((f64::from(s).log2() * ZOOM_LEVELS).round())
}

/// Whether s is already on a zoom level.
pub fn on_level(s: f32) -> bool {
    (s - zoom_level(s)).abs() <= s * 1e-5
}

/// The largest zoom level not above s.
pub fn zoom_level_at_most(s: f32) -> f32 {
    zoom_level_of((f64::from(s).log2() * ZOOM_LEVELS + 1e-6).floor())
}

/// Level n, the n-th 12th of a doubling from 1, within the zoom's limits.
pub fn zoom_level_of(n: f64) -> f32 {
    let lo = (f64::from(MIN_SCALE).log2() * ZOOM_LEVELS).ceil();
    let hi = (f64::from(MAX_SCALE).log2() * ZOOM_LEVELS).floor();
    (n.clamp(lo, hi) / ZOOM_LEVELS).exp2() as f32
}

/// The factor ⌘ and the wheel zoom by for dy pixels of scrolling.
pub fn wheel_zoom(dy: f32) -> f32 {
    1.0016f64.powf(-f64::from(dy)) as f32
}
