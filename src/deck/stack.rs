//! The 块堆 block: a wallet of blocks. Its cards stand one in front of the
//! other, headers peeking out. A click opens it: hovering its middle pushes
//! the card under the pointer out, hovering its top or bottom edge — or the
//! wheel — scrolls the cards. Space opens a long preview of the card out,
//! ←/→ turn it, space/Esc or a click beside it close it. Dragging a card's
//! header out of the pile plucks it: let go on the canvas it is taken out
//! there, on another stack it moves in, on its own (or Esc) it goes back.

use std::time::Instant;

use gpui::{
    AnyElement, Context, FontWeight, Hsla, InteractiveElement, IntoElement, KeyDownEvent, MouseButton,
    ParentElement, ScrollWheelEvent, SharedString, StatefulInteractiveElement, Styled, Window, div,
    prelude::FluentBuilder, px, svg,
};

use super::{DeckView, Drag, TITLE_H, kind_icon, message, shadow};
use crate::markdown;
use crate::model::{Block, BlockKind};
use crate::theme::{MdColors, Theme, rgba_u8};
use crate::{icons, preview};

/// How fast hovering an edge scrolls the cards, in cards a second.
const EDGE_SPEED: f32 = 5.0;
/// How far a press moves before it plucks the card out.
const PLUCK_SLOP: f32 = 6.0;

/// A value that moves toward its target as a damped spring does: over
/// about `dur` seconds, overshooting by about `bounce`.
#[derive(Clone, Copy, Debug, Default)]
pub(super) struct Spring {
    pub x: f32,
    pub v: f32,
}

impl Spring {
    pub fn at(x: f32) -> Spring {
        Spring { x, v: 0. }
    }

    /// Moves it dt seconds toward target; says whether it still moves.
    pub fn step(&mut self, target: f32, dt: f32, dur: f32, bounce: f32) -> bool {
        let omega = 2. * std::f32::consts::PI / dur;
        let zeta = 1. - bounce;
        let mut left = dt.min(0.1);
        while left > 0. {
            let h = left.min(1. / 240.);
            let a = -omega * omega * (self.x - target) - 2. * zeta * omega * self.v;
            self.v += a * h;
            self.x += self.v * h;
            left -= h;
        }
        let moving = (self.x - target).abs() > 1e-3 * (1. + target.abs()) || self.v.abs() > 1e-2;
        if !moving {
            *self = Spring::at(target);
        }
        moving
    }
}

/// A stack's state between frames.
#[derive(Default)]
pub(super) struct StackRt {
    /// The card pushed out, or last out: an index into its items.
    pub pick: usize,
    /// It is out now.
    pub out: bool,
    /// How far the cards are scrolled, in cards.
    pub scroll: f32,
    /// Where the pointer is in the body, while it is over it.
    pub hover: Option<f32>,
    /// Where each card shows, easing toward where it goes.
    pub ys: std::collections::HashMap<String, (Spring, Spring)>,
    /// How far its cards stand apart, out of the long preview's way.
    pub spread: Spring,
    pub frame: Option<Instant>,
}

/// A card pressed in a pile, or plucked out and following the pointer.
pub(super) struct Pluck {
    pub stack: String,
    pub card: String,
    pub from: (f32, f32),
    pub at: (f32, f32),
    pub active: bool,
}

/// Where a stack's cards lie in its body, in window pixels.
#[derive(Clone, Copy, Debug)]
pub(super) struct Geom {
    pub h: f32,
    pub pad: f32,
    pub peek: f32,
    pub card_h: f32,
    pub reveal: f32,
    pub lift: f32,
    pub band: f32,
    pub count: usize,
    pub max: f32,
}

impl Geom {
    pub fn new(h: f32, s: f32, count: usize) -> Geom {
        let pad = 12. * s;
        let peek = 32. * s;
        let card_h = (h - 2. * pad - 2. * peek).max(2. * peek);
        Geom {
            h,
            pad,
            peek,
            card_h,
            reveal: card_h - peek + 6. * s,
            lift: 8. * s,
            band: (30. * s).min(h / 5.),
            count,
            max: (count as f32 - 1. - (h - 2. * pad - card_h) / peek).max(0.),
        }
    }

    /// The top of card i with the cards scrolled to sc: those scrolled past
    /// tuck up behind the first.
    pub fn slot(&self, i: usize, sc: f32) -> f32 {
        let at = i as f32 - sc;
        if at < 0. { self.pad + at * self.peek * 0.3 } else { self.pad + at * self.peek }
    }

