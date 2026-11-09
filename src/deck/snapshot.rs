//! Blocks' bodies as pictures (GPUI's `snapshot_capture` / `paint_snapshot`,
//! our patch). Laying a block's text out and painting its glyphs is most of
//! what a frame of a page of text costs, and a glide changes the scale every
//! frame. A picture of how a body was last drawn shows instead, stretched.
//!
//! Playing (focus play), the content does not change: bodies are pictures
//! the whole time. Come to rest at a stop, each is drawn once at that scale
//! and pictured, then its picture shows, pixel for pixel; a glide stretches
//! those pictures, and only a body coming into sight is drawn and pictured.
//! A frame pictures at most `CAPTURES_PER_FRAME` bodies, nearest the middle
//! of the window first (`plan_captures`); the rest show their old picture
//! stretched, or, with none, the card's bare color, until their turn. A body
//! scrolled while playing is drawn live a while (`LIVE_AFTER_WHEEL`), so its
//! list takes the wheel, and pictured again after.
//!
//! Not playing, a glide (an agent's `set_view`) pictures its bodies in its
//! first frames and stretches them after; at rest bodies are drawn as they are.
//!
//! A picture is of a version of the content (`version_of`); one of another
//! version is not shown. One holds only the part of the body that was in
//! the window. Bodies that change on their own (terminals, web pages) or are
//! being worked in are not pictured.

use std::collections::HashSet;
use std::hash::{Hash, Hasher};
use std::time::{Duration, Instant};

use gpui::{AnyElement, Bounds, Hsla, IntoElement, ParentElement, Pixels, SnapshotInfo, Styled, Window, canvas, div, point, px, size};

use super::{DeckView, TITLE_H};
use crate::model::{Block, BlockKind};

/// How many bodies one frame lays out and pictures while playing.
const CAPTURES_PER_FRAME: usize = 2;
/// How long a body scrolled while playing stays drawn live.
pub(super) const LIVE_AFTER_WHEEL: Duration = Duration::from_millis(900);

/// How a body is drawn this frame.
pub(super) enum Plan {
    /// Laid out and drawn as it is.
    Body,
    /// Laid out and drawn, and kept as a picture.
    Capture { key: u64, version: u64 },
    /// Its picture, stretched to where it is now.
    Show { key: u64, version: u64 },
    /// Nothing yet: the card's own color, until it is pictured.
    Blank,
}

/// How a body stands with its picture this frame.
struct Picture {
    key: u64,
    version: u64,
    /// Its picture, of this version.
    info: Option<SnapshotInfo>,
    /// The picture holds all of the body that shows now.
    covers: bool,
    /// The picture is of this scale and this view of the body: it shows pixel for pixel.
    fresh: bool,
    /// How far the body's middle is from the window's, squared.
    rank: f32,
}

impl DeckView {
    /// Whether bodies are pictures now: playing, or gliding.
    fn picturing(&self) -> bool {
        self.fv.play.is_some() || self.glide.is_some()
    }

