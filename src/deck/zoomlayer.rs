#![forbid(unsafe_code)]
//! The blocks layer during a zoom: while a pinch or ⌘+wheel gesture is under
//! way, heavy layout and rasterization stay locked to discrete zoom levels
//! (`DeckView::lay` / `DeckView::content_scale`), so GPUI's `LineLayoutCache`
//! and `MetalAtlas` reuse shaped text, glyphs, SVGs, terminals and PDFs instead
//! of rebuilding them at every continuous float scale. Once the gesture rests
//! for 150ms, the camera glides onto the nearest zoom level over 120ms.

use std::time::{Duration, Instant};

use gpui::{Context, Window};

use super::DeckView;
use crate::model::{self, Camera, ZOOM_REBASE};

/// How long after the last wheel/pinch event the zoom layer waits before
/// snapping to the nearest zoom level and re-laying out blocks cleanly.
pub(super) const ZOOM_SETTLE_HOLD: Duration = Duration::from_millis(150);
/// How long the camera takes to glide onto the nearest zoom level after a gesture.
pub(super) const ZOOM_SNAP_TIME: Duration = Duration::from_millis(120);
/// Text levels per doubling while a gesture is under way: coarser than
/// `ZOOM_LEVELS`, so a pinch reshapes all text in sight a few times, not a dozen.
const GESTURE_TEXT_LEVELS: f64 = 3.0;
/// How far (in doublings) the camera may drift from the gesture's text level
/// before the text moves to another: half a level, and a little more so a
/// camera resting on the edge does not flip between two.
const GESTURE_TEXT_SWITCH: f32 = (0.5 / GESTURE_TEXT_LEVELS + 1.0 / 48.0) as f32;

/// The gesture text level nearest s.
fn gesture_text_level(s: f32) -> f32 {
    let n = (f64::from(s).log2() * GESTURE_TEXT_LEVELS).round();
    ((n / GESTURE_TEXT_LEVELS).exp2() as f32).clamp(model::MIN_SCALE, model::MAX_SCALE)
}

#[derive(Clone, Copy, Debug)]
pub(super) struct ZoomSnap {
    pub on: bool,
    pub from: f32,
    pub to: f32,
    pub at: Instant,
}

impl Default for ZoomSnap {
    fn default() -> Self {
        ZoomSnap { on: false, from: 1.0, to: 1.0, at: Instant::now() }
    }
}

impl DeckView {
    /// Marks the blocks layer as mid-zoom so `render` keeps content on discrete
    /// zoom levels until the gesture settles.
    pub fn zoom_gesture(&mut self) {
        self.zoom_settle = Some(Instant::now() + ZOOM_SETTLE_HOLD);
    }

    /// Notes that the canvas zoomed now, and schedules a wakeup once it settles.
    pub(super) fn note_zoom(&mut self, cx: &mut Context<Self>) {
        let now = Instant::now();
        self.zoomed_at = Some(now);
        self.zoom_gesture();
        if self.zoom_timer_active {
            return;
        }
        self.zoom_timer_active = true;
        cx.spawn(async move |this, cx| {
            loop {
                let wait = this
                    .update(cx, |d, _| {
                        let now = Instant::now();
                        match d.zoom_settle {
                            Some(until) if until > now => Some(until.duration_since(now) + Duration::from_millis(4)),
                            _ => None,
                        }
                    })
                    .ok()
                    .flatten();
                match wait {
                    Some(dur) => cx.background_executor().timer(dur).await,
                    None => {
                        let _ = this.update(cx, |d, cx| {
                            d.zoom_timer_active = false;
                            cx.notify();
                        });
                        break;
                    }
                }
            }
        })
        .detach();
    }

