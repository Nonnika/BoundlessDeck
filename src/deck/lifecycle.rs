//! A block's lifecycle: four pairs of events, each a layer inside the one
//! before it, so a layer is always left before the one around it.
//!
//! ```text
//! onCreate ─▶ onOpen ─▶ onResume ─▶ onFocus ⇄ onBlur
//!                          ▲ │                   │
//!                          │ ▼                   ▼
//!                         onPause ─────▶ onClose or onDelete
//! ```
//!
//! - **Create / Delete**, the data: `on_create` once in a block's life, when
//!   it is first made (not when the deck opens, not when undo brings it
//!   back); `on_delete` each time it leaves the deck. Deleting can be undone,
//!   so `on_delete` lets the runtime go (a shell, pages) and leaves what is
//!   outside the deck (a memo's file) alone.
//! - **Open / Close**, the canvas: a stay in the open deck begins with
//!   `on_open` (the deck opened, the block was made, or undo brought it back)
//!   and ends with `on_close` (the deck closes, before it is saved) or
//!   `on_delete`. Light work only: the runtime starts on the first resume.
//! - **Resume / Pause**, the screen: resumed while drawn whole; paused while
//!   off screen, in a stack, zoomed out to its frame (LOD), or while the
//!   window is not the active one. A shell keeps running while paused.
//! - **Focus / Blur**, the block worked in: at most one, and only a resumed
//!   one. The deck remembers which block focus belongs to (`focus_target`),
//!   so a block blurred by a pause is focused again when it resumes, and an
//!   edit under way survives (`Blur::Paused` keeps the editor).
//!
//! Every event goes out from here, on the UI thread: `life_tick` each frame
//! works out where each block should be and sends what changed;
//! `life_delete` and `life_close` end a stay. A kind reacts through its
//! `BlockHooks`, every method of which does nothing by default.

use gpui::{Context, Focusable, Window};

use super::DeckView;
use crate::model::{Block, BlockKind, LOD_SCALE};

/// Why a block is paused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Pause {
    Offscreen,
    Stacked,
    /// Zoomed out to its frame and title.
    Lod,
    /// The window is not the active one.
    WindowInactive,
    /// Its stay ends: the deck closes or the block leaves it.
    Leaving,
}

/// Why a block loses focus.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Blur {
    /// The user went to work elsewhere: an edit ends.
    Moved,
    /// It paused; focus comes back on resume, an edit under way is kept.
    Paused,
    /// Its stay ends: an edit ends.
    Leaving,
}

/// Where a block is in its lifecycle; part of its `Runtime`.
#[derive(Clone, Copy, Debug, Default)]
pub(super) struct Life {
    pub open: bool,
    pub shown: bool,
    pub focused: bool,
}

/// What a kind of block does at each event: every method does nothing
/// unless the kind says otherwise. Each gets the block as it is (or, for
/// `on_delete`, as it was when it left).
pub(super) trait BlockHooks: Sync {
    fn on_create(&self, _d: &mut DeckView, _b: &Block, _cx: &mut Context<DeckView>) {}
    fn on_open(&self, _d: &mut DeckView, _b: &Block, _cx: &mut Context<DeckView>) {}
    fn on_resume(&self, _d: &mut DeckView, _b: &Block, _cx: &mut Context<DeckView>) {}
    fn on_pause(&self, _d: &mut DeckView, _b: &Block, _why: Pause, _cx: &mut Context<DeckView>) {}
    fn on_focus(&self, _d: &mut DeckView, _b: &Block, _window: &mut Window, _cx: &mut Context<DeckView>) {}
    fn on_blur(&self, _d: &mut DeckView, _b: &Block, _why: Blur, _cx: &mut Context<DeckView>) {}
    fn on_close(&self, _d: &mut DeckView, _b: &Block, _cx: &mut Context<DeckView>) {}
    fn on_delete(&self, _d: &mut DeckView, _b: &Block, _cx: &mut Context<DeckView>) {}
}

/// A kind with nothing to do at any event.
struct Plain;
impl BlockHooks for Plain {}