    /// The top of card i with card `out` pushed out.
    pub fn top(&self, i: usize, sc: f32, out: Option<usize>) -> f32 {
        let mut y = self.slot(i, sc);
        match out {
            Some(o) if i > o => y += self.reveal,
            Some(o) if i == o => y -= self.lift,
            _ => {}
        }
        y
    }

    /// The card under y, the front one where they overlap.
    pub fn card_at(&self, y: f32, sc: f32, out: Option<usize>) -> usize {
        (0..self.count).rev().find(|&i| y >= self.top(i, sc, out)).unwrap_or(0)
    }
}

/// The color a card of kind k is marked with.
fn tint(k: BlockKind) -> Hsla {
    let (r, g, b) = match k {
        BlockKind::Note => (0xE3, 0xA9, 0x2B),
        BlockKind::Memo => (0x2F, 0x9E, 0x6E),
        BlockKind::File => (0x3B, 0x7F, 0xE0),
        BlockKind::Browser => (0x8B, 0x5C, 0xF6),
        BlockKind::Chart => (0xD9, 0x48, 0x0F),
        _ => (0x6B, 0x72, 0x80),
    };
    rgba_u8(r, g, b, 1.)
}

/// A stack's body width at scale s.
fn b_w(st: &Block, s: f32) -> f32 {
    st.w * s
}

fn cut(s: &str, max: usize) -> String {
    let mut out: String = s.chars().take(max).collect();
    if s.chars().count() > max {
        out.push('…');
    }
    out
}

impl DeckView {
    fn members(&self, st: &Block) -> Vec<Block> {
        st.items.iter().filter_map(|id| self.m.block(id).cloned()).collect()
    }

    fn stack_rt(&mut self, id: &str) -> &mut StackRt {
        &mut self.rts.entry(id.to_string()).or_default().stack
    }

    /// The geometry of a stack's body at the camera's scale.
    fn geom(&self, st: &Block, count: usize) -> Geom {
        let s = self.m.cam.s;
        Geom::new(st.h * s - TITLE_H * s, s, count)
    }

    /// The body's top-left in the window.
    fn body_origin(&self, st: &Block) -> (f32, f32) {
        let (sx, sy) = self.m.cam.to_screen(st.x, st.y);
        (sx, sy + TITLE_H * self.m.cam.s)
    }

    /// The stack selected, if the selection is exactly one stack.
    fn selected_stack(&self) -> Option<String> {
        if self.m.sel.len() != 1 {
            return None;
        }
        let id = self.m.sel.iter().next()?;
        self.m.block(id).filter(|b| b.kind == BlockKind::Stack).map(|b| b.id.clone())
    }

    /// The stack under a window point, other than `except`: where a block let go there goes.
    pub(super) fn stack_at(&self, x: f32, y: f32, except: &str) -> Option<String> {
        let (wx, wy) = self.m.cam.to_world(x, y);
        let hidden = self.m.stacked_ids();
        self.m
            .blocks
            .iter()
            .rev()
            .filter(|b| b.kind == BlockKind::Stack && b.id != except && !hidden.contains(&b.id))
            .find(|b| b.contains(wx, wy))
            .map(|b| b.id.clone())
    }

    /// A press on a stack's body: it opens, the card under the pointer comes
    /// out, and a drag of that card's header may pluck it.
    pub(super) fn stack_press(&mut self, id: &str, x: f32, y: f32) {
        let Some(st) = self.m.block(id).cloned() else { return };
        self.m.sel = std::collections::HashSet::from([id.to_string()]);
        self.stack_open = Some(id.to_string());
        let members = self.members(&st);
        if members.is_empty() {
            return;
        }
        let g = self.geom(&st, members.len());
        let (_, oy) = self.body_origin(&st);
        let by = y - oy;
        let rt = self.stack_rt(id);
        if by < g.band && rt.scroll > 0. || by > g.h - g.band {
            return; // an edge: hovering it scrolls
        }
        let out = rt.out.then_some(rt.pick);
        let i = g.card_at(by, rt.scroll, out);
        rt.pick = i;
        rt.out = true;
        // A press on the card's header may pluck it.
        let top = g.top(i, rt.scroll, Some(i));
        if by - top <= g.peek {
            self.pluck = Some(Pluck {
                stack: id.to_string(),
                card: members[i].id.clone(),
                from: (x, y),
                at: (x, y),
                active: false,
            });
            self.drag = Some(Drag::Pluck);
        }
    }

