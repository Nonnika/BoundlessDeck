//! The deck's view: the canvas, its blocks and the chrome around them.
//!
//! The canvas is laid out with the camera each frame: block X/Y/W/H and
//! font sizes are multiplied by its scale, so text stays crisp at any zoom.
//! Only blocks the window shows some of are built.
//!
//! Input: each card and resize handle takes its own presses and
//! occludes what is under it; a press nowhere else reaches the canvas.
//! Moves, releases and the wheel go through window-wide listeners the grid
//! registers as it paints, so a drag carries on over anything, and a wheel
//! no body took pans or zooms the canvas.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime};

use gpui::{
    AnyElement, App, AppContext, Context, DispatchPhase, Entity, EventEmitter, ExternalPaths, FocusHandle, Focusable,
    Hsla, InteractiveElement, IntoElement, MouseButton, MouseMoveEvent, MouseUpEvent, ParentElement, Render,
    ScrollWheelEvent, SharedString, Styled, Subscription, UniformListScrollHandle, Window, actions, canvas, div, fill,
    point, px, size,
};

mod body;
mod browser;
mod camera;
mod card;
mod chrome;
mod editing;
mod endpoint;
mod focusview;
mod input;
mod inspector;
mod lifecycle;
mod load;
mod memofile;
mod photozoom;
mod snapshot;
mod popin;
mod progress;
mod sharpen;
mod stack;
mod styled;
mod zoomlayer;

use crate::ui::editor::Editor;
use crate::content::markdown;
use crate::content::memo;
use crate::model::{self, Block, BlockKind, Geometry, LOD_SCALE, Model, file_block_size, new_block_of};
use crate::content::terminal::Terminal;
use crate::ui::theme::Theme;

actions!(
    deck,
    [
        FitAll,
        ZoomIn,
        ZoomOut,
        Undo,
        Redo,
        DeleteSelected,
        NewNote,
        NewMemo,
        NewFile,
        NewTerminal,
        NewBrowser,
        NewChart,
        NewStack,
        NewText,
        NewShape,
        GoHome,
        ToggleFocusView,
        KeepFocus,
        PlayFocus,
    ]
);

/// The height of a block's title bar at 100%.
const TITLE_H: f32 = 26.0;
const TITLE_FONT: f32 = 12.0;
/// The size of a code file's text at 100%.
const EDITOR_FONT: f32 = 12.0;
/// The size of a document's text at 100%.
const DOC_FONT: f32 = 14.0;
/// The band along the window's top whose empty canvas drags the window.
const DRAG_BAND: f32 = 38.0;
/// Where top-left chrome starts, clear of the traffic lights.
const CHROME_LEFT: f32 = 84.0;
const CHROME_TOP: f32 = 8.0;
/// How long after its last move a scroll the canvas took is still the canvas's.
const GESTURE_HOLD: Duration = Duration::from_millis(300);
/// How long one frame may spend bringing blocks back from LOD to full content:
/// zooming in past `LOD_SCALE`, the rest wait under their mask, so the layout,
/// shaping and starting of them all is spread out instead of one stall. One
/// block always gets through, so a frame full of slow ones still makes progress.
const LOD_WAKE_BUDGET: Duration = Duration::from_millis(6);
/// How long after an edit asks for a save deck.json is written (`request_save`).
const SAVE_IDLE: Duration = Duration::from_millis(250);

/// A frame's allowance for blocks leaving LOD: about `LOD_WAKE_BUDGET` of work,
/// at least one block. `draw` begins it before building the blocks that wait,
/// `block_view` asks, and `draw` ends it once they are built.
#[derive(Default)]
struct LodWake {
    /// When this frame's waking budget runs out; set while a block waits.
    until: Option<Instant>,
    /// A block has left LOD this frame.
    woke: bool,
}

impl LodWake {
    /// Starts a frame: the budget runs from now if a block waits.
    fn begin(&mut self, waiting: bool, now: Instant) {
        self.until = waiting.then(|| now + LOD_WAKE_BUDGET);
        self.woke = false;
    }

    /// Whether the next block may leave LOD now: one always may, then until the
    /// frame's budget is spent.
    fn may(&mut self, now: Instant) -> bool {
        if self.woke {
            return self.until.is_some_and(|until| now < until);
        }
        self.woke = true;
        true
    }

    /// The frame is done with it.
    fn end(&mut self) {
        self.until = None;
        self.woke = false;
    }
}

/// What the deck asks of the app.
pub enum DeckEvent {
    Home,
    Open(String),
    Pick,
}

#[derive(Clone, Copy, PartialEq)]
enum EditPart {
    Title,
    Body,
    /// A browser's address bar.
    Address,
}

struct Editing {
    id: String,
    part: EditPart,
    editor: Entity<Editor>,
    from_title: String,
    _sub: Subscription,
}

enum Drag {
    Pan { from: (f32, f32), cam: model::Camera },
    Marquee { from: (f32, f32), to: (f32, f32) },
    Move { id: String, off: (f32, f32), from: (f32, f32), moved: bool, before: HashMap<String, Geometry> },
    Resize { id: String, from: (f32, f32), start: (f32, f32), before: Geometry },
    /// A stack's card pressed, maybe plucked out (`DeckView::pluck`).
    Pluck,
}

#[derive(Clone)]
enum Popup {
    Workspaces,
    Life { id: String, at: (f32, f32) },
    ChartType { id: String, at: (f32, f32) },
}