    /// Block `b`'s body and its picture this frame; none if it is not pictured.
    fn picture_of(&self, b: &Block, dark: bool, window: &Window, now: Instant) -> Option<Picture> {
        if !pictured(b.kind)
            || self.active.as_deref() == Some(b.id.as_str())
            || self.editing.as_ref().is_some_and(|e| e.id == b.id)
            || self.drag.is_some()
            || self.photo_showing()
            || self.canvas_scroll_until.is_some_and(|t| t > now)
            || self.rts.get(&b.id).and_then(|r| r.live_until).is_some_and(|t| t > now)
        {
            return None;
        }
        let cam = self.m.cam;
        let (sx, sy) = cam.to_screen(b.x, b.y);
        let body = Bounds {
            origin: point(px(sx), px(sy + TITLE_H * cam.s)),
            size: size(px(b.w * cam.s), px(((b.h - TITLE_H) * cam.s).max(0.))),
        };
        let view = Bounds { origin: point(px(0.), px(0.)), size: size(px(self.view.0), px(self.view.1)) };
        let seen = body.intersect(&view);
        if f32::from(seen.size.width) < 4. || f32::from(seen.size.height) < 4. {
            return None;
        }
        let key = key_of(&b.id);
        let version = self.version_of(b, dark);
        let info = window.snapshot_info(key).filter(|i| i.version == version);
        let now_part = part_of(seen, body);
        let covers = info.is_some_and(|i| {
            let had = part_of(i.visible, i.bounds);
            let e = 0.01;
            had.0 <= now_part.0 + e && had.1 <= now_part.1 + e && had.2 >= now_part.2 - e && had.3 >= now_part.3 - e
        });
        let fresh = info.is_some_and(|i| {
            let had = part_of(i.visible, i.bounds);
            let same = |a: f32, b: f32| (a - b).abs() < 0.002;
            (f32::from(i.bounds.size.width - body.size.width)).abs() < 0.5
                && (f32::from(i.bounds.size.height - body.size.height)).abs() < 0.5
                && same(had.0, now_part.0)
                && same(had.1, now_part.1)
                && same(had.2, now_part.2)
                && same(had.3, now_part.3)
        });
        let (mx, my) = (self.view.0 / 2., self.view.1 / 2.);
        let (cx, cy) = (f32::from(body.origin.x + body.size.width / 2.), f32::from(body.origin.y + body.size.height / 2.));
        Some(Picture { key, version, info, covers, fresh, rank: (cx - mx).powi(2) + (cy - my).powi(2) })
    }

    /// Before the blocks of a frame are built, while playing: which bodies
    /// this frame lays out and pictures (`CAPTURES_PER_FRAME`), nearest the
    /// middle of the window first. Gliding, those with no picture that holds
    /// what shows of them; at rest, those whose picture is not of this scale.
    pub(super) fn plan_captures(&mut self, visible: &[String], dark: bool, window: &Window) {
        self.captures.clear();
        if self.fv.play.is_none() {
            return;
        }
        let now = Instant::now();
        let gliding = self.glide.is_some();
        let mut wanted: Vec<(f32, &String)> = visible
            .iter()
            .filter_map(|id| {
                let b = self.m.block(id)?;
                let p = self.picture_of(b, dark, window, now)?;
                let needs = if gliding { !p.covers } else { !p.fresh };
                needs.then_some((p.rank, id))
            })
            .collect();
        wanted.sort_by(|a, b| a.0.total_cmp(&b.0));
        self.captures = wanted.into_iter().take(CAPTURES_PER_FRAME).map(|(_, id)| id.clone()).collect();
    }

    /// How block `b`'s body is drawn this frame.
    pub(super) fn body_plan(&mut self, b: &Block, dark: bool, window: &Window, now: Instant) -> Plan {
        if !self.picturing() {
            return Plan::Body;
        }
        if self.rts.get(&b.id).and_then(|r| r.live_until).is_some_and(|t| t > now) {
            // Scrolled a moment ago: live, and pictured again once it rests.
            self.animating = true;
            return Plan::Body;
        }
        let Some(p) = self.picture_of(b, dark, window, now) else { return Plan::Body };
        let (key, version) = (p.key, p.version);
        if self.fv.play.is_none() {
            // A glide, not playing: pictured in its first frames, stretched after.
            return if p.covers { Plan::Show { key, version } } else { Plan::Capture { key, version } };
        }
        if p.fresh || (self.glide.is_some() && p.covers) {
            return Plan::Show { key, version };
        }
        if self.captures.contains(&b.id) {
            return Plan::Capture { key, version };
        }
        // Waiting its turn: its old picture stretched, or the bare card.
        self.animating = true;
        if p.info.is_some() { Plan::Show { key, version } } else { Plan::Blank }
    }

