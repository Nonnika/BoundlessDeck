#![forbid(unsafe_code)]
//! The blocks layer during a zoom: while a pinch or ⌘+wheel gesture is under
//! way, heavy layout and rasterization stay locked to discrete zoom levels
//! (`DeckView::lay` / `DeckView::content_scale`), so GPUI's `LineLayoutCache`
//! and `MetalAtlas` reuse shaped text, glyphs, SVGs, terminals and PDFs instead
//! of rebuilding them at every continuous float scale. Once the gesture rests
//! for 150ms, the camera glides onto the nearest zoom level over 120ms.
//!
//! Text holds still while the canvas zooms: through a gesture (or a glide)
//! it keeps the scale it had when the zoom began, so nothing reshapes and
//! nothing jumps mid-way. Once the zoom rests, the scale it is set at
//! (`content_scale`) follows the level the camera lands on as a critically
//! damped spring (`TextTween`): off from rest, no corner in its motion, and
//! it rests exactly on the level, where the caches hit.

use std::time::{Duration, Instant};

use gpui::{Context, Window};

use super::DeckView;
use crate::model::{self, Camera, ZOOM_REBASE};

/// How long after the last wheel/pinch event the zoom layer waits before
/// snapping to the nearest zoom level and re-laying out blocks cleanly.
pub(super) const ZOOM_SETTLE_HOLD: Duration = Duration::from_millis(150);
/// How long the camera takes to glide onto the nearest zoom level after a gesture.
pub(super) const ZOOM_SNAP_TIME: Duration = Duration::from_millis(120);
/// How quickly text follows its level after a zoom: the smoothing time of
/// each of the two springs it goes through (see `TextTween`).
pub(super) const TEXT_SMOOTH: f32 = 0.05;
/// A frame longer than this moves text no further: a slow frame does not skip ahead.
const TEXT_STEP: f32 = 1. / 30.;
/// Close enough, in the log of the scale (0.4%, under a tenth of a point
/// for body text): it rests on the level and the caches hit again.
const TEXT_REST: f32 = 0.004;

/// The scale text is set at, following its level through two critically
/// damped springs in a row, in the log of the scale (a doubling looks the
/// same at any size): a point between follows the level, and the text
/// follows the point. Its speed and how fast that changes are both
/// continuous, so it sets off gently, is quickest mid-way and comes to rest
/// gently; it never overshoots, and a new level mid-way keeps the motion it has.
#[derive(Clone, Copy, Debug, Default)]
pub(super) struct TextTween {
    /// Where it is now (0 before the first frame) and its speed; the point
    /// between and its speed, both in log scale (per second); and when it
    /// last moved.
    shown: f32,
    vel: f32,
    mid: f32,
    mid_vel: f32,
    last: Option<Instant>,
}

/// One critically damped spring over dt from x (going at v) toward `to`,
/// exactly as far as it goes (the smoothing of Game Programming Gems 4, 1.10).
fn spring(x: f32, v: f32, to: f32, dt: f32) -> (f32, f32) {
    let w = 2. / TEXT_SMOOTH;
    let k = w * dt;
    let decay = 1. / (1. + k + 0.48 * k * k + 0.235 * k * k * k);
    let off = x - to;
    let push = (v + w * off) * dt;
    let (mut nx, mut nv) = (to + (off + push) * decay, (v - w * push) * decay);
    // Never past where it goes.
    if (to - x) * (nx - to) > 0. {
        (nx, nv) = (to, 0.);
    }
    (nx, nv)
}

impl TextTween {
    fn rest(target: f32) -> TextTween {
        TextTween { shown: target, vel: 0., mid: target.ln(), mid_vel: 0., last: None }
    }