/// Whether a marquee from `from` to `to` (world) has grown past a click and
/// lies over r (window points).
fn marquee_over(cam: model::Camera, from: (f32, f32), to: (f32, f32), r: (f32, f32, f32, f32)) -> bool {
    let (ax, ay) = cam.to_screen(from.0, from.1);
    let (bx, by) = cam.to_screen(to.0, to.1);
    let (x0, x1, y0, y1) = (ax.min(bx), ax.max(bx), ay.min(by), ay.max(by));
    (x1 - x0 > 3. || y1 - y0 > 3.) && x0 < r.0 + r.2 && x1 > r.0 && y0 < r.1 + r.3 && y1 > r.1
}

/// Finds a Markdown picture's source by the address it is written with.
type ImageFinder = dyn Fn(&str) -> Option<gpui::ImageSource>;

/// A block's state between frames: never saved.
#[derive(Default)]
struct Runtime {
    /// A text file's lines, read once.
    lines: Option<Result<Arc<Vec<SharedString>>, String>>,
    /// A Markdown file's text, read once.
    doc: Option<Result<Arc<String>, String>>,
    loading: bool,
    /// A Word document read as Markdown, its pictures by name.
    word: Option<Result<Arc<WordDoc>, String>>,
    /// A PDF open for its preview, and its page as last rendered: page, width, image.
    pdf: Option<Result<Arc<crate::content::pdf::Doc>, String>>,
    pdf_img: Option<(usize, usize, Arc<gpui::RenderImage>)>,
    /// The page as rendered before, under the new one while that fades in.
    pdf_prev: Option<Arc<gpui::RenderImage>>,
    pdf_rendering: bool,
    /// Parsed Markdown AST cached across frames while text and palette are unchanged.
    md_cache: Option<(String, markdown::Parsed)>,
    /// The list a Markdown body is shown in, laying out only the blocks in
    /// sight, and the parse it lists.
    md_list: Option<(gpui::ListState, markdown::Parsed)>,
    /// Parsed Markdown AST for a stack card's snippet face.
    face_md_cache: Option<(String, markdown::Parsed)>,
    /// Parsed chart table cached across frames while `b.text` is unchanged.
    chart_cache: Option<(String, Arc<crate::content::chart::Data>)>,
    list: UniformListScrollHandle,
    term: Option<Entity<Terminal>>,
    term_err: Option<String>,
    _term_sub: Option<Subscription>,
    memo: MemoRt,
    stack: stack::StackRt,
    /// A browser's pages, one for each tab opened so far, by the tab's key.
    webs: HashMap<u64, browser::WebRt>,
    web_err: Option<String>,
    /// A video file's player: its page's key, and the path it plays.
    video: Option<(u64, String)>,
    /// Where the block is in its lifecycle (`lifecycle`).
    life: lifecycle::Life,
    /// The block was last drawn zoomed out, as its frame and title alone.
    lod: bool,
    /// Drawn live until then, not as its picture: scrolled while playing (`snapshot`).
    live_until: Option<Instant>,
    /// The body fading in on coming back from LOD.
    appear: sharpen::Sharpen,
    /// A sharper version of the content (a bitmap, a native view) fading in over the old.
    sharpen: sharpen::Sharpen,
    /// The entrance of a block just made.
    pop: popin::PopIn,
    /// Since when the block has been off screen: long enough, and what it
    /// read is let go (`sweep_hidden`).
    hidden_since: Option<Instant>,
}

impl Runtime {
    /// Lets go of what was read from the block's file and what was made of it
    /// (pages, parses): it is read again when it shows. A list's scroll stays.
    fn forget_file(&mut self) {
        self.lines = None;
        self.doc = None;
        self.word = None;
        self.pdf = None;
        self.pdf_img = None;
        self.pdf_prev = None;
        self.md_cache = None;
        self.face_md_cache = None;
    }
}

struct WordDoc {
    markdown: String,
    media: HashMap<String, Arc<gpui::Image>>,
}

fn image_format(name: &str) -> gpui::ImageFormat {
    match name.rsplit('.').next().map(str::to_ascii_lowercase).as_deref() {
        Some("jpg" | "jpeg") => gpui::ImageFormat::Jpeg,
        Some("gif") => gpui::ImageFormat::Gif,
        Some("webp") => gpui::ImageFormat::Webp,
        Some("bmp") => gpui::ImageFormat::Bmp,
        Some("svg") => gpui::ImageFormat::Svg,
        Some("tif" | "tiff") => gpui::ImageFormat::Tiff,
        _ => gpui::ImageFormat::Png,
    }
}

/// Where a block ranks when blocks come back from LOD: nearest the middle of
/// the window first, then higher up, so a zoom-in surfaces from its focus out.
fn wake_rank(cam: model::Camera, b: &Block, (cw, ch): (f32, f32)) -> (f32, f32) {
    let (mx, my) = (cw / 2., ch / 2.);
    let (x, y) = cam.to_screen(b.x + b.w / 2., b.y + b.h / 2.);
    ((x - mx).powi(2) + (y - my).powi(2), y)
}

/// What shows around a PDF page, as a reader's gray desk.
fn pdf_backdrop(t: &Theme) -> Hsla {
    if t.dark { gpui::rgb(0x2a2b2f).into() } else { gpui::rgb(0xececef).into() }
}

