#![forbid(unsafe_code)]
//! The blocks layer during a zoom. Everything is laid out at the camera's
//! own scale every frame, text included, so a pinch or ⌘+wheel enlarges the
//! canvas the way a photo is enlarged: smoothly, in proportion, the words
//! keeping their lines (their size and the width they wrap in scale
//! together), nothing frozen, nothing switching to another size, and
//! nothing moving by itself once the fingers stop.
//!
//! What stays coarse while zooming is raster work: `DeckView::lay` holds the
//! scale a PDF page was rasterized at (rebased past `ZOOM_REBASE`) until the
//! gesture has rested for `ZOOM_SETTLE_HOLD`, and pictures of web pages
//! stand in for the pages themselves (`zooming`).

use std::time::{Duration, Instant};

use gpui::Context;

use super::DeckView;
use crate::model::{self, Camera, ZOOM_REBASE};

/// How long after the last wheel/pinch event a zoom counts as over: then
/// pages and pictures are made sharp at the scale the camera rests at.
pub(super) const ZOOM_SETTLE_HOLD: Duration = Duration::from_millis(150);

impl DeckView {
    /// Marks the canvas as mid-zoom until it has rested for `ZOOM_SETTLE_HOLD`.
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

    /// Keeps `lay` (the scale rasters are made at) while a zoom is under
    /// way, rebasing it only if the camera has gone past `ZOOM_REBASE` from
    /// it; once the zoom rests, `lay` is the camera again.
    pub fn settle_layer(&mut self) {
        if self.lay.s == 0.0 {
            self.lay = self.m.cam;
        }
        if self.zoom_settle.is_some_and(|until| Instant::now() < until) {
            if self.m.cam.s < self.lay.s * ZOOM_REBASE || self.m.cam.s > self.lay.s / ZOOM_REBASE {
                self.lay = Camera { x: self.m.cam.x, y: self.m.cam.y, s: model::zoom_level(self.m.cam.s) };
            }
            return;
        }
        self.zoom_settle = None;
        self.lay = self.m.cam;
    }

    /// Ratio between the live camera and the camera rasters were made at.
    pub fn layer_scale(&self) -> f32 {
        if self.lay.s == 0.0 { 1.0 } else { self.m.cam.s / self.lay.s }
    }

    /// Window-pixel translation of the laid-out blocks layer.
    pub fn layer_offset(&self) -> (f32, f32) {
        ((self.lay.x - self.m.cam.x) * self.m.cam.s, (self.lay.y - self.m.cam.y) * self.m.cam.s)
    }

    /// Whether the camera has moved away from `lay`.
    pub fn layer_moved(&self) -> bool {
        let (x, y) = self.layer_offset();
        (self.layer_scale() - 1.0).abs() > 1e-6 || x != 0.0 || y != 0.0
    }

    /// The scale content is set at: the camera's, every frame.
    pub fn content_scale(&self) -> f32 {
        self.m.cam.s
    }

    /// Whether a zoom is under way (or has not yet rested).
    pub(super) fn zooming(&self) -> bool {
        self.zoom_settle.is_some_and(|t| Instant::now() < t) || self.layer_moved()
    }
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
            "[zoom] {n} 帧  平均 {avg:.2}ms  p50 {:.2}ms  p95 {:.2}ms  最大 {:.2}ms  >8.3ms {}  >16.7ms {}",
            ms(frames[n / 2]),
            ms(frames[(n * 95 / 100).min(n - 1)]),
            ms(frames[n - 1]),
            over(8.33),
            over(16.67),
        );
        frames.clear();
    }
}
