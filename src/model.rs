//! The deck's model: its blocks, the camera, the selection, undo and redo,
//! and deck.json. Nothing here draws or needs a window, so it is tested
//! headless; `deck.rs` is the view over it.
//!
//! deck.json is the same file the Go build reads and writes: field names
//! and omitted empties match, so either build opens a folder the other saved.
//! What only this build keeps (a browser's tabs) the Go build leaves alone;
//! `url` stays the page the Go build shows.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

mod block;
mod camera;
mod focus;
mod persist;

pub use block::{
    BLOCK_REACH_SIDE, BLOCK_REACH_TOP, Block, BlockKind, MIN_BLOCK_H, MIN_BLOCK_W, SHAPES, Style, file_block_size,
    file_name, new_block_of, next_tab_key, scale_keeping_ratio,
};
pub use camera::{
    Camera, LOD_SCALE, MAX_SCALE, MIN_SCALE, ZOOM_KEY_STEPS, ZOOM_LEVELS, ZOOM_REBASE, wheel_zoom, zoom_level,
    zoom_level_at_most, zoom_level_of,
};
pub use focus::{Focus, FocusLine};
pub use persist::{data_dir, write_file_atomic};
// Named only by tests, now that a zoom no longer comes to rest on a level.
#[cfg(test)]
pub use camera::on_level;
// Named only by tests; elsewhere tabs are reached through `Block`.
#[cfg(test)]
pub use block::WebTab;