#[derive(Default)]
struct MemoRt {
    text: String,
    saved: String,
    loaded: bool,
    err: Option<String>,
    save_err: Option<String>,
    modified: Option<SystemTime>,
    checked: Option<Instant>,
    edited: Option<Instant>,
    saving: bool,
}

pub struct DeckView {
    pub m: Model,
    /// The camera rasters (PDF pages) are made at; held while a zoom is under way (`zoomlayer`).
    pub lay: model::Camera,
    /// Until when a wheel/pinch zoom is considered in-flight.
    pub zoom_settle: Option<Instant>,
    zoom_timer_active: bool,
    /// This frame's allowance for blocks leaving LOD (`LodWake`).
    lod_wake: LodWake,
    /// Something drawn this frame is fading: the next frame is asked for.
    animating: bool,
    /// A panel of the window's (the welcome) lies over the deck: web pages give way to their pictures.
    pub covered: bool,
    /// Where the system's Fonts and Colors panels send their picks (`nativepanel`).
    panel_tx: smol::channel::Sender<crate::platform::nativepanel::Pick>,
    /// Which color of the styled blocks a custom pick goes to.
    color_slot: inspector::Slot,
    /// The style panel showed this frame: web pages under it give way.
    insp: inspector::Inspector,
    /// Agents' tasks, as `process` tells them: the capsule at the bottom.
    tasks: progress::Tasks,
    /// 焦点视图 and playing 焦点.
    fv: focusview::FocusView,
    /// Zooming as a photo: the picture scaled on the GPU (`photozoom`).
    photo: photozoom::Photo,
    /// How many glides have started: pictures of blocks are of one glide (`snapshot`).
    glides: u64,
    /// The bodies this frame pictures, while playing (`snapshot::plan_captures`).
    captures: snapshot::Captures,
    /// Lists in block bodies lay out only what the window can show (`body_room`).
    clip_bodies: bool,
    /// The window this deck is drawn in: what `screenshot` captures.
    win: Option<gpui::AnyWindowHandle>,
    /// The camera gliding to where an agent asked the view to be: from, to, since.
    glide: Option<camera::Glide>,
    focus: FocusHandle,
    rts: HashMap<String, Runtime>,
    view: (f32, f32),
    drag: Option<Drag>,
    editing: Option<Editing>,
    /// The block worked in: its body takes presses and the wheel.
    active: Option<String>,
    /// The 块堆 a click opened: hovering it pushes its cards out.
    stack_open: Option<String>,
    /// The 块堆 whose long preview shows.
    preview: Option<String>,
    /// The 块堆 whose long preview, closed, still shrinks back into its card.
    preview_leaving: Option<String>,
    /// How far the long preview has grown out of its card, 0 to 1.
    preview_t: stack::Spring,
    preview_frame: Option<Instant>,
    /// Where, in the window, the card the long preview shows lies.
    preview_from: Option<(f32, f32, f32, f32)>,
    pluck: Option<stack::Pluck>,
    /// Where the pointer was last seen, in the window.
    last_pointer: (f32, f32),
    popup: Option<Popup>,
    /// The recent folders, for the workspace menu.
    pub workspaces: Vec<String>,
    canvas_scroll_until: Option<Instant>,
    zoom_anchor: (f32, f32),
    zoom_gen: u64,
    wake_at: Option<Instant>,
    /// An edit asked for a save that has not been written yet (`request_save`).
    save_pending: bool,
    /// The workspace's MCP endpoint while it runs, or why it could not start.
    mcp: Option<crate::mcp::server::Server>,
    mcp_err: Option<String>,
    /// How agents start the bridge to this deck.
    bridge: Option<crate::mcp::BridgeCmd>,
    /// Write the project-level MCP configs into the folder (设置 → 项目级配置).
    pub project_mcp: bool,
    /// What the browser blocks' pages say, from WebKit's callbacks.
    web_tx: smol::channel::Sender<browser::WebEvent>,
    /// The pages this frame showed; the others are hidden at its end.
    /// The tabs whose pages this frame showed, by key.
    web_shown: HashSet<u64>,
    /// Where this frame lays the blocks out, back to front: what covers a page.
    frame_rects: Vec<(String, (f32, f32, f32, f32))>,
    /// The resize handles this frame drew, in window points: a press on one
    /// over a web page comes to the deck (see `webfocus`).
    resize_corners: Vec<[f32; 4]>,
    /// When the canvas last zoomed by a gesture.
    zoomed_at: Option<Instant>,
    /// When blocks long off screen were last let go of (`sweep_hidden`).
    swept_at: Option<Instant>,
}

impl EventEmitter<DeckEvent> for DeckView {}