    /// Keeps `lay` frozen while a zoom gesture is active (rebasing only if the
    /// camera crosses `ZOOM_REBASE`), then glides `m.cam` onto the nearest
    /// zoom level and syncs `lay` to `m.cam`.
    pub fn settle_layer(&mut self, window: Option<&mut Window>) {
        if self.lay.s == 0.0 {
            self.lay = self.m.cam;
        }
        let now = Instant::now();
        if self.zoom_settle.is_some_and(|until| now < until) {
            self.snap.on = false;
            // If a single gesture spans more than 2x, rebase to the nearest
            // zoom level so scaled content never drifts too far.
            if self.m.cam.s < self.lay.s * ZOOM_REBASE || self.m.cam.s > self.lay.s / ZOOM_REBASE {
                self.lay = Camera { x: self.m.cam.x, y: self.m.cam.y, s: model::zoom_level(self.m.cam.s) };
            }
            // Text keeps its level until the camera has left it by half a coarse level.
            let s = self.m.cam.s;
            if self.text_s.is_none_or(|t| (s / t).log2().abs() > GESTURE_TEXT_SWITCH) {
                self.text_s = Some(gesture_text_level(s));
            }
            return;
        }
        self.zoom_settle = None;
        self.text_s = None;
        // Once the gesture rests, glide to the nearest zoom level: lay the
        // blocks out once at `to`, and ease `m.cam` onto it. Text that follows
        // the camera continuously has no level to land on, so it stays put.
        if stepped_text() && !self.snap.on && (self.lay.s - self.m.cam.s).abs() > 1e-6 && !model::on_level(self.m.cam.s) {
            let to = model::zoom_level(self.m.cam.s);
            let Some(w) = window else {
                self.zoom_to(to);
                self.lay = self.m.cam;
                return;
            };
            self.snap = ZoomSnap { on: true, from: self.m.cam.s, to, at: now };
            self.lay = Camera { x: self.m.cam.x, y: self.m.cam.y, s: to };
            w.request_animation_frame();
            return;
        }
        if self.snap.on {
            let t = now.duration_since(self.snap.at).as_secs_f32() / ZOOM_SNAP_TIME.as_secs_f32();
            if t < 1.0 {
                let e = 1.0 - (1.0 - t) * (1.0 - t); // ease-out quad
                self.zoom_to(self.snap.from + (self.snap.to - self.snap.from) * e);
                if let Some(w) = window {
                    w.request_animation_frame();
                }
                return;
            }
            self.zoom_to(self.snap.to);
            self.snap.on = false;
        }
        self.lay = self.m.cam;
    }

    /// Sets `m.cam.s` to `to`, keeping the zoom anchor at the same window point.
    pub fn zoom_to(&mut self, to: f32) {
        let (mut mx, mut my) = self.last_pointer;
        if mx == 0.0 && my == 0.0 {
            mx = self.view.0 / 2.0;
            my = self.view.1 / 2.0;
        }
        self.m.zoom_at(mx, my, to / self.m.cam.s);
    }

    /// Ratio between the live camera and the camera blocks were laid out with.
    pub fn layer_scale(&self) -> f32 {
        if self.lay.s == 0.0 { 1.0 } else { self.m.cam.s / self.lay.s }
    }

    /// Window-pixel translation of the laid-out blocks layer.
    pub fn layer_offset(&self) -> (f32, f32) {
        ((self.lay.x - self.m.cam.x) * self.m.cam.s, (self.lay.y - self.m.cam.y) * self.m.cam.s)
    }

    /// Whether the blocks layer is currently scaled or translated away from `lay`.
    pub fn layer_moved(&self) -> bool {
        let (x, y) = self.layer_offset();
        (self.layer_scale() - 1.0).abs() > 1e-6 || x != 0.0 || y != 0.0
    }

    /// The discrete scale used for text shaping, glyph/SVG rasterization and
    /// text wrapping while zooming so `LineLayoutCache` and `MetalAtlas` hit
    /// cache instead of reshaping/rasterizing at every continuous float scale:
    /// during a gesture a coarse level (`GESTURE_TEXT_LEVELS`), during the snap
    /// the level being glided to.
    pub fn content_scale(&self) -> f32 {
        if !stepped_text() {
            self.m.cam.s
        } else if self.snap.on && self.lay.s > 0.0 {
            self.lay.s
        } else if let Some(s) = self.text_s {
            s
        } else {
            self.m.cam.s
        }
    }