    pub(super) fn pluck_move(&mut self, x: f32, y: f32) {
        if let Some(p) = &mut self.pluck {
            p.at = (x, y);
            if !p.active && ((x - p.from.0).abs() > PLUCK_SLOP || (y - p.from.1).abs() > PLUCK_SLOP) {
                p.active = true;
            }
        }
    }

    /// Lets go of a card plucked out: on the canvas it is taken out there,
    /// on another stack it moves in, on its own it goes back.
    pub(super) fn pluck_release(&mut self, x: f32, y: f32) -> bool {
        let Some(p) = self.pluck.take() else { return false };
        if !p.active {
            return false;
        }
        let (wx, wy) = self.m.cam.to_world(x, y);
        let own = self.m.block(&p.stack).is_some_and(|st| st.contains(wx, wy));
        if own {
            return false;
        }
        if let Some(other) = self.stack_at(x, y, &p.stack) {
            return self.m.stack_add(&other, &p.card);
        }
        let Some(card) = self.m.block(&p.card) else { return false };
        let w = card.w;
        let done = self.m.stack_take(&p.stack, &p.card, wx - w / 2., wy - 12.);
        if done {
            self.stack_open = None;
        }
        done
    }

    fn step_preview(&mut self, delta: i32) {
        let Some(id) = self.preview.clone() else { return };
        let n = self.m.block(&id).map(|b| self.members(b).len()).unwrap_or(0);
        if n == 0 {
            return;
        }
        let rt = self.stack_rt(&id);
        rt.pick = ((rt.pick as i32 + delta).rem_euclid(n as i32)) as usize;
        rt.out = true;
    }

    /// Space, Esc and the arrows, while the deck itself has the keyboard:
    /// what is typed into an editor or a terminal is theirs.
    pub(super) fn stack_keys(&mut self, ev: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        if !self.focus.is_focused(window) {
            return;
        }
        let ks = &ev.keystroke;
        let m = ks.modifiers;
        if m.platform || m.control || m.alt {
            return;
        }
        match ks.key.as_str() {
            "escape" => {
                if self.pluck.as_ref().is_some_and(|p| p.active) {
                    self.pluck = None;
                    self.drag = None;
                } else if self.preview.is_some() {
                    self.close_preview();
                } else if self.stack_open.take().is_some() {
                } else {
                    return;
                }
            }
            "space" => {
                if self.preview.is_some() {
                    self.close_preview();
                } else {
                    let Some(id) = self.selected_stack() else { return };
                    if self.m.block(&id).is_none_or(|b| self.members(b).is_empty()) {
                        return;
                    }
                    self.stack_open = Some(id.clone());
                    self.stack_rt(&id).out = true;
                    if self.preview_leaving.as_deref() != Some(id.as_str()) {
                        self.preview_t = Spring::at(0.); // it grows out of its card
                    }
                    self.preview_leaving = None;
                    self.preview = Some(id);
                }
            }
            "right" if self.preview.is_some() => self.step_preview(1),
            "left" if self.preview.is_some() => self.step_preview(-1),
            _ => return,
        }
        cx.stop_propagation();
        cx.notify();
    }

    /// Closes the long preview: it shrinks back into its card.
    pub(super) fn close_preview(&mut self) {
        if let Some(id) = self.preview.take() {
            self.preview_leaving = Some(id);
        }
    }

    /// Closes the open stack once it is no longer the one thing selected.
    pub(super) fn watch_stack(&mut self) {
        let Some(id) = self.stack_open.clone() else { return };
        if self.preview.is_some() || self.preview_leaving.is_some() || self.pluck.is_some() {
            return;
        }
        let gone = self.m.block(&id).is_none() || self.m.stacked_ids().contains(&id);
        if gone || self.m.sel.len() != 1 || !self.m.sel.contains(&id) || self.editing.is_some() {
            if let Some(rt) = self.rts.get_mut(&id) {
                rt.stack.out = false;
            }
            self.stack_open = None;
        }
        if self.preview.as_ref().is_some_and(|p| self.m.block(p).is_none()) {
            self.preview = None;
        }
        if self.preview_leaving.as_ref().is_some_and(|p| self.m.block(p).is_none()) {
            self.preview_leaving = None;
        }
    }