impl Focusable for DeckView {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl DeckView {
    pub fn new(m: Model, workspaces: Vec<String>, cx: &mut Context<Self>) -> DeckView {
        // Pinches on the trackpad zoom about the pointer (pinch.rs); a deck
        // taking over from another drains what that one left.
        let pinches = crate::platform::pinch::channel();
        while pinches.try_recv().is_ok() {}
        cx.spawn(async move |this, cx| {
            while let Ok((mut x, mut y, mut k)) = pinches.recv().await {
                if this.update(cx, |d, cx| {
                    while let Ok((nx, ny, nk)) = pinches.try_recv() {
                        x = nx;
                        y = ny;
                        k *= nk;
                    }
                    d.zoom_at(x, y, k, cx);
                }).is_err() {
                    break;
                }
            }
        })
        .detach();
        let (web_tx, web_rx) = smol::channel::unbounded();
        cx.spawn(async move |this, cx| {
            while let Ok(ev) = web_rx.recv().await {
                if this.update(cx, |d, cx| d.on_web_event(ev, cx)).is_err() {
                    break;
                }
            }
        })
        .detach();
        let (panel_tx, panel_rx) = smol::channel::unbounded();
        cx.spawn(async move |this, cx| {
            while let Ok(pick) = panel_rx.recv().await {
                if this.update(cx, |d, cx| d.on_pick(pick, cx)).is_err() {
                    break;
                }
            }
        })
        .detach();
        let lay = m.cam;
        DeckView {
            panel_tx,
            color_slot: inspector::Slot::Fill,
            insp: inspector::Inspector::default(),
            tasks: progress::Tasks::default(),
            fv: focusview::FocusView::default(),
            photo: photozoom::Photo::default(),
            glides: 0,
            captures: Default::default(),
            clip_bodies: true,
            m,
            lay,
            zoom_settle: None,
            zoom_timer_active: false,
            lod_wake: LodWake::default(),
            animating: false,
            covered: false,
            win: None,
            glide: None,
            focus: cx.focus_handle(),
            rts: HashMap::new(),
            view: (1280.0, 800.0),
            drag: None,
            editing: None,
            active: None,
            stack_open: None,
            preview: None,
            preview_leaving: None,
            preview_t: stack::Spring::default(),
            preview_frame: None,
            preview_from: None,
            pluck: None,
            last_pointer: (0.0, 0.0),
            popup: None,
            workspaces,
            canvas_scroll_until: None,
            zoom_anchor: (0.0, 0.0),
            zoom_gen: 0,
            wake_at: None,
            save_pending: false,
            mcp: None,
            mcp_err: None,
            bridge: None,
            project_mcp: false,
            web_tx,
            web_shown: HashSet::new(),
            frame_rects: Vec::new(),
            resize_corners: Vec::new(),
            zoomed_at: None,
            swept_at: None,
        }
    }

    // ---- saving and closing ----

    /// Writes deck.json off the UI thread, `SAVE_IDLE` after the first edit
    /// asks: a run of edits (dragging a slider, typing) is one snapshot and
    /// one write, not one each. `close` writes whatever is still waiting.
    fn request_save(&mut self, cx: &mut Context<Self>) {
        if self.save_pending {
            return;
        }
        self.save_pending = true;
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(SAVE_IDLE).await;
            let _ = this.update(cx, |d, cx| {
                if !std::mem::take(&mut d.save_pending) {
                    return; // closed meanwhile: written there
                }
                if let Some(write) = d.m.save_job() {
                    cx.background_spawn(async move { write() }).detach();
                }
            });
        })
        .detach();
    }

    /// Ends every block's stay (`on_close`: notes written, shells hung up), then saves.
    pub fn close(&mut self, cx: &mut Context<Self>) {
        self.photo_off();
        self.stop_mcp();
        self.life_delete(cx);
        self.life_close(cx);
        self.stop_editing(cx);
        self.save_pending = false;
        let _ = self.m.save(); // after any write under way, and no older one lands after it
    }

    /// Hangs up the block's shell, if it has one.
    fn hang_up(&mut self, id: &str, cx: &mut Context<Self>) {
        if let Some(t) = self.rts.get(id).and_then(|r| r.term.clone()) {
            t.update(cx, |t, _| t.close());
        }
    }

    // ---- edits ----

    fn undo(&mut self, _: &Undo, _: &mut Window, cx: &mut Context<Self>) {
        if self.editing.is_none() {
            self.m.undo_last();
            self.after_edit(cx);
        }
    }
    fn redo(&mut self, _: &Redo, _: &mut Window, cx: &mut Context<Self>) {
        if self.editing.is_none() {
            self.m.redo_last();
            self.after_edit(cx);
        }
    }
    fn delete_selected(&mut self, _: &DeleteSelected, _: &mut Window, cx: &mut Context<Self>) {
        if self.editing.is_none() {
            self.m.delete_selected();
            self.after_edit(cx);
        }
    }

    fn after_edit(&mut self, cx: &mut Context<Self>) {
        self.life_delete(cx);
        self.request_save(cx);
        cx.notify();
    }

    /// Where a block added by shortcut lands: under the mouse, or in the middle.
    fn pointer_or_middle(&self, window: &Window) -> (f32, f32) {
        let p = window.mouse_position();
        let (x, y) = (f32::from(p.x), f32::from(p.y));
        if x > 0.0 && y > 0.0 && x < self.view.0 && y < self.view.1 {
            (x, y)
        } else {
            (self.view.0 / 2.0, self.view.1 / 2.0)
        }
    }

    pub fn add_kind_at(&mut self, kind: BlockKind, sx: f32, sy: f32, window: &mut Window, cx: &mut Context<Self>) {
        match kind {
            BlockKind::File => self.pick_file(sx, sy, cx),
            BlockKind::Memo => self.new_memo(sx, sy, window, cx),
            _ => {
                let id = self.add_at(kind, sx, sy);
                match kind {
                    BlockKind::Note => self.start_editing(&id, EditPart::Body, window, cx),
                    // New words are there to be typed over.
                    BlockKind::Text => {
                        self.start_editing(&id, EditPart::Body, window, cx);
                        if let Some(ed) = self.editing(&id, EditPart::Body).cloned() {
                            ed.update(cx, |e, _| e.select_all_text());
                        }
                    }
                    // A new browser is there to type an address in.
                    BlockKind::Browser => self.start_editing(&id, EditPart::Address, window, cx),
                    _ => {}
                }
                self.after_edit(cx);
            }
        }
    }

    fn add_at(&mut self, kind: BlockKind, sx: f32, sy: f32) -> String {
        let (wx, wy) = self.m.cam.to_world(sx, sy);
        let mut b = new_block_of(kind, self.m.now_ms());
        // Centered where asked, over whatever is there.
        (b.x, b.y) = (wx - b.w / 2.0, wy - b.h / 2.0);
        self.m.add_block(b)
    }

    fn new_memo(&mut self, sx: f32, sy: f32, window: &mut Window, cx: &mut Context<Self>) {
        let id = self.add_at(BlockKind::Memo, sx, sy);
        let root = self.m.root.clone();
        match memo::create_file(&root, memo::MEMO_TITLE) {
            Ok(name) => {
                if let Some(b) = self.m.block_mut(&id) {
                    b.title = memo::title_of(&name);
                    b.path = name;
                }
                let rt = self.rts.entry(id.clone()).or_default();
                rt.memo.loaded = true;
                rt.memo.modified = None;
            }
            Err(e) => self.rts.entry(id.clone()).or_default().memo.err = Some(e.to_string()),
        }
        self.start_editing(&id, EditPart::Body, window, cx);
        self.after_edit(cx);
    }

    fn pick_file(&mut self, sx: f32, sy: f32, cx: &mut Context<Self>) {
        let rx = cx.prompt_for_paths(gpui::PathPromptOptions {
            files: true,
            directories: false,
            multiple: true,
            prompt: Some("链接".into()),
        });
        cx.spawn(async move |this, cx| {
            let Ok(Ok(Some(paths))) = rx.await else { return };
            let _ = this.update(cx, |d, cx| {
                d.drop_files(&paths, sx, sy);
                d.after_edit(cx);
            });
        }).detach();
    }

    /// Adds a file block for each file dropped, where it landed.
    pub fn drop_files(&mut self, paths: &[PathBuf], x: f32, y: f32) -> usize {
        let (mut wx, mut wy) = self.m.cam.to_world(x, y);
        let mut added = 0;
        for p in paths {
            let Ok(abs) = std::path::absolute(p) else { continue };
            if !abs.is_file() {
                continue; // a folder is no block yet
            }
            let (w, h) = file_block_size(&abs);
            let (px_, py_) = (wx - w / 2.0, wy - h / 2.0);
            let mut b = Block::new(BlockKind::File);
            b.title = model::file_name(&abs.to_string_lossy());
            b.path = self.m.link_path(&abs);
            (b.x, b.y, b.w, b.h) = (px_, py_, w, h);
            self.m.add_block(b);
            wx += 28.0;
            wy += 28.0;
            added += 1;
        }
        added
    }

    // ---- the view ----

    fn track_active(&mut self, window: &Window, cx: &App) {
        for (id, rt) in &self.rts {
            if let Some(t) = &rt.term
                && t.focus_handle(cx).is_focused(window)
            {
                self.active = Some(id.clone());
            }
        }
    }

}

