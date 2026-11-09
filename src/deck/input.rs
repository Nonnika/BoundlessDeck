//! Presses, drags, releases and the wheel on the canvas and its blocks,
//! and which block has the focus.

use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};

use gpui::{Context, MouseDownEvent, MouseMoveEvent, MouseUpEvent, ScrollWheelEvent, Window, px};

use super::{DRAG_BAND, DeckView, Drag, EditPart, GESTURE_HOLD, TITLE_H, marquee_over};
use crate::model::{BlockKind, Geometry, LOD_SCALE, scale_keeping_ratio, wheel_zoom};

impl DeckView {
    /// A press on empty canvas: the marquee, a double click adds a QuickLook,
    /// the band along the top drags the window.
    pub(super) fn on_canvas_down(&mut self, ev: &MouseDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let (x, y) = (f32::from(ev.position.x), f32::from(ev.position.y));
        self.popup = None;
        self.focus_block(None, window, cx);
        if y < DRAG_BAND {
            if ev.click_count == 2 {
                window.titlebar_double_click();
            } else {
                window.start_window_move();
            }
            return;
        }
        if ev.click_count == 2 {
            self.add_kind_at(BlockKind::Note, x, y, window, cx);
            return;
        }
        let w = self.m.cam.to_world(x, y);
        self.drag = Some(Drag::Marquee { from: w, to: w });
        cx.notify();
    }

    pub(super) fn on_pan_down(&mut self, ev: &MouseDownEvent, _: &mut Window, cx: &mut Context<Self>) {
        self.glide = None;
        let (x, y) = (f32::from(ev.position.x), f32::from(ev.position.y));
        self.drag = Some(Drag::Pan { from: (x, y), cam: self.m.cam });
        cx.notify();
    }

    pub(super) fn on_move(&mut self, ev: &MouseMoveEvent, cx: &mut Context<Self>) {
        let (x, y) = (f32::from(ev.position.x), f32::from(ev.position.y));
        self.last_pointer = (x, y);
        if self.scrub_move(ev, cx) || self.focus_drag_move(ev, cx) {
            return;
        }
        let cam = self.m.cam;
        match &mut self.drag {
            None => return,
            Some(Drag::Pluck) => self.pluck_move(x, y),
            Some(Drag::Pan { from, cam: c0 }) => {
                self.m.cam.x = c0.x - (x - from.0) / cam.s;
                self.m.cam.y = c0.y - (y - from.1) / cam.s;
            }
            Some(Drag::Marquee { to, .. }) => *to = cam.to_world(x, y),
            Some(Drag::Move { id, off, from, moved, .. }) => {
                if (x - from.0).abs() > 3.0 || (y - from.1).abs() > 3.0 {
                    *moved = true;
                }
                let (wx, wy) = cam.to_world(x, y);
                let (ox, oy) = *off;
                let id = id.clone();
                if let Some(b) = self.m.block_mut(&id) {
                    b.x = wx + ox;
                    b.y = wy + oy;
                }
            }
            Some(Drag::Resize { id, from, start, .. }) => {
                let (wx, wy) = cam.to_world(x, y);
                let (w, h) = scale_keeping_ratio(start.0, start.1, wx - from.0, wy - from.1);
                let id = id.clone();
                if let Some(b) = self.m.block_mut(&id) {
                    b.w = w;
                    b.h = h;
                }
            }
        }
        cx.notify();
    }

    pub(super) fn on_up(&mut self, ev: &MouseUpEvent, window: &mut Window, cx: &mut Context<Self>) {
        if self.scrub_up(window, cx) || self.focus_drag_up(cx) {
            return;
        }
        let (x, y) = (f32::from(ev.position.x), f32::from(ev.position.y));
        let Some(drag) = self.drag.take() else { return };
        match drag {
            Drag::Pan { .. } => {}
            Drag::Pluck => {
                if self.pluck_release(x, y) {
                    self.after_edit(cx);
                }
                self.pluck = None;
            }
            Drag::Marquee { from, to } => {
                let (x0, y0) = (from.0.min(to.0), from.1.min(to.1));
                self.m.select_in(x0, y0, (to.0 - from.0).abs(), (to.1 - from.1).abs());
            }
            Drag::Move { id, moved, before, .. } => {
                if moved {
                    // Let go over a stack: the stack takes it.
                    let stack = self.stack_at(x, y, &id);
                    match stack {
                        Some(st) if self.m.block(&id).is_some_and(|b| b.kind != BlockKind::Stack) => {
                            // Back where it was first, so an undo of the stack edit shows it there.
                            if let Some(g) = before.get(&id)
                                && let Some(b) = self.m.block_mut(&id)
                            {
                                (b.x, b.y) = (g.x, g.y);
                            }
                            self.m.stack_add(&st, &id);
                        }
                        _ => {
                            self.m.record_moves(&before);
                        }
                    }
                    self.after_edit(cx);
                }
            }
            Drag::Resize { id, before, .. } => {
                let mut b = HashMap::new();
                b.insert(id, before);
                self.m.record_moves(&b);
                self.after_edit(cx);
            }
        }
        cx.notify();
    }