/// The shell starts on the first resume, keeps running while paused, takes
/// the keyboard with the focus, and hangs up when its stay ends.
struct TerminalHooks;
impl BlockHooks for TerminalHooks {
    fn on_resume(&self, d: &mut DeckView, b: &Block, cx: &mut Context<DeckView>) {
        d.start_terminal(&b.id, cx);
    }
    fn on_focus(&self, d: &mut DeckView, b: &Block, window: &mut Window, cx: &mut Context<DeckView>) {
        if let Some(t) = d.rts.get(&b.id).and_then(|r| r.term.clone()) {
            window.focus(&t.focus_handle(cx));
        }
    }
    fn on_blur(&self, d: &mut DeckView, b: &Block, _: Blur, cx: &mut Context<DeckView>) {
        if let Some(t) = d.rts.get(&b.id).and_then(|r| r.term.clone()) {
            t.update(cx, |t, cx| t.cancel_composition(cx));
        }
    }
    fn on_close(&self, d: &mut DeckView, b: &Block, cx: &mut Context<DeckView>) {
        d.hang_up(&b.id, cx);
    }
    fn on_delete(&self, d: &mut DeckView, b: &Block, cx: &mut Context<DeckView>) {
        d.hang_up(&b.id, cx);
    }
}

/// What was typed reaches the file whenever the note pauses or its stay ends.
struct MemoHooks;
impl BlockHooks for MemoHooks {
    fn on_pause(&self, d: &mut DeckView, b: &Block, _: Pause, _: &mut Context<DeckView>) {
        d.flush_memo(b);
    }
    fn on_close(&self, d: &mut DeckView, b: &Block, _: &mut Context<DeckView>) {
        d.flush_memo(b);
    }
    fn on_delete(&self, d: &mut DeckView, b: &Block, _: &mut Context<DeckView>) {
        d.flush_memo(b);
    }
}

/// A video file's player pauses when the block does (but not for another
/// app coming to the front), and goes when its stay ends.
struct FileHooks;
impl BlockHooks for FileHooks {
    fn on_pause(&self, d: &mut DeckView, b: &Block, why: Pause, _: &mut Context<DeckView>) {
        if why != Pause::WindowInactive {
            d.pause_video(&b.id);
        }
    }
    fn on_close(&self, d: &mut DeckView, b: &Block, _: &mut Context<DeckView>) {
        d.drop_video(&b.id);
    }
    fn on_delete(&self, d: &mut DeckView, b: &Block, _: &mut Context<DeckView>) {
        d.drop_video(&b.id);
    }
}

/// The hooks of a kind.
pub(super) fn hooks(kind: BlockKind) -> &'static dyn BlockHooks {
    match kind {
        BlockKind::Terminal => &TerminalHooks,
        BlockKind::Memo => &MemoHooks,
        BlockKind::File => &FileHooks,
        _ => &Plain,
    }
}

/// Where a block should be on screen: resumed (None) or paused, and why.
pub(super) fn pause_for(window_active: bool, stacked: bool, on_screen: bool, lod: bool) -> Option<Pause> {
    if !window_active {
        Some(Pause::WindowInactive)
    } else if stacked {
        Some(Pause::Stacked)
    } else if !on_screen {
        Some(Pause::Offscreen)
    } else if lod {
        Some(Pause::Lod)
    } else {
        None
    }
}

impl DeckView {
    /// The block focus belongs to: the one being edited, else the one worked in.
    pub(super) fn focus_target(&self) -> Option<String> {
        self.editing.as_ref().map(|e| e.id.clone()).or_else(|| self.active.clone())
    }

    /// Each frame: brings every block to where the deck says it should be,
    /// sending the events on the way. `visible` are the blocks drawn.
    pub(super) fn life_tick(&mut self, visible: &[String], window: &mut Window, cx: &mut Context<Self>) {
        let now = std::time::Instant::now();
        for id in std::mem::take(&mut self.m.created) {
            if let Some(b) = self.m.block(&id).cloned() {
                self.rts.entry(id).or_default().pop.arm(now);
                hooks(b.kind).on_create(self, &b, cx);
            }
        }
        let window_active = window.is_window_active();
        let lod = self.m.cam.s < LOD_SCALE;
        let stacked = self.m.stacked_ids();
        let visible: std::collections::HashSet<&str> = visible.iter().map(String::as_str).collect();
        // Most frames send nothing: look first, and copy a block only to hand it to a hook.
        let mut due = Vec::new();
        for b in &self.m.blocks {
            let rt = self.rts.get(&b.id).map(|r| r.life);
            let open = rt.is_some_and(|l| l.open);
            // A block held under its mask a frame longer (`LOD_WAKE_PER_FRAME`) resumes with it.
            let lod = lod || self.rts.get(&b.id).is_some_and(|r| r.lod);
            let want = pause_for(window_active, stacked.contains(&b.id), visible.contains(b.id.as_str()), lod);
            let shown = open && rt.is_some_and(|l| l.shown);
            if !open || shown != want.is_none() {
                due.push(b.id.clone());
            }
        }
        for id in due {
            let Some(b) = self.m.block(&id).cloned() else { continue };
            let b = &b;
            let rt = self.rts.entry(b.id.clone()).or_default();
            if !rt.life.open {
                rt.life = Life { open: true, ..Life::default() };
                hooks(b.kind).on_open(self, b, cx);
            }
            let lod = lod || self.rts.get(&b.id).is_some_and(|r| r.lod);
            let want = pause_for(window_active, stacked.contains(&b.id), visible.contains(b.id.as_str()), lod);
            let shown = self.rts.get(&b.id).is_some_and(|r| r.life.shown);
            match (shown, want) {
                (true, Some(why)) => self.pause(b, why, cx),
                (false, None) => {
                    if let Some(rt) = self.rts.get_mut(&b.id) {
                        rt.life.shown = true;
                    }
                    hooks(b.kind).on_resume(self, b, cx);
                }
                _ => {}
            }
        }
        self.sync_focus(window, cx);
    }