    /// What decides how block `b`'s body looks, short of the scale: a picture
    /// of another version is not shown. Not playing, each glide pictures anew.
    fn version_of(&self, b: &Block, dark: bool) -> u64 {
        let mut h = std::collections::hash_map::DefaultHasher::new();
        b.kind.as_str().hash(&mut h);
        if self.fv.play.is_none() {
            self.glides.hash(&mut h);
        }
        (b.w.to_bits(), b.h.to_bits(), b.page).hash(&mut h);
        (&b.text, &b.path, &b.chart, dark).hash(&mut h);
        if let Some(rt) = self.rts.get(&b.id) {
            let ptr = |a: Option<usize>| a.unwrap_or(0);
            ptr(rt.lines.as_ref().and_then(|r| r.as_ref().ok()).map(|a| std::sync::Arc::as_ptr(a) as usize)).hash(&mut h);
            ptr(rt.doc.as_ref().and_then(|r| r.as_ref().ok()).map(|a| std::sync::Arc::as_ptr(a) as usize)).hash(&mut h);
            ptr(rt.word.as_ref().and_then(|r| r.as_ref().ok()).map(|a| std::sync::Arc::as_ptr(a) as usize)).hash(&mut h);
            ptr(rt.pdf_img.as_ref().map(|(p, _, img)| *p ^ (std::sync::Arc::as_ptr(img) as usize))).hash(&mut h);
            if let Some((st, parsed)) = &rt.md_list {
                parsed.ptr().hash(&mut h);
                let top = st.logical_scroll_top();
                (top.item_ix, f32::from(top.offset_in_item).to_bits()).hash(&mut h);
            }
            let off = rt.list.0.borrow().base_handle.offset();
            (f32::from(off.x).to_bits(), f32::from(off.y).to_bits()).hash(&mut h);
            rt.loading.hash(&mut h);
        }
        h.finish()
    }

    /// A wheel over block `id`'s body while playing: it is drawn live a while,
    /// so its list scrolls. True if it was a picture (this wheel is spent).
    pub(super) fn body_wheel(&mut self, id: &str, now: Instant) -> bool {
        if self.fv.play.is_none() {
            return false;
        }
        let Some(rt) = self.rts.get_mut(id) else { return false };
        let was_live = rt.live_until.is_some_and(|t| t > now);
        rt.live_until = Some(now + LIVE_AFTER_WHEEL);
        !was_live
    }
}

/// The part `part` of `of`, as fractions of it: left, top, right, bottom.
fn part_of(part: Bounds<Pixels>, of: Bounds<Pixels>) -> (f32, f32, f32, f32) {
    let (ow, oh) = (f32::from(of.size.width).max(1.), f32::from(of.size.height).max(1.));
    (
        f32::from(part.origin.x - of.origin.x) / ow,
        f32::from(part.origin.y - of.origin.y) / oh,
        f32::from(part.right() - of.origin.x) / ow,
        f32::from(part.bottom() - of.origin.y) / oh,
    )
}

/// Whether a kind's body is pictured: content not changing by itself. A
/// QuickLook's paper is a gradient: its picture holds the text alone, laid
/// over the paper drawn live (`body_background`).
fn pictured(kind: BlockKind) -> bool {
    matches!(kind, BlockKind::Memo | BlockKind::Note | BlockKind::File | BlockKind::Chart)
}

/// The picture's key for a block.
fn key_of(id: &str) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    "body".hash(&mut h);
    id.hash(&mut h);
    h.finish()
}

/// What a body's picture is taken over: the card's color, or for a
/// QuickLook, nothing (the paper under it is drawn live).
pub(super) fn body_background(kind: BlockKind, surface: Hsla) -> Hsla {
    if kind == BlockKind::Note { gpui::transparent_black() } else { surface }
}

/// The ids pictured this frame (`plan_captures`).
pub(super) type Captures = HashSet<String>;

/// The body drawn by plan: `body` as it is, kept as a picture over `background`, the picture, or nothing.
pub(super) fn body_by(plan: Plan, body: AnyElement, background: Hsla) -> AnyElement {
    match plan {
        Plan::Body => body,
        Plan::Capture { key, version } => {
            gpui::snapshot_capture(key, version, background, div().size_full().child(body)).into_any_element()
        }
        Plan::Show { key, version } => canvas(|_, _, _| (), move |bounds, _, window, _| {
            window.paint_snapshot(key, version, bounds, 1.);
        })
        .size_full()
        .into_any_element(),
        Plan::Blank => div().size_full().into_any_element(),
    }
}
