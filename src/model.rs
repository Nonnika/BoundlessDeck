//! The deck's model: its blocks, the camera, the selection, undo and redo,
//! and deck.json. Nothing here draws or needs a window, so it is tested
//! headless; `deck.rs` is the view over it.
//!
//! deck.json is the same file the Go build reads and writes: field names
//! and omitted empties match, so either build opens a folder the other saved.
//! What only this build keeps (a browser's tabs) the Go build leaves alone;
//! `url` stays the page the Go build shows.

use std::collections::{HashMap, HashSet};
use std::fs;
use std::io::Write as _;
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use serde::{Deserialize, Deserializer, Serialize, Serializer};


/// The folder inside a deck's folder that holds its canvas.
pub const DECK_DIR: &str = ".boundlessdeck";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub enum BlockKind {
    /// QuickLook: a sticky note whose text lives in deck.json, and passes.
    #[default]
    Note,
    /// 笔记: a Markdown file of the deck's folder.
    Memo,
    File,
    Terminal,
    Browser,
    Chart,
    Stack,
    /// Words on the canvas, styled; no title shows until it is selected.
    Text,
    /// A rectangle, ellipse, diamond or triangle, styled, maybe with words in it.
    Shape,
}

impl BlockKind {
    /// Every kind, in the order the toolbar lists them.
    pub const ALL: [BlockKind; 9] = [
        BlockKind::Note,
        BlockKind::Memo,
        BlockKind::File,
        BlockKind::Terminal,
        BlockKind::Browser,
        BlockKind::Chart,
        BlockKind::Stack,
        BlockKind::Text,
        BlockKind::Shape,
    ];

    /// Kinds drawn as they are, without a card: their title shows only while selected.
    pub fn bare(self) -> bool {
        matches!(self, BlockKind::Text | BlockKind::Shape)
    }

    pub fn as_str(self) -> &'static str {
        match self {
            BlockKind::Note => "note",
            BlockKind::Memo => "memo",
            BlockKind::File => "file",
            BlockKind::Terminal => "terminal",
            BlockKind::Browser => "browser",
            BlockKind::Chart => "chart",
            BlockKind::Stack => "stack",
            BlockKind::Text => "text",
            BlockKind::Shape => "shape",
        }
    }

    pub fn parse(s: &str) -> Option<BlockKind> {
        Self::ALL.into_iter().find(|k| k.as_str() == s)
    }

    /// The kind's name over MCP. There 笔记 is "note" and QuickLook
    /// "quicklook"; deck.json keeps `as_str`, which the Go version reads.
    pub fn mcp_name(self) -> &'static str {
        match self {
            BlockKind::Note => "quicklook",
            BlockKind::Memo => "note",
            k => k.as_str(),
        }
    }

    /// A kind named over MCP; "memo", its old name, still means 笔记.
    pub fn parse_mcp(s: &str) -> Option<BlockKind> {
        if s == "memo" {
            return Some(BlockKind::Memo);
        }
        Self::ALL.into_iter().find(|k| k.mcp_name() == s)
    }

    /// The kind's name as the UI shows it.
    pub fn label(self) -> &'static str {
        match self {
            BlockKind::Note => "QuickLook",
            BlockKind::Memo => "笔记",
            BlockKind::File => "文件",
            BlockKind::Terminal => "终端",
            BlockKind::Browser => "浏览器",
            BlockKind::Chart => "图表",
            BlockKind::Stack => "块堆",
            BlockKind::Text => "文字",
            BlockKind::Shape => "图形",
        }
    }
}

impl Serialize for BlockKind {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for BlockKind {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        // An empty kind is a note, as decks of old saved them.
        if s.is_empty() {
            return Ok(BlockKind::Note);
        }
        BlockKind::parse(&s).ok_or_else(|| serde::de::Error::custom(format!("未知的 block 类型 {s}")))
    }
}

fn is_false(b: &bool) -> bool {
    !*b
}
fn is_zero_i32(v: &i32) -> bool {
    *v == 0
}
fn is_zero_i64(v: &i64) -> bool {
    *v == 0
}
fn is_zero_usize(v: &usize) -> bool {
    *v == 0
}
fn one_or_none(v: &[WebTab]) -> bool {
    v.len() <= 1
}

/// One tab of a browser block.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct WebTab {
    #[serde(default)]
    pub url: String,
    /// The page's title, as it last said.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub title: String,
    /// Tells the tab's page apart while the deck is open; never saved.
    #[serde(skip)]
    pub key: u64,
}