    /// Whether a block dragged, or a card plucked, would go into st if let go now.
    fn drop_here(&self, st: &str) -> bool {
        if let Some(p) = self.pluck.as_ref().filter(|p| p.active) {
            return self.stack_at(p.at.0, p.at.1, &p.stack).as_deref() == Some(st);
        }
        match &self.drag {
            Some(Drag::Move { id, moved: true, .. }) if id != st => {
                let Some(b) = self.m.block(id) else { return false };
                if b.kind == BlockKind::Stack {
                    return false;
                }
                let p = self.last_pointer;
                self.stack_at(p.0, p.1, id).as_deref() == Some(st)
            }
            _ => false,
        }
    }

    /// The line a card's face shows when it holds no Markdown.
    fn snippet(&self, n: &Block) -> String {
        match n.kind {
            BlockKind::Note => cut(n.text.trim(), 360),
            BlockKind::Memo | BlockKind::File => n.path.clone(),
            BlockKind::Terminal => n.term.as_ref().map(|t| t.dir.clone()).filter(|d| !d.is_empty()).unwrap_or("终端".into()),
            BlockKind::Browser => if n.url.is_empty() { "空白页面".into() } else { n.url.clone() },
            BlockKind::Chart => crate::chart::parse(&n.text).summary(crate::chart::of(&n.chart)),
            BlockKind::Stack => String::new(),
        }
    }

    /// The Markdown a card's face shows rendered, loading what it needs; "" for none.
    fn face_markdown(&mut self, n: &Block, cx: &mut Context<Self>) -> String {
        let src = match n.kind {
            BlockKind::Note => n.text.clone(),
            BlockKind::Memo => {
                self.load_memo(&n.id, cx);
                self.rts.get(&n.id).filter(|r| r.memo.loaded).map(|r| r.memo.text.clone()).unwrap_or_default()
            }
            BlockKind::File => match preview::of(&n.path) {
                preview::Kind::Markdown => {
                    self.load_doc(&n.id, cx);
                    match self.rts.get(&n.id).and_then(|r| r.doc.clone()) {
                        Some(Ok(t)) => (*t).clone(),
                        _ => String::new(),
                    }
                }
                preview::Kind::Word => {
                    self.load_word(&n.id, cx);
                    match self.rts.get(&n.id).and_then(|r| r.word.clone()) {
                        Some(Ok(d)) => d.markdown.clone(),
                        _ => String::new(),
                    }
                }
                _ => String::new(),
            },
            _ => String::new(),
        };
        if src.trim().is_empty() { String::new() } else { cut(&src, 2000) }
    }