fn message(text: String, color: Hsla, s: f32) -> AnyElement {
    div().size_full().p(px(10. * s)).text_size(px(12. * s)).text_color(color).child(text).into_any_element()
}

fn shadow(t: &Theme) -> gpui::BoxShadow {
    gpui::BoxShadow { color: t.shade, offset: point(px(0.), px(4.)), blur_radius: px(16.), spread_radius: px(0.) }
}

fn kind_icon(k: BlockKind) -> &'static str {
    match k {
        BlockKind::Note => "note",
        BlockKind::Memo => "memo",
        BlockKind::File => "file",
        BlockKind::Terminal => "terminal",
        BlockKind::Browser => "globe",
        BlockKind::Chart => "chart",
        BlockKind::Stack => "stack",
        BlockKind::Text => "type",
        BlockKind::Shape => "shapes",
    }
}

impl Render for DeckView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let backdrop = u32::from_str_radix(crate::ui::theme::hex(Theme::of(window).background).trim_start_matches('#'), 16).unwrap_or(0);
        crate::platform::layerzoom::attach(window, backdrop);
        self.photo_tick(cx);
        // Zooming as a photo, the frame is drawn with the camera the picture
        // was drawn with; Core Animation shows it where the live camera sees it.
        let live = self.photo_drawn().map(|drawn| std::mem::replace(&mut self.m.cam, drawn));
        let deck = self.draw(window, cx);
        if let Some(live) = live {
            self.m.cam = live;
        }
        deck
    }
}

