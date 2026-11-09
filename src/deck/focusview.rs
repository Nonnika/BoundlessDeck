//! 焦点视图 and 放映: views of the canvas kept as 焦点 (`model::focus`), and
//! played in order like slides.
//!
//! The focus view dims the canvas and marks each focus with a dot in the
//! accent at the center of its view, the dots of the line shown joined in
//! its order. A panel on the left lists them: rows are dragged (or moved
//! with ← →) into order, renamed with a double click, taken again from the
//! view, or let go. A line of its own is made from the panel, and a click
//! on a dot adds that focus to it; a dot dragged moves its focus.
//!
//! The panel is the style panel's (`inspector`) mirror on the left: as wide,
//! as far off the edge, sliding in from the window's left edge and back out
//! smeared the same way, in the same time. The canvas dims with it.
//! ⌘⇧F keeps the view as a focus, the view open or not.
//!
//! F5 plays: the foci in their list's order while there are no lines, the
//! line when there is one, and with more it asks which (lines only; the
//! list itself is not one to play): space, → or
//! ↓ go on, return, ← or ↑ go back, Esc ends. The camera glides from stop
//! to stop as `set_view` does; the canvas can still be dragged and zoomed
//! while playing, and the chrome steps aside.

use std::time::{Duration, Instant};

use gpui::{
    AnyElement, AppContext, Context, CursorStyle, Entity, Focusable, InteractiveElement, IntoElement, KeyDownEvent,
    MouseButton, MouseDownEvent, MouseMoveEvent, ParentElement, SharedString, StatefulInteractiveElement, Styled,
    Subscription, Window, canvas, div, point, prelude::FluentBuilder, px, svg,
};

use super::inspector::{self, ENTER, GHOSTS, LEAVE, MAX_TRAIL, SHUTTER, TRAVEL, ease, ease_rate};
use super::{DeckView, KeepFocus, PlayFocus, ToggleFocusView, sharpen, shadow};
use crate::editor::{Editor, EditorEvent};
use crate::icons;
use crate::mcp::tools::{PlayCmd, Playing};
use crate::model::Focus;
use crate::theme::Theme;

/// The panel: where it sits and how wide, as the style panel on the right; and its rows.
const PANEL_LEFT: f32 = inspector::PANEL_RIGHT;
const PANEL_TOP: f32 = inspector::PANEL_TOP;
const PANEL_W: f32 = inspector::PANEL_W;
const ROW_H: f32 = 34.;
const ROW_GAP: f32 = 2.;
/// A dot's size, and how far a press on one goes before it is a drag.
const DOT: f32 = 14.;
const SLOP: f32 = 3.;
/// How long the stop's name shows while playing, and how long it takes to fade.
const HUD_SHOW: Duration = Duration::from_millis(2200);
const HUD_FADE: Duration = Duration::from_millis(400);

#[derive(Default)]
pub(super) struct FocusView {
    /// The focus view is shown.
    pub on: bool,
    /// The line the panel shows and the dots join: none is the list itself.
    line: Option<String>,
    /// The row picked in the panel.
    sel: Option<usize>,
    rename: Option<Rename>,
    row_drag: Option<RowDrag>,
    dot_drag: Option<DotDrag>,
    pub play: Option<Play>,
    /// Asking which line to play: the one picked, 0 being the list itself.
    chooser: Option<usize>,
    /// Where the panel covers the window this frame, for web pages under it to give way.
    pub rect: Option<(f32, f32, f32, f32)>,
    /// The canvas is to take the keyboard on the next frame.
    grab: bool,
    /// How far open the view is, 0 to 1, and when that was so; `at` is none while it rests.
    pub p: f32,
    at: Option<Instant>,
    /// A word that the view was kept, for a moment (⌘⇧F).
    toast: Option<(String, Instant)>,
}


pub(super) struct Play {
    line: Option<String>,
    at: usize,
    /// When it last stepped: the stop's name shows a while after.
    since: Instant,
}

#[derive(Clone, Debug, PartialEq)]
enum Target {
    Focus(String),
    Line(String),
}

struct Rename {
    what: Target,
    editor: Entity<Editor>,
    _sub: Subscription,
}

struct RowDrag {
    from: usize,
    y0: f32,
    dy: f32,
    moved: bool,
}

struct DotDrag {
    id: String,
    from: (f32, f32),
    start: (f32, f32),
    moved: bool,
}

/// Where a row dragged dy from `from` lands, among n.
fn row_target(from: usize, dy: f32, n: usize) -> usize {
    let steps = (dy / (ROW_H + ROW_GAP)).round() as i64;
    (from as i64 + steps).clamp(0, n.saturating_sub(1) as i64) as usize
}

impl DeckView {
    /// Where the window is playing, as MCP tools see it.
    pub(super) fn playing(&self) -> Option<Playing> {
        self.fv.play.as_ref().map(|p| Playing { line: p.line.clone(), at: p.at })
    }

    pub(super) fn toggle_focus_view(&mut self, _: &ToggleFocusView, window: &mut Window, cx: &mut Context<Self>) {
        self.set_focus_view(!self.fv.on, window, cx);
    }

    fn set_focus_view(&mut self, on: bool, window: &mut Window, cx: &mut Context<Self>) {
        self.fv.on = on;
        self.fv.rename = None;
        self.fv.row_drag = None;
        self.fv.dot_drag = None;
        if on {
            self.fv.play = None;
            // The keyboard to the canvas, for ← → and Esc.
            self.focus_block(None, window, cx);
        }
        cx.notify();
    }