    pub(super) fn stack_body(&mut self, st: &Block, t: &Theme, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let s = self.m.cam.s;
        let plucked = self.pluck.as_ref().filter(|p| p.active && p.stack == st.id).map(|p| p.card.clone());
        let members: Vec<Block> =
            self.members(st).into_iter().filter(|m| Some(&m.id) != plucked.as_ref()).collect();
        let drop = self.drop_here(&st.id);
        if members.is_empty() {
            return div()
                .size_full()
                .p(px(14. * s))
                .child(
                    div()
                        .size_full()
                        .flex()
                        .flex_col()
                        .items_center()
                        .justify_center()
                        .gap(px(6. * s))
                        .rounded(px(10. * s))
                        .border(px(if drop { 2. } else { 1. }))
                        .border_color(if drop { t.accent } else { t.border })
                        .child(svg().path(icons::path("stack")).size(px(30. * s)).text_color(t.muted))
                        .child(div().text_size(px(13. * s)).text_color(t.muted).child("拖 block 的胶带或标题栏过来"))
                        .child(
                            div().text_size(px(11. * s)).text_color(t.muted).child("松手即收纳 · 点击后悬停翻看 · 空格预览"),
                        ),
                )
                .into_any_element();
        }
        let g = self.geom(st, members.len());
        let open = self.stack_open.as_deref() == Some(&st.id);
        let open_preview = self.preview.as_deref() == Some(&st.id);
        let leaving = self.preview_leaving.as_deref() == Some(&st.id);
        let shown = open_preview || leaving;
        // While the preview shrinks back, its card shows again once it is home.
        let card_home = leaving && self.preview_t.x < 0.08;
        let (body_x, body_y) = self.body_origin(st);
        let dragging = self.drag.is_some();
        let now = Instant::now();
        let id = st.id.clone();
        let rt = self.stack_rt(&id);
        let dt = rt.frame.map_or(0., |f| now.duration_since(f).as_secs_f32().min(0.05));
        rt.frame = Some(now);
        rt.pick = rt.pick.min(members.len() - 1);
        rt.scroll = rt.scroll.clamp(0., g.max);
        let mut animating = false;
        // Hovering an open stack: its edges scroll, its middle pushes cards out.
        if open && !shown && !dragging {
            if let Some(y) = rt.hover {
                let target = if y < g.band && rt.scroll > 0. {
                    Some(0.)
                } else if y > g.h - g.band && rt.scroll < g.max {
                    Some(g.max)
                } else {
                    None
                };
                match target {
                    Some(to) => {
                        rt.out = false;
                        let step = EDGE_SPEED * dt.max(1. / 120.);
                        rt.scroll = if to > rt.scroll { (rt.scroll + step).min(to) } else { (rt.scroll - step).max(to) };
                        animating = true;
                    }
                    None => {
                        let out = rt.out.then_some(rt.pick);
                        rt.pick = g.card_at(y, rt.scroll, out);
                        rt.out = true;
                    }
                }
            }
        }
        if shown {
            // The long preview turns the cards: the one it shows is out and in view.
            rt.out = true;
            let fit = (g.h - 2. * g.pad - g.card_h) / g.peek;
            rt.scroll = rt.scroll.max(rt.pick as f32 - fit).min(rt.pick as f32).clamp(0., g.max);
        }
        let out = (rt.out && (open || shown)).then_some(rt.pick);
        let (sc, pick) = (rt.scroll, rt.pick);

        // Each card eases toward where it goes.
        // The cards spring to where they go; with the long preview open the
        // others stand apart, out of its way.
        let spread_to = if open_preview { 1. } else { 0. };
        animating |= rt.spread.step(spread_to, dt, 0.46, 0.14);
        let spread = rt.spread.x;
        let mut cards: Vec<(Block, usize, f32, f32)> = Vec::new();
        let mut preview_from = None;
        for (i, m) in members.iter().enumerate() {
            let target_y = g.top(i, sc, out);
            let target_l = if out == Some(i) { 1. } else { 0. };
            let e = rt.ys.entry(m.id.clone()).or_insert((Spring::at(target_y), Spring::at(target_l)));
            if dt > 0. {
                animating |= e.0.step(target_y, dt, 0.38, 0.18);
                animating |= e.1.step(target_l, dt, 0.38, 0.18);
            }
            let (mut y, l) = (e.0.x, e.1.x);
            if let Some(o) = out {
                if i < o {
                    y -= spread * (g.peek * 1.5 + (o - i - 1) as f32 * g.peek * 0.6);
                } else if i > o {
                    y += spread * (g.peek * 2. + (i - o - 1) as f32 * g.peek * 0.6);
                }
            }
            if shown && out == Some(i) {
                // It is the preview now: where it lies is where the preview
                // grows from and goes back into.
                preview_from = Some((body_x + g.pad - l * 4. * s, body_y + y, (b_w(st, s) - 2. * g.pad) + l * 8. * s, g.card_h));
                if !card_home {
                    continue;
                }
            }
            if i as f32 - sc < -3. || y > g.h || y + g.card_h < 0. {
                continue;
            }
            cards.push((m.clone(), i, y, l));
        }
        if preview_from.is_some() {
            self.preview_from = preview_from;
        }
        if animating {
            window.request_animation_frame();
        }

        let count = members.len();
        let pill = if drop {
            "松手收进这一堆".to_string()
        } else if let Some(o) = out {
            format!("{} / {count} · 空格长预览 · 拖动卡片标题取出", o + 1)
        } else if open {
            format!("{count} 张 · 悬停推出 · 上下边缘滚动")
        } else {
            format!("{count} 张 · 点击翻看")
        };
        let pill_hot = drop || out.is_some();

        let mut children: Vec<AnyElement> = Vec::new();
        let n_cards = cards.len();
        for ci in 0..n_cards {
            let (m, _, y, l) = cards[ci].clone();
            let next_y = cards.get(ci + 1).map(|c| c.2);
            let bottom = next_y.unwrap_or(f32::MAX).min(y + g.card_h).min(g.h);
            let face_shows = bottom - (y + g.peek) > g.peek / 3.;
            let face: AnyElement = if !face_shows {
                div().into_any_element()
            } else if m.kind == BlockKind::Chart {
                crate::chartview::render_compact(&m.text, &m.chart, s, t)
            } else {
                let md = self.face_markdown(&m, cx);
                if md.is_empty() {
                    div().text_size(px(11. * s)).text_color(t.muted).child(self.snippet(&m)).into_any_element()
                } else {
                    markdown::render(&md, 11. * s, &MdColors::of_theme(t))
                }
            };
            let tn = tint(m.kind);
            let title = if m.title.trim().is_empty() { m.default_title() } else { m.title.clone() };
            let face_h = (bottom - y - g.peek - 4. * s).max(0.);
            children.push(
                div()
                    .absolute()
                    .left(px(g.pad - l * 4. * s))
                    .right(px(g.pad - l * 4. * s))
                    .top(px(y))
                    .h(px(g.card_h))
                    .rounded(px(9. * s))
                    .overflow_hidden()
                    .bg(t.surface)
                    .border_1()
                    .border_color(if l > 0.5 { tn.opacity(0.6) } else { t.border })
                    .shadow(vec![gpui::BoxShadow {
                        color: if t.dark { rgba_u8(0, 0, 0, 0.42) } else { rgba_u8(0, 0, 0, 0.16) },
                        offset: gpui::point(px(0.), px((2. + 6. * l) * s)),
                        blur_radius: px((8. + 16. * l) * s),
                        spread_radius: px(0.),
                    }])
                    .child(div().absolute().size_full().bg(tn.opacity(0.07 + 0.05 * l)))
                    .child(div().absolute().top_0().left_0().right_0().h(px(3. * s)).bg(tn.opacity(0.85)))
                    .child(
                        div()
                            .h(px(g.peek))
                            .flex()
                            .flex_row()
                            .items_center()
                            .gap(px(6. * s))
                            .px(px(12. * s))
                            .child(div().size(px(6. * s)).rounded(px(3. * s)).bg(tn))
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .overflow_hidden()
                                    .whitespace_nowrap()
                                    .text_size(px(12.5 * s))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(t.text)
                                    .child(title),
                            )
                            .child(div().text_size(px(10. * s)).text_color(t.muted).child(m.kind.label())),
                    )
                    .child(div().mx(px(12. * s)).h(px(1.)).bg(t.border.opacity(0.6)))
                    .child(div().h(px(face_h)).px(px(14. * s)).pt(px(7. * s)).overflow_hidden().child(face))
                    .into_any_element(),
            );
        }