/// The folder inside a deck's folder that holds its canvas.
pub const DECK_DIR: &str = ".boundlessdeck";

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Geometry {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl Geometry {
    pub fn of(b: &Block) -> Geometry {
        Geometry { x: b.x, y: b.y, w: b.w, h: b.h }
    }
    fn apply(&self, b: &mut Block) {
        b.x = self.x;
        b.y = self.y;
        b.w = self.w;
        b.h = self.h;
    }
}

#[derive(Clone, Debug)]
pub struct MoveChange {
    pub id: String,
    pub before: Geometry,
    pub after: Geometry,
}

#[derive(Clone, Debug)]
pub struct StackEdit {
    pub stack: String,
    pub before: Vec<String>,
    pub after: Vec<String>,
}

/// One undoable edit of the deck.
#[derive(Clone, Debug)]
pub enum Change {
    /// A block added. `block` is what it was when last taken off by an undo.
    Add { block: Block, index: usize, sel: HashSet<String> },
    /// A block removed, as it was.
    Remove { block: Block, index: usize },
    /// Blocks moved or resized at once.
    Moves(Vec<MoveChange>),
    /// What stacks hold.
    Stack(Vec<StackEdit>),
    /// Changes that undo as one, applied in order.
    Group(Vec<Change>),
}

/// The deck's state without its window.
#[derive(Default)]
pub struct Model {
    /// The deck's folder: terminals start here, paths in it are stored relative to it.
    pub root: PathBuf,
    /// Where the deck is saved: `<root>/.boundlessdeck/deck.json` unless a test put it elsewhere.
    pub file: Option<PathBuf>,
    pub cam: Camera,
    pub blocks: Vec<Block>,
    /// Blocks deck.json holds that this build cannot read (a newer kind, say),
    /// kept as they were and saved back after the others.
    pub foreign: Vec<serde_json::Value>,
    /// 焦点 and the lines through them (`focus`), saved at deck.json's top level.
    pub focus: Vec<Focus>,
    pub focus_lines: Vec<FocusLine>,
    /// What deck.json's top level holds that this build does not know, saved back as it was.
    pub extra: serde_json::Map<String, serde_json::Value>,
    pub sel: HashSet<String>,
    pub undo: Vec<Change>,
    pub redo: Vec<Change>,
    /// Blocks made since the view last looked (not those undo brings back): their `on_create`.
    pub created: Vec<String>,
    /// Blocks that left the deck since the view last looked, as they were: their `on_delete`.
    pub hung_up: Vec<Block>,
    /// The clock QuickLook blocks go by, in Unix ms; the wall clock when None.
    pub clock: Option<i64>,
}

impl Model {
    pub fn new(root: impl Into<PathBuf>) -> Model {
        Model { root: root.into(), ..Default::default() }
    }

    pub fn now_ms(&self) -> i64 {
        self.clock.unwrap_or_else(|| chrono::Utc::now().timestamp_millis())
    }

    // ---- the camera ----

    /// Zooms by factor, keeping the world point under (sx, sy) where it is.
    pub fn zoom_at(&mut self, sx: f32, sy: f32, factor: f32) {
        if factor <= 0.0 || !factor.is_finite() {
            return;
        }
        let (wx, wy) = self.cam.to_world(sx, sy);
        self.cam.s *= factor;
        self.cam.clamp();
        self.cam.x = wx - sx / self.cam.s;
        self.cam.y = wy - sy / self.cam.s;
    }

    /// Zooms by key steps about (sx, sy): from a level to a level.
    pub fn zoom_step(&mut self, sx: f32, sy: f32, steps: i32) {
        let n = (f64::from(self.cam.s).log2() * ZOOM_LEVELS).round() + f64::from(steps * ZOOM_KEY_STEPS);
        let to = zoom_level_of(n);
        self.zoom_at(sx, sy, to / self.cam.s);
        self.cam.s = to;
    }

    /// Frames every block on the canvas, or resets the camera when there is none.
    pub fn fit_all(&mut self, cw: f32, ch: f32) {
        let hidden = self.stacked_ids();
        let vis: Vec<&Block> = self.blocks.iter().filter(|b| !hidden.contains(&b.id)).collect();
        let Some(first) = vis.first() else {
            self.cam = Camera::default();
            return;
        };
        let (mut min_x, mut min_y) = (first.x, first.y);
        let (mut max_x, mut max_y) = (first.x + first.w, first.y + first.h);
        for b in &vis[1..] {
            min_x = min_x.min(b.x);
            min_y = min_y.min(b.y);
            max_x = max_x.max(b.x + b.w);
            max_y = max_y.max(b.y + b.h);
        }
        const PAD: f32 = 60.0;
        let (w, h) = (max_x - min_x + 2.0 * PAD, max_y - min_y + 2.0 * PAD);
        let s = (cw / w).min(ch / h);
        self.cam.s = zoom_level_at_most(s.clamp(MIN_SCALE, 1.0));
        self.cam.x = min_x - PAD - (cw / self.cam.s - w) / 2.0;
        self.cam.y = min_y - PAD - (ch / self.cam.s - h) / 2.0;
    }

    // ---- blocks ----

    pub fn index_of(&self, id: &str) -> Option<usize> {
        self.blocks.iter().position(|b| b.id == id)
    }

    pub fn block(&self, id: &str) -> Option<&Block> {
        self.blocks.iter().find(|b| b.id == id)
    }

    pub fn block_mut(&mut self, id: &str) -> Option<&mut Block> {
        self.blocks.iter_mut().find(|b| b.id == id)
    }

    pub fn new_id(&self) -> String {
        loop {
            let id = format!("b{:06x}", rand::random::<u32>() & 0xff_ffff);
            let foreign = self.foreign.iter().any(|v| v.get("id").and_then(|i| i.as_str()) == Some(id.as_str()));
            if self.index_of(&id).is_none() && !foreign {
                return id;
            }
        }
    }

    /// Puts a block on the canvas and selects it: the only way a block
    /// joins the deck, so every addition undoes. Returns its ID.
    pub fn add_block(&mut self, mut b: Block) -> String {
        if b.id.is_empty() {
            b.id = self.new_id();
        }
        if b.w <= 0.0 {
            b.w = 360.0;
        }
        if b.h <= 0.0 {
            b.h = 260.0;
        }
        self.forget_redo();
        self.undo.push(Change::Add { block: b.clone(), index: self.blocks.len(), sel: self.sel.clone() });
        let id = b.id.clone();
        self.created.push(id.clone());
        self.blocks.push(b);
        self.sel = HashSet::from([id.clone()]);
        id
    }

    /// Takes a block off the canvas; its terminal hangs up.
    pub fn remove_block(&mut self, id: &str) {
        let Some(i) = self.index_of(id) else { return };
        self.forget_redo();
        let block = self.detach(i);
        self.undo.push(Change::Remove { block, index: i });
        self.sel.remove(id);
    }

    /// Removes the selected blocks, the last first, so the indices kept stay true.
    pub fn delete_selected(&mut self) {
        for i in (0..self.blocks.len()).rev() {
            if self.sel.contains(&self.blocks[i].id) {
                let id = self.blocks[i].id.clone();
                self.remove_block(&id);
            }
        }
    }

    fn attach(&mut self, index: usize, b: Block) {
        let index = index.min(self.blocks.len());
        self.blocks.insert(index, b);
    }

    fn detach(&mut self, index: usize) -> Block {
        let b = self.blocks.remove(index);
        self.hung_up.push(b.clone());
        b
    }

    /// Drops what redo would redo: the blocks it added stay off for good.
    fn forget_redo(&mut self) {
        for c in self.redo.drain(..) {
            if let Change::Add { block, .. } = c {
                self.hung_up.push(block);
            }
        }
    }

    pub fn undo_last(&mut self) {
        let Some(mut c) = self.undo.pop() else { return };
        self.reverse(&mut c);
        self.redo.push(c);
    }

    fn reverse(&mut self, c: &mut Change) {
        match c {
            Change::Group(cs) => {
                for c in cs.iter_mut().rev() {
                    self.reverse(c);
                }
            }
            Change::Add { block, sel, .. } => {
                if let Some(i) = self.index_of(&block.id) {
                    *block = self.detach(i);
                }
                self.sel = sel.clone();
            }
            Change::Remove { block, index, .. } => {
                self.attach(*index, block.clone());
                self.sel = HashSet::from([block.id.clone()]);
                self.rescue_note(&block.id.clone());
            }
            Change::Moves(moves) => {
                for m in moves.iter().rev() {
                    if let Some(b) = self.blocks.iter_mut().find(|b| b.id == m.id) {
                        m.before.apply(b);
                    }
                }
            }
            Change::Stack(edits) => self.restore_stack_edits(edits, false),
        }
    }

    pub fn redo_last(&mut self) {
        let Some(mut c) = self.redo.pop() else { return };
        self.apply(&mut c);
        self.undo.push(c);
    }

    fn apply(&mut self, c: &mut Change) {
        match c {
            Change::Group(cs) => {
                for c in cs.iter_mut() {
                    self.apply(c);
                }
            }
            Change::Add { block, index, .. } => {
                self.attach(*index, block.clone());
                self.sel = HashSet::from([block.id.clone()]);
            }
            Change::Remove { block, .. } => {
                if let Some(i) = self.index_of(&block.id) {
                    *block = self.detach(i);
                }
                self.sel.remove(&block.id);
            }
            Change::Moves(moves) => {
                for m in moves.iter() {
                    if let Some(b) = self.blocks.iter_mut().find(|b| b.id == m.id) {
                        m.after.apply(b);
                    }
                }
            }
            Change::Stack(edits) => self.restore_stack_edits(edits, true),
        }
    }

    /// Moves blocks to where a layout puts them (top-left only) as one
    /// undoable change; returns how many moved.
    pub fn apply_placements(&mut self, ps: &[(String, f32, f32)]) -> usize {
        let mut moves = Vec::new();
        for (id, x, y) in ps {
            let Some(b) = self.block(id) else { continue };
            if b.x == *x && b.y == *y {
                continue;
            }
            let before = Geometry::of(b);
            moves.push(MoveChange { id: id.clone(), before, after: Geometry { x: *x, y: *y, ..before } });
        }
        if moves.is_empty() {
            return 0;
        }
        self.forget_redo();
        for m in &moves {
            if let Some(b) = self.block_mut(&m.id) {
                b.x = m.after.x;
                b.y = m.after.y;
            }
        }
        let n = moves.len();
        self.undo.push(Change::Moves(moves));
        n
    }

    /// Replaces a stack's items as one undoable edit, taking them from any
    /// other stack they were in.
    pub fn set_stack_items(&mut self, stack: &str, ids: &[String]) {
        let mut edits = Vec::new();
        for o in self.blocks.iter().filter(|o| o.kind == BlockKind::Stack && o.id != stack) {
            let after: Vec<String> = o.items.iter().filter(|i| !ids.contains(i)).cloned().collect();
            if after.len() != o.items.len() {
                edits.push(StackEdit { stack: o.id.clone(), before: o.items.clone(), after });
            }
        }
        let Some(st) = self.block(stack) else { return };
        if st.items != ids {
            edits.push(StackEdit { stack: stack.to_string(), before: st.items.clone(), after: ids.to_vec() });
        }
        if edits.is_empty() {
            return;
        }
        self.forget_redo();
        self.restore_stack_edits(&edits, true);
        self.undo.push(Change::Stack(edits));
    }

    /// Records a move or resize done by hand, from the boxes before it, as one undoable change.
    pub fn record_moves(&mut self, before: &HashMap<String, Geometry>) -> usize {
        let moves: Vec<MoveChange> = self
            .blocks
            .iter()
            .filter_map(|b| {
                let g = *before.get(&b.id)?;
                let now = Geometry::of(b);
                (g != now).then(|| MoveChange { id: b.id.clone(), before: g, after: now })
            })
            .collect();
        if moves.is_empty() {
            return 0;
        }
        self.forget_redo();
        let n = moves.len();
        self.undo.push(Change::Moves(moves));
        n
    }

    /// Brings the block to the top, over the others.
    pub fn raise(&mut self, id: &str) {
        if let Some(i) = self.index_of(id)
            && i + 1 < self.blocks.len()
        {
            let b = self.blocks.remove(i);
            self.blocks.push(b);
        }
    }

    /// The blocks the window shows some of, tape and shadow included, back to front.
    pub fn visible(&self, vw: f32, vh: f32) -> Vec<usize> {
        let hidden = self.stacked_ids();
        let c = self.cam;
        (0..self.blocks.len())
            .filter(|&i| {
                let b = &self.blocks[i];
                if hidden.contains(&b.id) {
                    return false;
                }
                let (sx, sy) = c.to_screen(b.x, b.y);
                let (w, h) = (b.w * c.s, b.h * c.s);
                let (top, side) = (BLOCK_REACH_TOP * c.s, BLOCK_REACH_SIDE * c.s);
                !(sx + w + side <= 0.0 || sy + h + side <= 0.0 || sx - side >= vw || sy - top >= vh)
            })
            .collect()
    }

    /// Selects the blocks a marquee, in world coordinates, touches.
    pub fn select_in(&mut self, x: f32, y: f32, w: f32, h: f32) {
        self.sel.clear();
        if w * self.cam.s < 3.0 && h * self.cam.s < 3.0 {
            return; // a press alone clears the selection
        }
        let hidden = self.stacked_ids();
        for b in &self.blocks {
            if !hidden.contains(&b.id) && b.x < x + w && b.x + b.w > x && b.y < y + h && b.y + b.h > y {
                self.sel.insert(b.id.clone());
            }
        }
    }

    // ---- stacks ----

    /// Every block ID a stack holds.
    pub fn stacked_ids(&self) -> HashSet<String> {
        self.blocks
            .iter()
            .filter(|b| b.kind == BlockKind::Stack)
            .flat_map(|b| b.items.iter().cloned())
            .collect()
    }

    /// Puts a block into a stack, last, taking it from any other stack it
    /// was in, as one undoable edit. A stack never holds a stack.
    pub fn stack_add(&mut self, stack: &str, id: &str) -> bool {
        let Some(st) = self.block(stack) else { return false };
        let Some(b) = self.block(id) else { return false };
        if st.kind != BlockKind::Stack || b.kind == BlockKind::Stack || stack == id {
            return false;
        }
        if st.items.iter().any(|i| i == id) {
            self.sel = HashSet::from([stack.to_string()]);
            return false;
        }
        let mut edits: Vec<StackEdit> = self
            .blocks
            .iter()
            .filter(|o| o.kind == BlockKind::Stack && o.id != stack && o.items.iter().any(|i| i == id))
            .map(|o| StackEdit {
                stack: o.id.clone(),
                before: o.items.clone(),
                after: o.items.iter().filter(|i| *i != id).cloned().collect(),
            })
            .collect();
        let before = st.items.clone();
        let mut after = before.clone();
        after.push(id.to_string());
        edits.push(StackEdit { stack: stack.to_string(), before, after });
        self.forget_redo();
        self.restore_stack_edits(&edits, true);
        self.undo.push(Change::Stack(edits));
        self.sel = HashSet::from([stack.to_string()]);
        true
    }

    /// Takes a block out of its stack onto the canvas at (x, y), selected:
    /// the membership change and where it lands undo as one.
    pub fn stack_take(&mut self, stack: &str, id: &str, x: f32, y: f32) -> bool {
        let Some(st) = self.block(stack) else { return false };
        if !st.items.iter().any(|i| i == id) {
            return false;
        }
        let Some(g) = self.block(id).map(Geometry::of) else { return false };
        let edit = StackEdit {
            stack: stack.to_string(),
            before: st.items.clone(),
            after: st.items.iter().filter(|i| *i != id).cloned().collect(),
        };
        self.forget_redo();
        self.restore_stack_edits(std::slice::from_ref(&edit), true);
        let b = self.block_mut(id).expect("checked above");
        b.x = x;
        b.y = y;
        let after = Geometry::of(b);
        // One undo: the move and the stack edit go back together.
        self.undo.push(Change::Group(vec![
            Change::Stack(vec![edit]),
            Change::Moves(vec![MoveChange { id: id.to_string(), before: g, after }]),
        ]));
        self.raise(id);
        self.sel = HashSet::from([id.to_string()]);
        true
    }

    fn restore_stack_edits(&mut self, edits: &[StackEdit], after: bool) {
        for e in edits {
            if let Some(st) = self.block_mut(&e.stack) {
                st.items = if after { e.after.clone() } else { e.before.clone() };
            }
        }
    }

    /// Drops stack items whose block is gone, duplicates, stacks and itself.
    pub fn prune_stacks(&mut self) {
        let ids: HashMap<String, BlockKind> = self.blocks.iter().map(|b| (b.id.clone(), b.kind)).collect();
        for st in self.blocks.iter_mut().filter(|b| b.kind == BlockKind::Stack) {
            let mut seen = HashSet::new();
            let me = st.id.clone();
            st.items.retain(|id| {
                seen.insert(id.clone()) && *id != me && ids.get(id).is_some_and(|k| *k != BlockKind::Stack)
            });
        }
    }

    // ---- QuickLook lifespans ----

    /// Removes the QuickLook blocks whose time is up (not `editing`), and
    /// returns the ms until something of theirs changes on screen, or None.
    pub fn expire_notes(&mut self, editing: Option<&str>) -> (bool, Option<i64>) {
        let now = self.now_ms();
        let mut gone = false;
        let mut wake: Option<i64> = None;
        let mut soonest = |w: i64| {
            let w = w.max(1000);
            wake = Some(wake.map_or(w, |v| v.min(w)));
        };
        for i in (0..self.blocks.len()).rev() {
            let b = &self.blocks[i];
            if b.kind != BlockKind::Note || b.expires <= 0 {
                continue;
            }
            let left = b.expires - now;
            if left <= 0 {
                if editing == Some(b.id.as_str()) {
                    continue;
                }
                let id = b.id.clone();
                self.remove_block(&id);
                gone = true;
                continue;
            }
            soonest(left);
            if left <= crate::lifespan::NOTE_FADE_MS {
                soonest(3000);
            } else {
                soonest(left % 60_000);
            }
        }
        (gone, wake)
    }

    /// Drops QuickLook blocks that ran out while the deck was closed: not an edit.
    pub fn drop_expired(&mut self) {
        let now = self.now_ms();
        self.blocks.retain(|b| !(b.kind == BlockKind::Note && b.expires > 0 && b.expires <= now));
    }

    /// Gives a QuickLook block an undo brought back a little more time.
    fn rescue_note(&mut self, id: &str) {
        let now = self.now_ms();
        if let Some(b) = self.block_mut(id)
            && b.kind == BlockKind::Note
            && b.expires > 0
            && b.expires - now < crate::lifespan::NOTE_RESCUE_MS
        {
            b.expires = now + crate::lifespan::NOTE_RESCUE_MS;
        }
    }

}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::Path;

    use super::*;

    fn deck() -> Model {
        let mut m = Model::new("/tmp/deck-root");
        m.clock = Some(1_000_000);
        for (i, t) in ["a", "b"].into_iter().enumerate() {
            let mut b = Block::new(BlockKind::Note);
            b.title = t.into();
            b.x = i as f32 * 400.0;
            b.y = 0.0;
            m.add_block(b);
        }
        m.undo.clear();
        m
    }

    #[test]
    fn zoom_keeps_the_point_under_the_pointer() {
        let mut m = deck();
        let before = m.cam.to_world(300.0, 200.0);
        m.zoom_at(300.0, 200.0, 1.7);
        let after = m.cam.to_world(300.0, 200.0);
        assert!((before.0 - after.0).abs() < 1e-3 && (before.1 - after.1).abs() < 1e-3);
        assert!((m.cam.s - 1.7).abs() < 1e-6);
    }

    #[test]
    fn zoom_is_clamped_and_steps_land_on_levels() {
        let mut m = deck();
        m.zoom_at(0.0, 0.0, 1000.0);
        assert_eq!(m.cam.s, MAX_SCALE);
        m.cam.s = 1.0;
        m.zoom_step(0.0, 0.0, 1);
        assert!((m.cam.s - 2f32.powf(0.25)).abs() < 1e-5);
        m.zoom_step(0.0, 0.0, -1);
        assert_eq!(m.cam.s, 1.0);
        assert_eq!(zoom_level(1.02), 1.0);
        assert!(zoom_level_at_most(0.99) < 1.0);
    }

    #[test]
    fn add_remove_undo_redo() {
        let mut m = deck();
        let id = m.add_block(Block::new(BlockKind::Terminal));
        assert_eq!(m.blocks.len(), 3);
        assert!(m.sel.contains(&id));
        m.undo_last();
        assert_eq!(m.blocks.len(), 2);
        assert!(m.hung_up.iter().any(|b| b.id == id));
        m.redo_last();
        assert_eq!(m.blocks.len(), 3);
        m.sel = HashSet::from([m.blocks[0].id.clone()]);
        let first = m.blocks[0].id.clone();
        m.delete_selected();
        assert!(m.index_of(&first).is_none());
        m.undo_last();
        assert_eq!(m.index_of(&first), Some(0), "an undone removal goes back where it was");
    }

    #[test]
    fn a_new_edit_forgets_redo() {
        let mut m = deck();
        let id = m.add_block(Block::new(BlockKind::Note));
        m.undo_last();
        m.add_block(Block::new(BlockKind::Note));
        assert!(m.redo.is_empty());
        assert!(m.index_of(&id).is_none());
    }

    #[test]
    fn moves_undo_as_one() {
        let mut m = deck();
        let before: HashMap<_, _> = m.blocks.iter().map(|b| (b.id.clone(), Geometry::of(b))).collect();
        for b in &mut m.blocks {
            b.x += 50.0;
        }
        assert_eq!(m.record_moves(&before), 2);
        m.undo_last();
        assert_eq!(m.blocks[1].x, 400.0);
        m.redo_last();
        assert_eq!(m.blocks[1].x, 450.0);
    }

    #[test]
    fn visible_skips_blocks_out_of_view_and_stacked() {
        let mut m = deck();
        assert_eq!(m.visible(300.0, 300.0), vec![0]);
        assert_eq!(m.visible(1000.0, 1000.0), vec![0, 1]);
        let st = m.add_block(Block::new(BlockKind::Stack));
        let a = m.blocks[0].id.clone();
        assert!(m.stack_add(&st, &a));
        assert!(!m.visible(1000.0, 1000.0).contains(&0));
        m.undo_last();
        assert!(m.visible(1000.0, 1000.0).contains(&0));
    }

    #[test]
    fn stacks_take_blocks_from_each_other_and_give_them_back() {
        let mut m = deck();
        let (a, b) = (m.blocks[0].id.clone(), m.blocks[1].id.clone());
        let s1 = m.add_block(Block::new(BlockKind::Stack));
        let s2 = m.add_block(Block::new(BlockKind::Stack));
        assert!(m.stack_add(&s1, &a));
        assert!(m.stack_add(&s1, &b));
        assert_eq!(m.block(&s1).unwrap().items, vec![a.clone(), b.clone()]);
        assert!(!m.stack_add(&s1, &s2), "no stack in a stack");
        assert!(m.stack_add(&s2, &a));
        assert_eq!(m.block(&s1).unwrap().items, vec![b.clone()]);
        assert!(m.stack_take(&s2, &a, 900.0, 900.0));
        assert_eq!((m.block(&a).unwrap().x, m.stacked_ids().contains(&a)), (900.0, false));
        m.undo_last();
        assert!(m.block(&s2).unwrap().items.contains(&a));
        assert_eq!(m.block(&a).unwrap().x, 0.0, "one undo puts it back in, where it was");
        m.redo_last();
        assert_eq!(m.block(&a).unwrap().x, 900.0);
    }

    #[test]
    fn marquee_selects_what_it_touches() {
        let mut m = deck();
        m.select_in(-10.0, -10.0, 50.0, 50.0);
        assert_eq!(m.sel.len(), 1);
        m.select_in(0.0, 0.0, 1.0, 1.0);
        assert!(m.sel.is_empty(), "a click is no marquee");
    }

    #[test]
    fn fit_all_frames_every_block() {
        let mut m = deck();
        m.fit_all(1280.0, 800.0);
        for b in &m.blocks {
            let (x0, y0) = m.cam.to_screen(b.x, b.y);
            let (x1, y1) = m.cam.to_screen(b.x + b.w, b.y + b.h);
            assert!(x0 >= 0.0 && y0 >= 0.0 && x1 <= 1280.0 && y1 <= 800.0);
        }
        assert_eq!(m.cam.s, zoom_level(m.cam.s));
    }

    #[test]
    fn expired_notes_go_and_undo_rescues_them() {
        let mut m = deck();
        m.blocks[0].expires = 1_000_000 + 5_000;
        let id = m.blocks[0].id.clone();
        m.clock = Some(1_000_000 + 6_000);
        let (gone, _) = m.expire_notes(None);
        assert!(gone && m.index_of(&id).is_none());
        m.undo_last();
        let b = m.block(&id).unwrap();
        assert_eq!(b.expires, 1_006_000 + crate::lifespan::NOTE_RESCUE_MS);
    }

    #[test]
    fn link_path_is_relative_inside_the_root() {
        let m = Model::new("/a/b");
        assert_eq!(m.link_path(Path::new("/a/b/c/d.txt")), "c/d.txt");
        assert_eq!(m.link_path(Path::new("/a/x.txt")), "/a/x.txt");
        assert_eq!(m.path_of("c/d.txt"), PathBuf::from("/a/b/c/d.txt"));
    }

    #[test]
    fn deck_json_round_trips_and_reads_the_go_format() {
        let dir = tempfile::tempdir().unwrap();
        let go = r#"{
  "version": 1,
  "root": "/elsewhere",
  "camX": -12.5,
  "camY": 40,
  "camS": 0.75,
  "blocks": [
    {"id": "b000001", "kind": "", "title": "旧", "x": 1, "y": 2, "w": 300, "h": 200},
    {"id": "b000002", "kind": "terminal", "title": "终端", "x": 0, "y": 0, "w": 560, "h": 340,
     "term": {"history": "aGVsbG8=", "dir": "src", "savedAt": "2026-11-07T10:00:00.5+08:00"}},
    {"id": "b000003", "kind": "stack", "title": "块堆", "x": 0, "y": 0, "w": 380, "h": 300,
     "items": ["b000001", "gone", "b000001"]},
    {"id": "b000004", "kind": "chart", "title": "图表", "x": 0, "y": 0, "w": 480, "h": 320,
     "chart": "pie", "text": "a,1", "url": "https://x", "page": 3}
  ]
}"#;
        let file = dir.path().join(DECK_DIR).join("deck.json");
        fs::create_dir_all(file.parent().unwrap()).unwrap();
        fs::write(&file, go).unwrap();
        let mut m = Model::new(dir.path());
        m.load().unwrap();
        assert_eq!(m.root, dir.path(), "the folder is the root, wherever it moved");
        assert_eq!(m.cam, Camera { x: -12.5, y: 40.0, s: 0.75 });
        assert_eq!(m.blocks[0].kind, BlockKind::Note);
        assert_eq!(m.blocks[1].kind, BlockKind::Terminal);
        assert_eq!(m.blocks[2].items, vec!["b000001"]);
        assert_eq!(m.blocks[3].page, 3);
        m.save().unwrap();
        let v: serde_json::Value = serde_json::from_slice(&fs::read(&file).unwrap()).unwrap();
        assert_eq!(v["blocks"][0]["kind"], "note");
        assert!(v["blocks"][0].get("path").is_none(), "empties are left out");
        assert!(v["blocks"][1].get("term").is_none(), "a terminal's old state is not kept");
        assert_eq!(v["blocks"][3]["chart"], "pie");
        let mut again = Model::new(dir.path());
        again.load().unwrap();
        assert_eq!(again.blocks, m.blocks);
    }

    #[test]
    fn blocks_of_unknown_kinds_are_kept_as_they_were() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join(DECK_DIR).join("deck.json");
        fs::create_dir_all(file.parent().unwrap()).unwrap();
        let later = r#"{"version": 1, "blocks": [
            {"id": "b000001", "kind": "note", "title": "便签", "x": 0, "y": 0, "w": 200, "h": 80, "text": "你好"},
            {"id": "b000002", "kind": "hologram", "x": 5, "y": 5, "w": 10, "h": 10, "glow": [1, 2, 3]}
        ]}"#;
        fs::write(&file, later).unwrap();
        let mut m = Model::new(dir.path());
        m.load().unwrap();
        assert_eq!(m.blocks.len(), 1);
        assert_eq!((m.blocks[0].kind, m.blocks[0].text.as_str()), (BlockKind::Note, "你好"));
        assert_eq!(m.foreign.len(), 1);
        m.save().unwrap();
        let v: serde_json::Value = serde_json::from_slice(&fs::read(&file).unwrap()).unwrap();
        assert_eq!(v["blocks"][1]["kind"], "hologram");
        assert_eq!(v["blocks"][1]["glow"], serde_json::json!([1, 2, 3]), "kept as it was");
        assert_eq!(v["blocks"][0]["text"], "你好");
        assert_ne!(m.new_id(), "b000002");
    }

    #[test]
    fn a_deck_that_cannot_be_read_is_not_written_over() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join(DECK_DIR).join("deck.json");
        fs::create_dir_all(file.parent().unwrap()).unwrap();
        fs::write(&file, "{ not json").unwrap();
        let mut m = Model::new(dir.path());
        assert!(m.load().is_err());
        assert!(m.blocks.is_empty(), "nothing seeded over it");
        assert_eq!(fs::read_to_string(&file).unwrap(), "{ not json");
    }

    #[test]
    fn a_folder_without_a_deck_is_seeded() {
        let dir = tempfile::tempdir().unwrap();
        let mut m = Model::new(dir.path());
        m.load().unwrap();
        assert_eq!(m.blocks.len(), 1);
        assert!(m.sel.is_empty());
    }

    #[test]
    fn scale_keeping_ratio_follows_the_diagonal() {
        let (w, h) = scale_keeping_ratio(400.0, 200.0, 400.0, 200.0);
        assert_eq!((w, h), (800.0, 400.0));
        let (w, h) = scale_keeping_ratio(400.0, 200.0, -1000.0, -1000.0);
        assert!(w >= MIN_BLOCK_W && h >= MIN_BLOCK_H);
        assert!((w / h - 2.0).abs() < 1e-4);
    }

    #[test]
    fn browser_tabs_open_close_and_follow_url() {
        let mut b = Block::new(BlockKind::Browser);
        b.url = "https://a".into();
        b.ensure_tabs();
        assert_eq!(b.tabs.len(), 1);
        assert!(serde_json::to_value(&b).unwrap().get("tabs").is_none(), "one tab is just the url");

        b.open_tab("https://b".into(), false);
        assert_eq!((b.tab, b.url.as_str()), (0, "https://a"), "a tab opened behind stays shut");
        let key_c = b.open_tab("https://c".into(), true);
        assert_eq!((b.tab, b.url.as_str()), (1, "https://c"), "c goes right after the open tab");
        assert_eq!(b.tabs.iter().map(|t| t.url.as_str()).collect::<Vec<_>>(), ["https://a", "https://c", "https://b"]);
        assert_eq!(b.tab_of(key_c), Some(1));

        // An agent or the Go build changes url alone: the open tab follows.
        b.url = "https://c2".into();
        b.ensure_tabs();
        assert_eq!(b.tabs[1].url, "https://c2");

        // Closing the open tab opens the one after it; closing the last opens the one before.
        b.close_tab(1);
        assert_eq!((b.tab, b.url.as_str()), (1, "https://b"));
        b.close_tab(1);
        assert_eq!((b.tab, b.url.as_str()), (0, "https://a"));
        assert!(b.close_tab(0).is_none(), "the last tab stays");

        let v = serde_json::to_value({
            let mut b2 = b.clone();
            b2.open_tab("https://d".into(), true);
            b2
        })
        .unwrap();
        assert_eq!(v["tab"], 1);
        assert_eq!(v["tabs"][1]["url"], "https://d");
        assert_eq!(v["url"], "https://d", "the Go build still sees the open page");
    }

    #[test]
    fn on_level_recognizes_discrete_zoom_steps() {
        for step in -24..=24 {
            let s = zoom_level_of(f64::from(step));
            assert!(on_level(s), "step {step} ({s}) should be on a level");
        }
        assert!(!on_level(1.03));
        assert!(!on_level(0.75));
    }
}
