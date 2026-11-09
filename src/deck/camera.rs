//! Moving the camera: zooming at a point, fitting every block, and the
//! glide to where an agent asked.

use std::time::{Duration, Instant};

use gpui::{Context, Window};

use super::{DeckView, FitAll, ZoomIn, ZoomOut, sharpen};
use crate::model::{self, Camera};

impl DeckView {
    /// Moves the camera to `to`, gliding unless asked not to (or the system
    /// asks for less motion). The person's own pan or zoom stops a glide.
    pub(super) fn glide_to(&mut self, from: Camera, to: Camera, animate: bool, cx: &mut Context<Self>) {
        if animate && !sharpen::reduce_motion() && self.view.0 > 0. {
            self.m.cam = from;
            self.glide = Some(Glide { from, to, f: 0., last: None });
        } else {
            self.m.cam = to;
            self.lay = to;
            self.glide = None;
        }
        cx.notify();
    }

    /// One frame of a glide: the view's center goes straight there, the
    /// scale eases between the two, dipping out on a long way so the trip
    /// shows where it goes.
    ///
    /// It goes on by the time since the last frame, but never by more than
    /// `GLIDE_STEP`: frames that change the scale lay text out anew and can
    /// be slow, and timed by the clock alone a few of them would skip the
    /// zoom, the view seeming to jump. Slow frames make the trip longer instead.
    pub(super) fn tick_glide(&mut self, cx: &mut Context<Self>) {
        let Some(g) = self.glide.as_mut() else { return };
        let now = Instant::now();
        // The first frame only starts the clock: the time before it may be long.
        g.f = advance(g.f, g.last.map_or(0., |t| (now - t).as_secs_f32()));
        g.last = Some(now);
        let (from, to, f) = (g.from, g.to, g.f);
        if f >= 1. {
            self.m.cam = to;
            self.glide = None;
            if from.s != to.s {
                self.note_zoom(cx);
            }
            self.request_save(cx);
            return;
        }
        self.m.cam = glide_at(from, to, self.view, f);
        if self.m.cam.s != from.s {
            self.note_zoom(cx);
        }
        self.animating = true;
    }

    pub(super) fn zoom_at(&mut self, x: f32, y: f32, factor: f32, cx: &mut Context<Self>) {
        self.glide = None;
        self.last_pointer = (x, y);
        self.zoom_anchor = (x, y);
        self.zoom_gen += 1;
        self.m.zoom_at(x, y, factor);
        self.note_zoom(cx);
    }

    pub(super) fn fit_all(&mut self, _: &FitAll, _: &mut Window, cx: &mut Context<Self>) {
        self.m.fit_all(self.view.0, self.view.1);
        self.lay = self.m.cam;
        self.zoom_settle = None;
        self.snap.on = false;
        cx.notify();
    }
    pub(super) fn zoom_in(&mut self, _: &ZoomIn, _: &mut Window, cx: &mut Context<Self>) {
        self.m.zoom_step(self.view.0 / 2.0, self.view.1 / 2.0, 1);
        self.lay = self.m.cam;
        self.zoom_settle = None;
        self.snap.on = false;
        cx.notify();
    }
    pub(super) fn zoom_out(&mut self, _: &ZoomOut, _: &mut Window, cx: &mut Context<Self>) {
        self.m.zoom_step(self.view.0 / 2.0, self.view.1 / 2.0, -1);
        self.lay = self.m.cam;
        self.zoom_settle = None;
        self.snap.on = false;
        cx.notify();
    }
}

/// How long the camera takes to glide where an agent asked.
const GLIDE_TIME: Duration = Duration::from_millis(520);
/// The most of the glide's time one frame may take, in seconds.
const GLIDE_STEP: f32 = 1. / 24.;

/// How far along a glide is after a frame dt seconds long, from f: never more than a step further.
fn advance(f: f32, dt: f32) -> f32 {
    (f + dt.clamp(0., GLIDE_STEP) / GLIDE_TIME.as_secs_f32()).min(1.)
}

/// A glide under way: from where, to where, how far along (0 to 1), and when its last frame was.
#[derive(Clone, Copy, Debug)]
pub(super) struct Glide {
    pub from: Camera,
    pub to: Camera,
    f: f32,
    last: Option<Instant>,
}

/// The camera f (0 to 1) of the way from `from` to `to` in a view of size
/// view: eased in and out; the center straight, the scale geometric, and
/// dipping out mid-way when the trip is long next to the view.
fn glide_at(from: Camera, to: Camera, view: (f32, f32), f: f32) -> Camera {
    let e = if f < 0.5 { 4. * f * f * f } else { 1. - (-2. * f + 2.).powi(3) / 2. };
    let (vw, vh) = view;
    let (ax, ay) = from.to_world(vw / 2., vh / 2.);
    let (bx, by) = to.to_world(vw / 2., vh / 2.);
    let (cx, cy) = (ax + (bx - ax) * e, ay + (by - ay) * e);
    let mut s = (from.s.ln() + (to.s.ln() - from.s.ln()) * e).exp();
    // How many views long the trip is, at the smaller scale: far trips dip out.
    let far = ((bx - ax).hypot(by - ay) * from.s.min(to.s) / vw.max(1.)).min(4.);
    let dip = 1. - (far / 4. * 0.6) * (std::f32::consts::PI * e).sin();
    s = (s * dip).clamp(model::MIN_SCALE, model::MAX_SCALE);
    Camera { x: cx - vw / 2. / s, y: cy - vh / 2. / s, s }
}

#[cfg(test)]
mod tests {
    use super::{GLIDE_STEP, GLIDE_TIME, advance, glide_at};
    use crate::model::Camera;

    #[test]
    fn a_glide_starts_and_ends_where_asked_and_dips_on_a_long_way() {
        let view = (1000., 800.);
        let from = Camera { x: 0., y: 0., s: 1. };
        let to = Camera { x: 5000., y: 0., s: 1. };
        let start = glide_at(from, to, view, 0.);
        assert!((start.x - from.x).abs() < 1e-3 && (start.s - 1.).abs() < 1e-6);
        let end = glide_at(from, to, view, 1.);
        assert!((end.x - to.x).abs() < 1e-2 && (end.s - 1.).abs() < 1e-4);
        let mid = glide_at(from, to, view, 0.5);
        assert!(mid.s < 0.6, "a long way zooms out mid-way: {}", mid.s);
        let near = glide_at(from, Camera { x: 100., y: 0., s: 1. }, view, 0.5);
        assert!(near.s > 0.95, "a short way hardly does: {}", near.s);
    }

    #[test]
    fn a_slow_frame_does_not_skip_the_way() {
        let frame = 1. / 60.;
        assert!((advance(0., frame) - frame / GLIDE_TIME.as_secs_f32()).abs() < 1e-6, "a smooth frame keeps time");
        // A frame of 150 ms, as one that lays all text out anew may take, goes only a step.
        let slow = advance(0.3, 0.15);
        assert!((slow - (0.3 + GLIDE_STEP / GLIDE_TIME.as_secs_f32())).abs() < 1e-6);
        assert!(slow - 0.3 < 0.1, "at most a tenth of the way a frame");
        assert_eq!(advance(0.99, 1.), 1., "it ends where it was going");
    }
}