    /// F5: plays. With no lines, the foci in their list's order; with one,
    /// that line; with more, it asks which (the lines only). Playing, it ends.
    pub(super) fn play_focus(&mut self, _: &PlayFocus, window: &mut Window, cx: &mut Context<Self>) {
        if self.fv.play.is_some() {
            self.stop_play(cx);
            return;
        }
        match self.m.focus_lines.len() {
            0 => self.start_play(None, 0, window, cx),
            1 => {
                let line = self.m.focus_lines[0].id.clone();
                self.start_play(Some(line), 0, window, cx);
            }
            _ => {
                let at = self.fv.line.as_deref().and_then(|l| self.m.focus_lines.iter().position(|x| x.id == l)).unwrap_or(0);
                self.fv.chooser = Some(at);
                self.focus_block(None, window, cx);
                cx.notify();
            }
        }
    }

    /// Starts playing a line at a stop. With nothing to play, the focus view opens to make some.
    fn start_play(&mut self, line: Option<String>, at: usize, window: &mut Window, cx: &mut Context<Self>) {
        self.fv.chooser = None;
        if self.m.stops(line.as_deref()).is_empty() {
            self.fv.line = line;
            self.set_focus_view(true, window, cx);
            return;
        }
        self.fv.on = false;
        self.focus_block(None, window, cx);
        self.go_to_stop(line, at, cx);
    }

    /// Glides to a stop of a line, playing it.
    fn go_to_stop(&mut self, line: Option<String>, at: usize, cx: &mut Context<Self>) {
        let stops = self.m.stops(line.as_deref());
        let Some(f) = stops.get(at.min(stops.len().saturating_sub(1))).map(|f| (*f).clone()) else {
            self.stop_play(cx);
            return;
        };
        let at = at.min(stops.len() - 1);
        self.fv.play = Some(Play { line, at, since: Instant::now() });
        let to = f.camera(self.view);
        self.glide_to(self.m.cam, to, true, cx);
    }

    fn step_play(&mut self, by: i64, cx: &mut Context<Self>) {
        let Some(p) = &self.fv.play else { return };
        let n = self.m.stops(p.line.as_deref()).len() as i64;
        let to = (p.at as i64 + by).clamp(0, (n - 1).max(0)) as usize;
        let line = p.line.clone();
        if to == p.at {
            // Already at the end (or the start): say so, without moving.
            if let Some(p) = self.fv.play.as_mut() {
                p.since = Instant::now();
            }
            cx.notify();
            return;
        }
        self.go_to_stop(line, to, cx);
    }

    fn stop_play(&mut self, cx: &mut Context<Self>) {
        self.fv.play = None;
        self.fv.chooser = None;
        cx.notify();
    }

    /// What `focus_play` asked. The keyboard comes to the canvas on the next
    /// frame (`grab`), so space and Esc reach playing.
    pub(super) fn apply_play(&mut self, cmd: PlayCmd, cx: &mut Context<Self>) {
        match cmd {
            PlayCmd::Stop => self.stop_play(cx),
            PlayCmd::Go(p) => {
                self.fv.on = false;
                self.fv.chooser = None;
                if self.fv.play.is_none() {
                    self.fv.grab = true;
                }
                self.go_to_stop(p.line, p.at, cx);
            }
        }
    }

    /// At the start of a frame: the keyboard to the canvas, when playing began from an agent.
    pub(super) fn grab_keys(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if std::mem::take(&mut self.fv.grab) {
            self.focus_block(None, window, cx);
        }
    }

    /// Keys while playing, choosing a line, or in the focus view. True when one was used.
    pub(super) fn focus_keys(&mut self, ev: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) -> bool {
        if !self.focus.is_focused(window) {
            return false;
        }
        let ks = &ev.keystroke;
        if ks.modifiers.platform || ks.modifiers.control || ks.modifiers.alt {
            return false;
        }
        let key = ks.key.as_str();
        if let Some(i) = self.fv.chooser {
            let n = self.m.focus_lines.len().max(1);
            match key {
                "up" => self.fv.chooser = Some((i + n - 1) % n),
                "down" | "tab" => self.fv.chooser = Some((i + 1) % n),
                "enter" | "space" => {
                    let line = self.m.focus_lines.get(i).map(|l| l.id.clone());
                    self.start_play(line, 0, window, cx);
                }
                "escape" => self.fv.chooser = None,
                _ => return false,
            }
            cx.notify();
            return true;
        }
        if self.fv.play.is_some() {
            match key {
                "space" | "right" | "down" | "pagedown" => self.step_play(1, cx),
                "enter" | "left" | "up" | "pageup" => self.step_play(-1, cx),
                "home" => self.step_play(-10_000, cx),
                "end" => self.step_play(10_000, cx),
                "escape" => self.stop_play(cx),
                _ => return false,
            }
            return true;
        }
        if self.fv.on && self.fv.rename.is_none() {
            let n = self.m.stops(self.fv.line.as_deref()).len();
            match (key, self.fv.sel.filter(|&i| i < n)) {
                ("left" | "up", Some(i)) if i > 0 => {
                    self.m.move_stop(self.fv.line.as_deref(), i, i - 1);
                    self.fv.sel = Some(i - 1);
                    self.focus_edited(cx);
                }
                ("right" | "down", Some(i)) if i + 1 < n => {
                    self.m.move_stop(self.fv.line.as_deref(), i, i + 1);
                    self.fv.sel = Some(i + 1);
                    self.focus_edited(cx);
                }
                ("left" | "up" | "right" | "down", _) => {}
                ("backspace" | "delete", Some(i)) => self.remove_row(i, cx),
                ("escape", _) => self.set_focus_view(false, window, cx),
                _ => return false,
            }
            return true;
        }
        false
    }

