//! The deck's view: the canvas, its blocks and the chrome around them.
//!
//! The canvas is laid out with the camera each frame: block X/Y/W/H and
//! font sizes are multiplied by its scale, so text stays crisp at any zoom.
//! Only blocks the window shows some of are built.
//!
//! Input: each card, tape and resize handle takes its own presses and
//! occludes what is under it; a press nowhere else reaches the canvas.
//! Moves, releases and the wheel go through window-wide listeners the grid
//! registers as it paints, so a drag carries on over anything, and a wheel
//! no body took pans or zooms the canvas.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime};

use gpui::{
    AnyElement, App, AppContext, Context, Div, DispatchPhase, Entity, EventEmitter, ExternalPaths, FocusHandle, Focusable,
    FontWeight, Hsla, InteractiveElement, IntoElement, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent,
    ObjectFit, ParentElement, PathBuilder, Render, ScrollWheelEvent, SharedString, Stateful,
    StatefulInteractiveElement, Styled, StyledImage, Subscription, UniformListScrollHandle, Window, actions,
    canvas, div, fill, img, linear_color_stop, linear_gradient, point, prelude::FluentBuilder, px, size, svg,
    uniform_list,
};

mod browser;
mod stack;

use crate::editor::{Editor, EditorEvent};
use crate::lifespan::{self, LIFE_CHOICES};
use crate::markdown;
use crate::memo;
use crate::model::{
    self, Block, BlockKind, Geometry, LOD_SCALE, Model, file_block_size, new_block_of, scale_keeping_ratio,
    wheel_zoom,
};
use crate::recents::short_path;
use crate::tape;
use crate::terminal::{self, Terminal, TerminalEvent};
use crate::termstate::{self, TermState};
use crate::theme::{MdColors, Paper, Theme};
use crate::{icons, preview};

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
        GoHome,
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
/// How long a zoom rests before it glides to the nearest zoom level.
const ZOOM_REST: Duration = Duration::from_millis(150);

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
    pdf: Option<Result<Arc<crate::pdf::Doc>, String>>,
    pdf_img: Option<(usize, usize, Arc<gpui::RenderImage>)>,
    pdf_rendering: bool,
    list: UniformListScrollHandle,
    term: Option<Entity<Terminal>>,
    term_err: Option<String>,
    term_used: bool,
    _term_sub: Option<Subscription>,
    memo: MemoRt,
    stack: stack::StackRt,
    web: Option<browser::WebRt>,
    web_err: Option<String>,
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
    focus: FocusHandle,
    rts: HashMap<String, Runtime>,
    view: (f32, f32),
    drag: Option<Drag>,
    editing: Option<Editing>,
    /// The block worked in: its body takes presses and the wheel.
    active: Option<String>,
    /// The block whose title bar the last press was on: a click without a drag edits its title.
    pressed_title: Option<String>,
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
    save_seq: Arc<Mutex<u64>>,
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
    web_shown: HashSet<String>,
    /// Where this frame lays the blocks out, back to front: what covers a page.
    frame_rects: Vec<(String, (f32, f32, f32, f32))>,
    /// When the canvas last zoomed by a gesture.
    zoomed_at: Option<Instant>,
}

impl EventEmitter<DeckEvent> for DeckView {}

impl Focusable for DeckView {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl DeckView {
    pub fn new(m: Model, workspaces: Vec<String>, cx: &mut Context<Self>) -> DeckView {
        let (web_tx, web_rx) = smol::channel::unbounded();
        cx.spawn(async move |this, cx| {
            while let Ok(ev) = web_rx.recv().await {
                if this.update(cx, |d, cx| d.on_web_event(ev, cx)).is_err() {
                    break;
                }
            }
        })
        .detach();
        DeckView {
            m,
            focus: cx.focus_handle(),
            rts: HashMap::new(),
            view: (1280.0, 800.0),
            drag: None,
            editing: None,
            active: None,
            pressed_title: None,
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
            save_seq: Arc::new(Mutex::new(0)),
            mcp: None,
            mcp_err: None,
            bridge: None,
            project_mcp: false,
            web_tx,
            web_shown: HashSet::new(),
            frame_rects: Vec::new(),
            zoomed_at: None,
        }
    }

    // ---- the MCP endpoint ----

    /// Serves this deck to agents: its socket, its descriptor, the active-deck registry.
    pub fn start_mcp(&mut self, cx: &mut Context<Self>) {
        use crate::mcp::{self, rpc::Job};
        let (tx, rx) = smol::channel::unbounded::<(Job, std::sync::mpsc::SyncSender<Result<serde_json::Value, String>>)>();
        // Each job runs on the UI thread, in the deck's own add/remove/undo path.
        cx.spawn(async move |this, cx| {
            while let Ok((job, reply)) = rx.recv().await {
                let out = this.update(cx, |d, cx| d.run_job(job, cx)).unwrap_or_else(|_| Err("这个工作区已经关了".into()));
                let _ = reply.send(out);
            }
        })
        .detach();
        let run: Arc<mcp::rpc::Runner> = Arc::new(move |job| {
            let (reply, answer) = std::sync::mpsc::sync_channel(1);
            tx.send_blocking((job, reply)).map_err(|_| "这个工作区已经关了".to_string())?;
            answer.recv_timeout(Duration::from_secs(15)).map_err(|_| "窗口 15s 没有响应".to_string())?
        });
        let root = self.m.root.clone();
        let sock = mcp::socket_path(&root);
        match mcp::server::Server::start(&sock, root.to_string_lossy().into_owned(), run) {
            Ok(server) => {
                let exe = std::env::current_exe().map(|p| p.to_string_lossy().into_owned()).unwrap_or("boundlessdeck".into());
                let names = mcp::tools::tool_names();
                let ep = mcp::Endpoint {
                    name: mcp::tools::SERVER_NAME.into(),
                    transport: "unix".into(),
                    socket: server.socket.to_string_lossy().into_owned(),
                    token: server.token.clone(),
                    bridge: mcp::BridgeCmd {
                        command: exe,
                        args: vec!["mcp-stdio".into(), "--workspace".into(), root.to_string_lossy().into_owned()],
                    },
                    tools: names.len(),
                    tool_names: names.into_iter().map(String::from).collect(),
                    protocol_version: mcp::PROTOCOL_VERSION.into(),
                };
                if let Err(e) = mcp::publish(&root, &ep) {
                    self.mcp_err = Some(format!("写 MCP 配置：{e}"));
                }
                self.bridge = Some(ep.bridge.clone());
                self.mcp = Some(server);
                if self.project_mcp
                    && let Err(e) = self.set_project_mcp(true, cx)
                {
                    self.mcp_err = Some(e);
                }
            }
            Err(e) => self.mcp_err = Some(e.to_string()),
        }
    }

    /// Writes the project-level configs now (on), or stops writing them (off).
    pub fn set_project_mcp(&mut self, on: bool, cx: &mut Context<Self>) -> Result<(), String> {
        self.project_mcp = on;
        cx.notify();
        if !on {
            return Ok(());
        }
        let Some(b) = &self.bridge else { return Ok(()) };
        let warnings = crate::mcp::config::write_project_configs(&self.m.root, &b.command, &b.args);
        if warnings.is_empty() { Ok(()) } else { Err(warnings.join("；")) }
    }

    fn stop_mcp(&mut self) {
        if let Some(s) = self.mcp.take() {
            s.stop();
            crate::mcp::unpublish(&self.m.root);
        }
    }

    fn run_job(&mut self, job: crate::mcp::rpc::Job, cx: &mut Context<Self>) -> Result<serde_json::Value, String> {
        use crate::mcp::{rpc::Job, tools::Ctx};
        let info = match &self.mcp {
            Some(s) => serde_json::json!({
                "running": true, "socket": s.socket.to_string_lossy(), "startedAt": s.started.to_rfc3339(),
                "tools": crate::mcp::tools::tool_names().len(), "configFile": ".boundlessdeck/mcp.json",
            }),
            None => serde_json::json!({"running": false, "error": self.mcp_err}),
        };
        let mut ctx = Ctx { m: &mut self.m, view: self.view, mcp: info, dirty: false, memos: vec![], reload: vec![] };
        let out = match job {
            Job::Tool(name, args) => ctx.call(&name, &args),
            Job::Resource(uri) => ctx.read_resource(&uri),
        };
        let (dirty, memos, reload) = (ctx.dirty, ctx.memos, ctx.reload);
        for (id, text) in memos {
            let rt = self.rts.entry(id).or_default();
            if rt.memo.loaded {
                rt.memo.text = text.clone();
                rt.memo.saved = text;
                rt.memo.modified = None;
            }
        }
        for id in reload {
            if let Some(rt) = self.rts.get_mut(&id) {
                rt.lines = None;
                rt.doc = None;
                rt.word = None;
                rt.pdf = None;
                rt.pdf_img = None;
                rt.memo = MemoRt::default();
                rt.web = None;
                rt.web_err = None;
            }
        }
        if dirty {
            self.after_edit(cx);
        }
        out
    }