impl PartialEq for WebTab {
    fn eq(&self, o: &WebTab) -> bool {
        self.url == o.url && self.title == o.title
    }
}

impl WebTab {
    pub fn new(url: String) -> WebTab {
        WebTab { url, title: String::new(), key: next_tab_key() }
    }

    /// What its tab says: the page's title, else its address.
    pub fn label(&self) -> String {
        if !self.title.trim().is_empty() {
            self.title.clone()
        } else if !self.url.is_empty() {
            self.url.split("://").nth(1).unwrap_or(&self.url).trim_end_matches('/').to_string()
        } else {
            "新标签页".into()
        }
    }
}

pub(crate) fn next_tab_key() -> u64 {
    static NEXT: AtomicU64 = AtomicU64::new(1);
    NEXT.fetch_add(1, Ordering::Relaxed)
}

/// One card of the canvas. Positions and sizes are in world coordinates;
/// the camera turns them into pixels of the window.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct Block {
    pub id: String,
    #[serde(default)]
    pub kind: BlockKind,
    #[serde(default)]
    pub title: String,
    /// The user named the block: titles a shell gives it no longer replace it.
    #[serde(default, skip_serializing_if = "is_false")]
    pub named: bool,
    #[serde(default)]
    pub x: f32,
    #[serde(default)]
    pub y: f32,
    #[serde(default)]
    pub w: f32,
    #[serde(default)]
    pub h: f32,
    /// The file a file block or 笔记 links to: relative to the root when inside it.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub path: String,
    /// The body of a QuickLook, or the table a chart draws.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub text: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub chart: String,
    /// The page a browser shows: its open tab's.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub url: String,
    /// A browser's tabs, the open one's address being `url`; kept only when there are two or more.
    #[serde(default, skip_serializing_if = "one_or_none")]
    pub tabs: Vec<WebTab>,
    /// Which of `tabs` is open.
    #[serde(default, skip_serializing_if = "is_zero_usize")]
    pub tab: usize,
    #[serde(default, skip_serializing_if = "is_zero_i32")]
    pub page: i32,
    /// When a QuickLook goes by itself, in Unix ms; 0 keeps it.
    #[serde(default, skip_serializing_if = "is_zero_i64")]
    pub expires: i64,
    /// The blocks a 块堆 holds, by ID, the top card first.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub items: Vec<String>,
    /// A 图形's outline: `rect` (the default), `ellipse`, `diamond` or `triangle`.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub shape: String,
    /// How a 文字 or 图形 looks; each part left out takes its kind's default.
    #[serde(default, skip_serializing_if = "Style::is_default")]
    pub style: Style,
}

/// The shapes a 图形 can be.
pub const SHAPES: &[&str] = &["rect", "ellipse", "diamond", "triangle"];

/// How a 文字 or 图形 looks. Colors are a palette name (`theme::INKS`, which
/// follow light and dark), `#rrggbb`, or `none`; empty takes the default.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Style {
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub fill: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub stroke: String,
    /// The words' color.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub color: String,
    /// A font family installed on the system; empty is the system font.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub font: String,
    /// Font size in points at 100%; 0 is the kind's default.
    #[serde(default, skip_serializing_if = "is_zero_f32")]
    pub size: f32,
    /// CSS weight, 100–900; 0 is regular.
    #[serde(default, skip_serializing_if = "is_zero_u16")]
    pub weight: u16,
    #[serde(default, skip_serializing_if = "is_false")]
    pub italic: bool,
    /// Corner radius in points; none is the kind's default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub radius: Option<f32>,
    /// `left`, `center` or `right`; empty is the kind's default.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub align: String,
    /// `top`, `middle` or `bottom`; empty is the kind's default.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub valign: String,
}

impl Style {
    pub fn is_default(&self) -> bool {
        *self == Style::default()
    }
}

fn is_zero_f32(v: &f32) -> bool {
    *v == 0.0
}

fn is_zero_u16(v: &u16) -> bool {
    *v == 0
}

impl Block {
    pub fn new(kind: BlockKind) -> Block {
        Block { kind, ..Default::default() }
    }

    /// The title a block has until it is named.
    pub fn default_title(&self) -> String {
        match self.kind {
            BlockKind::File if !self.path.is_empty() => file_name(&self.path),
            BlockKind::Memo if !self.path.is_empty() => crate::memo::title_of(&self.path),
            BlockKind::Memo => crate::memo::MEMO_TITLE.to_string(),
            k => k.label().to_string(),
        }
    }

