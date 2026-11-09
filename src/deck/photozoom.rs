//! Zooming as a photo is zoomed. A pinch or ⌘+wheel does not lay the canvas
//! out anew each frame (that reshapes and rasterizes all the text in sight,
//! and drops frames): the picture last drawn is scaled and moved by Core
//! Animation instead (`platform::layerzoom`), on the GPU, and GPUI keeps
//! drawing, when it draws at all, with the camera the picture was drawn
//! with (`Photo::drawn`).
//!
//! The chrome (toolbar, status bar, panels) is drawn in the same picture and
//! would be scaled with it, so it steps aside while zooming and fades back
//! in after (`CHROME_IN`).
//!
//! The picture is swapped for a sharp one only when the zoom rests, in the
//! frame the map comes off (`layerzoom::present`). Not on the way: a swap
//! lays out everything in sight at a new scale on the main thread (tens of
//! ms on a page of text), and one mid-gesture stalls the fingers. Zoomed in
//! a long way the picture softens until then; zoomed out past it, the
//! canvas's color shows at its edges (`layerzoom`'s underlay).
//!
//! Zooming out, the first picture is drawn wider than the window: `OVERSCAN`
//! times the view around the pointer, at a smaller scale, and shown scaled
//! up to look as the view does.

use std::time::{Duration, Instant};

use gpui::Context;

use super::DeckView;
use crate::model::{Camera, LOD_SCALE, MIN_SCALE};
use crate::platform::layerzoom::{self, Map, map_between};

/// Zooming out, how much wider than the view the picture is drawn: as far
/// out as a gesture goes before its edges show the canvas's bare color.
/// Only the blocks in the view it started from are drawn whole in it, the
/// rest as far away ones are (`photo_far`), so a wide picture costs about
/// what the view did.
const OVERSCAN: f32 = 4.0;
/// How long the chrome takes to come back.
const CHROME_IN: Duration = Duration::from_millis(180);

#[derive(Default)]
pub(super) struct Photo {
    /// While zooming as a photo: the camera the picture showing was drawn with.
    drawn: Option<Camera>,
    /// A swap is on its way (to start or end): the map waits for it.
    pending: bool,
    /// The zoom went out last: a new picture is drawn wider than the view.
    out: bool,
    /// Zooming out, the world the view showed when the picture was taken
    /// (x, y, w, h): blocks outside it are drawn far (`photo_far`).
    core: Option<(f32, f32, f32, f32)>,
    /// When the chrome came back.
    chrome_since: Option<Instant>,
}

#[derive(Clone, Copy)]
enum Swap {
    Start,
    End,
}

impl DeckView {
    /// The camera the picture showing was drawn with, while zooming as a photo.
    pub(super) fn photo_drawn(&self) -> Option<Camera> {
        self.photo.drawn
    }

    /// After a zoom or pan gesture moved the camera: the picture moves, or a
    /// frame is drawn, as zooming goes.
    pub(super) fn photo_moved(&mut self, zoomed: bool, before: Camera, cx: &mut Context<Self>) {
        if zoomed && self.m.cam.s != before.s {
            self.photo.out = self.m.cam.s < before.s;
        }
        let Some(drawn) = self.photo.drawn else {
            if zoomed && layerzoom::available() {
                // The first picture shows the view as it was, drawn wider if going out.
                self.photo.drawn = Some(picture_for(before, self.last_pointer, self.photo.out));
                self.photo.core = self.photo.out.then(|| (before.x, before.y, self.view.0 / before.s, self.view.1 / before.s));
                self.photo.pending = true;
                self.photo_swap(Swap::Start, cx);
            } else {
                cx.notify();
            }
            return;
        };
        if self.photo.pending {
            return;
        }
        // No new picture while the fingers move: one costs a layout of all
        // in sight at a new scale, tens of ms on a page of text, and stalls
        // the gesture. Scaled up it softens, scaled down the canvas's color
        // shows at its edges (`layerzoom`'s underlay), until it rests.
        super::zoomlayer::framestat::bare(!covers(drawn, self.m.cam, self.view));
        layerzoom::set(map_between(drawn, self.m.cam));
        // The frame that ends the zoom lays every line in sight out at the
        // scale it rests at: they are laid out ahead, off this thread, while
        // the fingers move (GPUI's `prefetch_scaled`, our patch).
        if let Some(ts) = &self.text_sys {
            ts.prefetch_scaled(self.m.cam.s / drawn.s);
        }
    }

    /// Before each frame: a zoom that has rested ends, the sharp picture taking over.
    pub(super) fn photo_tick(&mut self, cx: &mut Context<Self>) {
        let resting = !self.zoom_settle.is_some_and(|t| Instant::now() < t);
        if self.photo.drawn.is_some() && !self.photo.pending && resting {
            self.photo.pending = true;
            self.photo_swap(Swap::End, cx);
        }
        if self.photo.chrome_since.is_some_and(|t| t.elapsed() < CHROME_IN) {
            self.animating = true;
        }
    }

    /// The zoom has rested (the settle timer): the sharp picture takes over
    /// now, without first drawing a frame of the old one. False when no
    /// picture is showing.
    pub(super) fn photo_rest(&mut self, cx: &mut Context<Self>) -> bool {
        if self.photo.drawn.is_none() {
            return false;
        }
        // A swap on its way draws a frame, and that frame's `photo_tick` ends it.
        if !self.photo.pending {
            self.photo.pending = true;
            self.photo_swap(Swap::End, cx);
        }
        true
    }