    fn focus_edited(&mut self, cx: &mut Context<Self>) {
        self.request_save(cx);
        cx.notify();
    }

    /// ⌘⇧F: keeps the view as a focus, and says so.
    pub(super) fn keep_focus(&mut self, _: &KeepFocus, _: &mut Window, cx: &mut Context<Self>) {
        let name = self.keep_view(cx);
        self.fv.toast = Some((format!("已存为焦点「{name}」"), Instant::now()));
        cx.notify();
    }

    /// Keeps the view as a focus, last in the list and on the line shown. Returns its name.
    fn keep_view(&mut self, cx: &mut Context<Self>) -> String {
        let (x, y, s) = Focus::of_camera(self.m.cam, self.view);
        let id = self.m.add_focus(None, x, y, s);
        let name = self.m.focus(&id).map(|f| f.name.clone()).unwrap_or_default();
        if let Some(l) = self.fv.line.clone()
            && let Some(line) = self.m.focus_lines.iter_mut().find(|x| x.id == l)
        {
            line.stops.push(id);
        }
        self.fv.sel = Some(self.m.stops(self.fv.line.as_deref()).len().saturating_sub(1));
        self.focus_edited(cx);
        name
    }

    /// A row let go: from a line its stop, from the list the focus itself.
    fn remove_row(&mut self, i: usize, cx: &mut Context<Self>) {
        match self.fv.line.clone() {
            Some(l) => {
                if let Some(line) = self.m.focus_lines.iter_mut().find(|x| x.id == l)
                    && i < line.stops.len()
                {
                    line.stops.remove(i);
                }
            }
            None => {
                if let Some(id) = self.m.focus.get(i).map(|f| f.id.clone()) {
                    self.m.remove_focus(&id);
                }
            }
        }
        let n = self.m.stops(self.fv.line.as_deref()).len();
        self.fv.sel = self.fv.sel.filter(|_| n > 0).map(|s| s.min(n - 1));
        self.focus_edited(cx);
    }

    /// Glides to a focus to look at it, not playing.
    fn look_at(&mut self, id: &str, cx: &mut Context<Self>) {
        let Some(f) = self.m.focus(id) else { return };
        let to = f.camera(self.view);
        self.glide_to(self.m.cam, to, true, cx);
    }

    fn start_rename(&mut self, what: Target, window: &mut Window, cx: &mut Context<Self>) {
        let name = match &what {
            Target::Focus(id) => self.m.focus(id).map(|f| f.name.clone()),
            Target::Line(id) => self.m.focus_line(id).map(|l| l.name.clone()),
        }
        .unwrap_or_default();
        let t = Theme::of(window);
        let editor = cx.new(|cx| {
            let mut e = Editor::new(name, false, window, cx);
            e.font_size = 12.5;
            e.color = t.text;
            e.muted = t.muted;
            e.accent = t.accent;
            e.valign = 0.5;
            e.select_all_text();
            e
        });
        let sub = cx.subscribe(&editor, |d: &mut DeckView, ed, ev: &EditorEvent, cx| {
            if !matches!(ev, EditorEvent::Done | EditorEvent::Blurred) {
                return;
            }
            let Some(r) = d.fv.rename.take() else { return };
            let name = ed.read(cx).text.trim().to_string();
            if !name.is_empty() {
                match r.what {
                    Target::Focus(id) => {
                        if let Some(f) = d.m.focus.iter_mut().find(|f| f.id == id) {
                            f.name = name;
                        }
                    }
                    Target::Line(id) => {
                        if let Some(l) = d.m.focus_lines.iter_mut().find(|l| l.id == id) {
                            l.name = name;
                        }
                    }
                }
                d.request_save(cx);
            }
            cx.notify();
        });
        crate::webfocus::release();
        window.focus(&editor.focus_handle(cx));
        self.fv.rename = Some(Rename { what, editor, _sub: sub });
        cx.notify();
    }

    /// The pointer moving while a row or a dot is pressed. False when neither is.
    pub(super) fn focus_drag_move(&mut self, ev: &MouseMoveEvent, cx: &mut Context<Self>) -> bool {
        let (x, y) = (f32::from(ev.position.x), f32::from(ev.position.y));
        if let Some(rd) = self.fv.row_drag.as_mut() {
            rd.dy = y - rd.y0;
            rd.moved |= rd.dy.abs() > SLOP;
            cx.notify();
            return true;
        }
        let cam = self.m.cam;
        if let Some(dd) = self.fv.dot_drag.as_mut() {
            if !dd.moved && (x - dd.from.0).abs().max((y - dd.from.1).abs()) < SLOP {
                return true;
            }
            dd.moved = true;
            let (nx, ny) = (dd.start.0 + (x - dd.from.0) / cam.s, dd.start.1 + (y - dd.from.1) / cam.s);
            let id = dd.id.clone();
            if let Some(f) = self.m.focus.iter_mut().find(|f| f.id == id) {
                (f.x, f.y) = (nx, ny);
            }
            cx.notify();
            return true;
        }
        false
    }

