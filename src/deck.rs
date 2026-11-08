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
use std::rc::Rc;
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
mod lifecycle;
mod popin;
mod sharpen;
mod stack;
mod zoomlayer;

use crate::editor::{Editor, EditorEvent};
use crate::lifespan::{self, LIFE_CHOICES};
use crate::markdown;
use crate::memo;
use crate::model::{
    self, Block, BlockKind, Camera, Geometry, LOD_SCALE, Model, file_block_size, new_block_of, scale_keeping_ratio,
    wheel_zoom,
};
use crate::recents::short_path;
use crate::tape;
use crate::terminal::{self, Terminal, TerminalEvent};
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
/// How many blocks may come back from LOD to full content in one frame: zooming
/// in past `LOD_SCALE`, the rest wait a frame or two under their mask, so the
/// layout, shaping and starting of them all is spread out instead of one stall.
const LOD_WAKE_PER_FRAME: usize = 2;

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
    pdf: Option<Result<Arc<crate::pdf::Doc>, String>>,
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
    chart_cache: Option<(String, Arc<crate::chart::Data>)>,
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
    /// The body fading in on coming back from LOD.
    appear: sharpen::Sharpen,
    /// A sharper version of the content (a bitmap, a native view) fading in over the old.
    sharpen: sharpen::Sharpen,
    /// The entrance of a block just made.
    pop: popin::PopIn,
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
    /// The camera the blocks layer is currently laid out at; frozen during zoom gestures.
    pub lay: model::Camera,
    /// Until when a wheel/pinch zoom is considered in-flight.
    pub zoom_settle: Option<Instant>,
    snap: zoomlayer::ZoomSnap,
    zoom_timer_active: bool,
    /// The text level held while a zoom gesture is under way (`content_scale`).
    text_s: Option<f32>,
    /// Blocks still allowed out of LOD this frame (`LOD_WAKE_PER_FRAME`).
    lod_wake: usize,
    /// Something drawn this frame is fading: the next frame is asked for.
    animating: bool,
    /// A panel of the window's (the welcome) lies over the deck: web pages give way to their pictures.
    pub covered: bool,
    /// The camera gliding to where an agent asked the view to be: from, to, since.
    glide: Option<(Camera, Camera, Instant)>,
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
    /// The tabs whose pages this frame showed, by key.
    web_shown: HashSet<u64>,
    /// Where this frame lays the blocks out, back to front: what covers a page.
    frame_rects: Vec<(String, (f32, f32, f32, f32))>,
    /// The resize handles this frame drew, in window points: a press on one
    /// over a web page comes to the deck (see `webfocus`).
    resize_corners: Vec<[f32; 4]>,
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
        // Pinches on the trackpad zoom about the pointer (pinch.rs); a deck
        // taking over from another drains what that one left.
        let pinches = crate::pinch::channel();
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
                    cx.notify();
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
        let lay = m.cam;
        DeckView {
            m,
            lay,
            zoom_settle: None,
            snap: zoomlayer::ZoomSnap::default(),
            zoom_timer_active: false,
            text_s: None,
            lod_wake: LOD_WAKE_PER_FRAME,
            animating: false,
            covered: false,
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
            save_seq: Arc::new(Mutex::new(0)),
            mcp: None,
            mcp_err: None,
            bridge: None,
            project_mcp: false,
            web_tx,
            web_shown: HashSet::new(),
            frame_rects: Vec::new(),
            resize_corners: Vec::new(),
            zoomed_at: None,
        }
    }

    // ---- the MCP endpoint ----

    /// Serves this deck to agents: its socket, its descriptor, the active-deck registry.
    pub fn start_mcp(&mut self, cx: &mut Context<Self>) {
        use crate::mcp::{self, rpc::Job};
        type Reply = std::sync::mpsc::SyncSender<Result<serde_json::Value, String>>;
        let (tx, rx) = smol::channel::unbounded::<(Job, mcp::Caller, Reply)>();
        // Each job runs on the UI thread, in the deck's own add/remove/undo path.
        cx.spawn(async move |this, cx| {
            while let Ok((job, caller, reply)) = rx.recv().await {
                let out = this
                    .update(cx, |d, cx| d.run_job(job, &caller, cx))
                    .unwrap_or_else(|_| Err("这个工作区已经关了".into()));
                let _ = reply.send(out);
            }
        })
        .detach();
        let run: Arc<mcp::server::DeckRunner> = Arc::new(move |job, caller| {
            let (reply, answer) = std::sync::mpsc::sync_channel(1);
            tx.send_blocking((job, caller.clone(), reply)).map_err(|_| "这个工作区已经关了".to_string())?;
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

    fn run_job(
        &mut self,
        job: crate::mcp::rpc::Job,
        caller: &crate::mcp::Caller,
        cx: &mut Context<Self>,
    ) -> Result<serde_json::Value, String> {
        use crate::mcp::{rpc::Job, tools::Ctx};
        let here = crate::mcp::tools::Here {
            caller: caller.clone(),
            focused: self.focus_target(),
        };
        let info = match &self.mcp {
            Some(s) => serde_json::json!({
                "running": true, "socket": s.socket.to_string_lossy(), "startedAt": s.started.to_rfc3339(),
                "tools": crate::mcp::tools::tool_names().len(), "configFile": ".boundlessdeck/mcp.json",
            }),
            None => serde_json::json!({"running": false, "error": self.mcp_err}),
        };
        // Mid-glide, tools see the camera where it is going.
        let shown = self.glide.map(|(_, to, _)| std::mem::replace(&mut self.m.cam, to));
        let mut ctx = Ctx { m: &mut self.m, view: self.view, mcp: info, here, dirty: false, memos: vec![], reload: vec![], view_from: None };
        let out = match job {
            Job::Tool(name, args) => ctx.call(&name, &args),
            Job::Resource(uri) => ctx.read_resource(&uri),
        };
        let (dirty, memos, reload, view_from) = (ctx.dirty, ctx.memos, ctx.reload, ctx.view_from);
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
                rt.md_cache = None;
                rt.face_md_cache = None;
                rt.chart_cache = None;
                rt.memo = MemoRt::default();
                // The open tab's page goes again where it was sent; the other tabs keep theirs.
                match self.m.block(&id).and_then(|b| b.tabs.get(b.tab)) {
                    Some(tab) => {
                        rt.webs.remove(&tab.key);
                    }
                    None => rt.webs.clear(),
                }
                rt.web_err = None;
            }
        }
        if dirty {
            self.after_edit(cx);
        }
        // Saved where it goes (above), then shown going there from where it shows now.
        match (view_from, shown) {
            (Some((from, animate)), shown) => {
                let to = self.m.cam;
                self.glide_to(shown.unwrap_or(from), to, animate, cx);
            }
            (None, Some(cam)) => self.m.cam = cam,
            (None, None) => {}
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

    /// Ends every block's stay (`on_close`: notes written, shells hung up), then saves.
    pub fn close(&mut self, cx: &mut Context<Self>) {
        self.stop_mcp();
        self.life_delete(cx);
        self.life_close(cx);
        self.stop_editing(cx);
        *self.save_seq.lock().unwrap() += 1; // no write under way overtakes this one
        let _ = self.m.save();
    }

    /// Hangs up the block's shell, if it has one.
    fn hang_up(&mut self, id: &str, cx: &mut Context<Self>) {
        if let Some(t) = self.rts.get(id).and_then(|r| r.term.clone()) {
            t.update(cx, |t, _| t.close());
        }
    }

    // ---- the camera ----

    /// Moves the camera to `to`, gliding unless asked not to (or the system
    /// asks for less motion). The person's own pan or zoom stops a glide.
    fn glide_to(&mut self, from: Camera, to: Camera, animate: bool, cx: &mut Context<Self>) {
        if animate && !sharpen::reduce_motion() && self.view.0 > 0. {
            self.m.cam = from;
            self.glide = Some((from, to, Instant::now()));
        } else {
            self.m.cam = to;
            self.lay = to;
            self.glide = None;
        }
        cx.notify();
    }

    /// One frame of a glide: the view's center goes straight there, the
    /// scale eases between the two, dipping out on a long way so the trip
    /// shows where it goes.
    fn tick_glide(&mut self, cx: &mut Context<Self>) {
        let Some((from, to, at)) = self.glide else { return };
        let f = (at.elapsed().as_secs_f32() / GLIDE_TIME.as_secs_f32()).min(1.);
        if f >= 1. {
            self.m.cam = to;
            self.glide = None;
            if from.s != to.s {
                self.note_zoom(cx);
            }
            self.request_save(cx);
            return;
        }
        self.m.cam = glide_at(from, to, self.view, f);
        if self.m.cam.s != from.s {
            self.note_zoom(cx);
        }
        self.animating = true;
    }

    fn zoom_at(&mut self, x: f32, y: f32, factor: f32, cx: &mut Context<Self>) {
        self.glide = None;
        self.last_pointer = (x, y);
        self.zoom_anchor = (x, y);
        self.zoom_gen += 1;
        self.m.zoom_at(x, y, factor);
        self.note_zoom(cx);
    }

    fn fit_all(&mut self, _: &FitAll, _: &mut Window, cx: &mut Context<Self>) {
        self.m.fit_all(self.view.0, self.view.1);
        self.lay = self.m.cam;
        self.zoom_settle = None;
        self.snap.on = false;
        cx.notify();
    }
    fn zoom_in(&mut self, _: &ZoomIn, _: &mut Window, cx: &mut Context<Self>) {
        self.m.zoom_step(self.view.0 / 2.0, self.view.1 / 2.0, 1);
        self.lay = self.m.cam;
        self.zoom_settle = None;
        self.snap.on = false;
        cx.notify();
    }
    fn zoom_out(&mut self, _: &ZoomOut, _: &mut Window, cx: &mut Context<Self>) {
        self.m.zoom_step(self.view.0 / 2.0, self.view.1 / 2.0, -1);
        self.lay = self.m.cam;
        self.zoom_settle = None;
        self.snap.on = false;
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
            // 笔记 and QuickLook edit Typora-style: rendered but for the lines being edited.
            e.markdown = part == EditPart::Body && matches!(kind, BlockKind::Memo | BlockKind::Note);
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
        crate::webfocus::release();
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
                    b.ensure_tabs();
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

    /// Writes what was typed in the note, here and now: it pauses, leaves, or the deck closes.
    /// Takes the block as it is or was, as one that left the deck is no longer in the model.
    fn flush_memo(&mut self, b: &Block) {
        if b.kind != BlockKind::Memo || b.path.is_empty() {
            return;
        }
        let path = self.m.path_of(&b.path);
        if let Some(rt) = self.rts.get_mut(&b.id)
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
                    Ok(img) => {
                        // The sharper page fades in over the one it replaces.
                        rt.pdf_prev = rt.pdf_img.take().map(|(_, _, old)| old);
                        rt.pdf_img = Some((page, width, img));
                        rt.sharpen.begin(Instant::now());
                    }
                    Err(e) => rt.pdf = Some(Err(e)),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn pdf_body(&mut self, b: &Block, t: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let s = self.content_scale();
        self.open_pdf(&b.id, cx);
        let doc = match self.rts.get(&b.id).and_then(|r| r.pdf.clone()) {
            None => return message("加载中…".into(), t.muted, s),
            Some(Err(e)) => return message(e, t.danger, s),
            Some(Ok(doc)) => doc,
        };
        let pages = doc.pages();
        let page = (b.page.max(0) as usize).min(pages - 1);
        let has_img = self.rts.get(&b.id).is_some_and(|r| r.pdf_img.is_some());
        if !self.zooming() || !has_img {
            let render_s = if self.lay.s > 0.0 { self.lay.s } else { s };
            self.render_pdf(&b.id, page, crate::pdf::width_for(b.w * render_s), cx);
        }
        let now = Instant::now();
        let rt = self.rts.entry(b.id.clone()).or_default();
        let fading = rt.sharpen.tick(now);
        if !fading {
            rt.pdf_prev = None;
        }
        let alpha = rt.sharpen.alpha(now);
        let pic = |im| img(gpui::ImageSource::Render(im)).size_full().object_fit(ObjectFit::Contain).into_any_element();
        let img_el = rt.pdf_img.clone().map(|(_, _, im)| {
            if fading { sharpen::crossfade(rt.pdf_prev.clone().map(pic), pic(im), alpha).into_any_element() } else { pic(im) }
        });
        self.animating |= fading;
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

    /// Starts the block's shell in the deck's folder.
    fn start_terminal(&mut self, id: &str, cx: &mut Context<Self>) {
        let Some(b) = self.m.block(id) else { return };
        let rt = self.rts.entry(id.to_string()).or_default();
        if rt.term.is_some() || rt.term_err.is_some() {
            return;
        }
        let (cols, rows) = terminal::grid_for(b.w, b.h - TITLE_H);
        let term = match Terminal::spawn(&self.m.root, cols, rows, id) {
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
        self.focus_block(None, window, cx);
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
        self.glide = None;
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

    fn on_up(&mut self, ev: &MouseUpEvent, _: &mut Window, cx: &mut Context<Self>) {
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

    fn on_wheel(&mut self, ev: &ScrollWheelEvent, cx: &mut Context<Self>) {
        self.glide = None;
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

    /// A press on a card: the title bar selects and drags it, a double click
    /// on it edits the title; a click on a QuickLook's body edits it, on a
    /// 笔记's focuses it (a double click edits it).
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
        if in_title {
            if ev.click_count >= 2 && !far {
                self.start_editing(id, EditPart::Title, window, cx);
                return;
            }
            self.m.raise(id);
            if !self.m.sel.contains(id) {
                self.m.sel = HashSet::from([id.to_string()]);
            }
            self.begin_move(id, x, y);
            window.focus(&self.focus);
            cx.notify();
        } else {
            self.m.sel.clear();
            match kind {
                BlockKind::Note => self.start_editing(id, EditPart::Body, window, cx),
                // 笔记: a click focuses it, a double click edits it.
                BlockKind::Memo => {
                    if ev.click_count >= 2 {
                        self.start_editing(id, EditPart::Body, window, cx);
                    } else if self.editing(id, EditPart::Body).is_none() {
                        self.focus_block(Some(id), window, cx);
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
                _ => self.focus_block(Some(id), window, cx),
            }
        }
        cx.notify();
    }

    /// Makes `id` the block worked in, or none: the one way a block gains or
    /// loses focus. Editing elsewhere ends, and the keyboard goes to the
    /// block's terminal or to the canvas. Nothing else changes: losing focus
    /// never hides, reloads or redraws a block's content, so no kind needs
    /// blur handling of its own. (A page clicked into gives back the keyboard
    /// and the wheel on the next press elsewhere, in `webfocus`.)
    fn focus_block(&mut self, id: Option<&str>, window: &mut Window, cx: &mut Context<Self>) {
        if self.editing.as_ref().is_some_and(|e| Some(e.id.as_str()) != id) {
            self.stop_editing(cx);
        }
        self.active = id.map(str::to_string);
        crate::webfocus::release();
        // The canvas takes the keyboard unless the block's `on_focus` does; one
        // focused already keeps it.
        if !id.is_some_and(|i| self.rts.get(i).is_some_and(|r| r.life.focused)) {
            window.focus(&self.focus);
        }
        self.sync_focus(window, cx);
    }

    /// Whether a canvas gesture under way changes what lies at r (window
    /// points): a marquee grown over it, or a pan that has moved the canvas.
    /// A press alone changes nothing, so it hides no page.
    fn gesture_covers(&self, r: (f32, f32, f32, f32)) -> bool {
        match &self.drag {
            Some(Drag::Pan { cam, .. }) => cam.x != self.m.cam.x || cam.y != self.m.cam.y,
            Some(Drag::Marquee { from, to }) => marquee_over(self.m.cam, *from, *to, r),
            _ => false,
        }
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
        self.m.sel.clear();
        self.canvas_scroll_until = None;
        self.focus_block(Some(id), window, cx);
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

    pub(super) fn parsed_markdown(&mut self, id: &str, src: &str, c: &MdColors, dark: bool) -> markdown::Parsed {
        let rt = self.rts.entry(id.to_string()).or_default();
        if let Some((cached_src, parsed)) = &rt.md_cache
            && cached_src == src
            && parsed.is_dark() == dark
        {
            return parsed.clone();
        }
        let parsed = markdown::parse_doc(src, c, dark);
        rt.md_cache = Some((src.to_string(), parsed.clone()));
        parsed
    }

    pub(super) fn face_parsed_markdown(
        &mut self,
        b: &Block,
        c: &MdColors,
        dark: bool,
        cx: &mut Context<Self>,
    ) -> Option<markdown::Parsed> {
        let md = self.face_markdown(b, cx);
        if md.is_empty() {
            return None;
        }
        let rt = self.rts.entry(b.id.clone()).or_default();
        if let Some((cached_src, parsed)) = &rt.face_md_cache
            && cached_src == &md
            && parsed.is_dark() == dark
        {
            return Some(parsed.clone());
        }
        let parsed = markdown::parse_doc(&md, c, dark);
        rt.face_md_cache = Some((md, parsed.clone()));
        Some(parsed)
    }

    /// A Markdown body as it reads, in a list of its blocks that lays out only
    /// those in sight: a long document costs what shows, also while zooming.
    /// Its scroll stays where it was when the text changes.
    #[allow(clippy::too_many_arguments)]
    fn markdown_list(
        &mut self,
        id: &str,
        parsed: markdown::Parsed,
        size: f32,
        colors: MdColors,
        images: Rc<ImageFinder>,
        pad: [f32; 4],
        outer_w: f32,
        toggle: Option<markdown::OnToggle>,
    ) -> AnyElement {
        let rt = self.rts.entry(id.to_string()).or_default();
        let state = match &mut rt.md_list {
            Some((st, p)) if p.same(&parsed) => st.clone(),
            Some((st, p)) => {
                let top = st.logical_scroll_top();
                st.reset(parsed.len());
                st.scroll_to(gpui::ListOffset { item_ix: top.item_ix.min(parsed.len().saturating_sub(1)), offset_in_item: top.offset_in_item });
                *p = parsed.clone();
                st.clone()
            }
            None => {
                let st = gpui::ListState::new(parsed.len(), gpui::ListAlignment::Top, px(100.));
                rt.md_list = Some((st.clone(), parsed.clone()));
                st
            }
        };
        // GPUI's list keeps only its top and bottom padding (items start at its
        // left edge, as wide as it), so each item carries the sides itself.
        let (pr, pl) = (px(pad[1]), px(pad[3]));
        div()
            .size_full()
            .child(
                gpui::list(state, move |ix, _, _| {
                    div().pl(pl).pr(pr).child(parsed.render_item(ix, size, &colors, &*images, toggle.as_ref())).into_any_element()
                })
                .w(px(outer_w.max(40.)))
                .h_full()
                .pt(px(pad[0]))
                .pb(px(pad[2])),
            )
            .into_any_element()
    }

    /// What a click on a checkbox of block `id`'s Markdown does: flips that task.
    fn task_toggle(&self, id: &str, cx: &mut Context<Self>) -> markdown::OnToggle {
        let this = cx.entity().downgrade();
        let id = id.to_string();
        Rc::new(move |at, _, cx| {
            let _ = this.update(cx, |d, cx| d.toggle_task(&id, at, cx));
        })
    }

    /// Ticks or unticks the task whose bracketed byte is at `at` in a 笔记's
    /// text (written to its file) or a QuickLook's (saved with the deck).
    fn toggle_task(&mut self, id: &str, at: usize, cx: &mut Context<Self>) {
        let Some(b) = self.m.block_mut(id) else { return };
        match b.kind {
            BlockKind::Memo => {
                let Some(rt) = self.rts.get_mut(id).filter(|r| r.memo.loaded) else { return };
                let Some(next) = markdown::toggle_task(&rt.memo.text, at) else { return };
                rt.memo.text = next;
                self.save_memo(id, cx);
            }
            BlockKind::Note => {
                let Some(next) = markdown::toggle_task(&b.text, at) else { return };
                b.text = next;
                self.request_save(cx);
            }
            _ => return,
        }
        cx.notify();
    }

    pub(super) fn chart_data(&mut self, b: &Block) -> Arc<crate::chart::Data> {
        let rt = self.rts.entry(b.id.clone()).or_default();
        if let Some((cached_src, data)) = &rt.chart_cache
            && cached_src == &b.text
        {
            return data.clone();
        }
        let data = Arc::new(crate::chart::parse(&b.text));
        rt.chart_cache = Some((b.text.clone(), data.clone()));
        data
    }

    fn block_view(&mut self, id: &str, t: &Theme, window: &mut Window, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let Some(b) = self.m.block(id).cloned() else { return vec![] };
        let s = self.m.cam.s;
        let (sx, sy) = self.m.cam.to_screen(b.x, b.y);
        let (w, h) = (b.w * s, b.h * s);
        let selected = self.m.sel.contains(id);
        let opacity = if b.kind == BlockKind::Note { lifespan::note_opacity(b.expires, self.m.now_ms()) } else { 1.0 };
        let mut out = Vec::new();

        // Just made: it opens out of a line (`popin`); the card sits in a clip that grows.
        let pop = if s >= LOD_SCALE { self.rts.get_mut(id).and_then(|r| r.pop.frame(Instant::now())) } else { None };
        let clip = pop.map(|f| {
            let (cw, ch) = ((w * f.width).max(2.), (h * f.height).max(2.));
            ((w - cw) / 2., (h - ch) / 2., cw, ch)
        });
        self.animating |= pop.is_some();

        let note = b.kind == BlockKind::Note;
        let paper = Paper::of(t);
        let eid = id.to_string();
        let mut card = div()
            .id(SharedString::from(format!("block:{id}")))
            .absolute()
            .left(px(clip.map_or(sx, |c| -c.0)))
            .top(px(clip.map_or(sy, |c| -c.1)))
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

        // Zoomed out, the body is a mask of the card's own background; coming
        // back, the mask fades off the content.
        let mask = |d: Div| {
            if note {
                d.bg(linear_gradient(180., linear_color_stop(paper.top, 0.), linear_color_stop(paper.bottom, 1.)))
            } else {
                d.bg(t.surface)
            }
        };
        let rt = self.rts.entry(id.to_string()).or_default();
        // A block coming back from LOD waits its turn (`LOD_WAKE_PER_FRAME`).
        let held = s >= LOD_SCALE && rt.lod && self.lod_wake == 0;
        if held {
            self.animating = true;
        } else if s >= LOD_SCALE && rt.lod {
            self.lod_wake -= 1;
        }
        if s < LOD_SCALE || held {
            rt.lod = true;
            // Zoomed out, a card is its frame, its title and the mask.
            let cs = self.content_scale();
            let color = if note { paper.text } else { t.text };
            card = card
                .child(
                    div()
                        .h(px(TITLE_H * s))
                        .flex_none()
                        .px(px(8. * s))
                        .flex()
                        .items_center()
                        .text_size(px(TITLE_FONT * cs))
                        .text_color(color)
                        .whitespace_nowrap()
                        .child(b.title.clone()),
                )
                .child(mask(div().flex_1().w_full().min_h_0()));
            out.push(card.into_any_element());
            return out;
        }

        // Back from LOD: the mask over the body fades off under the title it kept.
        let now = Instant::now();
        if std::mem::take(&mut rt.lod) {
            rt.appear.reveal(now);
        }
        let appearing = rt.appear.tick(now);
        let appear = rt.appear.alpha(now);
        self.animating |= appearing;
        card = card.child(self.title_bar(&b, t, &paper, window, cx));
        let body = self.body(&b, t, &paper, window, cx);
        // A block worked in keeps the wheel: its content scrolls (it was
        // offered the wheel first), and the canvas does not pan under it.
        // GPUI's scrolling leaves the event going, so it is stopped here.
        let wid = id.to_string();
        let mut body_box = div()
            .relative()
            .flex_1()
            .w_full()
            .min_h_0()
            .overflow_hidden()
            .child(body)
            .on_scroll_wheel(cx.listener(move |d, ev: &ScrollWheelEvent, _, cx| {
                let zoom = ev.modifiers.platform || ev.modifiers.control;
                let gesture = d.canvas_scroll_until.is_some_and(|t| t > Instant::now());
                let worked_in = d.active.as_deref() == Some(wid.as_str()) || d.editing(&wid, EditPart::Body).is_some();
                if worked_in && !zoom && !gesture {
                    cx.stop_propagation();
                }
            }));
        if let Some(cover) = self.body_cover(&b, cx) {
            body_box = body_box.child(cover);
        }
        if appearing {
            body_box = body_box.child(mask(div().absolute().top_0().left_0().size_full()).opacity(1. - appear));
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
        if let (Some(f), Some((ox, oy, cw, ch))) = (pop, clip) {
            out.extend(popin_layers(f, (sx + ox, sy + oy, cw, ch), card.into_any_element(), if note { paper.top } else { t.surface }, t, s));
            return out;
        }
        out.push(card.into_any_element());

        // The resize handle, in the bottom-right corner.
        let hs = (22. * s).max(14.);
        if w >= hs * 3. && h >= hs * 3. {
            self.resize_corners.push([sx + w - hs, sy + h - hs, hs, hs]);
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
        let cs = self.content_scale();
        let zooming = self.zooming();
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
            .text_size(px(TITLE_FONT * cs))
            .text_color(color)
            .when(note, |d| d.bg(paper.header).font_weight(FontWeight::SEMIBOLD));
        if let Some(ed) = self.editing(&b.id, EditPart::Title).cloned() {
            ed.update(cx, |e, _| {
                e.font_size = TITLE_FONT * cs;
                e.color = color;
                e.muted = t.muted;
                e.accent = t.accent;
            });
            bar = bar.child(div().flex_1().min_w_0().child(ed));
        } else {
            bar = bar.child(
                // While zooming the box changes width every frame: a clipped title
                // keeps its shaped line, an ellipsis would cut it anew each frame.
                div()
                    .flex_1()
                    .min_w_0()
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .when(!zooming, |d| d.text_ellipsis())
                    .child(b.title.clone()),
            );
        }
        if note {
            bar = bar.child(self.life_chip(b, paper, cx));
        } else if b.kind == BlockKind::Chart {
            bar = bar.child(self.chart_controls(b, t, cx));
        } else {
            bar = bar.child(
                div().text_size(px(10. * cs)).text_color(t.muted).whitespace_nowrap().child(b.kind.label()),
            );
        }
        bar.into_any_element()
    }

    /// A chart's type chip (a menu of types) and its table button.
    fn chart_controls(&mut self, b: &Block, t: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let s = self.content_scale();
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
        let s = self.content_scale();
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
        let s = self.content_scale();
        match b.kind {
            BlockKind::Note => {
                if let Some(ed) = self.editing(&b.id, EditPart::Body).cloned() {
                    ed.update(cx, |e, _| {
                        e.font_size = 14. * s;
                        e.color = paper.text;
                        e.muted = paper.muted;
                        e.accent = t.accent;
                    });
                    // Laid out at the content scale, so text wraps the same all through a zoom.
                    return div()
                        .w(px(b.w * s))
                        .h_full()
                        .flex_none()
                        .pt(px(6. * s))
                        .px(px(8. * s))
                        .pb(px(22. * s))
                        .child(ed)
                        .into_any_element();
                }
                let pad = [12. * s, 14. * s, 26. * s, 14. * s];
                if b.text.is_empty() {
                    let hint = div().text_size(px(14. * s)).text_color(paper.muted).child("点击编辑，支持 Markdown");
                    return live_preview(&b.id, hint.into_any_element(), pad, b.w * s);
                }
                let colors = MdColors::of_paper(paper);
                let parsed = self.parsed_markdown(&b.id, &b.text, &colors, t.dark);
                let toggle = self.task_toggle(&b.id, cx);
                self.markdown_list(&b.id, parsed, 14. * s, colors, Rc::new(markdown::local_image), pad, b.w * s, Some(toggle))
            }
            BlockKind::Memo => self.memo_body(b, t, cx),
            BlockKind::File => self.file_body(b, t, window, cx),
            BlockKind::Terminal => {
                // The shell starts on the block's first resume (`lifecycle`).
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
                let data = self.chart_data(b);
                let chart = crate::chartview::render_data(data, &b.chart, s, t);
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
                    .when(!side, |d| d.h(px((b.h - TITLE_H) * s * 0.45)).w(px(b.w * s)).border_b_1())
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
        let s = self.content_scale();
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
            return div().w(px(b.w * s)).h_full().flex_none().py(px(8. * s)).px(px(12. * s)).child(ed).into_any_element();
        }
        let save_err = rt.memo.save_err.clone();
        let text = rt.memo.text.clone();
        self.check_memo(&b.id, cx);
        let colors = MdColors::of_theme(t);
        let pad = [12. * s, 16. * s, 12. * s, 16. * s];
        if save_err.is_none() && !text.trim().is_empty() {
            let parsed = self.parsed_markdown(&b.id, &text, &colors, t.dark);
            let toggle = self.task_toggle(&b.id, cx);
            return self.markdown_list(&b.id, parsed, size, colors, Rc::new(markdown::local_image), pad, b.w * s, Some(toggle));
        }
        let content = div()
            .flex()
            .flex_col()
            .gap(px(6. * s))
            .when_some(save_err, |d, e| d.child(div().text_size(px(12. * s)).text_color(t.danger).child(e)))
            .child(if text.trim().is_empty() {
                div().text_size(px(size)).text_color(t.muted).child("双击开始写，支持 Markdown").into_any_element()
            } else {
                let parsed = self.parsed_markdown(&b.id, &text, &colors, t.dark);
                markdown::render_parsed(&parsed, size, &colors)
            })
            .into_any_element();
        live_preview(&b.id, content, pad, b.w * s)
    }

    fn file_body(&mut self, b: &Block, t: &Theme, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let s = self.content_scale();
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
                    Some(Ok(text)) => {
                        let colors = MdColors::of_theme(t);
                        let parsed = self.parsed_markdown(&b.id, &text, &colors, t.dark);
                        let images = Rc::new(move |url: &str| {
                            if url.starts_with("http://") || url.starts_with("https://") {
                                return None;
                            }
                            let p = std::path::Path::new(url);
                            Some(gpui::ImageSource::from(if p.is_absolute() { p.to_path_buf() } else { base.join(p) }))
                        });
                        self.markdown_list(&b.id, parsed, DOC_FONT * s, colors, images, [12. * s, 16. * s, 12. * s, 16. * s], b.w * s, None)
                    }
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
                        let colors = MdColors::of_theme(t);
                        let parsed = self.parsed_markdown(&b.id, &doc.markdown, &colors, t.dark);
                        let images = Rc::new(move |url: &str| {
                            let name = url.strip_prefix(crate::word::MEDIA_SCHEME)?;
                            d2.media.get(name).map(|im| gpui::ImageSource::Image(im.clone()))
                        });
                        self.markdown_list(&b.id, parsed, DOC_FONT * s, colors, images, [12. * s, 16. * s, 16. * s, 16. * s], b.w * s, None)
                    }
                }
            }
            preview::Kind::Pdf => self.pdf_body(b, t, cx),
            preview::Kind::Video => self.video_body(b, t, window),
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
                        .font_family(crate::font::mono())
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
fn live_preview(id: &str, content: AnyElement, pad: [f32; 4], outer_w: f32) -> AnyElement {
    div()
        .id(SharedString::from(format!("live:{id}")))
        .size_full()
        .overflow_y_scroll()
        .child(div().w(px(outer_w.max(40.))).pt(px(pad[0])).pr(px(pad[1])).pb(px(pad[2])).pl(px(pad[3])).child(content))
        .into_any_element()
}

/// A block entering (`popin`): its glow, then its card in a clip at r (window
/// points) that grows, the part not yet printed covered in blank, the scan
/// line on that cover's edge, and the box's bright edge.
fn popin_layers(f: popin::Frame, r: (f32, f32, f32, f32), card: AnyElement, blank: Hsla, t: &Theme, s: f32) -> [AnyElement; 2] {
    let (x, y, w, h) = r;
    let at = || div().absolute().left(px(x)).top(px(y)).w(px(w)).h(px(h));
    let layer = || div().absolute().left_0().top_0().size_full();
    let glow = at()
        .shadow(vec![gpui::BoxShadow {
            color: t.accent.opacity(0.6 * f.glow),
            offset: point(px(0.), px(0.)),
            blur_radius: px(22. * s.min(1.5)),
            spread_radius: px(1.),
        }])
        .into_any_element();
    let line = (2. * s.min(1.5)).max(1.);
    let scan_y = h * f.scan;
    let clip = at()
        .overflow_hidden()
        .occlude()
        .child(card)
        .when(f.scan < 1., |d| {
            d.child(div().absolute().left_0().right_0().top(px(scan_y)).bottom_0().bg(blank)).child(
                div()
                    .absolute()
                    .left_0()
                    .right_0()
                    .top(px((scan_y - line / 2.).max(0.)))
                    .h(px(line))
                    .bg(t.accent.opacity(f.glow.max(0.3)))
                    .when(f.height < 1., |d| d.opacity(0.)),
            )
        })
        .child(layer().bg(t.accent.opacity(f.tint)))
        .child(layer().border_1().border_color(t.accent.opacity(f.glow)))
        .into_any_element();
    [glow, clip]
}

/// How long the camera takes to glide where an agent asked.
const GLIDE_TIME: Duration = Duration::from_millis(520);

/// The camera f (0 to 1) of the way from `from` to `to` in a view of size
/// view: eased in and out; the center straight, the scale geometric, and
/// dipping out mid-way when the trip is long next to the view.
fn glide_at(from: Camera, to: Camera, view: (f32, f32), f: f32) -> Camera {
    let e = if f < 0.5 { 4. * f * f * f } else { 1. - (-2. * f + 2.).powi(3) / 2. };
    let (vw, vh) = view;
    let (ax, ay) = from.to_world(vw / 2., vh / 2.);
    let (bx, by) = to.to_world(vw / 2., vh / 2.);
    let (cx, cy) = (ax + (bx - ax) * e, ay + (by - ay) * e);
    let mut s = (from.s.ln() + (to.s.ln() - from.s.ln()) * e).exp();
    // How many views long the trip is, at the smaller scale: far trips dip out.
    let far = ((bx - ax).hypot(by - ay) * from.s.min(to.s) / vw.max(1.)).min(4.);
    let dip = 1. - (far / 4. * 0.6) * (std::f32::consts::PI * e).sin();
    s = (s * dip).clamp(model::MIN_SCALE, model::MAX_SCALE);
    Camera { x: cx - vw / 2. / s, y: cy - vh / 2. / s, s }
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
        self.tick_glide(cx);
        self.settle_layer(Some(window));
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
        self.lod_wake = LOD_WAKE_PER_FRAME;
        let mut blocks = Vec::new();
        self.resize_corners.clear();
        for id in &visible {
            blocks.extend(self.block_view(id, &t, window, cx));
        }

        self.hide_unshown_pages();
        if std::mem::take(&mut self.animating) {
            window.request_animation_frame();
        }
        crate::webfocus::guard(window);
        crate::webfocus::set_corners(self.resize_corners.clone());

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
            // TEMP: painted last, so it marks the end of the frame's work (`framestat`).
            .child(canvas(|_, _, _| (), |_, _, _, _| zoomlayer::framestat::end()).absolute().size_0())
    }
}

#[cfg(test)]
mod glide_tests {
    use super::glide_at;
    use crate::model::Camera;

    #[test]
    fn a_glide_starts_and_ends_where_asked_and_dips_on_a_long_way() {
        let view = (1000., 800.);
        let from = Camera { x: 0., y: 0., s: 1. };
        let to = Camera { x: 5000., y: 0., s: 1. };
        let start = glide_at(from, to, view, 0.);
        assert!((start.x - from.x).abs() < 1e-3 && (start.s - 1.).abs() < 1e-6);
        let end = glide_at(from, to, view, 1.);
        assert!((end.x - to.x).abs() < 1e-2 && (end.s - 1.).abs() < 1e-4);
        let mid = glide_at(from, to, view, 0.5);
        assert!(mid.s < 0.6, "a long way zooms out mid-way: {}", mid.s);
        let near = glide_at(from, Camera { x: 100., y: 0., s: 1. }, view, 0.5);
        assert!(near.s > 0.95, "a short way hardly does: {}", near.s);
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
        snap: zoomlayer::ZoomSnap,
    }

    impl Harness {
        fn new() -> Self {
            let m = Model::new(std::path::Path::new("/tmp"));
            let lay = m.cam;
            Self { m, lay, zoom_settle: None, snap: zoomlayer::ZoomSnap::default() }
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
                self.snap.on = false;
                if self.m.cam.s < self.lay.s * ZOOM_REBASE || self.m.cam.s > self.lay.s / ZOOM_REBASE {
                    self.lay = Camera { x: self.m.cam.x, y: self.m.cam.y, s: zoom_level(self.m.cam.s) };
                }
                return;
            }
            self.zoom_settle = None;
            if !self.snap.on && (self.lay.s - self.m.cam.s).abs() > 1e-6 && !on_level(self.m.cam.s) {
                let to = zoom_level(self.m.cam.s);
                self.m.zoom_at(640.0, 400.0, to / self.m.cam.s);
                self.lay = self.m.cam;
                return;
            }
            self.lay = self.m.cam;
        }

        fn layer_scale(&self) -> f32 {
            self.m.cam.s / self.lay.s
        }
    }

    #[test]
    fn zoom_layer_holds_scale_during_gesture_and_snaps_on_rest() {
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

        // Once the gesture rests, `m.cam.s` snaps to the nearest zoom level and `lay == m.cam`.
        h.m.zoom_at(640.0, 400.0, 0.77 / h.m.cam.s);
        h.zoom_settle = Some(Instant::now() - Duration::from_millis(10));
        h.settle();
        assert!(on_level(h.m.cam.s));
        assert_eq!(h.lay, h.m.cam);
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
