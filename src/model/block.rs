//! A block as deck.json keeps it: its kind, fields, style and a browser's
//! tabs, and the size each kind starts at.

use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};

use serde::{Deserialize, Deserializer, Serialize, Serializer};

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

pub fn next_tab_key() -> u64 {
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

/// What of a block shows outside its box at 100%: its shadow around.
pub const BLOCK_REACH_TOP: f32 = 12.0;
pub const BLOCK_REACH_SIDE: f32 = 10.0;

/// The least a block can be resized to, in world units.
pub const MIN_BLOCK_W: f32 = 140.0;
pub const MIN_BLOCK_H: f32 = 90.0;

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