    /// Gives the block the title its shell says, unless the user named it.
    pub fn auto_title(&mut self, title: &str) {
        if !self.named && !title.trim().is_empty() {
            self.title = title.to_string();
        }
    }

    /// Makes a browser's tabs whole: one at least, the open one in range and
    /// showing `url` (which wins, as the Go build or an agent may have changed
    /// it alone), each with its key.
    pub fn ensure_tabs(&mut self) {
        if self.tabs.is_empty() {
            self.tabs.push(WebTab::new(self.url.clone()));
        }
        self.tab = self.tab.min(self.tabs.len() - 1);
        self.tabs[self.tab].url = self.url.clone();
        for t in &mut self.tabs {
            if t.key == 0 {
                t.key = next_tab_key();
            }
        }
    }

    /// Opens tab i: its address becomes `url`, its page's title the block's.
    pub fn select_tab(&mut self, i: usize) {
        self.ensure_tabs();
        if i >= self.tabs.len() {
            return;
        }
        self.tab = i;
        self.url = self.tabs[i].url.clone();
        let title = self.tabs[i].title.clone();
        self.auto_title(&title);
    }

    /// Adds a tab after the open one, opening it if asked; its key.
    pub fn open_tab(&mut self, url: String, open: bool) -> u64 {
        self.ensure_tabs();
        let t = WebTab::new(url);
        let key = t.key;
        self.tabs.insert(self.tab + 1, t);
        if open {
            self.select_tab(self.tab + 1);
        }
        key
    }

    /// Closes tab i, unless it is the last; the one beside it opens if it was open.
    pub fn close_tab(&mut self, i: usize) -> Option<WebTab> {
        self.ensure_tabs();
        if self.tabs.len() <= 1 || i >= self.tabs.len() {
            return None;
        }
        let gone = self.tabs.remove(i);
        let open = if i < self.tab || (i == self.tab && i == self.tabs.len()) { self.tab - 1 } else { self.tab };
        // `url` was the closed tab's when it was open: the next one's now.
        self.tab = open;
        self.url = self.tabs[open].url.clone();
        self.select_tab(open);
        Some(gone)
    }

    /// The index of the tab with key, if it is still open.
    pub fn tab_of(&self, key: u64) -> Option<usize> {
        self.tabs.iter().position(|t| t.key == key)
    }

    pub fn contains(&self, x: f32, y: f32) -> bool {
        x >= self.x && x < self.x + self.w && y >= self.y && y < self.y + self.h
    }
}

pub fn file_name(p: &str) -> String {
    Path::new(p)
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| p.to_string())
}

/// Shows the world point (x, y) at the window's top-left corner, s times as large.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Camera {
    pub x: f32,
    pub y: f32,
    pub s: f32,
}

impl Default for Camera {
    fn default() -> Self {
        Camera { x: 0.0, y: 0.0, s: 1.0 }
    }
}

impl Camera {
    pub fn to_screen(&self, wx: f32, wy: f32) -> (f32, f32) {
        ((wx - self.x) * self.s, (wy - self.y) * self.s)
    }
    pub fn to_world(&self, sx: f32, sy: f32) -> (f32, f32) {
        (sx / self.s + self.x, sy / self.s + self.y)
    }
    fn clamp(&mut self) {
        self.s = self.s.clamp(MIN_SCALE, MAX_SCALE);
    }
}

pub const MIN_SCALE: f32 = 0.08;
pub const MAX_SCALE: f32 = 4.0;
/// Below this scale a block shows its frame and title alone.
pub const LOD_SCALE: f32 = 0.3;
/// During a zoom gesture the blocks layer is re-laid out only after the camera crosses this ratio from it.
pub const ZOOM_REBASE: f32 = 0.5;
/// Zoom levels per doubling: the camera rests on one.
pub const ZOOM_LEVELS: f64 = 12.0;
/// How many levels a key press zooms by.
pub const ZOOM_KEY_STEPS: i32 = 3;

/// The zoom level nearest s.
pub fn zoom_level(s: f32) -> f32 {
    zoom_level_of((f64::from(s).log2() * ZOOM_LEVELS).round())
}

/// Whether s is already on a zoom level.
pub fn on_level(s: f32) -> bool {
    (s - zoom_level(s)).abs() <= s * 1e-5
}

/// The largest zoom level not above s.
pub fn zoom_level_at_most(s: f32) -> f32 {
    zoom_level_of((f64::from(s).log2() * ZOOM_LEVELS + 1e-6).floor())
}