    /// Gives focus to the block it belongs to, if that one is resumed, and
    /// takes it from any other: blur first, then focus.
    pub(super) fn sync_focus(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let want = self.focus_target().filter(|id| self.rts.get(id).is_some_and(|r| r.life.shown));
        let losing: Vec<String> =
            self.rts.iter().filter(|(id, r)| r.life.focused && Some(*id) != want.as_ref()).map(|(id, _)| id.clone()).collect();
        for id in losing {
            if let Some(b) = self.m.block(&id).cloned() {
                self.blur(&b, Blur::Moved, cx);
            }
        }
        let Some(id) = want else { return };
        let Some(rt) = self.rts.get_mut(&id) else { return };
        if rt.life.focused {
            return;
        }
        rt.life.focused = true;
        let Some(b) = self.m.block(&id).cloned() else { return };
        // An edit kept through a pause takes the keyboard back where it was.
        if let Some(e) = self.editing.as_ref().filter(|e| e.id == id) {
            let f = e.editor.focus_handle(cx);
            window.focus(&f);
        }
        hooks(b.kind).on_focus(self, &b, window, cx);
    }

    fn blur(&mut self, b: &Block, why: Blur, cx: &mut Context<Self>) {
        let Some(rt) = self.rts.get_mut(&b.id) else { return };
        if !rt.life.focused {
            return;
        }
        rt.life.focused = false;
        if why != Blur::Paused && self.editing.as_ref().is_some_and(|e| e.id == b.id) {
            self.stop_editing(cx);
        }
        hooks(b.kind).on_blur(self, b, why, cx);
    }

    fn pause(&mut self, b: &Block, why: Pause, cx: &mut Context<Self>) {
        let leaving = why == Pause::Leaving;
        self.blur(b, if leaving { Blur::Leaving } else { Blur::Paused }, cx);
        let Some(rt) = self.rts.get_mut(&b.id) else { return };
        if !rt.life.shown {
            return;
        }
        rt.life.shown = false;
        hooks(b.kind).on_pause(self, b, why, cx);
    }

    /// Ends the stays of the blocks that left the deck: blur, pause, `on_delete`,
    /// and their runtime goes. One that undo already brought back stays.
    pub(super) fn life_delete(&mut self, cx: &mut Context<Self>) {
        for b in std::mem::take(&mut self.m.hung_up) {
            if self.m.block(&b.id).is_some() {
                continue;
            }
            if self.rts.get(&b.id).is_some_and(|r| r.life.open) {
                self.blur(&b, Blur::Leaving, cx);
                self.pause(&b, Pause::Leaving, cx);
                hooks(b.kind).on_delete(self, &b, cx);
            }
            self.rts.remove(&b.id);
            if self.active.as_deref() == Some(b.id.as_str()) {
                self.active = None;
            }
        }
    }

    /// Ends every block's stay as the deck closes: blur, pause, `on_close`.
    /// The caller saves after.
    pub(super) fn life_close(&mut self, cx: &mut Context<Self>) {
        let blocks: Vec<Block> = self.m.blocks.clone();
        for b in &blocks {
            if self.rts.get(&b.id).is_some_and(|r| r.life.open) {
                self.blur(b, Blur::Leaving, cx);
                self.pause(b, Pause::Leaving, cx);
                hooks(b.kind).on_close(self, b, cx);
            }
        }
        self.rts.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_block_pauses_for_the_first_reason_that_holds() {
        assert_eq!(pause_for(true, false, true, false), None, "drawn whole: resumed");
        assert_eq!(pause_for(false, false, true, false), Some(Pause::WindowInactive));
        assert_eq!(pause_for(true, true, false, false), Some(Pause::Stacked), "a stacked block is not drawn either");
        assert_eq!(pause_for(true, false, false, false), Some(Pause::Offscreen));
        assert_eq!(pause_for(true, false, true, true), Some(Pause::Lod));
    }
}