    /// Letting go of a row or a dot: a drag puts it there; a press that did
    /// not move picks it (and, for a dot, adds it to the line shown).
    pub(super) fn focus_drag_up(&mut self, cx: &mut Context<Self>) -> bool {
        if let Some(rd) = self.fv.row_drag.take() {
            let line = self.fv.line.clone();
            if rd.moved {
                let n = self.m.stops(line.as_deref()).len();
                let to = row_target(rd.from, rd.dy, n);
                self.m.move_stop(line.as_deref(), rd.from, to);
                self.fv.sel = Some(to);
                self.focus_edited(cx);
            } else {
                self.fv.sel = Some(rd.from);
                if let Some(id) = self.m.stops(line.as_deref()).get(rd.from).map(|f| f.id.clone()) {
                    self.look_at(&id, cx);
                }
                cx.notify();
            }
            return true;
        }
        if let Some(dd) = self.fv.dot_drag.take() {
            if dd.moved {
                self.focus_edited(cx);
                return true;
            }
            match self.fv.line.clone() {
                Some(l) => {
                    if let Some(line) = self.m.focus_lines.iter_mut().find(|x| x.id == l) {
                        line.stops.push(dd.id.clone());
                        self.fv.sel = Some(line.stops.len() - 1);
                    }
                    self.focus_edited(cx);
                }
                None => {
                    self.fv.sel = self.m.focus.iter().position(|f| f.id == dd.id);
                    cx.notify();
                }
            }
            return true;
        }
        false
    }

    /// Before the blocks are drawn: where the panel will be.
    pub(super) fn focus_tick(&mut self) {
        // Opening and closing: the panel unfolds, the canvas dims. Playing
        // starts at once: the view it leaves does not fade out over it.
        let goal = if self.fv.on && self.fv.play.is_none() { 1. } else { 0. };
        if sharpen::reduce_motion() || self.fv.p == goal || (goal == 0. && self.fv.play.is_some()) {
            self.fv.p = goal;
            self.fv.at = None;
        } else {
            let fv = &mut self.fv;
            let now = Instant::now();
            // The first frame of a move only starts its clock.
            let dt = fv.at.map_or(0., |a| (now - a).as_secs_f32());
            fv.at = Some(now);
            let dur = if goal > 0. { ENTER } else { LEAVE };
            fv.p = if goal > 0. { (fv.p + dt / dur.as_secs_f32()).min(1.) } else { (fv.p - dt / dur.as_secs_f32()).max(0.) };
            self.animating = true;
        }
        let left = PANEL_LEFT - (1. - ease(self.fv.p)) * TRAVEL;
        self.fv.rect = (self.fv.p > 0.).then_some((left, PANEL_TOP, PANEL_W, self.view.1 - PANEL_TOP - 48.));
        if let Some((_, at)) = &self.fv.toast {
            let e = at.elapsed();
            if e >= HUD_SHOW + HUD_FADE {
                self.fv.toast = None;
            } else if e > HUD_SHOW.saturating_sub(Duration::from_millis(50)) {
                self.animating = true;
            }
        }
        if let Some(p) = &self.fv.play {
            let n = self.m.stops(p.line.as_deref()).len();
            if n == 0 {
                self.fv.play = None;
            } else if p.at >= n {
                self.fv.play.as_mut().unwrap().at = n - 1;
            }
            // The name fades on its own: frames until it has.
            if let Some(p) = &self.fv.play {
                let e = p.since.elapsed();
                if e < HUD_SHOW + HUD_FADE && e > HUD_SHOW.saturating_sub(Duration::from_millis(50)) {
                    self.animating = true;
                }
            }
        }
    }

    /// Everything the focus view, the line chooser and playing draw over the canvas.
    pub(super) fn focus_layer(&mut self, t: &Theme, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let mut out = Vec::new();
        if self.fv.p > 0. {
            let e = ease(self.fv.p);
            out.push(div().absolute().left_0().top_0().size_full().opacity(e).children(self.focus_overlay(t, cx)).into_any_element());
            out.push(self.focus_slide(t, cx));
        }
        if let Some(c) = self.focus_hud(t) {
            out.push(c);
        }
        if let Some(c) = self.toast(t) {
            out.push(c);
        }
        if let Some(c) = self.chooser(t, cx) {
            out.push(c);
        }
        out
    }

