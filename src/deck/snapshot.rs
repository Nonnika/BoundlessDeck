//! Blocks' bodies as pictures while the camera glides. A glide (focus
//! play, an agent's `set_view`) changes the scale every frame, and laying a
//! block's text out and painting its glyphs at each new size was most of
//! what such a frame cost. So in a glide's first frames each body is drawn
//! as it is and kept as a picture (GPUI's `snapshot_capture`, our patch),
//! and in the frames after it is that picture, stretched, that shows. When
//! the glide ends, bodies are drawn as they are again.
//!
//! A picture is of a version of the content (`version`); one of another
//! version is not shown. One holds only the part of the body that was in
//! the window: a body more of which comes into sight is drawn and pictured
//! anew. Bodies that change on their own (terminals, web pages), are being
//! worked in, or are not flat content on the card's own color are not
//! pictured.

use std::hash::{Hash, Hasher};
use std::time::Instant;

use gpui::{AnyElement, Bounds, Hsla, IntoElement, ParentElement, Styled, Window, canvas, div, point, px, size};

use super::{DeckView, TITLE_H};
use crate::model::{Block, BlockKind};

/// How a body is drawn this frame.
pub(super) enum Plan {
    /// Laid out and drawn as it is.
    Body,
    /// Laid out and drawn, and kept as a picture.
    Capture { key: u64, version: u64 },
    /// Its picture, stretched to where it is now.
    Show { key: u64, version: u64 },
}

impl DeckView {
    /// How block `b`'s body, at sx, sy, w by h on the screen, is drawn this frame.
    pub(super) fn body_plan(&mut self, b: &Block, (sx, sy, w, h): (f32, f32, f32, f32), dark: bool, window: &Window, now: Instant) -> Plan {
        if self.glide.is_none()
            || !pictured(b.kind)
            || self.active.as_deref() == Some(b.id.as_str())
            || self.editing.as_ref().is_some_and(|e| e.id == b.id)
            || self.drag.is_some()
            || self.photo_showing()
            || self.canvas_scroll_until.is_some_and(|t| t > now)
        {
            return Plan::Body;
        }
        let s = self.m.cam.s;
        let body = Bounds { origin: point(px(sx), px(sy + TITLE_H * s)), size: size(px(w), px((h - TITLE_H * s).max(0.))) };
        let view = Bounds { origin: point(px(0.), px(0.)), size: size(px(self.view.0), px(self.view.1)) };
        let seen = body.intersect(&view);
        if f32::from(seen.size.width) < 4. || f32::from(seen.size.height) < 4. {
            return Plan::Body;
        }
        let key = key_of(&b.id);
        let version = self.version_of(b, dark);
        let info = window.snapshot_info(key).filter(|i| i.version == version);
        // The part of the body in sight now, and in the picture, as fractions of the body.
        let frac = |part: Bounds<gpui::Pixels>, of: Bounds<gpui::Pixels>| {
            let (ow, oh) = (f32::from(of.size.width).max(1.), f32::from(of.size.height).max(1.));
            (
                f32::from(part.origin.x - of.origin.x) / ow,
                f32::from(part.origin.y - of.origin.y) / oh,
                f32::from(part.right() - of.origin.x) / ow,
                f32::from(part.bottom() - of.origin.y) / oh,
            )
        };
        let now_part = frac(seen, body);
        // Gliding: the picture, if it holds all of the body that shows; else
        // the body, laid out and pictured for the frames after.
        let covers = info.is_some_and(|i| {
            let had = frac(i.visible, i.bounds);
            let e = 0.01;
            had.0 <= now_part.0 + e && had.1 <= now_part.1 + e && had.2 >= now_part.2 - e && had.3 >= now_part.3 - e
        });
        if covers { Plan::Show { key, version } } else { Plan::Capture { key, version } }
    }

    /// What decides how block `b`'s body looks, short of the scale: a picture
    /// of another version is not shown.
    fn version_of(&self, b: &Block, dark: bool) -> u64 {
        let mut h = std::collections::hash_map::DefaultHasher::new();
        b.kind.as_str().hash(&mut h);
        self.glides.hash(&mut h);
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
}

/// Whether a kind's body is pictured: flat content on the card's color, not
/// changing by itself.
fn pictured(kind: BlockKind) -> bool {
    matches!(kind, BlockKind::Memo | BlockKind::File | BlockKind::Chart)
}

/// The picture's key for a block.
fn key_of(id: &str) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    "body".hash(&mut h);
    id.hash(&mut h);
    h.finish()
}

/// The body drawn by plan: `body` as it is, kept as a picture over `background`, or the picture.
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
    }
}