    /// Whether a zoom gesture or post-zoom snap glide is currently active.
    pub(super) fn zooming(&self) -> bool {
        self.zoom_settle.is_some_and(|t| Instant::now() < t) || self.snap.on || self.layer_moved()
    }
}

/// TEMP（缩放方案 A 试验）：默认手势期间文字按粗档位分档、松手吸附到档位；
/// `BOUNDLESSDECK_ZOOM_TEXT=continuous` 让文字随相机连续缩放，用来对比帧耗时。
pub(super) fn stepped_text() -> bool {
    static STEPS: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *STEPS.get_or_init(|| !std::env::var("BOUNDLESSDECK_ZOOM_TEXT").is_ok_and(|v| v == "continuous"))
}

/// TEMP（缩放方案 A 试验）：设置 `BOUNDLESSDECK_FRAME_STATS=1` 后，每次缩放手势结束
/// 在 stderr 打印 CPU 帧时间（render 开始到最后一个元素 paint 完）。
pub(super) mod framestat {
    use std::sync::Mutex;
    use std::time::{Duration, Instant};

    struct S {
        start: Option<Instant>,
        zooming: bool,
        frames: Vec<Duration>,
    }

    static STATE: Mutex<S> = Mutex::new(S { start: None, zooming: false, frames: Vec::new() });

    pub fn enabled() -> bool {
        static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
        *ON.get_or_init(|| std::env::var("BOUNDLESSDECK_FRAME_STATS").is_ok_and(|v| v == "1"))
    }

    /// At the top of `render`: starts timing this frame; reports a gesture that just ended.
    pub fn begin(zooming: bool) {
        if !enabled() {
            return;
        }
        let mut s = STATE.lock().unwrap();
        if s.zooming && !zooming {
            report(&mut s.frames);
        }
        s.zooming = zooming;
        s.start = zooming.then(Instant::now);
    }

    /// From the last thing painted: the frame's work is done.
    pub fn end() {
        if !enabled() {
            return;
        }
        let mut s = STATE.lock().unwrap();
        if let Some(t) = s.start.take() {
            s.frames.push(t.elapsed());
        }
    }

    fn report(frames: &mut Vec<Duration>) {
        if frames.is_empty() {
            return;
        }
        frames.sort();
        let ms = |d: Duration| d.as_secs_f64() * 1000.;
        let n = frames.len();
        let avg = frames.iter().map(|d| ms(*d)).sum::<f64>() / n as f64;
        let over = |lim: f64| frames.iter().filter(|d| ms(**d) > lim).count();
        eprintln!(
            "[zoom {}] {n} 帧  平均 {avg:.2}ms  p50 {:.2}ms  p95 {:.2}ms  最大 {:.2}ms  >8.3ms {}  >16.7ms {}",
            if super::stepped_text() { "steps" } else { "continuous" },
            ms(frames[n / 2]),
            ms(frames[(n * 95 / 100).min(n - 1)]),
            ms(frames[n - 1]),
            over(8.33),
            over(16.67),
        );
        frames.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gesture_text_levels_are_coarse_and_hold() {
        // Three levels per doubling: 1, 2^(1/3), 2^(2/3), 2.
        assert!((gesture_text_level(1.05) - 1.0).abs() < 1e-6);
        assert!((gesture_text_level(1.2) - 2f32.powf(1. / 3.)).abs() < 1e-5);
        assert!((gesture_text_level(1.95) - 2.0).abs() < 1e-6);
        assert_eq!(gesture_text_level(100.), model::MAX_SCALE);
        // A level is held a little past the halfway point to the next one.
        let half = 0.5 / GESTURE_TEXT_LEVELS as f32;
        assert!(GESTURE_TEXT_SWITCH > half && GESTURE_TEXT_SWITCH < 2. * half);
    }
}