    /// The canvas dimmed; the frame of the focus picked; the line's path; a dot per focus.
    fn focus_overlay(&mut self, t: &Theme, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let cam = self.m.cam;
        let (vw, vh) = self.view;
        let mut out = vec![div().absolute().left_0().top_0().size_full().bg(t.background.opacity(0.62)).into_any_element()];

        let stops: Vec<Focus> = self.m.stops(self.fv.line.as_deref()).into_iter().cloned().collect();
        let picked = self.fv.sel.and_then(|i| stops.get(i)).map(|f| f.id.clone());
        let dragged = self.fv.dot_drag.as_ref().filter(|d| d.moved).map(|d| d.id.clone());
        // The view the focus picked (or dragged) shows, at this window's size.
        if let Some(f) = dragged.as_ref().or(picked.as_ref()).and_then(|id| self.m.focus(id)) {
            let (fw, fh) = (vw / f.s * cam.s, vh / f.s * cam.s);
            let (sx, sy) = cam.to_screen(f.x, f.y);
            out.push(
                div()
                    .absolute()
                    .left(px(sx - fw / 2.))
                    .top(px(sy - fh / 2.))
                    .w(px(fw))
                    .h(px(fh))
                    .border(px(1.5))
                    .border_color(t.accent.opacity(0.75))
                    .rounded(px(6.))
                    .bg(t.accent.opacity(0.04))
                    .into_any_element(),
            );
        }
        // The path, stop to stop, with an arrowhead halfway along each leg.
        let pts: Vec<(f32, f32)> = stops.iter().map(|f| cam.to_screen(f.x, f.y)).collect();
        let accent = t.accent;
        out.push(
            canvas(|_, _, _| (), move |_, _, window, _| {
                for w in pts.windows(2) {
                    let ((x0, y0), (x1, y1)) = (w[0], w[1]);
                    let len = (x1 - x0).hypot(y1 - y0);
                    if len < DOT {
                        continue;
                    }
                    let (ux, uy) = ((x1 - x0) / len, (y1 - y0) / len);
                    let mut p = gpui::PathBuilder::stroke(px(2.));
                    p.move_to(point(px(x0 + ux * DOT * 0.7), px(y0 + uy * DOT * 0.7)));
                    p.line_to(point(px(x1 - ux * DOT * 0.7), px(y1 - uy * DOT * 0.7)));
                    if let Ok(p) = p.build() {
                        window.paint_path(p, accent.opacity(0.7));
                    }
                    let (mx, my) = ((x0 + x1) / 2. + ux * 5., (y0 + y1) / 2. + uy * 5.);
                    let mut h = gpui::PathBuilder::fill();
                    h.move_to(point(px(mx), px(my)));
                    h.line_to(point(px(mx - ux * 10. - uy * 5.), px(my - uy * 10. + ux * 5.)));
                    h.line_to(point(px(mx - ux * 10. + uy * 5.), px(my - uy * 10. - ux * 5.)));
                    h.close();
                    if let Ok(h) = h.build() {
                        window.paint_path(h, accent);
                    }
                }
            })
            .absolute()
            .left_0()
            .top_0()
            .size_full()
            .into_any_element(),
        );
        // A dot per focus: on the line shown, solid, with its places in it; else hollow.
        let foci: Vec<Focus> = self.m.focus.clone();
        for f in foci {
            let (sx, sy) = cam.to_screen(f.x, f.y);
            if sx < -200. || sy < -60. || sx > vw + 200. || sy > vh + 60. {
                continue;
            }
            let places: Vec<String> =
                stops.iter().enumerate().filter(|(_, s)| s.id == f.id).map(|(i, _)| (i + 1).to_string()).collect();
            let on = !places.is_empty();
            let picked = picked.as_deref() == Some(f.id.as_str());
            let id = f.id.clone();
            let (fx, fy) = (f.x, f.y);
            out.push(
                div()
                    .id(SharedString::from(format!("focus-dot:{}", f.id)))
                    .absolute()
                    .left(px(sx - DOT / 2.))
                    .top(px(sy - DOT / 2.))
                    .size(px(DOT))
                    .rounded_full()
                    .border_2()
                    .border_color(if on { t.accent } else { t.muted })
                    .when(on, |d| d.bg(t.accent))
                    .when(!on, |d| d.bg(t.surface))
                    .when(picked, |d| d.shadow(vec![gpui::BoxShadow {
                        color: t.accent.opacity(0.45),
                        offset: point(px(0.), px(0.)),
                        blur_radius: px(0.),
                        spread_radius: px(4.),
                    }]))
                    .cursor(CursorStyle::PointingHand)
                    .occlude()
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |d, ev: &MouseDownEvent, _, cx| {
                            let p = (f32::from(ev.position.x), f32::from(ev.position.y));
                            d.fv.dot_drag = Some(DotDrag { id: id.clone(), from: p, start: (fx, fy), moved: false });
                            cx.stop_propagation();
                        }),
                    )
                    .into_any_element(),
            );
            let label = if on { format!("{} · {}", places.join("、"), f.name) } else { f.name.clone() };
            out.push(
                div()
                    .absolute()
                    .left(px(sx + DOT / 2. + 6.))
                    .top(px(sy - 10.))
                    .h(px(20.))
                    .px(px(7.))
                    .flex()
                    .items_center()
                    .rounded(px(6.))
                    .bg(t.surface)
                    .border_1()
                    .border_color(if on { t.accent.opacity(0.5) } else { t.border })
                    .text_size(px(11.))
                    .whitespace_nowrap()
                    .text_color(if on { t.text } else { t.muted })
                    .child(label)
                    .into_any_element(),
            );
        }
        out
    }

    /// The panel on the left: the lines, the foci of the one shown, and keeping the view.
    /// The panel sliding in from the left edge, or out to it, smeared as the
    /// style panel is (`inspector::inspector`), the trail on the side it comes from.
    fn focus_slide(&mut self, t: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let p = self.fv.p;
        let coming = self.fv.on && self.fv.play.is_none();
        let left = PANEL_LEFT - (1. - ease(p)) * TRAVEL;
        // How far it moves in a shutter's time: the length of its smear, behind it.
        let trail = if self.fv.at.is_some() {
            let dur = if coming { ENTER } else { LEAVE };
            (TRAVEL * ease_rate(p) / dur.as_secs_f32() * SHUTTER).min(MAX_TRAIL)
        } else {
            0.
        };
        let behind = if coming { -1. } else { 1. };
        let blur = trail / MAX_TRAIL;
        let mut layer = div().absolute().top(px(PANEL_TOP)).left(px(left)).w(px(PANEL_W));
        if trail > 1. {
            for k in (1..=GHOSTS).rev() {
                let f = k as f32 / GHOSTS as f32;
                let ghost = self.focus_panel(false, t, cx);
                layer = layer.child(
                    div()
                        .id(("focus-ghost", k))
                        .absolute()
                        .top_0()
                        .left(px(behind * trail * f))
                        .w(px(PANEL_W))
                        .opacity((0.42 * (1. - f * 0.8) * blur.max(0.35)).min(1.))
                        .child(ghost),
                );
            }
            // The copies are only seen: this takes the presses that would reach them.
            layer = layer.child(div().absolute().top_0().bottom_0().left(px(-trail)).right(px(-trail)).occlude());
        }
        let main = self.focus_panel(true, t, cx);
        layer.child(div().relative().opacity(1. - 0.5 * blur).child(main)).into_any_element()
    }

    /// The panel's content. `live` is the one used; the smear's copies show
    /// names being typed as they were.
    fn focus_panel(&mut self, live: bool, t: &Theme, cx: &mut Context<Self>) -> gpui::Stateful<gpui::Div> {
        let line = self.fv.line.clone();
        let stops: Vec<Focus> = self.m.stops(line.as_deref()).into_iter().cloned().collect();
        let n = stops.len();
        let renaming = self.fv.rename.as_ref().filter(|_| live).map(|r| (r.what.clone(), r.editor.clone()));
        let drag = self.fv.row_drag.as_ref().filter(|r| r.moved).map(|r| (r.from, r.dy, row_target(r.from, r.dy, n)));

        let head = div()
            .flex()
            .flex_row()
            .items_center()
            .gap(px(6.))
            .child(div().flex_1().text_size(px(13.)).font_weight(gpui::FontWeight::SEMIBOLD).text_color(t.text).child("焦点"))
            .child(
                icon_button("focus-close", "x", t).on_click(cx.listener(|d, _, window, cx| d.set_focus_view(false, window, cx))),
            );

        // The lines: the list itself first, each line, and one more.
        let mut chips = div().flex().flex_row().flex_wrap().gap(px(4.));
        let chip = |id: SharedString, on: bool| {
            div()
                .id(id)
                .h(px(24.))
                .px(px(9.))
                .flex()
                .items_center()
                .gap(px(4.))
                .rounded(px(12.))
                .text_size(px(11.5))
                .cursor_pointer()
                .when(on, |d| d.bg(t.accent.opacity(0.16)).text_color(t.accent))
                .when(!on, |d| d.bg(t.background).text_color(t.text).hover(|d| d.bg(t.surface_hover)))
        };
        chips = chips.child(chip("line-all".into(), line.is_none()).child(format!("全部焦点 {}", self.m.focus.len())).on_click(
            cx.listener(|d, _, _, cx| {
                d.fv.line = None;
                d.fv.sel = None;
                cx.notify();
            }),
        ));
        for l in self.m.focus_lines.clone() {
            let on = line.as_deref() == Some(l.id.as_str());
            if let Some((Target::Line(rid), ed)) = &renaming
                && *rid == l.id
            {
                chips = chips.child(div().w(px(120.)).h(px(24.)).px(px(8.)).rounded(px(12.)).border_1().border_color(t.accent).child(ed.clone()));
                continue;
            }
            let (id, id2, id3) = (l.id.clone(), l.id.clone(), l.id.clone());
            chips = chips.child(
                chip(SharedString::from(format!("line:{}", l.id)), on)
                    .child(format!("{} {}", l.name, l.stops.len()))
                    .when(on, |d| {
                        d.child(
                            div()
                                .id(SharedString::from(format!("line-del:{}", id3)))
                                .size(px(14.))
                                .flex()
                                .items_center()
                                .justify_center()
                                .rounded_full()
                                .hover(|d| d.bg(t.surface_hover))
                                .child(svg().path(icons::path("x")).size(px(10.)).text_color(t.muted))
                                .on_click(cx.listener(move |d, _, _, cx| {
                                    cx.stop_propagation();
                                    d.m.remove_focus_line(&id3);
                                    d.fv.line = None;
                                    d.fv.sel = None;
                                    d.focus_edited(cx);
                                })),
                        )
                    })
                    .on_click(cx.listener(move |d, ev: &gpui::ClickEvent, window, cx| {
                        if ev.click_count() >= 2 {
                            d.start_rename(Target::Line(id2.clone()), window, cx);
                            return;
                        }
                        d.fv.line = Some(id.clone());
                        d.fv.sel = None;
                        cx.notify();
                    })),
            );
        }
        chips = chips.child(
            chip("line-add".into(), false)
                .text_color(t.muted)
                .child(svg().path(icons::path("plus")).size(px(11.)).text_color(t.muted))
                .child("焦点线")
                .on_click(cx.listener(|d, _, _, cx| {
                    let id = d.m.add_focus_line(None);
                    d.fv.line = Some(id);
                    d.fv.sel = None;
                    d.focus_edited(cx);
                })),
        );

        // The rows.
        let mut list = div().relative().flex().flex_col().gap(px(ROW_GAP));
        if stops.is_empty() {
            let hint = if line.is_some() {
                "这条线还没有焦点。点画布上的圆点，按顺序把焦点连进来。"
            } else {
                "还没有焦点。把视野移到要讲的地方，点下面的按钮存下来。"
            };
            list = list.child(div().py(px(14.)).px(px(4.)).text_size(px(12.)).line_height(px(18.)).text_color(t.muted).child(hint));
        }
        for (i, f) in stops.iter().enumerate() {
            let sel = self.fv.sel == Some(i);
            let lift = drag.filter(|d| d.0 == i).map(|d| d.1);
            let name: AnyElement = match &renaming {
                Some((Target::Focus(rid), ed)) if *rid == f.id && sel => {
                    div().flex_1().min_w_0().h(px(24.)).px(px(4.)).rounded(px(5.)).border_1().border_color(t.accent).child(ed.clone()).into_any_element()
                }
                _ => div()
                    .flex_1()
                    .min_w_0()
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .text_ellipsis()
                    .text_size(px(12.5))
                    .text_color(t.text)
                    .child(f.name.clone())
                    .into_any_element(),
            };
            let (fid, fid2) = (f.id.clone(), f.id.clone());
            let row = div()
                .id(("focus-row", i))
                .relative()
                .when_some(lift, |d, dy| d.top(px(dy)).bg(t.surface_hover).shadow(vec![shadow(t)]))
                .h(px(ROW_H))
                .flex()
                .flex_row()
                .items_center()
                .gap(px(8.))
                .px(px(8.))
                .rounded(px(7.))
                .cursor(CursorStyle::OpenHand)
                .when(sel && lift.is_none(), |d| d.bg(t.accent.opacity(0.14)))
                .when(!sel && lift.is_none(), |d| d.hover(|d| d.bg(t.surface_hover)))
                .child(
                    div()
                        .flex_none()
                        .size(px(20.))
                        .flex()
                        .items_center()
                        .justify_center()
                        .rounded_full()
                        .bg(if sel { t.accent } else { t.background })
                        .text_size(px(10.5))
                        .text_color(if sel { t.accent_text } else { t.muted })
                        .child((i + 1).to_string()),
                )
                .child(name)
                .child(div().flex_none().text_size(px(10.5)).text_color(t.muted).child(format!("{:.0}%", f.s * 100.)))
                .child(icon_button(SharedString::from(format!("focus-retake:{i}")), "reload", t).on_click(cx.listener(move |d, _, _, cx| {
                    cx.stop_propagation();
                    let (x, y, s) = Focus::of_camera(d.m.cam, d.view);
                    if let Some(f) = d.m.focus.iter_mut().find(|f| f.id == fid) {
                        (f.x, f.y, f.s) = (x, y, s);
                    }
                    d.focus_edited(cx);
                })))
                .child(icon_button(SharedString::from(format!("focus-del:{i}")), "x", t).on_click(cx.listener(move |d, _, _, cx| {
                    cx.stop_propagation();
                    d.remove_row(i, cx);
                })))
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |d, ev: &MouseDownEvent, window, cx| {
                        if ev.click_count >= 2 {
                            d.fv.sel = Some(i);
                            d.start_rename(Target::Focus(fid2.clone()), window, cx);
                            return;
                        }
                        d.fv.row_drag = Some(RowDrag { from: i, y0: f32::from(ev.position.y), dy: 0., moved: false });
                        cx.notify();
                    }),
                );
            list = list.child(row);
        }
        // Where a dragged row would land.
        if let Some((from, _, to)) = drag
            && to != from
        {
            let at = if to > from { to + 1 } else { to } as f32;
            list = list.child(div().absolute().left(px(4.)).right(px(4.)).top(px(at * (ROW_H + ROW_GAP) - 2.)).h(px(2.)).rounded_full().bg(t.accent));
        }

        let keep = div()
            .id("focus-keep")
            .h(px(32.))
            .flex()
            .flex_row()
            .items_center()
            .justify_center()
            .gap(px(6.))
            .rounded(px(8.))
            .border_1()
            .border_color(t.border)
            .bg(t.background)
            .hover(|d| d.bg(t.surface_hover))
            .cursor_pointer()
            .text_size(px(12.5))
            .text_color(t.text)
            .child(svg().path(icons::path("plus")).size(px(13.)).text_color(t.text))
            .child("把当前视野存为焦点")
            .on_click(cx.listener(|d, _, _, cx| {
                d.keep_view(cx);
            }));

        div()
            .id("focus-panel")
            .w(px(PANEL_W))
            .max_h(px((self.view.1 - PANEL_TOP - 60.).max(120.)))
            .overflow_y_scroll()
            .flex()
            .flex_col()
            .gap(px(14.))
            .p(px(14.))
            .bg(t.surface)
            .border_1()
            .border_color(t.border)
            .rounded(px(12.))
            .shadow(vec![gpui::BoxShadow {
                color: t.shade,
                offset: point(px(0.), px(6.)),
                blur_radius: px(20.),
                spread_radius: px(0.),
            }])
            .occlude()
            .font_family(".SystemUIFont")
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_scroll_wheel(|_, _, cx| cx.stop_propagation())
            .child(head)
            .child(chips)
            .child(list)
            .child(keep)
    }

    /// While playing: the stop's name and place, for a moment after each step.
    fn focus_hud(&self, t: &Theme) -> Option<AnyElement> {
        let p = self.fv.play.as_ref()?;
        let e = p.since.elapsed();
        let alpha = if e < HUD_SHOW { 1. } else { 1. - ((e - HUD_SHOW).as_secs_f32() / HUD_FADE.as_secs_f32()).min(1.) };
        if alpha <= 0. {
            return None;
        }
        let stops = self.m.stops(p.line.as_deref());
        let f = stops.get(p.at)?;
        let n = stops.len();
        let edge = if p.at + 1 == n && n > 1 { " · 最后一个" } else { "" };
        Some(
            div()
                .absolute()
                .left_0()
                .right_0()
                .bottom(px(58.))
                .flex()
                .justify_center()
                .opacity(alpha)
                .child(
                    div()
                        .h(px(34.))
                        .px(px(16.))
                        .flex()
                        .flex_row()
                        .items_center()
                        .gap(px(10.))
                        .rounded_full()
                        .bg(t.surface)
                        .border_1()
                        .border_color(t.border)
                        .shadow(vec![shadow(t)])
                        .text_size(px(12.5))
                        .child(div().size(px(8.)).rounded_full().bg(t.accent))
                        .child(div().font_weight(gpui::FontWeight::SEMIBOLD).text_color(t.text).child(f.name.clone()))
                        .child(div().text_color(t.muted).child(format!("{} / {n}{edge}", p.at + 1)))
                        .child(div().text_size(px(11.)).text_color(t.muted).child("空格 下一个 · ↩ 上一个 · Esc 结束")),
                )
                .into_any_element(),
        )
    }

    /// A word at the bottom that the view was kept.
    fn toast(&self, t: &Theme) -> Option<AnyElement> {
        let (text, at) = self.fv.toast.as_ref()?;
        let e = at.elapsed();
        let alpha = if e < HUD_SHOW { 1. } else { 1. - ((e - HUD_SHOW).as_secs_f32() / HUD_FADE.as_secs_f32()).min(1.) };
        Some(
            div()
                .absolute()
                .left_0()
                .right_0()
                .bottom(px(58.))
                .flex()
                .justify_center()
                .opacity(alpha)
                .child(
                    div()
                        .h(px(32.))
                        .px(px(14.))
                        .flex()
                        .flex_row()
                        .items_center()
                        .gap(px(8.))
                        .rounded_full()
                        .bg(t.surface)
                        .border_1()
                        .border_color(t.border)
                        .shadow(vec![shadow(t)])
                        .text_size(px(12.5))
                        .text_color(t.text)
                        .child(svg().path(icons::path("focus")).size(px(14.)).text_color(t.accent))
                        .child(text.clone()),
                )
                .into_any_element(),
        )
    }

    /// The 放映 button beside 焦点 in the pill: plays the line the panel shows, or asks which.
    pub(super) fn play_button(&self, t: &Theme, cx: &mut Context<Self>) -> gpui::Stateful<gpui::Div> {
        let color = if self.m.focus.is_empty() { t.muted } else { t.text };
        div()
            .id("focus-play")
            .flex()
            .flex_row()
            .items_center()
            .gap(px(5.))
            .px(px(8.))
            .py(px(5.))
            .rounded(px(8.))
            .cursor_pointer()
            .hover(|d| d.bg(t.surface_hover))
            .child(svg().path(icons::path("play")).size(px(13.)).text_color(color))
            .child(div().text_size(px(12.)).text_color(color).child("放映"))
            .on_click(cx.listener(|d, _, window, cx| {
                // The line the panel shows plays; the list itself is no line to play.
                match d.fv.line.clone().filter(|_| d.fv.on) {
                    Some(line) => d.start_play(Some(line), 0, window, cx),
                    None => d.play_focus(&PlayFocus, window, cx),
                }
            }))
    }

    /// Which line to play, asked in the middle of the window.
    fn chooser(&self, t: &Theme, cx: &mut Context<Self>) -> Option<AnyElement> {
        let pick = self.fv.chooser?;
        let rows: Vec<(Option<String>, String, usize)> =
            self.m.focus_lines.iter().map(|l| (Some(l.id.clone()), l.name.clone(), l.stops.len())).collect();
        let items = rows.into_iter().enumerate().map(|(i, (line, name, n))| {
            let on = i == pick;
            div()
                .id(("play-line", i))
                .h(px(34.))
                .px(px(12.))
                .flex()
                .flex_row()
                .items_center()
                .gap(px(10.))
                .rounded(px(8.))
                .cursor_pointer()
                .when(on, |d| d.bg(t.accent.opacity(0.16)))
                .when(!on, |d| d.hover(|d| d.bg(t.surface_hover)))
                .child(div().flex_1().text_size(px(13.)).text_color(if on { t.accent } else { t.text }).child(name))
                .child(div().text_size(px(11.5)).text_color(t.muted).child(format!("{n} 个焦点")))
                .on_click(cx.listener(move |d, _, window, cx| d.start_play(line.clone(), 0, window, cx)))
        });
        Some(
            div()
                .id("play-chooser-cover")
                .absolute()
                .left_0()
                .top_0()
                .size_full()
                .flex()
                .items_center()
                .justify_center()
                .bg(t.background.opacity(0.35))
                .occlude()
                .on_mouse_down(MouseButton::Left, cx.listener(|d, _, _, cx| {
                    d.fv.chooser = None;
                    cx.notify();
                }))
                .child(
                    div()
                        .id("play-chooser")
                        .w(px(320.))
                        .flex()
                        .flex_col()
                        .gap(px(4.))
                        .p(px(12.))
                        .bg(t.surface)
                        .border_1()
                        .border_color(t.border)
                        .rounded(px(12.))
                        .shadow(vec![shadow(t)])
                        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                        .child(div().px(px(4.)).pb(px(6.)).text_size(px(13.)).font_weight(gpui::FontWeight::SEMIBOLD).text_color(t.text).child("放映哪条焦点线"))
                        .children(items)
                        .child(div().px(px(4.)).pt(px(6.)).text_size(px(11.)).text_color(t.muted).child("↑ ↓ 选择 · ↩ 开始 · Esc 取消")),
                )
                .into_any_element(),
        )
    }
}

fn icon_button(id: impl Into<SharedString>, icon: &'static str, t: &Theme) -> gpui::Stateful<gpui::Div> {
    div()
        .id(id.into())
        .flex_none()
        .size(px(22.))
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(6.))
        .cursor_pointer()
        .hover(|d| d.bg(t.surface_hover))
        .child(svg().path(icons::path(icon)).size(px(12.)).text_color(t.muted))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_row_dragged_lands_a_row_per_rows_height() {
        let stride = ROW_H + ROW_GAP;
        assert_eq!(row_target(1, 0., 4), 1);
        assert_eq!(row_target(1, stride * 0.6, 4), 2);
        assert_eq!(row_target(1, -stride * 3., 4), 0, "no further than the first");
        assert_eq!(row_target(1, stride * 9., 4), 3, "no further than the last");
    }
}