/// Level n, the n-th 12th of a doubling from 1, within the zoom's limits.
pub fn zoom_level_of(n: f64) -> f32 {
    let lo = (f64::from(MIN_SCALE).log2() * ZOOM_LEVELS).ceil();
    let hi = (f64::from(MAX_SCALE).log2() * ZOOM_LEVELS).floor();
    (n.clamp(lo, hi) / ZOOM_LEVELS).exp2() as f32
}

/// The factor ⌘ and the wheel zoom by for dy pixels of scrolling.
pub fn wheel_zoom(dy: f32) -> f32 {
    1.0016f64.powf(-f64::from(dy)) as f32
}

/// What of a block shows outside its box at 100%: its shadow around.
pub const BLOCK_REACH_TOP: f32 = 12.0;
pub const BLOCK_REACH_SIDE: f32 = 10.0;

/// The least a block can be resized to, in world units.
pub const MIN_BLOCK_W: f32 = 140.0;
pub const MIN_BLOCK_H: f32 = 90.0;

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

    // ---- paths ----

    /// A block's path, resolved against the root.
    pub fn path_of(&self, p: &str) -> PathBuf {
        let p = Path::new(p);
        if p.is_absolute() { p.to_path_buf() } else { self.root.join(p) }
    }

    /// p relative to the root when it is inside it, so the folder can move.
    pub fn link_path(&self, p: &Path) -> String {
        match p.strip_prefix(&self.root) {
            Ok(rel) if !rel.as_os_str().is_empty() && !rel.components().any(|c| c == Component::ParentDir) => {
                rel.to_string_lossy().into_owned()
            }
            _ => p.to_string_lossy().into_owned(),
        }
    }

    // ---- deck.json ----

    pub fn deck_file(&self) -> Option<PathBuf> {
        if let Some(f) = &self.file {
            return Some(f.clone());
        }
        if self.root.as_os_str().is_empty() {
            return None;
        }
        Some(self.root.join(DECK_DIR).join("deck.json"))
    }

    /// Reads the deck saved in its folder; a folder with none gets a new deck.
    /// A deck.json there that cannot be read is an error, and nothing is
    /// loaded, so saving does not write over it. Blocks in it this build does
    /// not know are kept aside (`foreign`) and saved back.
    pub fn load(&mut self) -> Result<(), String> {
        let Some(file) = self.deck_file() else {
            self.seed();
            return Ok(());
        };
        let data = match fs::read(&file) {
            Ok(d) => d,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                self.seed();
                return Ok(());
            }
            Err(e) => return Err(e.to_string()),
        };
        let ws: RawWorkspace = serde_json::from_slice(&data).map_err(|e| e.to_string())?;
        if self.root.as_os_str().is_empty() {
            self.root = PathBuf::from(ws.root);
        }
        let mut blocks = Vec::new();
        for v in ws.blocks {
            match serde_json::from_value::<Block>(v.clone()) {
                Ok(b) => blocks.push(b),
                Err(_) => self.foreign.push(v),
            }
        }
        self.blocks = blocks;
        // The Go build may have changed a browser's url alone: its open tab follows.
        for b in self.blocks.iter_mut().filter(|b| b.kind == BlockKind::Browser) {
            b.ensure_tabs();
        }
        self.cam = Camera { x: ws.cam_x, y: ws.cam_y, s: if ws.cam_s == 0.0 { 1.0 } else { ws.cam_s } };
        self.drop_expired();
        self.prune_stacks();
        Ok(())
    }

    pub fn snapshot(&self) -> Vec<u8> {
        let ws = WorkspaceRef {
            version: 1,
            root: self.root.to_string_lossy().into_owned(),
            cam_x: self.cam.x,
            cam_y: self.cam.y,
            cam_s: self.cam.s,
            blocks: (&self.blocks, &self.foreign),
        };
        serde_json::to_vec_pretty(&ws).unwrap_or_default()
    }

    /// Writes deck.json atomically, here and now.
    pub fn save(&self) -> std::io::Result<()> {
        let Some(path) = self.deck_file() else { return Ok(()) };
        write_file_atomic(&path, &self.snapshot())
    }

    fn seed(&mut self) {
        let mut b = Block::new(BlockKind::Note);
        b.title = "开始".into();
        b.x = 80.0;
        b.y = 80.0;
        b.w = 420.0;
        b.h = 250.0;
        b.text = "- 双指滑动平移，⌘ + 滚轮缩放，⌘0 显示全部\n\
                  - 拖顶部的胶带移动，拖右下角等比缩放\n\
                  - 点胶带、框选或 Shift + 点击来选中，⌘D 删除\n\
                  - 右上角新建 QuickLook、笔记、链接文件、打开终端\n\
                  - 左上角回到主页，画布会保存在这个文件夹里"
            .into();
        self.add_block(b);
        self.sel.clear();
    }
}