    // ---- saving and closing ----

    /// Writes deck.json off the UI thread: only the newest snapshot is written.
    fn request_save(&mut self, cx: &mut Context<Self>) {
        let Some(path) = self.m.deck_file() else { return };
        let data = self.m.snapshot();
        let seq = self.save_seq.clone();
        let mine = {
            let mut s = seq.lock().unwrap();
            *s += 1;
            *s
        };
        cx.background_spawn(async move {
            let s = seq.lock().unwrap();
            if *s == mine {
                let _ = model::write_file_atomic(&path, &data);
            }
        })
        .detach();
    }

    /// Keeps what each used terminal shows, saves the deck, and hangs the shells up.
    pub fn close(&mut self, cx: &mut Context<Self>) {
        self.stop_mcp();
        self.stop_editing(cx);
        self.keep_terminals(cx);
        let ids: Vec<String> = self.m.blocks.iter().map(|b| b.id.clone()).collect();
        for id in ids {
            self.flush_memo(&id);
        }
        *self.save_seq.lock().unwrap() += 1; // no write under way overtakes this one
        let _ = self.m.save();
        for (_, rt) in self.rts.drain() {
            if let Some(t) = rt.term {
                t.update(cx, |t, _| t.close());
            }
        }
    }

    fn keep_terminals(&mut self, cx: &mut Context<Self>) {
        let ids: Vec<String> = self.m.blocks.iter().map(|b| b.id.clone()).collect();
        for id in ids {
            self.keep_terminal(&id, cx);
        }
    }

    /// Keeps what a terminal the user typed into shows, and where its shell is.
    fn keep_terminal(&mut self, id: &str, cx: &mut Context<Self>) {
        let Some(rt) = self.rts.get(id) else { return };
        let (Some(t), true) = (rt.term.as_ref(), rt.term_used) else { return };
        let (history, dir) = t.read_with(cx, |t, _| (t.history(), t.dir()));
        let dir = dir.map(|d| self.m.link_path(&d)).unwrap_or_default();
        if let Some(b) = self.m.block_mut(id) {
            b.term = Some(TermState::new(history, dir));
        }
    }

    /// Lets go of what blocks that left the deck hold.
    fn hang_up_gone(&mut self, cx: &mut Context<Self>) {
        let gone: Vec<String> = std::mem::take(&mut self.m.hung_up);
        for id in gone {
            self.keep_terminal(&id, cx);
            self.flush_memo(&id);
            if let Some(rt) = self.rts.remove(&id)
                && let Some(t) = rt.term
            {
                t.update(cx, |t, _| t.close());
            }
            if self.active.as_deref() == Some(&id) {
                self.active = None;
            }
        }
    }

    // ---- the camera ----