impl DeckView {
    fn draw(&mut self, window: &mut Window, cx: &mut Context<Self>) -> gpui::Stateful<gpui::Div> {
        let t = Theme::of(window);
        let vs = window.viewport_size();
        self.view = (f32::from(vs.width), f32::from(vs.height));
        self.win = Some(window.window_handle());
        self.grab_keys(window, cx);
        self.tick_glide(cx);
        self.settle_layer();
        zoomlayer::framestat::begin(self.zooming());
        self.track_active(window, cx);
        self.expire_notes(window, cx);
        self.life_delete(cx);
        self.watch_stack();
        if let Some(e) = &self.editing
            && self.m.block(&e.id).is_none()
        {
            self.editing = None;
        }

        let cam = self.m.cam;
        let dot = t.muted.opacity(0.55);
        let me = cx.entity();
        let grid = canvas(|_, _, _| (), move |bounds, _, window, _| {
            // The world's dots, at least 24 px apart on the screen.
            let mut step = 20f32;
            while step * cam.s < 24. {
                step *= 2.5;
            }
            while step * cam.s > 90. {
                step /= 2.5;
            }
            let (w, h) = (f32::from(bounds.size.width), f32::from(bounds.size.height));
            let x0 = (cam.x / step).floor() * step;
            let y0 = (cam.y / step).floor() * step;
            let mut wy = y0;
            loop {
                let py = (wy - cam.y) * cam.s;
                if py > h {
                    break;
                }
                let mut wx = x0;
                loop {
                    let px_ = (wx - cam.x) * cam.s;
                    if px_ > w {
                        break;
                    }
                    if px_ >= 0. && py >= 0. {
                        window.paint_quad(
                            fill(gpui::Bounds::new(point(px(px_ - 1.5), px(py - 1.5)), size(px(3.), px(3.))), dot)
                                .corner_radii(px(1.5)),
                        );
                    }
                    wx += step;
                }
                wy += step;
            }
            // Window-wide: a drag goes on over anything; a wheel no body took is the canvas's.
            let m1 = me.clone();
            window.on_mouse_event(move |ev: &MouseMoveEvent, phase, _, cx| {
                if phase == DispatchPhase::Bubble {
                    m1.update(cx, |d, cx| d.on_move(ev, cx));
                }
            });
            let m2 = me.clone();
            window.on_mouse_event(move |ev: &MouseUpEvent, phase, window, cx| {
                if phase == DispatchPhase::Bubble {
                    m2.update(cx, |d, cx| d.on_up(ev, window, cx));
                }
            });
            let m3 = me.clone();
            window.on_mouse_event(move |ev: &ScrollWheelEvent, phase, _, cx| {
                if phase == DispatchPhase::Bubble {
                    m3.update(cx, |d, cx| d.on_wheel(ev, cx));
                }
            });
        })
        .absolute()
        .left_0()
        .top_0()
        .size_full();

        // Known before the blocks are drawn: web pages under the style panel give way.
        self.inspector_tick();
        self.tasks_tick();
        self.focus_tick();
        let visible: Vec<String> =
            self.m.visible(self.view.0, self.view.1).into_iter().map(|i| self.m.blocks[i].id.clone()).collect();
        self.frame_rects = visible
            .iter()
            .filter_map(|id| self.m.block(id))
            .map(|b| {
                let (x, y) = cam.to_screen(b.x, b.y);
                (b.id.clone(), (x, y, b.w * cam.s, b.h * cam.s))
            })
            .collect();
        self.life_tick(&visible, window, cx);
        self.sweep_hidden();
        self.resize_corners.clear();
        // Playing, the bodies this frame pictures, nearest the middle first (`snapshot`).
        self.plan_captures(&visible, t.dark, window);
        let mut built: Vec<Option<Vec<AnyElement>>> = (0..visible.len()).map(|_| None).collect();
        // Blocks coming back from LOD are built nearest the middle of the window
        // first, so they surface like a focus pulling in, and only until this
        // frame's waking budget (`LodWake`) is spent. Everything else is built,
        // and every element put back, in the stack's own order.
        let my = self.view.1 / 2.;
        let mut waking: Vec<(usize, f32, f32)> = Vec::new();
        for (i, id) in visible.iter().enumerate() {
            let waiting = self.rts.get(id).is_some_and(|r| r.lod) && self.m.cam.s >= LOD_SCALE;
            if waiting {
                let (d, y) = self.m.block(id).map_or((0.0, my), |b| wake_rank(cam, b, self.view));
                waking.push((i, d, y));
            } else {
                built[i] = Some(self.block_view(id, &t, window, cx));
            }
        }
        waking.sort_by(|a, b| a.1.total_cmp(&b.1).then(a.2.total_cmp(&b.2)));
        self.lod_wake.begin(!waking.is_empty(), Instant::now());
        for (i, _, _) in waking {
            built[i] = Some(self.block_view(&visible[i], &t, window, cx));
        }
        self.lod_wake.end();
        let mut blocks = Vec::new();
        for b in built.into_iter().flatten() {
            blocks.extend(b);
        }

        self.hide_unshown_pages();
        // While a photo is scaled, a frame would only redraw what is being
        // scaled, and hold up the gesture: animations wait for it to end.
        if std::mem::take(&mut self.animating) && !self.photo_showing() {
            window.request_animation_frame();
        }
        crate::platform::webfocus::guard(window);
        crate::platform::webfocus::set_corners(self.resize_corners.clone());

        // Playing, the chrome steps aside; zooming as a photo too (it would be
        // scaled with the picture), fading back in after (`photozoom`).
        let playing = self.fv.play.is_some();
        let chrome_alpha = self.chrome_alpha();
        let hidden = playing || chrome_alpha <= 0.;
        let inspector = if hidden { vec![] } else { self.inspector(&t, cx) };
        let focus_layer = if chrome_alpha <= 0. { vec![] } else { self.focus_layer(&t, cx) };
        let mut chrome: Vec<AnyElement> =
            if hidden { vec![] } else { vec![self.home_pill(&t, cx), self.toolbar(&t, cx)] };
        chrome.extend((!hidden).then(|| self.status_bar(&t)));
        chrome.extend(inspector);
        if chrome_alpha > 0. {
            chrome.extend(self.task_capsule(&t));
        }
        let chrome = div().absolute().top_0().left_0().size_full().opacity(chrome_alpha).children(chrome);
        let marquee = match &self.drag {
            Some(Drag::Marquee { from, to }) => {
                let (x0, y0) = cam.to_screen(from.0.min(to.0), from.1.min(to.1));
                Some(
                    div()
                        .absolute()
                        .left(px(x0))
                        .top(px(y0))
                        .w(px((to.0 - from.0).abs() * cam.s))
                        .h(px((to.1 - from.1).abs() * cam.s))
                        .border_1()
                        .border_color(t.accent)
                        .bg(t.accent.opacity(0.06))
                        .rounded(px(2.)),
                )
            }
            _ => None,
        };

        div()
            .id("deck")
            .key_context("Deck")
            .track_focus(&self.focus)
            .size_full()
            .relative()
            .overflow_hidden()
            .bg(t.background)
            .font_family(".SystemUIFont")
            .on_action(cx.listener(Self::fit_all))
            .on_action(cx.listener(Self::zoom_in))
            .on_action(cx.listener(Self::zoom_out))
            .on_action(cx.listener(Self::undo))
            .on_action(cx.listener(Self::redo))
            .on_action(cx.listener(Self::delete_selected))
            .on_action(cx.listener(|d, _: &NewNote, w, cx| {
                let (x, y) = d.pointer_or_middle(w);
                d.add_kind_at(BlockKind::Note, x, y, w, cx)
            }))
            .on_action(cx.listener(|d, _: &NewMemo, w, cx| {
                let (x, y) = d.pointer_or_middle(w);
                d.add_kind_at(BlockKind::Memo, x, y, w, cx)
            }))
            .on_action(cx.listener(|d, _: &NewFile, w, cx| {
                let (x, y) = d.pointer_or_middle(w);
                d.add_kind_at(BlockKind::File, x, y, w, cx)
            }))
            .on_action(cx.listener(|d, _: &NewTerminal, w, cx| {
                let (x, y) = d.pointer_or_middle(w);
                d.add_kind_at(BlockKind::Terminal, x, y, w, cx)
            }))
            .on_action(cx.listener(|d, _: &NewChart, w, cx| {
                let (x, y) = d.pointer_or_middle(w);
                d.add_kind_at(BlockKind::Chart, x, y, w, cx)
            }))
            .on_action(cx.listener(|d, _: &NewStack, w, cx| {
                let (x, y) = d.pointer_or_middle(w);
                d.add_kind_at(BlockKind::Stack, x, y, w, cx)
            }))
            .on_action(cx.listener(|d, _: &NewText, w, cx| {
                let (x, y) = d.pointer_or_middle(w);
                d.add_kind_at(BlockKind::Text, x, y, w, cx)
            }))
            .on_action(cx.listener(|d, _: &NewShape, w, cx| {
                let (x, y) = d.pointer_or_middle(w);
                d.add_kind_at(BlockKind::Shape, x, y, w, cx)
            }))
            .on_action(cx.listener(|d, _: &NewBrowser, w, cx| {
                let (x, y) = d.pointer_or_middle(w);
                d.add_kind_at(BlockKind::Browser, x, y, w, cx)
            }))
            .on_action(cx.listener(|_, _: &GoHome, _, cx| cx.emit(DeckEvent::Home)))
            .on_action(cx.listener(Self::toggle_focus_view))
            .on_action(cx.listener(Self::play_focus))
            .on_action(cx.listener(Self::keep_focus))
            .on_key_down(cx.listener(|d, ev, window, cx| {
                if d.focus_keys(ev, window, cx) {
                    cx.stop_propagation();
                } else {
                    d.stack_keys(ev, window, cx);
                }
            }))
            .on_mouse_down(MouseButton::Left, cx.listener(Self::on_canvas_down))
            .on_mouse_down(MouseButton::Right, cx.listener(Self::on_pan_down))
            .on_mouse_down(MouseButton::Middle, cx.listener(Self::on_pan_down))
            .on_drop(cx.listener(|d, paths: &ExternalPaths, window, cx| {
                let p = window.mouse_position();
                d.drop_files(paths.paths(), f32::from(p.x), f32::from(p.y));
                d.after_edit(cx);
            }))
            .child(grid)
            .children(blocks)
            .children(marquee)
            .children(focus_layer)
            .child(chrome)
            .children(self.stack_preview(&t, window, cx))
            .children(self.pluck_view(&t))
            .children(self.popup_view(&t, cx))
            // TEMP: painted last, so it marks the end of the frame's work (`framestat`).
            .child(canvas(|_, _, _| (), |_, _, _, _| zoomlayer::framestat::end()).absolute().size_0())
    }
}