    pub(super) fn on_wheel(&mut self, ev: &ScrollWheelEvent, cx: &mut Context<Self>) {
        self.glide = None;
        let d = ev.delta.pixel_delta(px(20.));
        let (mut dx, mut dy) = (f32::from(d.x), f32::from(d.y));
        let (x, y) = (f32::from(ev.position.x), f32::from(ev.position.y));
        if ev.modifiers.platform || ev.modifiers.control {
            self.zoom_at(x, y, wheel_zoom(-dy), cx);
            return;
        }
        if ev.modifiers.shift && dx == 0.0 {
            (dx, dy) = (dy, 0.0);
        }
        // The wheel scrolls content: the canvas moves the other way.
        let before = self.m.cam;
        self.m.cam.x -= dx / self.m.cam.s;
        self.m.cam.y -= dy / self.m.cam.s;
        let until = Instant::now() + GESTURE_HOLD;
        let was_holding = self.canvas_scroll_until.is_some_and(|t| t > Instant::now());
        self.canvas_scroll_until = Some(until);
        if !was_holding {
            // A frame to take the covers away once the gesture rests.
            cx.spawn(async move |this, cx| {
                loop {
                    cx.background_executor().timer(GESTURE_HOLD).await;
                    let done = this
                        .update(cx, |d, cx| {
                            let done = d.canvas_scroll_until.is_none_or(|t| t <= Instant::now());
                            if done {
                                cx.notify();
                            }
                            done
                        })
                        .unwrap_or(true);
                    if done {
                        break;
                    }
                }
            }).detach();
        }
        // Mid-zoom, the picture moves with it; else a frame is drawn.
        self.photo_moved(false, before, cx);
    }

    pub(super) fn begin_move(&mut self, id: &str, x: f32, y: f32) {
        let Some(b) = self.m.block(id) else { return };
        let (wx, wy) = self.m.cam.to_world(x, y);
        let sel: HashSet<String> = self.m.sel.clone();
        let mut before = HashMap::new();
        before.insert(id.to_string(), Geometry::of(b));
        let _ = sel;
        self.drag = Some(Drag::Move {
            id: id.to_string(),
            off: (b.x - wx, b.y - wy),
            from: (x, y),
            moved: false,
            before,
        });
    }

    /// A press on a card: the title bar selects and drags it, a double click
    /// on it edits the title; a click on a QuickLook's body edits it, on a
    /// 笔记's focuses it (a double click edits it).
    pub(super) fn on_card_down(&mut self, id: &str, ev: &MouseDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        cx.stop_propagation();
        self.popup = None;
        let Some(b) = self.m.block(id) else { return };
        let (x, y) = (f32::from(ev.position.x), f32::from(ev.position.y));
        let (_, sy) = self.m.cam.to_screen(b.x, b.y);
        let far = self.m.cam.s < LOD_SCALE;
        let kind = b.kind;
        // A 文字 or 图形 has no title bar: a press on it moves it like one,
        // and a double click edits its words. Its title floats above it.
        let in_title = if kind.bare() {
            y < sy || far || ev.click_count < 2
        } else {
            y - sy <= TITLE_H * self.m.cam.s || far
        };
        if self.active.as_deref() != Some(id) {
            self.active = None;
        }
        if self.editing.as_ref().is_some_and(|e| e.id != id) {
            self.stop_editing(cx);
        }
        if ev.modifiers.shift {
            if !self.m.sel.remove(id) {
                self.m.sel.insert(id.to_string());
            }
            cx.notify();
            return;
        }
        if in_title {
            if ev.click_count >= 2 && !far {
                self.start_editing(id, EditPart::Title, window, cx);
                return;
            }
            self.m.raise(id);
            if !self.m.sel.contains(id) {
                self.m.sel = HashSet::from([id.to_string()]);
            }
            self.begin_move(id, x, y);
            window.focus(&self.focus);
            cx.notify();
        } else {
            self.m.sel.clear();
            match kind {
                BlockKind::Note => self.start_editing(id, EditPart::Body, window, cx),
                // 笔记: a click focuses it, a double click edits it.
                BlockKind::Memo => {
                    if ev.click_count >= 2 {
                        self.start_editing(id, EditPart::Body, window, cx);
                    } else if self.editing(id, EditPart::Body).is_none() {
                        self.focus_block(Some(id), window, cx);
                    }
                }
                BlockKind::Chart => {
                    self.m.sel.insert(id.to_string());
                    if ev.click_count >= 2 {
                        self.start_editing(id, EditPart::Body, window, cx);
                    }
                }
                BlockKind::Stack => {
                    window.focus(&self.focus);
                    self.stack_press(id, x, y);
                }
                BlockKind::Text | BlockKind::Shape => {
                    self.m.sel.insert(id.to_string());
                    self.start_editing(id, EditPart::Body, window, cx);
                }
                _ => self.focus_block(Some(id), window, cx),
            }
        }
        cx.notify();
    }