    /// Whether the picture on screen is a photo being scaled: frames drawn
    /// now would only redraw what Core Animation is scaling, so fades and
    /// other animations wait for it to end instead of asking for frames.
    pub(super) fn photo_showing(&self) -> bool {
        self.photo.drawn.is_some()
    }

    /// Whether a block is drawn as far away ones are in the picture showing:
    /// zooming out, those outside the view it started from. They come back
    /// whole once it ends, a few a frame (`LodWake`), fading in.
    pub(super) fn photo_far(&self, b: &crate::model::Block) -> bool {
        let (Some(_), Some((x, y, w, h))) = (self.photo.drawn, self.photo.core) else { return false };
        b.x + b.w < x || b.y + b.h < y || b.x > x + w || b.y > y + h
    }

    /// How much of the chrome shows: none while zooming as a photo, then fading in.
    pub(super) fn chrome_alpha(&self) -> f32 {
        if self.photo.drawn.is_some() {
            return 0.;
        }
        match self.photo.chrome_since {
            Some(t) if !super::sharpen::reduce_motion() => {
                let f = (t.elapsed().as_secs_f32() / CHROME_IN.as_secs_f32()).min(1.);
                1. - (1. - f).powi(3)
            }
            _ => 1.,
        }
    }

    /// Swaps the picture in a task of its own, where nothing of GPUI is
    /// borrowed: the state changes, then the frame is drawn and shown with
    /// its map in one transaction.
    fn photo_swap(&mut self, swap: Swap, cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx| {
            let map = this.update(cx, |d, cx| {
                match swap {
                    Swap::Start => {}
                    Swap::End => {
                        d.photo.drawn = None;
                        d.photo.core = None;
                        d.photo.chrome_since = Some(Instant::now());
                    }
                }
                d.photo.pending = false;
                cx.notify();
                d.photo.drawn.map_or(Map::NONE, |drawn| map_between(drawn, d.m.cam))
            });
            if let Ok(map) = map {
                let t = Instant::now();
                layerzoom::present(map);
                super::zoomlayer::framestat::swap(t.elapsed());
                super::zoomlayer::framestat::swap_done();
            }
        })
        .detach();
    }

    /// Leaving the deck: the picture is the window's own again.
    pub(super) fn photo_off(&mut self) {
        if self.photo.drawn.take().is_some() {
            layerzoom::set(Map::NONE);
        }
    }
}

/// The camera to draw a picture of the view `live` with: the view itself
/// zooming in; zooming out, `OVERSCAN` times as wide around the pointer
/// (window points), but not so far out that blocks would be drawn as far
/// away ones are (`LOD_SCALE`) while the view shows them whole.
fn picture_for(live: Camera, (px, py): (f32, f32), out: bool) -> Camera {
    if !out {
        return live;
    }
    let mut s = (live.s / OVERSCAN).max(MIN_SCALE);
    if live.s >= LOD_SCALE {
        s = s.max(LOD_SCALE);
    }
    let (wx, wy) = live.to_world(px, py);
    Camera { x: wx - px / s, y: wy - py / s, s }
}

/// Whether a picture drawn with `drawn` still shows all of the view `live`
/// sees, in a window `view` points big: no bare edge would show.
fn covers(drawn: Camera, live: Camera, (vw, vh): (f32, f32)) -> bool {
    let e = 0.5 / live.s;
    live.x >= drawn.x - e
        && live.y >= drawn.y - e
        && live.x + vw / live.s <= drawn.x + vw / drawn.s + e
        && live.y + vh / live.s <= drawn.y + vh / drawn.s + e
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zooming_out_the_picture_is_wider_and_lasts_until_the_view_leaves_it() {
        let view = (1000., 800.);
        let live = Camera { x: 0., y: 0., s: 2. };
        let pointer = (300., 200.);
        let drawn = picture_for(live, pointer, true);
        assert!((drawn.s - 2. / OVERSCAN).abs() < 1e-6);
        // The point under the pointer stays under it.
        assert_eq!(drawn.to_screen(150., 100.).0.round(), 300.);
        // It shows the view, and the view zoomed out about the pointer, until OVERSCAN.
        assert!(covers(drawn, live, view));
        let mut m = crate::model::Model { cam: live, ..Default::default() };
        m.zoom_at(pointer.0, pointer.1, 1.1 / OVERSCAN);
        assert!(covers(drawn, m.cam, view), "still inside");
        m.zoom_at(pointer.0, pointer.1, 1. / 1.2);
        assert!(!covers(drawn, m.cam, view), "past it: its edges are bare");
        // Zooming in, the picture is the view itself.
        assert_eq!(picture_for(live, pointer, false), live);
    }

    #[test]
    fn a_wider_picture_does_not_drop_blocks_to_their_far_look() {
        let live = Camera { x: 0., y: 0., s: 0.35 };
        assert_eq!(picture_for(live, (0., 0.), true).s, LOD_SCALE);
        let far = Camera { x: 0., y: 0., s: 0.2 };
        assert!((picture_for(far, (0., 0.), true).s - (0.2 / OVERSCAN).max(MIN_SCALE)).abs() < 1e-6);
    }
}