    /// Goes on toward `target` at `now`; true while still on the way. Without
    /// `animate`, it is there at once.
    pub fn step(&mut self, target: f32, now: Instant, animate: bool) -> bool {
        if self.shown <= 0. || !animate || target <= 0. {
            *self = TextTween::rest(target);
            return false;
        }
        let (x, to) = (self.shown.ln(), target.ln());
        let still = self.vel.abs() < TEXT_REST * 10. && self.mid_vel.abs() < TEXT_REST * 10.;
        if (x - to).abs() < TEXT_REST && (self.mid - to).abs() < TEXT_REST && still {
            *self = TextTween::rest(target);
            return false;
        }
        // The first frame of a move only starts its clock.
        let dt = self.last.map_or(0., |l| now.saturating_duration_since(l).as_secs_f32().min(TEXT_STEP));
        self.last = Some(now);
        (self.mid, self.mid_vel) = spring(self.mid, self.mid_vel, to, dt);
        let (nx, nv) = spring(x, self.vel, self.mid, dt);
        self.vel = nv;
        self.shown = nx.exp();
        true
    }
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
            // Text holds the scale it had when the zoom began, until it rests.
            if self.text_s.is_none() {
                self.text_s = Some(self.content_scale());
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

    /// The scale content is set at this frame: the text level, eased onto
    /// when it changes (`TextTween`).
    pub fn content_scale(&self) -> f32 {
        if self.text_tween.shown > 0. { self.text_tween.shown } else { self.text_level() }
    }

    /// Once a frame, after the camera and the layer have moved: eases the
    /// text toward its level, asking for frames until it is there.
    pub(super) fn tween_text(&mut self, window: &mut Window) {
        let target = self.text_level();
        let animate = stepped_text() && !super::sharpen::reduce_motion();
        if self.text_tween.step(target, Instant::now(), animate) {
            window.request_animation_frame();
        }
    }

    /// The discrete scale used for text shaping, glyph/SVG rasterization and
    /// text wrapping while zooming so `LineLayoutCache` and `MetalAtlas` hit
    /// cache instead of reshaping/rasterizing at every continuous float scale:
    /// during a gesture the scale it had when the gesture began, during the
    /// snap the level being glided to.
    pub fn text_level(&self) -> f32 {
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
mod tween_tests {
    use super::*;

    /// Runs the tween at 60 frames a second from `t`, toward `to`, for `n` frames: the scales shown.
    fn run(tw: &mut TextTween, t: &mut Instant, to: f32, n: usize) -> Vec<f32> {
        (0..n)
            .map(|_| {
                *t += Duration::from_micros(16_667);
                tw.step(to, *t, true);
                tw.shown
            })
            .collect()
    }

    #[test]
    fn text_sets_off_gently_never_overshoots_and_rests_on_its_level() {
        let mut t = Instant::now();
        let mut tw = TextTween::default();
        assert!(!tw.step(1., t, true), "the first frame is where it is");
        assert!(tw.step(2., t, true), "a new level: on its way");
        assert_eq!(tw.shown, 1., "it starts from where it was");
        let xs = run(&mut tw, &mut t, 2., 40);
        // From rest: it speeds up over several frames, quickest mid-way, then slows.
        let steps: Vec<f32> = std::iter::once(1.).chain(xs.iter().copied()).collect::<Vec<_>>().windows(2).map(|w| w[1] - w[0]).collect();
        let peak = steps.iter().enumerate().max_by(|a, b| a.1.total_cmp(b.1)).unwrap().0;
        assert!(peak >= 3, "quickest a few frames in, not at once: {steps:?}");
        assert!(steps[..peak].windows(2).all(|w| w[1] >= w[0]), "speeding up to it: {steps:?}");
        assert!(steps.iter().all(|d| *d >= 0.), "never back");
        assert!(xs.iter().all(|x| *x <= 2.), "never past the level");
        assert_eq!(*xs.last().unwrap(), 2., "it rests exactly on the level, where caches hit");
        assert!(xs.iter().position(|x| *x == 2.).unwrap() < 30, "at rest within half a second: {xs:?}");
    }

    #[test]
    fn a_new_level_mid_way_keeps_its_speed() {
        let mut t = Instant::now();
        let mut tw = TextTween::default();
        tw.step(1., t, true);
        tw.step(4., t, true);
        let xs = run(&mut tw, &mut t, 4., 8);
        let (before, at) = (xs[6], xs[7]);
        // Sent back down: it turns, without a jump.
        let after = run(&mut tw, &mut t, 1., 1)[0];
        assert!((after - at).abs() < (at - before).abs() * 2., "no jump: {before} {at} {after}");
        // Asked for less motion: there at once.
        assert!(!tw.step(3., t, false));
        assert_eq!(tw.shown, 3.);
    }
}