        // How far the cards are scrolled, when they scroll.
        if g.max > 0. {
            let track = g.h - 2. * g.pad;
            let thumb = (track * (1. - g.max / count as f32)).max(16. * s);
            let ty = g.pad + (track - thumb) * sc / g.max;
            children.push(
                div()
                    .absolute()
                    .right(px(2.5 * s))
                    .top(px(ty))
                    .w(px(2.5 * s))
                    .h(px(thumb))
                    .rounded(px(1.25 * s))
                    .bg(t.muted.opacity(0.45))
                    .into_any_element(),
            );
        }
        // What the pile does now, in a pill at its foot.
        children.push(
            div()
                .absolute()
                .bottom(px(10. * s))
                .left_0()
                .right_0()
                .flex()
                .justify_center()
                .child(
                    div()
                        .px(px(9. * s))
                        .py(px(4. * s))
                        .rounded_full()
                        .bg(t.surface.opacity(0.94))
                        .border_1()
                        .border_color(t.border)
                        .shadow(vec![shadow(t)])
                        .text_size(px(10. * s))
                        .text_color(if pill_hot { t.accent } else { t.muted })
                        .child(pill),
                )
                .into_any_element(),
        );

        let (id_m, id_w, id_l) = (st.id.clone(), st.id.clone(), st.id.clone());
        let _ = pick;
        div()
            .id(SharedString::from(format!("stack:{}", st.id)))
            .relative()
            .size_full()
            .overflow_hidden()
            .when(drop, |d| d.border(px(2.)).border_color(t.accent).rounded(px(10. * s)))
            .on_mouse_move(cx.listener(move |d, ev: &gpui::MouseMoveEvent, _, cx| {
                let Some(st) = d.m.block(&id_m).cloned() else { return };
                let (_, oy) = d.body_origin(&st);
                d.stack_rt(&id_m).hover = Some(f32::from(ev.position.y) - oy);
                if d.stack_open.as_deref() == Some(id_m.as_str()) {
                    cx.notify();
                }
            }))
            .on_hover(cx.listener(move |d, hovered: &bool, _, cx| {
                if !*hovered {
                    d.stack_rt(&id_l).hover = None;
                    cx.notify();
                }
            }))
            .on_scroll_wheel(cx.listener(move |d, ev: &ScrollWheelEvent, _, cx| {
                if ev.modifiers.platform || ev.modifiers.control || d.stack_open.as_deref() != Some(id_w.as_str()) {
                    return; // the canvas's
                }
                let Some(st) = d.m.block(&id_w).cloned() else { return };
                let g = d.geom(&st, d.members(&st).len());
                if g.max <= 0. {
                    return;
                }
                let dy = f32::from(ev.delta.pixel_delta(px(20.)).y);
                let rt = d.stack_rt(&id_w);
                rt.scroll = (rt.scroll - dy / g.peek).clamp(0., g.max);
                rt.out = false;
                cx.stop_propagation();
                cx.notify();
            }))
            .children(children)
            .into_any_element()
    }

    /// The card plucked out, following the pointer, the size of its block.
    pub(super) fn pluck_view(&self, t: &Theme) -> Option<AnyElement> {
        let p = self.pluck.as_ref().filter(|p| p.active)?;
        let b = self.m.block(&p.card)?;
        let s = self.m.cam.s;
        let (w, h) = (b.w * s, b.h * s);
        Some(
            div()
                .absolute()
                .left(px(p.at.0 - w / 2.))
                .top(px(p.at.1 - 12. * s))
                .w(px(w))
                .h(px(h))
                .opacity(0.85)
                .rounded(px(9. * s))
                .bg(t.surface)
                .border(px(2.))
                .border_color(t.accent)
                .shadow(vec![shadow(t)])
                .overflow_hidden()
                .child(div().absolute().top_0().left_0().right_0().h(px(3. * s)).bg(tint(b.kind)))
                .child(
                    div()
                        .h(px(32. * s))
                        .flex()
                        .flex_row()
                        .items_center()
                        .gap(px(6. * s))
                        .px(px(12. * s))
                        .text_size(px(12.5 * s))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(t.text)
                        .child(svg().path(icons::path(kind_icon(b.kind))).size(px(13. * s)).text_color(t.muted))
                        .child(b.title.clone()),
                )
                .child(div().px(px(14. * s)).text_size(px(11. * s)).text_color(t.muted).child(self.snippet(b)))
                .into_any_element(),
        )
    }

    /// The long preview of a stack's card, over the window.
    pub(super) fn stack_preview(&mut self, t: &Theme, window: &mut Window, cx: &mut Context<Self>) -> Option<AnyElement> {
        let id = self.preview.clone().or_else(|| self.preview_leaving.clone())?;
        let open = self.preview.is_some();
        // It grows out of its card, and shrinks back into it.
        let now = Instant::now();
        let dt = self.preview_frame.map_or(1. / 60., |f| now.duration_since(f).as_secs_f32());
        self.preview_frame = Some(now);
        let moving = self.preview_t.step(if open { 1. } else { 0. }, dt, 0.42, 0.1);
        if !open && !moving {
            self.preview_leaving = None;
            self.preview_frame = None;
            return None;
        }
        if moving {
            window.request_animation_frame();
        }
        let p = self.preview_t.x;
        let st = self.m.block(&id).cloned()?;
        let members = self.members(&st);
        if members.is_empty() {
            self.preview = None;
            self.preview_leaving = None;
            return None;
        }
        let pick = self.stack_rt(&id).pick.min(members.len() - 1);
        let card = members[pick].clone();
        let (vw, vh) = self.view;
        let w = (vw - 160.).clamp(320., 860.);
        let h = (vh - 140.).max(240.);
        let to = ((vw - w) / 2., (vh - h) / 2., w, h);
        let from = self.preview_from.unwrap_or((to.0 + w * 0.3, to.1 + h * 0.3, w * 0.4, h * 0.4));
        let lerp = |a: f32, b: f32| a + (b - a) * p;
        let (x, y, w, h) = (lerp(from.0, to.0), lerp(from.1, to.1), lerp(from.2, to.2), lerp(from.3, to.3));

        // Its content, at 100%: the card's own body, but for what would run.
        let content = match card.kind {
            BlockKind::Terminal | BlockKind::Browser | BlockKind::Stack => {
                message(self.snippet(&card), t.muted, 1.)
            }
            _ => {
                let saved = self.m.cam;
                self.m.cam.s = 1.;
                let paper = crate::theme::Paper::of(t);
                let el = self.body(&card, t, &paper, window, cx);
                self.m.cam = saved;
                el
            }
        };
        let tn = tint(card.kind);
        let turn = |key: &'static str, glyph: &'static str, delta: i32, cx: &mut Context<Self>| {
            div()
                .id(key)
                .px(px(10.))
                .py(px(2.))
                .rounded(px(6.))
                .text_size(px(16.))
                .text_color(t.text)
                .hover(|d| d.bg(t.surface_hover))
                .cursor_pointer()
                .child(glyph)
                .on_click(cx.listener(move |d, _, _, cx| {
                    d.step_preview(delta);
                    cx.notify();
                }))
        };
        Some(
            div()
                .id("stack-preview")
                .absolute()
                .left_0()
                .top_0()
                .size_full()
                .bg(rgba_u8(0, 0, 0, if t.dark { 0.5 } else { 0.28 } * p.clamp(0., 1.)))
                .occlude()
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(|d, _, window, cx| {
                        d.close_preview();
                        window.focus(&d.focus);
                        cx.notify();
                    }),
                )
                .on_scroll_wheel(|_, _, cx| cx.stop_propagation())
                .child(
                    div()
                        .id("stack-preview-panel")
                        .absolute()
                        .left(px(x))
                        .top(px(y))
                        .w(px(w.max(40.)))
                        .h(px(h.max(40.)))
                        .flex()
                        .flex_col()
                        .rounded(px(14.))
                        .overflow_hidden()
                        .bg(t.surface)
                        .border_1()
                        .border_color(t.border)
                        .shadow(vec![gpui::BoxShadow {
                            color: rgba_u8(0, 0, 0, 0.35),
                            offset: gpui::point(px(0.), px(18.)),
                            blur_radius: px(48.),
                            spread_radius: px(0.),
                        }])
                        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                        .child(div().h(px(4.)).w_full().bg(tn))
                        .child(
                            div()
                                .h(px(46.))
                                .flex_none()
                                .flex()
                                .flex_row()
                                .items_center()
                                .gap(px(10.))
                                .px(px(16.))
                                .child(svg().path(icons::path(kind_icon(card.kind))).size(px(16.)).text_color(tn))
                                .child(
                                    div()
                                        .flex_1()
                                        .min_w_0()
                                        .overflow_hidden()
                                        .whitespace_nowrap()
                                        .text_size(px(15.))
                                        .font_weight(FontWeight::SEMIBOLD)
                                        .text_color(t.text)
                                        .child(card.title.clone()),
                                )
                                .child(turn("preview-prev", "‹", -1, cx))
                                .child(
                                    div()
                                        .text_size(px(12.))
                                        .text_color(t.muted)
                                        .child(format!("{} / {}", pick + 1, members.len())),
                                )
                                .child(turn("preview-next", "›", 1, cx)),
                        )
                        .child(div().h(px(1.)).w_full().bg(t.border))
                        .child(
                            div()
                                .flex_1()
                                .min_h_0()
                                .w_full()
                                .relative()
                                // Its content comes in once it has nearly grown.
                                .opacity(((p - 0.6) / 0.4).clamp(0., 1.))
                                .child(content),
                        )
                        .child(
                            div()
                                .h(px(30.))
                                .flex_none()
                                .flex()
                                .items_center()
                                .justify_center()
                                .text_size(px(11.))
                                .text_color(t.muted)
                                .child("←/→ 翻看 · 空格或 Esc 关闭"),
                        ),
                )
                .into_any_element(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cards_peek_and_push_out() {
        let g = Geom::new(400., 1., 6);
        assert_eq!(g.slot(0, 0.), 12.);
        assert_eq!(g.slot(1, 0.), 44.);
        // The card under a point is the front one there.
        assert_eq!(g.card_at(50., 0., None), 1);
        assert_eq!(g.card_at(5., 0., None), 0);
        // Card 1 out: the ones after it make room.
        assert_eq!(g.top(2, 0., Some(1)), g.slot(2, 0.) + g.reveal);
        assert_eq!(g.top(1, 0., Some(1)), g.slot(1, 0.) - g.lift);
        assert!(g.max > 0.);
        assert!(g.slot(0, 2.) < g.pad, "cards scrolled past tuck up behind");
    }

    #[test]
    fn springs_overshoot_a_little_and_settle() {
        let mut sp = Spring::at(0.);
        let mut peak = 0f32;
        let mut t = 0.;
        while sp.step(100., 1. / 60., 0.38, 0.18) && t < 3. {
            peak = peak.max(sp.x);
            t += 1. / 60.;
        }
        assert!(t < 1.5, "settles");
        assert_eq!(sp.x, 100.);
        assert!(peak > 100. && peak < 120., "bounces a little: {peak}");
    }

    #[test]
    fn a_few_cards_do_not_scroll() {
        assert_eq!(Geom::new(400., 1., 2).max, 0.);
    }
}