#[derive(Serialize)]
struct WorkspaceRef<'a> {
    version: i32,
    root: String,
    #[serde(rename = "camX")]
    cam_x: f32,
    #[serde(rename = "camY")]
    cam_y: f32,
    #[serde(rename = "camS")]
    cam_s: f32,
    #[serde(serialize_with = "blocks_then_foreign")]
    blocks: (&'a [Block], &'a [serde_json::Value]),
}

/// deck.json's blocks: those read, then those kept aside as they were.
fn blocks_then_foreign<S: Serializer>(b: &(&[Block], &[serde_json::Value]), s: S) -> Result<S::Ok, S::Error> {
    use serde::ser::SerializeSeq;
    let mut seq = s.serialize_seq(Some(b.0.len() + b.1.len()))?;
    for x in b.0 {
        seq.serialize_element(x)?;
    }
    for x in b.1 {
        seq.serialize_element(x)?;
    }
    seq.end()
}

/// deck.json as read: its blocks still JSON, each read on its own.
#[derive(Deserialize)]
struct RawWorkspace {
    #[serde(default)]
    root: String,
    #[serde(default, rename = "camX")]
    cam_x: f32,
    #[serde(default, rename = "camY")]
    cam_y: f32,
    #[serde(default, rename = "camS")]
    cam_s: f32,
    #[serde(default)]
    blocks: Vec<serde_json::Value>,
}

/// The app's own data folder (recents, settings), unless
/// `BOUNDLESSDECK_DATA_DIR` puts it elsewhere, as tests and trial runs do.
pub fn data_dir() -> Option<PathBuf> {
    match std::env::var_os("BOUNDLESSDECK_DATA_DIR") {
        Some(d) => Some(PathBuf::from(d)),
        None => dirs::data_dir().map(|d| d.join("BoundlessDeck")),
    }
}

/// Writes data through a temporary file and a rename: a reader never sees half a file.
pub fn write_file_atomic(path: &Path, data: &[u8]) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension(format!(
        "{}.tmp",
        path.extension().map(|e| e.to_string_lossy().into_owned()).unwrap_or_default()
    ));
    let mut f = fs::File::create(&tmp)?;
    f.write_all(data)?;
    f.sync_all()?;
    fs::rename(&tmp, path)
}

/// Scales w×h by how far the corner was dragged, along the block's diagonal,
/// no smaller than the least block size lets, in proportion.
pub fn scale_keeping_ratio(w: f32, h: f32, dx: f32, dy: f32) -> (f32, f32) {
    let k = ((w + dx) * w + (h + dy) * h) / (w * w + h * h);
    let k = k.max(MIN_BLOCK_W / w).max(MIN_BLOCK_H / h);
    (w * k, h * k)
}

/// The size a block linking the file starts at: a page for a PDF or Word, 16:9 for a video, wider for others.
pub fn file_block_size(path: &Path) -> (f32, f32) {
    match path.extension().and_then(|e| e.to_str()).map(str::to_ascii_lowercase).as_deref() {
        Some("pdf" | "docx" | "doc") => (440.0, 580.0),
        // 16:9 under the title bar.
        Some("mp4" | "m4v" | "mov" | "webm") => (640.0, 386.0),
        _ => (560.0, 420.0),
    }
}

/// A new block of the kind, at its default size, with its default title.
pub fn new_block_of(kind: BlockKind, now_ms: i64) -> Block {
    let mut b = Block::new(kind);
    b.title = kind.label().to_string();
    let (w, h) = match kind {
        BlockKind::Note => {
            b.expires = now_ms + crate::lifespan::NOTE_LIFE_MS;
            (360.0, 260.0)
        }
        BlockKind::Memo => (420.0, 320.0),
        BlockKind::File => (560.0, 420.0),
        BlockKind::Terminal => (560.0, 340.0),
        BlockKind::Browser => (760.0, 540.0),
        BlockKind::Stack => (380.0, 300.0),
        BlockKind::Chart => {
            b.chart = "bar".into();
            b.text = crate::chart::SAMPLE.into();
            (480.0, 320.0)
        }
        BlockKind::Text => {
            b.text = "双击编辑文字".into();
            (280.0, 90.0)
        }
        BlockKind::Shape => {
            b.shape = "rect".into();
            (220.0, 160.0)
        }
    };
    b.w = w;
    b.h = h;
    b
}

#[cfg(test)]
mod tests {
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