#[cfg(test)]
mod zoom_tests {
    use super::*;
    use crate::model::{Camera, ZOOM_REBASE, on_level, zoom_level};

    struct Harness {
        m: Model,
        lay: Camera,
        zoom_settle: Option<Instant>,
    }

    impl Harness {
        fn new() -> Self {
            let m = Model::new(std::path::Path::new("/tmp"));
            let lay = m.cam;
            Self { m, lay, zoom_settle: None }
        }

        fn zoom_gesture(&mut self) {
            self.zoom_settle = Some(Instant::now() + zoomlayer::ZOOM_SETTLE_HOLD);
        }

        fn settle(&mut self) {
            if self.lay.s == 0.0 {
                self.lay = self.m.cam;
            }
            let now = Instant::now();
            if self.zoom_settle.is_some_and(|u| now < u) {
                if self.m.cam.s < self.lay.s * ZOOM_REBASE || self.m.cam.s > self.lay.s / ZOOM_REBASE {
                    self.lay = Camera { x: self.m.cam.x, y: self.m.cam.y, s: zoom_level(self.m.cam.s) };
                }
                return;
            }
            self.zoom_settle = None;
            self.lay = self.m.cam;
        }

        fn layer_scale(&self) -> f32 {
            self.m.cam.s / self.lay.s
        }
    }