    fn zoom_at(&mut self, x: f32, y: f32, factor: f32, cx: &mut Context<Self>) {
        self.m.zoom_at(x, y, factor);
        self.note_zoom(cx);
        self.zoom_anchor = (x, y);
        self.zoom_gen += 1;
        let gen_now = self.zoom_gen;
        // Once the zoom rests, it glides to the nearest zoom level.
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(ZOOM_REST).await;
            for step in 1..=8 {
                let go_on = this
                    .update(cx, |d, cx| {
                        if d.zoom_gen != gen_now {
                            return false;
                        }
                        let to = model::zoom_level(d.m.cam.s);
                        let k = (to / d.m.cam.s).powf(1.0 / (9 - step) as f32);
                        let (ax, ay) = d.zoom_anchor;
                        d.m.zoom_at(ax, ay, k);
                        if step == 8 {
                            d.m.cam.s = to;
                        }
                        cx.notify();
                        true
                    })
                    .unwrap_or(false);
                if !go_on {
                    break;
                }
                cx.background_executor().timer(Duration::from_millis(15)).await;
            }
        }).detach();
    }

    fn fit_all(&mut self, _: &FitAll, _: &mut Window, cx: &mut Context<Self>) {
        self.m.fit_all(self.view.0, self.view.1);
        cx.notify();
    }
    fn zoom_in(&mut self, _: &ZoomIn, _: &mut Window, cx: &mut Context<Self>) {
        self.m.zoom_step(self.view.0 / 2.0, self.view.1 / 2.0, 1);
        cx.notify();
    }
    fn zoom_out(&mut self, _: &ZoomOut, _: &mut Window, cx: &mut Context<Self>) {
        self.m.zoom_step(self.view.0 / 2.0, self.view.1 / 2.0, -1);
        cx.notify();
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
        self.hang_up_gone(cx);
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
        (b.x, b.y) = self.m.free_spot(wx - b.w / 2.0, wy - b.h / 2.0, b.w, b.h);
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
            let (px_, py_) = self.m.free_spot(wx - w / 2.0, wy - h / 2.0, w, h);
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

    // ---- editing ----

    fn start_editing(&mut self, id: &str, part: EditPart, window: &mut Window, cx: &mut Context<Self>) {
        if self.editing.as_ref().is_some_and(|e| e.id == id && e.part == part) {
            return;
        }
        self.stop_editing(cx);
        let Some(b) = self.m.block(id) else { return };
        let kind = b.kind;
        let text = match (part, kind) {
            (EditPart::Title, _) => b.title.clone(),
            (EditPart::Address, _) => b.url.clone(),
            (EditPart::Body, BlockKind::Memo) => self.rts.get(id).map(|r| r.memo.text.clone()).unwrap_or_default(),
            (EditPart::Body, _) => b.text.clone(),
        };
        let from_title = b.title.clone();
        let multiline = part == EditPart::Body;
        let select_all = part != EditPart::Body;
        let editor = cx.new(|cx| {
            let mut e = Editor::new(text, multiline, window, cx);
            e.placeholder = match (part, kind) {
                (EditPart::Title, _) => "标题".into(),
                (EditPart::Address, _) => "输入网址或搜索 · 回车打开".into(),
                (_, BlockKind::Note) => "支持 Markdown · Esc 或 ⌘↵ 完成".into(),
                (_, BlockKind::Chart) => "名称\t数值 · 每行一项".into(),
                _ => "Markdown · Esc 或 ⌘↵ 完成".into(),
            };
            if select_all {
                e.select_all_text();
            }
            e
        });
        let eid = id.to_string();
        let sub = cx.subscribe(&editor, move |d: &mut DeckView, ed, ev: &EditorEvent, cx| match ev {
            EditorEvent::Changed => {
                let text = ed.read(cx).text.clone();
                let Some(e) = d.editing.as_ref().filter(|e| e.id == eid) else { return };
                let part = e.part;
                let is_memo = d.m.block(&eid).is_some_and(|b| b.kind == BlockKind::Memo);
                match part {
                    EditPart::Address => {} // taken as the edit ends
                    EditPart::Title => {
                        if let Some(b) = d.m.block_mut(&eid) {
                            b.title = text;
                        }
                    }
                    EditPart::Body if is_memo => {
                        let rt = d.rts.entry(eid.clone()).or_default();
                        rt.memo.text = text;
                        rt.memo.edited = Some(Instant::now());
                        d.schedule_memo_save(&eid, cx);
                    }
                    EditPart::Body => {
                        if let Some(b) = d.m.block_mut(&eid) {
                            b.text = text;
                        }
                    }
                }
                cx.notify();
            }
            EditorEvent::Done | EditorEvent::Blurred => {
                if d.editing.as_ref().is_some_and(|e| e.id == eid) {
                    // Done typing, the block stays the one worked in.
                    d.active = Some(eid.clone());
                    d.stop_editing(cx);
                    cx.notify();
                }
            }
        });
        let f = editor.focus_handle(cx);
        window.focus(&f);
        self.editing = Some(Editing { id: id.to_string(), part, editor, from_title, _sub: sub });
        cx.notify();
    }

    /// Keeps what was typed and shows the block as it reads again.
    fn stop_editing(&mut self, cx: &mut Context<Self>) {
        let Some(e) = self.editing.take() else { return };
        let root = self.m.root.clone();
        let typed = e.editor.read(cx).text.clone();
        let Some(b) = self.m.block_mut(&e.id) else { return };
        match e.part {
            EditPart::Address => {
                if let Ok(url) = browser::address_to_url(&typed)
                    && url != b.url
                {
                    b.url = url.clone();
                    self.web_go(&e.id, &url);
                }
            }
            EditPart::Title => {
                b.title = b.title.trim().to_string();
                if b.kind == BlockKind::Memo && b.title != e.from_title && !b.title.is_empty() {
                    let (rel, title) = (b.path.clone(), b.title.clone());
                    match memo::rename_file(&root, &rel, &title) {
                        Ok(next) => {
                            b.title = memo::title_of(&next);
                            b.path = next;
                            b.named = false;
                        }
                        Err(err) => {
                            b.title = e.from_title.clone();
                            self.rts.entry(e.id.clone()).or_default().memo.save_err = Some(format!("没能改名：{err}"));
                        }
                    }
                } else if b.title.is_empty() {
                    b.title = b.default_title();
                    b.named = false;
                } else if b.title != e.from_title {
                    b.named = true;
                }
            }
            EditPart::Body => {
                if b.kind == BlockKind::Memo {
                    self.save_memo(&e.id, cx);
                }
            }
        }
        // A QuickLook whose time ran out while it was written goes now.
        self.request_save(cx);
        cx.notify();
    }

    fn editing(&self, id: &str, part: EditPart) -> Option<&Entity<Editor>> {
        self.editing.as_ref().filter(|e| e.id == id && e.part == part).map(|e| &e.editor)
    }

    // ---- 笔记 files ----

    fn load_memo(&mut self, id: &str, cx: &mut Context<Self>) {
        let Some(b) = self.m.block(id) else { return };
        if b.path.is_empty() {
            return;
        }
        let rt = self.rts.entry(id.to_string()).or_default();
        if rt.loading || rt.memo.loaded || rt.memo.err.is_some() {
            return;
        }
        rt.loading = true;
        let path = self.m.path_of(&b.path);
        let id = id.to_string();
        cx.spawn(async move |this, cx| {
            let r = cx.background_spawn(async move { memo::read(&path) }).await;
            let _ = this.update(cx, |d, cx| {
                let rt = d.rts.entry(id).or_default();
                rt.loading = false;
                match r {
                    Ok((text, modified)) => {
                        rt.memo.text = text.clone();
                        rt.memo.saved = text;
                        rt.memo.loaded = true;
                        rt.memo.modified = Some(modified);
                        rt.memo.checked = Some(Instant::now());
                    }
                    Err(e) => rt.memo.err = Some(e),
                }
                cx.notify();
            });
        }).detach();
    }

    fn schedule_memo_save(&mut self, id: &str, cx: &mut Context<Self>) {
        let id = id.to_string();
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(memo::SAVE_IDLE).await;
            let _ = this.update(cx, |d, cx| {
                let rested = d
                    .rts
                    .get(&id)
                    .and_then(|r| r.memo.edited)
                    .is_some_and(|t| t.elapsed() >= memo::SAVE_IDLE - Duration::from_millis(20));
                if rested {
                    d.save_memo(&id, cx);
                }
            });
        }).detach();
    }

    /// Writes what was typed to the note's file, unless the file holds it already.
    fn save_memo(&mut self, id: &str, cx: &mut Context<Self>) {
        let Some(b) = self.m.block(id).filter(|b| b.kind == BlockKind::Memo && !b.path.is_empty()) else {
            return;
        };
        let path = self.m.path_of(&b.path);
        let Some(rt) = self.rts.get_mut(id) else { return };
        if !rt.memo.loaded || rt.memo.text == rt.memo.saved || rt.memo.saving {
            return;
        }
        rt.memo.saving = true;
        let text = rt.memo.text.clone();
        let id = id.to_string();
        cx.spawn(async move |this, cx| {
            let t2 = text.clone();
            let r = cx.background_spawn(async move { memo::write(&path, &t2) }).await;
            let _ = this.update(cx, |d, cx| {
                let rt = d.rts.entry(id.clone()).or_default();
                rt.memo.saving = false;
                match r {
                    Ok(m) => {
                        rt.memo.saved = text;
                        rt.memo.modified = Some(m);
                        rt.memo.save_err = None;
                    }
                    Err(e) => rt.memo.save_err = Some(format!("没能保存：{e}")),
                }
                // Typed on meanwhile: write that too.
                if rt.memo.text != rt.memo.saved {
                    d.save_memo(&id, cx);
                }
                cx.notify();
            });
        }).detach();
    }

    /// Writes what was typed in the note, here and now: the block leaves or the deck closes.
    fn flush_memo(&mut self, id: &str) {
        let Some(b) = self.m.block(id).filter(|b| b.kind == BlockKind::Memo && !b.path.is_empty()) else {
            return;
        };
        let path = self.m.path_of(&b.path);
        if let Some(rt) = self.rts.get_mut(id)
            && rt.memo.loaded
            && rt.memo.text != rt.memo.saved
            && memo::write(&path, &rt.memo.text).is_ok()
        {
            rt.memo.saved = rt.memo.text.clone();
        }
    }

    /// Reads the note's file again when something else changed it, at most every 2 s.
    fn check_memo(&mut self, id: &str, cx: &mut Context<Self>) {
        let Some(b) = self.m.block(id) else { return };
        let path = self.m.path_of(&b.path);
        let Some(rt) = self.rts.get_mut(id) else { return };
        let m = &mut rt.memo;
        if !m.loaded || rt.loading || m.saving || b.path.is_empty() || m.text != m.saved {
            return;
        }
        if m.checked.is_some_and(|t| t.elapsed() < memo::POLL) {
            return;
        }
        m.checked = Some(Instant::now());
        let Some(known) = m.modified else { return };
        rt.loading = true;
        let id = id.to_string();
        cx.spawn(async move |this, cx| {
            let r = cx.background_spawn(async move { memo::changed_since(&path, known) }).await;
            let _ = this.update(cx, |d, cx| {
                let rt = d.rts.entry(id).or_default();
                rt.loading = false;
                match r {
                    Err(e) => rt.memo.err = Some(e),
                    Ok(Some((text, m))) if rt.memo.text == rt.memo.saved => {
                        rt.memo.text = text.clone();
                        rt.memo.saved = text;
                        rt.memo.modified = Some(m);
                    }
                    _ => {}
                }
                cx.notify();
            });
        }).detach();
    }

    // ---- loads ----

    fn load_lines(&mut self, id: &str, cx: &mut Context<Self>) {
        let Some(b) = self.m.block(id) else { return };
        let path = self.m.path_of(&b.path);
        let rt = self.rts.entry(id.to_string()).or_default();
        if rt.loading || rt.lines.is_some() {
            return;
        }
        rt.loading = true;
        let id = id.to_string();
        cx.spawn(async move |this, cx| {
            let r = cx.background_spawn(async move { preview::read_lines(&path) }).await;
            let _ = this.update(cx, |d, cx| {
                let rt = d.rts.entry(id).or_default();
                rt.loading = false;
                rt.lines = Some(r.map(Arc::new));
                cx.notify();
            });
        }).detach();
    }

    fn load_doc(&mut self, id: &str, cx: &mut Context<Self>) {
        let Some(b) = self.m.block(id) else { return };
        let path = self.m.path_of(&b.path);
        let rt = self.rts.entry(id.to_string()).or_default();
        if rt.loading || rt.doc.is_some() {
            return;
        }
        rt.loading = true;
        let id = id.to_string();
        cx.spawn(async move |this, cx| {
            let r = cx.background_spawn(async move { preview::read_doc(&path) }).await;
            let _ = this.update(cx, |d, cx| {
                let rt = d.rts.entry(id).or_default();
                rt.loading = false;
                rt.doc = Some(r.map(Arc::new));
                cx.notify();
            });
        }).detach();
    }

    fn load_word(&mut self, id: &str, cx: &mut Context<Self>) {
        let Some(b) = self.m.block(id) else { return };
        let path = self.m.path_of(&b.path);
        let rt = self.rts.entry(id.to_string()).or_default();
        if rt.loading || rt.word.is_some() {
            return;
        }
        rt.loading = true;
        let id = id.to_string();
        cx.spawn(async move |this, cx| {
            let r = cx
                .background_spawn(async move {
                    crate::word::read(&path).map(|d| WordDoc {
                        markdown: d.markdown,
                        media: d
                            .media
                            .into_iter()
                            .map(|(name, bytes)| {
                                let img = Arc::new(gpui::Image::from_bytes(image_format(&name), bytes));
                                (name, img)
                            })
                            .collect(),
                    })
                })
                .await;
            let _ = this.update(cx, |d, cx| {
                let rt = d.rts.entry(id).or_default();
                rt.loading = false;
                rt.word = Some(r.map(Arc::new));
                cx.notify();
            });
        })
        .detach();
    }

    fn open_pdf(&mut self, id: &str, cx: &mut Context<Self>) {
        let Some(b) = self.m.block(id) else { return };
        let path = self.m.path_of(&b.path);
        let rt = self.rts.entry(id.to_string()).or_default();
        if rt.loading || rt.pdf.is_some() {
            return;
        }
        rt.loading = true;
        let id = id.to_string();
        cx.spawn(async move |this, cx| {
            let r = cx.background_spawn(async move { crate::pdf::Doc::open(&path) }).await;
            let _ = this.update(cx, |d, cx| {
                let rt = d.rts.entry(id).or_default();
                rt.loading = false;
                rt.pdf = Some(r.map(Arc::new));
                cx.notify();
            });
        })
        .detach();
    }

    /// Renders the page the block shows, unless it shows already or one renders.
    fn render_pdf(&mut self, id: &str, page: usize, width: usize, cx: &mut Context<Self>) {
        let Some(rt) = self.rts.get_mut(id) else { return };
        let Some(Ok(doc)) = rt.pdf.clone() else { return };
        if rt.pdf_rendering || rt.pdf_img.as_ref().is_some_and(|(p, w, _)| *p == page && *w == width) {
            return;
        }
        rt.pdf_rendering = true;
        let id = id.to_string();
        cx.spawn(async move |this, cx| {
            let r = cx
                .background_spawn(async move {
                    doc.render(page, width).and_then(|(w, h, px)| {
                        let buf = image::RgbaImage::from_raw(w as u32, h as u32, px).ok_or("无法绘制这一页")?;
                        Ok(Arc::new(gpui::RenderImage::new(vec![image::Frame::new(buf)])))
                    })
                })
                .await;
            let _ = this.update(cx, |d, cx| {
                let rt = d.rts.entry(id).or_default();
                rt.pdf_rendering = false;
                match r {
                    Ok(img) => rt.pdf_img = Some((page, width, img)),
                    Err(e) => rt.pdf = Some(Err(e)),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn pdf_body(&mut self, b: &Block, t: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let s = self.m.cam.s;
        self.open_pdf(&b.id, cx);
        let doc = match self.rts.get(&b.id).and_then(|r| r.pdf.clone()) {
            None => return message("加载中…".into(), t.muted, s),
            Some(Err(e)) => return message(e, t.danger, s),
            Some(Ok(doc)) => doc,
        };
        let pages = doc.pages();
        let page = (b.page.max(0) as usize).min(pages - 1);
        self.render_pdf(&b.id, page, crate::pdf::width_for(b.w * s), cx);
        let img_el = self.rts.get(&b.id).and_then(|r| r.pdf_img.clone()).map(|(_, _, im)| {
            img(gpui::ImageSource::Render(im)).size_full().object_fit(ObjectFit::Contain)
        });
        let ps = s.min(1.5);
        let turn = |key: &str, glyph: &'static str, to: Option<usize>, cx: &mut Context<Self>| {
            let id = b.id.clone();
            let can = to.is_some_and(|p| p < pages);
            div()
                .id(SharedString::from(format!("{key}:{}", b.id)))
                .px(px(9. * ps))
                .py(px(1. * ps))
                .text_size(px(15. * ps))
                .text_color(if can { t.text } else { t.muted.opacity(0.35) })
                .when(can, |d| d.cursor_pointer().hover(|d| d.bg(t.surface_hover)))
                .child(glyph)
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .on_click(cx.listener(move |d, _, _, cx| {
                    if let (true, Some(to)) = (can, to)
                        && let Some(b) = d.m.block_mut(&id)
                    {
                        b.page = to as i32;
                        d.request_save(cx);
                        cx.notify();
                    }
                }))
        };
        div()
            .size_full()
            .flex()
            .flex_col()
            .bg(pdf_backdrop(t))
            .child(div().flex_1().min_h_0().w_full().p(px(8. * s)).children(img_el))
            .when(pages > 1, |d| {
                d.child(
                    div().w_full().flex().justify_center().pb(px(8. * ps)).child(
                        div()
                            .flex()
                            .flex_row()
                            .items_center()
                            .gap(px(2. * ps))
                            .p(px(2. * ps))
                            .bg(t.surface.opacity(0.92))
                            .border_1()
                            .border_color(t.border)
                            .occlude()
                            .child(turn("pdf-prev", "‹", page.checked_sub(1), cx))
                            .child(
                                div()
                                    .px(px(4. * ps))
                                    .text_size(px(12. * ps))
                                    .text_color(t.muted)
                                    .child(format!("{} / {pages}", page + 1)),
                            )
                            .child(turn("pdf-next", "›", Some(page + 1), cx)),
                    ),
                )
            })
            .into_any_element()
    }

    /// Starts the block's shell: where its last one was, under what it showed.
    fn start_terminal(&mut self, id: &str, cx: &mut Context<Self>) {
        let Some(b) = self.m.block(id) else { return };
        let rt = self.rts.entry(id.to_string()).or_default();
        if rt.term.is_some() || rt.term_err.is_some() {
            return;
        }
        let dir = termstate::term_dir(&self.m.root, b.term.as_ref());
        let history = termstate::restored(b.term.as_ref());
        let (cols, rows) = terminal::grid_for(b.w, b.h - TITLE_H);
        let term = match Terminal::spawn(&dir, cols, rows, &history) {
            Ok(make) => cx.new(make),
            Err(e) => {
                rt.term_err = Some(format!("终端没能启动：{e}"));
                return;
            }
        };
        let eid = id.to_string();
        let sub = cx.subscribe(&term, move |d: &mut DeckView, _, ev: &TerminalEvent, cx| {
            match ev {
                TerminalEvent::Title(t) => {
                    if let Some(b) = d.m.block_mut(&eid) {
                        b.auto_title(t);
                    }
                }
                TerminalEvent::Exited(code) => {
                    if let Some(b) = d.m.block_mut(&eid) {
                        b.title = format!("{} — 已退出 ({code})", b.title);
                    }
                }
                TerminalEvent::Used => {
                    if let Some(rt) = d.rts.get_mut(&eid) {
                        rt.term_used = true;
                    }
                }
            }
            cx.notify();
        });
        rt.term = Some(term);
        rt._term_sub = Some(sub);
    }

    // ---- input ----

    /// A press on empty canvas: the marquee, a double click adds a QuickLook,
    /// the band along the top drags the window.
    fn on_canvas_down(&mut self, ev: &MouseDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let (x, y) = (f32::from(ev.position.x), f32::from(ev.position.y));
        self.popup = None;
        window.focus(&self.focus);
        self.active = None;
        self.stop_editing(cx);
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

    fn on_pan_down(&mut self, ev: &MouseDownEvent, _: &mut Window, cx: &mut Context<Self>) {
        let (x, y) = (f32::from(ev.position.x), f32::from(ev.position.y));
        self.drag = Some(Drag::Pan { from: (x, y), cam: self.m.cam });
        cx.notify();
    }

    fn on_move(&mut self, ev: &MouseMoveEvent, cx: &mut Context<Self>) {
        let (x, y) = (f32::from(ev.position.x), f32::from(ev.position.y));
        self.last_pointer = (x, y);
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

    fn on_up(&mut self, ev: &MouseUpEvent, window: &mut Window, cx: &mut Context<Self>) {
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
                } else if self.title_click_edits(&id) {
                    // A click on a title, without a drag, edits it.
                    self.start_editing(&id, EditPart::Title, window, cx);
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

    /// Whether a click on the block's title edits it: not when it was the tape.
    fn title_click_edits(&self, id: &str) -> bool {
        self.m.cam.s >= LOD_SCALE && self.m.block(id).is_some() && self.pressed_title.as_deref() == Some(id)
    }

    fn on_wheel(&mut self, ev: &ScrollWheelEvent, cx: &mut Context<Self>) {
        let d = ev.delta.pixel_delta(px(20.));
        let (mut dx, mut dy) = (f32::from(d.x), f32::from(d.y));
        let (x, y) = (f32::from(ev.position.x), f32::from(ev.position.y));
        if ev.modifiers.platform || ev.modifiers.control {
            self.zoom_at(x, y, wheel_zoom(-dy), cx);
            cx.notify();
            return;
        }
        if ev.modifiers.shift && dx == 0.0 {
            (dx, dy) = (dy, 0.0);
        }
        // The wheel scrolls content: the canvas moves the other way.
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
        cx.notify();
    }

    fn begin_move(&mut self, id: &str, x: f32, y: f32) {
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

    /// A press on a card: the title bar drags it, a click on a title edits
    /// it, a click on a note's body edits it.
    fn on_card_down(&mut self, id: &str, ev: &MouseDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        cx.stop_propagation();
        self.popup = None;
        let Some(b) = self.m.block(id) else { return };
        let (x, y) = (f32::from(ev.position.x), f32::from(ev.position.y));
        let (_, sy) = self.m.cam.to_screen(b.x, b.y);
        let far = self.m.cam.s < LOD_SCALE;
        let in_title = y - sy <= TITLE_H * self.m.cam.s || far;
        let kind = b.kind;
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
        self.m.sel.clear();
        self.pressed_title = None;
        if in_title {
            self.pressed_title = (!far).then(|| id.to_string());
            self.begin_move(id, x, y);
            window.focus(&self.focus);
        } else {
            match kind {
                BlockKind::Note | BlockKind::Memo => {
                    if self.active.as_deref() == Some(id) || kind == BlockKind::Note {
                        self.start_editing(id, EditPart::Body, window, cx);
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
                _ => {
                    self.active = Some(id.to_string());
                    window.focus(&self.focus);
                }
            }
        }
        cx.notify();
    }

    fn on_tape_down(&mut self, id: &str, ev: &MouseDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        cx.stop_propagation();
        self.popup = None;
        self.stop_editing(cx);
        window.focus(&self.focus);
        self.m.raise(id);
        if ev.modifiers.shift {
            if !self.m.sel.remove(id) {
                self.m.sel.insert(id.to_string());
            }
        } else if !self.m.sel.contains(id) {
            self.m.sel = HashSet::from([id.to_string()]);
        }
        self.pressed_title = None;
        self.begin_move(id, f32::from(ev.position.x), f32::from(ev.position.y));
        cx.notify();
    }

    fn on_resize_down(&mut self, id: &str, ev: &MouseDownEvent, _: &mut Window, cx: &mut Context<Self>) {
        cx.stop_propagation();
        let Some(b) = self.m.block(id) else { return };
        let from = self.m.cam.to_world(f32::from(ev.position.x), f32::from(ev.position.y));
        self.drag = Some(Drag::Resize { id: id.to_string(), from, start: (b.w, b.h), before: Geometry::of(b) });
        cx.notify();
    }

    /// The click on a body's cover that makes the block the one worked in.
    fn on_cover_down(&mut self, id: &str, ev: &MouseDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        cx.stop_propagation();
        self.popup = None;
        if ev.modifiers.shift {
            if !self.m.sel.remove(id) {
                self.m.sel.insert(id.to_string());
            }
            cx.notify();
            return;
        }
        self.stop_editing(cx);
        self.m.sel.clear();
        self.active = Some(id.to_string());
        self.canvas_scroll_until = None;
        match self.rts.get(id).and_then(|r| r.term.as_ref()) {
            Some(t) => window.focus(&t.focus_handle(cx)),
            None => window.focus(&self.focus),
        }
        cx.notify();
    }

    fn expire_notes(&mut self, window: &mut Window, cx: &mut Context<Self>) {
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

    fn block_view(&mut self, id: &str, t: &Theme, window: &mut Window, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let Some(b) = self.m.block(id).cloned() else { return vec![] };
        let s = self.m.cam.s;
        let (sx, sy) = self.m.cam.to_screen(b.x, b.y);
        let (w, h) = (b.w * s, b.h * s);
        let selected = self.m.sel.contains(id);
        let opacity = if b.kind == BlockKind::Note { lifespan::note_opacity(b.expires, self.m.now_ms()) } else { 1.0 };
        let mut out = Vec::new();

        // The tape, behind the card: what sticks out above it shows.
        let tp = tape::tape_of(&b.id, sx, sy, w, s);
        if tp.l >= 24.0 {
            let bx = tp.bounds();
            let held = matches!(&self.drag, Some(Drag::Move { id: d, .. }) if d == id) && self.pressed_title.is_none();
            let eid = id.to_string();
            let dark = t.dark;
            let accent = t.accent;
            out.push(
                div()
                    .id(SharedString::from(format!("tape:{id}")))
                    .absolute()
                    .left(px(bx.0))
                    .top(px(bx.1))
                    .w(px(bx.2))
                    .h(px(bx.3))
                    .opacity(opacity)
                    .occlude()
                    .cursor(if held { gpui::CursorStyle::ClosedHand } else { gpui::CursorStyle::OpenHand })
                    .child(
                        canvas(|_, _, _| (), move |bounds, _, window, _| {
                            let hovered = bounds.contains(&window.mouse_position());
                            tape::paint(
                                window,
                                &tp,
                                f32::from(bounds.origin.x) - bx.0,
                                f32::from(bounds.origin.y) - bx.1,
                                tape::Look { hovered: hovered || held, held, selected, dark, accent },
                            );
                        })
                        .size_full(),
                    )
                    .on_mouse_down(MouseButton::Left, cx.listener(move |d, ev, window, cx| d.on_tape_down(&eid, ev, window, cx)))
                    .into_any_element(),
            );
        }

        let note = b.kind == BlockKind::Note;
        let paper = Paper::of(t);
        let eid = id.to_string();
        let mut card = div()
            .id(SharedString::from(format!("block:{id}")))
            .absolute()
            .left(px(sx))
            .top(px(sy))
            .w(px(w))
            .h(px(h))
            .flex()
            .flex_col()
            .overflow_hidden()
            .occlude()
            .opacity(opacity)
            .on_mouse_down(MouseButton::Left, cx.listener(move |d, ev, window, cx| d.on_card_down(&eid, ev, window, cx)));
        card = if note {
            card.bg(linear_gradient(180., linear_color_stop(paper.top, 0.), linear_color_stop(paper.bottom, 1.)))
                .shadow(vec![gpui::BoxShadow {
                    color: paper.shadow,
                    offset: point(px(0.), px(3. * s.min(1.5))),
                    blur_radius: px(10. * s.min(1.5)),
                    spread_radius: px(0.),
                }])
        } else {
            card.bg(t.surface).shadow(vec![gpui::BoxShadow {
                color: t.shade,
                offset: point(px(0.), px(2. * s.min(1.5))),
                blur_radius: px(8. * s.min(1.5)),
                spread_radius: px(0.),
            }])
        };
        card = if selected {
            card.border(px(2.)).border_color(t.accent)
        } else if note {
            card
        } else {
            card.border_1().border_color(t.border)
        };

        if s < LOD_SCALE {
            // Zoomed out, a card is its frame and its title alone.
            let color = if note { paper.text } else { t.text };
            card = card.child(
                div()
                    .h(px(TITLE_H * s))
                    .px(px(8. * s))
                    .flex()
                    .items_center()
                    .text_size(px(TITLE_FONT * s))
                    .text_color(color)
                    .whitespace_nowrap()
                    .child(b.title.clone()),
            );
            out.push(card.into_any_element());
            return out;
        }

        card = card.child(self.title_bar(&b, t, &paper, window, cx));
        let body = self.body(&b, t, &paper, window, cx);
        let mut body_box = div().relative().flex_1().w_full().min_h_0().overflow_hidden().child(body);
        if let Some(cover) = self.body_cover(&b, cx) {
            body_box = body_box.child(cover);
        }
        card = card.child(body_box);
        if note {
            // The folded corner: the canvas shows where it was, the flap lies on the sheet.
            let fold = 22. * s;
            let (bg, flap) = (t.background, paper.flap);
            card = card.child(
                canvas(|_, _, _| (), move |bounds, _, window, _| {
                    let (x1, y1) = (bounds.right(), bounds.bottom());
                    let f = px(fold);
                    let mut cut = PathBuilder::fill();
                    cut.move_to(point(x1 - f, y1));
                    cut.line_to(point(x1, y1 - f));
                    cut.line_to(point(x1, y1));
                    cut.close();
                    if let Ok(p) = cut.build() {
                        window.paint_path(p, bg);
                    }
                    let mut fl = PathBuilder::fill();
                    fl.move_to(point(x1 - f, y1));
                    fl.line_to(point(x1, y1 - f));
                    fl.line_to(point(x1 - f, y1 - f));
                    fl.close();
                    if let Ok(p) = fl.build() {
                        window.paint_path(p, flap);
                    }
                })
                .absolute()
                .left_0()
                .top_0()
                .size_full(),
            );
        }
        out.push(card.into_any_element());

        // The resize handle, in the bottom-right corner.
        let hs = (22. * s).max(14.);
        if w >= hs * 3. && h >= hs * 3. {
            let eid = id.to_string();
            let grip = t.muted;
            let resizing = matches!(&self.drag, Some(Drag::Resize { id: d, .. }) if d == id);
            out.push(
                div()
                    .id(SharedString::from(format!("resize:{id}")))
                    .absolute()
                    .left(px(sx + w - hs))
                    .top(px(sy + h - hs))
                    .w(px(hs))
                    .h(px(hs))
                    .occlude()
                    .cursor(gpui::CursorStyle::ResizeUpLeftDownRight)
                    .when(!note, |d| {
                        d.child(
                            canvas(|_, _, _| (), move |bounds, _, window, _| {
                                let hovered = bounds.contains(&window.mouse_position()) || resizing;
                                tape::paint_grip(window, bounds, grip.opacity(if hovered { 1.0 } else { 0.45 }));
                            })
                            .size_full(),
                        )
                    })
                    .on_mouse_down(MouseButton::Left, cx.listener(move |d, ev, window, cx| d.on_resize_down(&eid, ev, window, cx)))
                    .into_any_element(),
            );
        }
        out
    }

    fn title_bar(&mut self, b: &Block, t: &Theme, paper: &Paper, _: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let s = self.m.cam.s;
        let note = b.kind == BlockKind::Note;
        let color = if note { paper.text } else { t.text };
        let mut bar = div()
            .h(px(TITLE_H * s))
            .w_full()
            .flex_none()
            .flex()
            .flex_row()
            .items_center()
            .gap(px(6. * s))
            .px(px(if note { 10. } else { 8. } * s))
            .text_size(px(TITLE_FONT * s))
            .text_color(color)
            .when(note, |d| d.bg(paper.header).font_weight(FontWeight::SEMIBOLD));
        if let Some(ed) = self.editing(&b.id, EditPart::Title).cloned() {
            ed.update(cx, |e, _| {
                e.font_size = TITLE_FONT * s;
                e.color = color;
                e.muted = t.muted;
                e.accent = t.accent;
            });
            bar = bar.child(div().flex_1().min_w_0().child(ed));
        } else {
            bar = bar.child(
                div().flex_1().min_w_0().overflow_hidden().whitespace_nowrap().text_ellipsis().child(b.title.clone()),
            );
        }
        if note {
            bar = bar.child(self.life_chip(b, paper, cx));
        } else if b.kind == BlockKind::Chart {
            bar = bar.child(self.chart_controls(b, t, cx));
        } else {
            bar = bar.child(
                div().text_size(px(10. * s)).text_color(t.muted).whitespace_nowrap().child(b.kind.label()),
            );
        }
        bar.into_any_element()
    }

    /// A chart's type chip (a menu of types) and its table button.
    fn chart_controls(&mut self, b: &Block, t: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let s = self.m.cam.s;
        let kind = crate::chart::of(&b.chart);
        let editing = self.editing(&b.id, EditPart::Body).is_some();
        let (id1, id2) = (b.id.clone(), b.id.clone());
        let chip = |id: SharedString| {
            div()
                .id(id)
                .flex()
                .flex_row()
                .items_center()
                .gap(px(4. * s))
                .px(px(6. * s))
                .py(px(2. * s))
                .rounded(px(5. * s))
                .text_size(px(11. * s))
                .text_color(t.muted)
                .hover(|d| d.bg(t.surface_hover))
                .cursor_pointer()
                .occlude()
        };
        div()
            .flex()
            .flex_row()
            .gap(px(2. * s))
            .child(
                chip(SharedString::from(format!("chart-kind:{}", b.id)))
                    .child(svg().path(icons::path(crate::chart::icon(kind))).size(px(12. * s)).text_color(t.muted))
                    .child(crate::chart::label(kind))
                    .child(svg().path(icons::path("chevron_down")).size(px(10. * s)).text_color(t.muted))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |d, ev: &MouseDownEvent, _, cx| {
                            cx.stop_propagation();
                            d.popup = Some(Popup::ChartType {
                                id: id1.clone(),
                                at: (f32::from(ev.position.x), f32::from(ev.position.y) + 10.),
                            });
                            cx.notify();
                        }),
                    ),
            )
            .child(
                chip(SharedString::from(format!("chart-table:{}", b.id)))
                    .when(editing, |d| d.bg(t.accent.opacity(0.15)).text_color(t.accent))
                    .child(svg().path(icons::path("table")).size(px(12. * s)).text_color(if editing { t.accent } else { t.muted }))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |d, _: &MouseDownEvent, window, cx| {
                            cx.stop_propagation();
                            if d.editing(&id2, EditPart::Body).is_some() {
                                d.stop_editing(cx);
                            } else {
                                d.m.sel = HashSet::from([id2.clone()]);
                                d.start_editing(&id2, EditPart::Body, window, cx);
                            }
                        }),
                    ),
            )
            .into_any_element()
    }

    /// How long a QuickLook has left; a click picks another span.
    fn life_chip(&mut self, b: &Block, paper: &Paper, cx: &mut Context<Self>) -> AnyElement {
        let s = self.m.cam.s;
        let label = if b.expires <= 0 { "∞".to_string() } else { lifespan::life_left(b.expires - self.m.now_ms()) };
        let id = b.id.clone();
        div()
            .id(SharedString::from(format!("life:{}", b.id)))
            .flex()
            .flex_row()
            .items_center()
            .gap(px(3. * s))
            .px(px(5. * s))
            .rounded(px(4. * s))
            .text_size(px(10.5 * s))
            .text_color(paper.muted)
            .hover(|d| d.bg(paper.flap.opacity(0.5)))
            .cursor_pointer()
            .occlude()
            .child(svg().path(icons::path("timer")).size(px(11. * s)).text_color(paper.muted))
            .child(label)
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |d, ev: &MouseDownEvent, _, cx| {
                    cx.stop_propagation();
                    d.popup = Some(Popup::Life {
                        id: id.clone(),
                        at: (f32::from(ev.position.x), f32::from(ev.position.y) + 8.),
                    });
                    cx.notify();
                }),
            )
            .into_any_element()
    }

    fn body(&mut self, b: &Block, t: &Theme, paper: &Paper, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let s = self.m.cam.s;
        match b.kind {
            BlockKind::Note => {
                if let Some(ed) = self.editing(&b.id, EditPart::Body).cloned() {
                    ed.update(cx, |e, _| {
                        e.font_size = 14. * s;
                        e.color = paper.text;
                        e.muted = paper.muted;
                        e.accent = t.accent;
                    });
                    return div()
                        .size_full()
                        .pt(px(6. * s))
                        .px(px(8. * s))
                        .pb(px(22. * s))
                        .child(ed)
                        .into_any_element();
                }
                let content = if b.text.is_empty() {
                    div().text_size(px(14. * s)).text_color(paper.muted).child("点击编辑，支持 Markdown").into_any_element()
                } else {
                    markdown::render(&b.text, 14. * s, &MdColors::of_paper(paper))
                };
                live_preview(&b.id, content, [12. * s, 14. * s, 26. * s, 14. * s])
            }
            BlockKind::Memo => self.memo_body(b, t, cx),
            BlockKind::File => self.file_body(b, t, window, cx),
            BlockKind::Terminal => {
                self.start_terminal(&b.id, cx);
                let rt = self.rts.get(&b.id);
                match rt.and_then(|r| r.term.clone()) {
                    Some(term) => {
                        let (cols, rows) = terminal::grid_for(b.w, b.h - TITLE_H);
                        term.update(cx, |tm, cx| {
                            if tm.font_size != terminal::FONT_SIZE * s {
                                tm.font_size = terminal::FONT_SIZE * s;
                                cx.notify();
                            }
                            tm.resize(cols, rows);
                        });
                        div().size_full().bg(terminal::background(t.dark)).child(term).into_any_element()
                    }
                    None => message(
                        rt.and_then(|r| r.term_err.clone()).unwrap_or_else(|| "启动终端…".into()),
                        t.muted,
                        s,
                    ),
                }
            }
            BlockKind::Chart => {
                let chart = crate::chartview::render(&b.text, &b.chart, s, t);
                let Some(ed) = self.editing(&b.id, EditPart::Body).cloned() else { return chart };
                ed.update(cx, |e, _| {
                    e.font_size = 12. * s;
                    e.color = t.text;
                    e.muted = t.muted;
                    e.accent = t.accent;
                    e.mono = true;
                });
                // The table beside the chart, which follows it live; under it when narrow.
                let side = b.w >= 520.;
                let table = div()
                    .flex_none()
                    .p(px(8. * s))
                    .bg(t.surface_hover)
                    .when(side, |d| d.w(px(b.w * s * 0.42)).h_full().border_r_1())
                    .when(!side, |d| d.h(px((b.h - TITLE_H) * s * 0.45)).w_full().border_b_1())
                    .border_color(t.border)
                    .child(ed);
                div()
                    .size_full()
                    .flex()
                    .when(side, |d| d.flex_row())
                    .when(!side, |d| d.flex_col())
                    .child(table)
                    .child(div().flex_1().min_w_0().min_h_0().child(chart))
                    .into_any_element()
            }
            BlockKind::Stack => self.stack_body(b, t, window, cx),
            BlockKind::Browser => self.browser_body(b, t, window, cx),
        }
    }

    fn memo_body(&mut self, b: &Block, t: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let s = self.m.cam.s;
        let size = DOC_FONT * s;
        let editing = self.editing(&b.id, EditPart::Body).cloned();
        let rt = self.rts.entry(b.id.clone()).or_default();
        if let Some(err) = &rt.memo.err {
            return message(err.clone(), t.danger, s);
        }
        if !rt.memo.loaded {
            self.load_memo(&b.id, cx);
            return message("加载中…".into(), t.muted, s);
        }
        if let Some(ed) = editing {
            // The text may have arrived after the edit began.
            let text = rt.memo.text.clone();
            ed.update(cx, |e, cx| {
                if e.text.is_empty() && !text.is_empty() {
                    e.set_text(text, cx);
                }
                e.font_size = size;
                e.color = t.text;
                e.muted = t.muted;
                e.accent = t.accent;
            });
            return div().size_full().py(px(8. * s)).px(px(12. * s)).child(ed).into_any_element();
        }
        let save_err = rt.memo.save_err.clone();
        let text = rt.memo.text.clone();
        self.check_memo(&b.id, cx);
        let colors = MdColors::of_theme(t);
        let content = div()
            .flex()
            .flex_col()
            .gap(px(6. * s))
            .when_some(save_err, |d, e| d.child(div().text_size(px(12. * s)).text_color(t.danger).child(e)))
            .child(if text.trim().is_empty() {
                div().text_size(px(size)).text_color(t.muted).child("点击开始写，支持 Markdown").into_any_element()
            } else {
                markdown::render(&text, size, &colors)
            })
            .into_any_element();
        live_preview(&b.id, content, [12. * s, 16. * s, 12. * s, 16. * s])
    }

    fn file_body(&mut self, b: &Block, t: &Theme, _: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let s = self.m.cam.s;
        let path = self.m.path_of(&b.path);
        match preview::of(&b.path) {
            preview::Kind::Image => div()
                .size_full()
                .child(img(path).size_full().object_fit(ObjectFit::Contain))
                .into_any_element(),
            preview::Kind::Markdown => {
                self.load_doc(&b.id, cx);
                let base = path.parent().map(|p| p.to_path_buf()).unwrap_or_default();
                match self.rts.get(&b.id).and_then(|r| r.doc.clone()) {
                    None => message("加载中…".into(), t.muted, s),
                    Some(Err(e)) => message(e, t.danger, s),
                    Some(Ok(text)) => live_preview(
                        &b.id,
                        markdown::render_with(&text, DOC_FONT * s, &MdColors::of_theme(t), &|url| {
                            if url.starts_with("http://") || url.starts_with("https://") {
                                return None;
                            }
                            let p = std::path::Path::new(url);
                            Some(gpui::ImageSource::from(if p.is_absolute() { p.to_path_buf() } else { base.join(p) }))
                        }),
                        [12. * s, 16. * s, 12. * s, 16. * s],
                    ),
                }
            }
            preview::Kind::Word => {
                self.load_word(&b.id, cx);
                match self.rts.get(&b.id).and_then(|r| r.word.clone()) {
                    None => message("加载中…".into(), t.muted, s),
                    Some(Err(e)) => message(e, t.danger, s),
                    Some(Ok(doc)) if doc.markdown.trim().is_empty() => message("（空文档）".into(), t.muted, s),
                    Some(Ok(doc)) => {
                        let d2 = doc.clone();
                        live_preview(
                            &b.id,
                            markdown::render_with(&doc.markdown, DOC_FONT * s, &MdColors::of_theme(t), &move |url| {
                                let name = url.strip_prefix(crate::word::MEDIA_SCHEME)?;
                                d2.media.get(name).map(|im| gpui::ImageSource::Image(im.clone()))
                            }),
                            [12. * s, 16. * s, 16. * s, 16. * s],
                        )
                    }
                }
            }
            preview::Kind::Pdf => self.pdf_body(b, t, cx),
            preview::Kind::Text => {
                self.load_lines(&b.id, cx);
                let rt = self.rts.entry(b.id.clone()).or_default();
                match rt.lines.clone() {
                    None => message("加载中…".into(), t.muted, s),
                    Some(Err(e)) => message(e, t.danger, s),
                    Some(Ok(lines)) => {
                        let size = EDITOR_FONT * s;
                        let lh = (size * 1.5).round().max(1.);
                        let digits = lines.len().to_string().len() as f32;
                        let (muted, text) = (t.muted, t.text);
                        let handle = rt.list.clone();
                        uniform_list(SharedString::from(format!("text:{}", b.id)), lines.len(), move |range, _, _| {
                            range
                                .map(|i| {
                                    div()
                                        .h(px(lh))
                                        .flex()
                                        .flex_row()
                                        .whitespace_nowrap()
                                        .child(
                                            div()
                                                .w(px(size * 0.62 * digits + size))
                                                .flex_none()
                                                .text_color(muted)
                                                .child(format!("{}", i + 1)),
                                        )
                                        .child(div().text_color(text).child(lines[i].clone()))
                                })
                                .collect()
                        })
                        .track_scroll(handle)
                        .size_full()
                        .px(px(6. * s))
                        .py(px(4. * s))
                        .font_family("Menlo")
                        .text_size(px(size))
                        .into_any_element()
                    }
                }
            }
        }
    }

    /// Covers a scrolling body while it is not the block worked in, or while
    /// a scroll the canvas took goes on: the wheel then moves the canvas, and
    /// a click makes the block the one worked in.
    fn body_cover(&self, b: &Block, cx: &mut Context<Self>) -> Option<AnyElement> {
        let scrolls = match b.kind {
            BlockKind::Terminal | BlockKind::Note | BlockKind::Memo => true,
            BlockKind::File => matches!(preview::of(&b.path), preview::Kind::Text | preview::Kind::Markdown | preview::Kind::Word),
            _ => false,
        };
        if !scrolls || self.editing(&b.id, EditPart::Body).is_some() {
            return None;
        }
        let gesture = self.canvas_scroll_until.is_some_and(|t| t > Instant::now());
        if self.active.as_deref() == Some(&b.id) && !gesture {
            return None;
        }
        let id = b.id.clone();
        Some(
            div()
                .id(SharedString::from(format!("cover:{}", b.id)))
                .absolute()
                .left_0()
                .top_0()
                .size_full()
                .occlude()
                .on_mouse_down(MouseButton::Left, cx.listener(move |d, ev, window, cx| d.on_cover_down(&id, ev, window, cx)))
                .into_any_element(),
        )
    }

    fn toolbar(&self, t: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let item = |id: &'static str, icon: &'static str, label: &'static str, kind: BlockKind, cx: &mut Context<Self>| {
            tool_button(id, icon, label, t).on_click(cx.listener(move |d, _, window, cx| {
                let (x, y) = (d.view.0 / 2.0, d.view.1 / 2.0);
                d.add_kind_at(kind, x, y, window, cx);
            }))
        };
        div()
            .id("toolbar")
            .absolute()
            .top(px(CHROME_TOP))
            .right(px(12.))
            .flex()
            .flex_row()
            .gap(px(2.))
            .p(px(5.))
            .bg(t.surface)
            .border_1()
            .border_color(t.border)
            .rounded(px(12.))
            .shadow(vec![shadow(t)])
            .occlude()
            .child(item("new-note", "note", "QuickLook", BlockKind::Note, cx))
            .child(item("new-memo", "memo", "笔记", BlockKind::Memo, cx))
            .child(item("new-file", "file", "文件", BlockKind::File, cx))
            .child(item("new-term", "terminal", "终端", BlockKind::Terminal, cx))
            .child(item("new-browser", "globe", "浏览器", BlockKind::Browser, cx))
            .child(item("new-chart", "chart", "图表", BlockKind::Chart, cx))
            .child(item("new-stack", "stack", "块堆", BlockKind::Stack, cx))
            .into_any_element()
    }

    fn home_pill(&self, t: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let name = model::file_name(&self.m.root.to_string_lossy());
        div()
            .id("home-pill")
            .absolute()
            .top(px(CHROME_TOP))
            .left(px(CHROME_LEFT))
            .flex()
            .flex_row()
            .items_center()
            .gap(px(2.))
            .p(px(5.))
            .bg(t.surface)
            .border_1()
            .border_color(t.border)
            .rounded(px(12.))
            .shadow(vec![shadow(t)])
            .occlude()
            .child(
                div()
                    .id("go-home")
                    .px(px(7.))
                    .py(px(5.))
                    .rounded(px(8.))
                    .hover(|d| d.bg(t.surface_hover))
                    .cursor_pointer()
                    .child(svg().path(icons::path("back")).size(px(15.)).text_color(t.text))
                    .on_click(cx.listener(|_, _, _, cx| cx.emit(DeckEvent::Home))),
            )
            .child(
                div()
                    .id("workspace")
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(6.))
                    .px(px(8.))
                    .py(px(5.))
                    .rounded(px(8.))
                    .hover(|d| d.bg(t.surface_hover))
                    .cursor_pointer()
                    .child(svg().path(icons::path("folder")).size(px(15.)).text_color(t.text))
                    .child(div().text_size(px(12.)).font_weight(FontWeight::SEMIBOLD).text_color(t.text).child(name))
                    .child(svg().path(icons::path("chevron_down")).size(px(13.)).text_color(t.muted))
                    .on_click(cx.listener(|d, _, _, cx| {
                        d.popup = match d.popup {
                            Some(Popup::Workspaces) => None,
                            _ => Some(Popup::Workspaces),
                        };
                        cx.notify();
                    })),
            )
            .into_any_element()
    }

    fn popup_view(&self, t: &Theme, cx: &mut Context<Self>) -> Option<AnyElement> {
        let popup = self.popup.clone()?;
        let menu = |x: f32, y: f32| {
            div()
                .id("popup")
                .absolute()
                .left(px(x))
                .top(px(y))
                .min_w(px(180.))
                .flex()
                .flex_col()
                .p(px(4.))
                .bg(t.surface)
                .border_1()
                .border_color(t.border)
                .rounded(px(8.))
                .shadow(vec![shadow(t)])
                .occlude()
                .text_size(px(13.))
                .text_color(t.text)
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        };
        let row = |id: SharedString, label: String, checked: bool| {
            div()
                .id(id)
                .flex()
                .flex_row()
                .gap(px(8.))
                .px(px(10.))
                .py(px(5.))
                .rounded(px(5.))
                .hover(|d| d.bg(t.accent).text_color(t.accent_text))
                .cursor_pointer()
                .child(div().w(px(12.)).child(if checked { "✓" } else { "" }))
                .child(label)
        };
        Some(match popup {
            Popup::Workspaces => {
                let root = self.m.root.to_string_lossy().into_owned();
                let mut paths = self.workspaces.clone();
                if !paths.contains(&root) {
                    paths.insert(0, root.clone());
                }
                let mut m = menu(CHROME_LEFT, CHROME_TOP + 44.);
                for (i, p) in paths.into_iter().take(12).enumerate() {
                    let label = format!(
                        "{}    {}",
                        model::file_name(&p),
                        short_path(&std::path::Path::new(&p).parent().map(|d| d.to_string_lossy().into_owned()).unwrap_or_default())
                    );
                    let here = p == root;
                    m = m.child(row(SharedString::from(format!("ws:{i}")), label, here).on_click(cx.listener(
                        move |d, _, _, cx| {
                            d.popup = None;
                            if !here {
                                cx.emit(DeckEvent::Open(p.clone()));
                            }
                            cx.notify();
                        },
                    )));
                }
                m.child(div().h(px(1.)).my(px(4.)).bg(t.border))
                    .child(row("ws:pick".into(), "打开文件夹…".into(), false).on_click(cx.listener(|d, _, _, cx| {
                        d.popup = None;
                        cx.emit(DeckEvent::Pick);
                        cx.notify();
                    })))
                    .into_any_element()
            }
            Popup::Life { id, at } => {
                let expires = self.m.block(&id).map(|b| b.expires).unwrap_or(0);
                let mut m = menu(at.0 - 90., at.1);
                for (label, span) in LIFE_CHOICES {
                    let id = id.clone();
                    m = m.child(
                        row(SharedString::from(format!("life:{label}")), label.into(), span == 0 && expires == 0)
                            .on_click(cx.listener(move |d, _, _, cx| {
                                let now = d.m.now_ms();
                                if let Some(b) = d.m.block_mut(&id) {
                                    b.expires = if span <= 0 { 0 } else { now + span };
                                }
                                d.popup = None;
                                d.request_save(cx);
                                cx.notify();
                            })),
                    );
                }
                m.into_any_element()
            }
            Popup::ChartType { id, at } => {
                let now = self.m.block(&id).map(|b| crate::chart::of(&b.chart));
                let mut m = menu(at.0 - 60., at.1);
                for (kind, name, label, _) in crate::chart::TYPES {
                    let id = id.clone();
                    m = m.child(
                        row(SharedString::from(format!("chart:{name}")), label.into(), now == Some(kind)).on_click(
                            cx.listener(move |d, _, _, cx| {
                                if let Some(b) = d.m.block_mut(&id) {
                                    b.chart = name.to_string();
                                }
                                d.popup = None;
                                d.request_save(cx);
                                cx.notify();
                            }),
                        ),
                    );
                }
                m.into_any_element()
            }
        })
    }

    fn status_bar(&self, t: &Theme) -> AnyElement {
        div()
            .absolute()
            .left(px(12.))
            .bottom(px(10.))
            .flex()
            .flex_row()
            .gap(px(10.))
            .px(px(8.))
            .py(px(4.))
            .bg(t.surface)
            .border_1()
            .border_color(t.border)
            .rounded(px(9.))
            .text_size(px(11.))
            .text_color(t.muted)
            .child(format!("{:.0}%", self.m.cam.s * 100.))
            .child(format!("{} 个 block · 选中 {}", self.m.blocks.len(), self.m.sel.len()))
            .child(match (&self.mcp, &self.mcp_err) {
                (Some(_), None) => div().child(format!("MCP · {} 个工具", crate::mcp::tools::tool_names().len())),
                (Some(_), Some(e)) => div().text_color(t.danger).child(format!("MCP · {e}")),
                (None, Some(e)) => div().text_color(t.danger).child(format!("MCP 未启动：{e}")),
                (None, None) => div(),
            })
            .into_any_element()
    }
}

/// A QuickLook's or 笔记's body as it reads, in a scroller of its own.
fn live_preview(id: &str, content: AnyElement, pad: [f32; 4]) -> AnyElement {
    div()
        .id(SharedString::from(format!("live:{id}")))
        .size_full()
        .overflow_y_scroll()
        .child(div().pt(px(pad[0])).pr(px(pad[1])).pb(px(pad[2])).pl(px(pad[3])).child(content))
        .into_any_element()
}

fn message(text: String, color: Hsla, s: f32) -> AnyElement {
    div().size_full().p(px(10. * s)).text_size(px(12. * s)).text_color(color).child(text).into_any_element()
}

fn shadow(t: &Theme) -> gpui::BoxShadow {
    gpui::BoxShadow { color: t.shade, offset: point(px(0.), px(4.)), blur_radius: px(16.), spread_radius: px(0.) }
}

fn tool_button(id: &'static str, icon: &'static str, label: &'static str, t: &Theme) -> Stateful<Div> {
    div()
        .id(id)
        .flex()
        .flex_row()
        .items_center()
        .gap(px(6.))
        .px(px(10.))
        .py(px(5.))
        .rounded(px(8.))
        .hover(|d| d.bg(t.surface_hover))
        .cursor_pointer()
        .text_size(px(12.))
        .text_color(t.text)
        .child(svg().path(icons::path(icon)).size(px(15.)).text_color(t.text))
        .child(label)
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
    }
}

impl Render for DeckView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = Theme::of(window);
        let vs = window.viewport_size();
        self.view = (f32::from(vs.width), f32::from(vs.height));
        self.track_active(window, cx);
        self.expire_notes(window, cx);
        self.hang_up_gone(cx);
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
        let mut blocks = Vec::new();
        for id in &visible {
            blocks.extend(self.block_view(id, &t, window, cx));
        }

        self.hide_unshown_pages();

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
            .on_action(cx.listener(|d, _: &NewBrowser, w, cx| {
                let (x, y) = d.pointer_or_middle(w);
                d.add_kind_at(BlockKind::Browser, x, y, w, cx)
            }))
            .on_action(cx.listener(|_, _: &GoHome, _, cx| cx.emit(DeckEvent::Home)))
            .on_key_down(cx.listener(Self::stack_keys))
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
            .child(self.home_pill(&t, cx))
            .child(self.toolbar(&t, cx))
            .child(self.status_bar(&t))
            .children(self.stack_preview(&t, window, cx))
            .children(self.pluck_view(&t))
            .children(self.popup_view(&t, cx))
    }
}