    /// Makes `id` the block worked in, or none: the one way a block gains or
    /// loses focus. Editing elsewhere ends, and the keyboard goes to the
    /// block's terminal or to the canvas. Nothing else changes: losing focus
    /// never hides, reloads or redraws a block's content, so no kind needs
    /// blur handling of its own. (A page clicked into gives back the keyboard
    /// and the wheel on the next press elsewhere, in `webfocus`.)
    pub(super) fn focus_block(&mut self, id: Option<&str>, window: &mut Window, cx: &mut Context<Self>) {
        if self.editing.as_ref().is_some_and(|e| Some(e.id.as_str()) != id) {
            self.stop_editing(cx);
        }
        self.active = id.map(str::to_string);
        crate::platform::webfocus::release();
        // The canvas takes the keyboard unless the block's `on_focus` does; one
        // focused already keeps it.
        if !id.is_some_and(|i| self.rts.get(i).is_some_and(|r| r.life.focused)) {
            window.focus(&self.focus);
        }
        self.sync_focus(window, cx);
    }

    /// Whether a canvas gesture under way changes what lies at r (window
    /// points): a marquee grown over it, or a pan that has moved the canvas.
    /// A press alone changes nothing, so it hides no page.
    pub(super) fn gesture_covers(&self, r: (f32, f32, f32, f32)) -> bool {
        match &self.drag {
            Some(Drag::Pan { cam, .. }) => cam.x != self.m.cam.x || cam.y != self.m.cam.y,
            Some(Drag::Marquee { from, to }) => marquee_over(self.m.cam, *from, *to, r),
            _ => false,
        }
    }

    pub(super) fn on_resize_down(&mut self, id: &str, ev: &MouseDownEvent, _: &mut Window, cx: &mut Context<Self>) {
        cx.stop_propagation();
        let Some(b) = self.m.block(id) else { return };
        let from = self.m.cam.to_world(f32::from(ev.position.x), f32::from(ev.position.y));
        self.drag = Some(Drag::Resize { id: id.to_string(), from, start: (b.w, b.h), before: Geometry::of(b) });
        cx.notify();
    }

    /// The click on a body's cover that makes the block the one worked in.
    pub(super) fn on_cover_down(&mut self, id: &str, ev: &MouseDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        cx.stop_propagation();
        self.popup = None;
        if ev.modifiers.shift {
            if !self.m.sel.remove(id) {
                self.m.sel.insert(id.to_string());
            }
            cx.notify();
            return;
        }
        self.m.sel.clear();
        self.canvas_scroll_until = None;
        self.focus_block(Some(id), window, cx);
        cx.notify();
    }

    pub(super) fn expire_notes(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let editing = self.editing.as_ref().map(|e| e.id.clone());
        let (gone, wake) = self.m.expire_notes(editing.as_deref());
        if gone {
            self.after_edit(cx);
        }
        let _ = window;
        if let Some(ms) = wake {
            let at = Instant::now() + Duration::from_millis(ms as u64);
            if self.wake_at.is_none_or(|w| w <= Instant::now() || at < w) {
                self.wake_at = Some(at);
                cx.spawn(async move |this, cx| {
                    cx.background_executor().timer(Duration::from_millis(ms as u64)).await;
                    let _ = this.update(cx, |d, cx| {
                        d.wake_at = None;
                        cx.notify();
                    });
                }).detach();
            }
        }
    }
}