    #[test]
    fn rasters_hold_their_scale_while_zooming_and_the_camera_rests_where_it_stopped() {
        let mut h = Harness::new();
        assert_eq!(h.lay.s, 1.0);

        // Mid-gesture: `lay.s` stays frozen at 1.0 while `m.cam.s` moves.
        for _ in 0..8 {
            h.m.zoom_at(640.0, 400.0, 0.96);
            h.zoom_gesture();
            h.settle();
        }
        assert_eq!(h.lay.s, 1.0);
        assert!((h.layer_scale() - h.m.cam.s).abs() < 1e-6);

        // Crossing `ZOOM_REBASE` (0.5x) rebases `lay.s` to a discrete zoom level.
        h.m.zoom_at(640.0, 400.0, 0.4 / h.m.cam.s);
        h.zoom_gesture();
        h.settle();
        assert!(on_level(h.lay.s));
        assert!((h.lay.s - zoom_level(0.4)).abs() < 1e-6);

        // Once the gesture rests, the camera stays where the fingers left it
        // (as a photo does, it does not move by itself) and rasters catch up.
        h.m.zoom_at(640.0, 400.0, 0.77 / h.m.cam.s);
        h.zoom_settle = Some(Instant::now() - Duration::from_millis(10));
        h.settle();
        assert!((h.m.cam.s - 0.77).abs() < 1e-5 && !on_level(h.m.cam.s));
        assert_eq!(h.lay, h.m.cam);
    }
}

#[cfg(test)]
mod lod_wake_tests {
    use super::*;

    #[test]
    fn waking_spends_the_frame_budget_but_always_lets_one_block_go() {
        let at = Instant::now();
        let mut w = LodWake::default();
        w.begin(true, at);
        assert!(w.may(at), "one block goes even if the budget were already spent");
        assert!(w.may(at + LOD_WAKE_BUDGET / 2), "within the budget");
        assert!(!w.may(at + LOD_WAKE_BUDGET), "the budget is spent");
        assert!(!w.may(at + LOD_WAKE_BUDGET * 2), "and stays spent");
    }

    #[test]
    fn a_slow_first_block_ends_the_frame() {
        let at = Instant::now();
        let mut w = LodWake::default();
        w.begin(true, at);
        assert!(w.may(at), "the first block");
        assert!(!w.may(at + LOD_WAKE_BUDGET + Duration::from_millis(1)), "the next waits for the next frame");
    }

    #[test]
    fn a_new_frame_starts_with_a_fresh_allowance() {
        let at = Instant::now();
        let mut w = LodWake::default();
        w.begin(true, at);
        assert!(w.may(at));
        assert!(!w.may(at + LOD_WAKE_BUDGET * 2));
        w.end();
        w.begin(true, at + Duration::from_secs(1));
        assert!(w.may(at + Duration::from_secs(1)), "a new frame wakes again");
    }

    #[test]
    fn waking_order_is_nearest_the_middle_then_higher_up() {
        let cam = model::Camera { x: 0., y: 0., s: 1. };
        let view = (1000., 800.);
        let block = |x, y, w, h| {
            let mut b = Block::new(BlockKind::Memo);
            (b.x, b.y, b.w, b.h) = (x, y, w, h);
            b
        };
        let middle = block(450., 350., 100., 100.); // center (500, 400)
        let top = block(450., 100., 100., 100.); // center (500, 150)
        let corner = block(0., 0., 100., 100.); // center (50, 50)
        assert!(wake_rank(cam, &middle, view).0 < wake_rank(cam, &top, view).0);
        assert!(wake_rank(cam, &top, view).0 < wake_rank(cam, &corner, view).0);
        let lower = block(450., 600., 100., 100.); // center (500, 650), as far as `top`
        assert_eq!(wake_rank(cam, &top, view).0, wake_rank(cam, &lower, view).0);
        assert!(
            wake_rank(cam, &top, view).1 < wake_rank(cam, &lower, view).1,
            "the higher one goes first"
        );
    }
}

#[cfg(test)]
mod focus_tests {
    use super::*;

    #[test]
    fn a_click_on_the_canvas_hides_no_page() {
        let cam = model::Camera { x: 0., y: 0., s: 1. };
        let page = (100., 100., 400., 300.);
        assert!(!marquee_over(cam, (50., 50.), (50., 50.), page), "a press alone");
        assert!(!marquee_over(cam, (150., 150.), (151., 152.), page), "a press that slipped a pixel");
        assert!(marquee_over(cam, (50., 50.), (200., 200.), page), "a marquee grown over the page");
        assert!(!marquee_over(cam, (600., 50.), (700., 90.), page), "a marquee beside it");
    }
}
