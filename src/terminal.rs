//! A terminal block's terminal: a shell in a pseudo-terminal, emulated by
//! alacritty_terminal, drawn cell by cell. Its grid keeps its rows and
//! columns as the canvas zooms: the text grows and shrinks, nothing reflows.

use std::borrow::Cow;
use std::path::Path;
use std::sync::Arc;

use alacritty_terminal::event::{Event as TermEvent, EventListener, WindowSize};
use alacritty_terminal::event_loop::{EventLoop, EventLoopSender, Msg};
use alacritty_terminal::grid::{Dimensions, Scroll};
use alacritty_terminal::sync::FairMutex;
use alacritty_terminal::term::cell::Flags;
use alacritty_terminal::term::{Config, Term, TermMode};
use alacritty_terminal::tty;
use alacritty_terminal::vte::ansi::{Color, NamedColor, Rgb};
use gpui::{
    App, Bounds, Context, Element, ElementId, Entity, EventEmitter, FocusHandle, Focusable, Font, FontStyle,
    FontWeight, GlobalElementId, Hsla, KeyDownEvent, LayoutId, MouseButton, MouseDownEvent, Pixels,
    ScrollWheelEvent, SharedString, Style, TextRun, Window, div, fill, point, prelude::*, px, relative, size,
};

/// The size of the terminal's text at 100%: the canvas's zoom scales it.
pub const FONT_SIZE: f32 = 13.0;
pub const LINE_HEIGHT: f32 = 1.25;
pub const FONT_FAMILY: &str = "Menlo";
/// The cell width of Menlo at 1px, to size the grid before a window measures it.
pub const CELL_W_PER_PX: f32 = 0.602;

#[derive(Clone)]
struct Listener(smol::channel::Sender<TermEvent>);

impl EventListener for Listener {
    fn send_event(&self, event: TermEvent) {
        let _ = self.0.try_send(event);
    }
}

struct GridSize {
    cols: usize,
    rows: usize,
}

impl Dimensions for GridSize {
    fn total_lines(&self) -> usize {
        self.rows
    }
    fn screen_lines(&self) -> usize {
        self.rows
    }
    fn columns(&self) -> usize {
        self.cols
    }
}

pub enum TerminalEvent {
    Title(String),
    Exited(i32),
}

pub struct Terminal {
    focus: FocusHandle,
    term: Arc<FairMutex<Term<Listener>>>,
    sender: EventLoopSender,
    cols: usize,
    rows: usize,
    pub font_size: f32,
    exited: bool,
    /// The composition the input method is building: drawn at the cursor, not yet sent.
    marked: Option<String>,
    /// Where the cursor was last painted, in window coordinates: where candidates appear.
    ime_cursor: Option<Bounds<Pixels>>,
    _events: gpui::Task<()>,
}

impl EventEmitter<TerminalEvent> for Terminal {}

impl Focusable for Terminal {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

/// The grid a block body of w×h world units holds at 100%.
pub fn grid_for(w: f32, h: f32) -> (usize, usize) {
    let cell_w = FONT_SIZE * CELL_W_PER_PX;
    let cell_h = (FONT_SIZE * LINE_HEIGHT).round();
    (((w - 16.0) / cell_w).floor().max(10.0) as usize, ((h - 12.0) / cell_h).floor().max(3.0) as usize)
}

impl Terminal {
    /// Starts a shell in dir; what it returns makes the terminal's entity.
    pub fn spawn(
        dir: &Path,
        cols: usize,
        rows: usize,
    ) -> anyhow::Result<impl FnOnce(&mut Context<Self>) -> Terminal + use<>> {
        let (tx, rx) = smol::channel::unbounded();
        let listener = Listener(tx);
        let mut env = std::collections::HashMap::new();
        env.insert("TERM".to_string(), "xterm-256color".to_string());
        env.insert("COLORTERM".to_string(), "truecolor".to_string());
        env.insert("TERM_PROGRAM".to_string(), "BoundlessDeck".to_string());
        let opts = tty::Options { shell: None, working_directory: Some(dir.to_path_buf()), drain_on_exit: false, env };
        let ws = window_size(cols, rows);
        let pty = tty::new(&opts, ws, 0)?;
        let config = Config { scrolling_history: 10_000, ..Config::default() };
        let term = Term::new(config, &GridSize { cols, rows }, listener.clone());
        let term = Arc::new(FairMutex::new(term));
        let event_loop = EventLoop::new(term.clone(), listener, pty, false, false)?;
        let sender = event_loop.channel();
        event_loop.spawn();

        Ok(move |cx: &mut Context<Self>| {
            let events = cx.spawn(async move |this, cx| {
                while let Ok(ev) = rx.recv().await {
                    let alive = this.update(cx, |t, cx| t.on_event(ev, cx)).is_ok();
                    if !alive {
                        break;
                    }
                }
            });
            Terminal {
                focus: cx.focus_handle(),
                term,
                sender,
                cols,
                rows,
                font_size: FONT_SIZE,
                exited: false,
                marked: None,
                ime_cursor: None,
                _events: events,
            }
        })
    }

    fn on_event(&mut self, ev: TermEvent, cx: &mut Context<Self>) {
        match ev {
            TermEvent::Wakeup | TermEvent::CursorBlinkingChange | TermEvent::MouseCursorDirty => cx.notify(),
            TermEvent::Title(t) => cx.emit(TerminalEvent::Title(t)),
            TermEvent::PtyWrite(s) => self.write(s.into_bytes()),
            TermEvent::ClipboardStore(_, s) => cx.write_to_clipboard(gpui::ClipboardItem::new_string(s)),
            TermEvent::ClipboardLoad(_, f) => {
                let text = cx.read_from_clipboard().and_then(|c| c.text()).unwrap_or_default();
                self.write(f(&text).into_bytes());
            }
            TermEvent::ColorRequest(i, f) => {
                let rgb = palette(i, false);
                self.write(f(rgb).into_bytes());
            }
            TermEvent::TextAreaSizeRequest(f) => self.write(f(window_size(self.cols, self.rows)).into_bytes()),
            TermEvent::ChildExit(code) => {
                if !self.exited {
                    self.exited = true;
                    cx.emit(TerminalEvent::Exited(code));
                }
            }
            TermEvent::Exit => {
                if !self.exited {
                    self.exited = true;
                    cx.emit(TerminalEvent::Exited(0));
                }
            }
            _ => {}
        }
    }

    fn write(&self, bytes: impl Into<Cow<'static, [u8]>>) {
        let _ = self.sender.send(Msg::Input(bytes.into()));
    }

    pub fn resize(&mut self, cols: usize, rows: usize) {
        if (cols, rows) == (self.cols, self.rows) {
            return;
        }
        self.cols = cols;
        self.rows = rows;
        self.term.lock().resize(GridSize { cols, rows });
        let _ = self.sender.send(Msg::Resize(window_size(cols, rows)));
    }

    /// Drops a composition the input method left unfinished: the focus left mid-way.
    pub fn cancel_composition(&mut self, cx: &mut Context<Self>) {
        if self.marked.take().is_some() {
            cx.notify();
        }
    }

    /// Hangs the shell up.
    pub fn close(&mut self) {
        let _ = self.sender.send(Msg::Shutdown);
    }


    fn on_key(&mut self, ev: &KeyDownEvent, _: &mut Window, cx: &mut Context<Self>) {
        let ks = &ev.keystroke;
        let m = ks.modifiers;
        // While composing, GPUI gives the input method every key first; one that reaches here
        // the input method let go of, so a composition still noted is stale (focus left mid-way):
        // dropped, or it would hold every key back from the shell.
        if self.marked.take().is_some() {
            cx.notify();
        }
        // Typing text: the input method decides, and what it commits arrives in
        // `replace_text_in_range`. Everything else is a key the terminal sends itself.
        if is_text_key(m, ks.key_char.as_deref()) {
            return;
        }
        if m.platform {
            if ks.key == "v" {
                if let Some(text) = cx.read_from_clipboard().and_then(|c| c.text()) {
                    let bracketed = self.term.lock().mode().contains(TermMode::BRACKETED_PASTE);
                    let text = text.replace("\r\n", "\r").replace('\n', "\r");
                    let bytes = if bracketed { format!("\x1b[200~{text}\x1b[201~") } else { text };
                    self.write(bytes.into_bytes());
                    cx.stop_propagation();
                }
            }
            return; // ⌘ and a key is the app's: a shortcut, not input
        }
        let app_cursor = self.term.lock().mode().contains(TermMode::APP_CURSOR);
        if let Some(bytes) = key_bytes(&ks.key, ks.key_char.as_deref(), m, app_cursor) {
            self.term.lock().scroll_display(Scroll::Bottom);
            self.write(bytes);
            cx.stop_propagation();
        }
    }

    fn on_scroll(&mut self, ev: &ScrollWheelEvent, _: &mut Window, cx: &mut Context<Self>) {
        if ev.modifiers.platform || ev.modifiers.control {
            return; // ⌘ and the wheel zoom the canvas
        }
        let lh = px(self.font_size * LINE_HEIGHT);
        let dy = f32::from(ev.delta.pixel_delta(lh).y) / f32::from(lh);
        let lines = dy.round() as i32;
        if lines != 0 {
            self.term.lock().scroll_display(Scroll::Delta(lines));
            cx.notify();
        }
        cx.stop_propagation();
    }
}

impl Drop for Terminal {
    fn drop(&mut self) {
        self.close();
    }
}

fn window_size(cols: usize, rows: usize) -> WindowSize {
    WindowSize {
        num_cols: cols as u16,
        num_lines: rows as u16,
        cell_width: (FONT_SIZE * CELL_W_PER_PX) as u16,
        cell_height: (FONT_SIZE * LINE_HEIGHT) as u16,
    }
}

/// Keys that type text: a printable character with no ⌃, ⌥ or ⌘. The input method takes them so
/// Chinese can be composed; what it commits reaches the shell in `replace_text_in_range`.
fn is_text_key(m: gpui::Modifiers, key_char: Option<&str>) -> bool {
    !m.control && !m.alt && !m.platform && key_char.is_some_and(|c| !c.is_empty() && !c.chars().any(char::is_control))
}

/// The bytes a key sends, as xterm does.
fn key_bytes(key: &str, key_char: Option<&str>, m: gpui::Modifiers, app_cursor: bool) -> Option<Vec<u8>> {
    let csi = |s: &str| -> Vec<u8> {
        let mods = 1 + (m.shift as u8) + 2 * (m.alt as u8) + 4 * (m.control as u8);
        if mods > 1 {
            format!("\x1b[1;{mods}{s}").into_bytes()
        } else if app_cursor {
            format!("\x1bO{s}").into_bytes()
        } else {
            format!("\x1b[{s}").into_bytes()
        }
    };
    let tilde = |n: u8| format!("\x1b[{n}~").into_bytes();
    let b: Vec<u8> = match key {
        "enter" => b"\r".to_vec(),
        "backspace" => {
            if m.alt {
                b"\x1b\x7f".to_vec()
            } else {
                b"\x7f".to_vec()
            }
        }
        "tab" => {
            if m.shift {
                b"\x1b[Z".to_vec()
            } else {
                b"\t".to_vec()
            }
        }
        "escape" => b"\x1b".to_vec(),
        "up" => csi("A"),
        "down" => csi("B"),
        "right" => csi("C"),
        "left" => csi("D"),
        "home" => csi("H"),
        "end" => csi("F"),
        "delete" => tilde(3),
        "pageup" => tilde(5),
        "pagedown" => tilde(6),
        "insert" => tilde(2),
        "f1" => b"\x1bOP".to_vec(),
        "f2" => b"\x1bOQ".to_vec(),
        "f3" => b"\x1bOR".to_vec(),
        "f4" => b"\x1bOS".to_vec(),
        "space" if m.control => vec![0],
        _ => {
            if m.control && key.len() == 1 {
                let c = key.as_bytes()[0].to_ascii_lowercase();
                return match c {
                    b'a'..=b'z' => Some(vec![c - b'a' + 1]),
                    b'[' => Some(vec![0x1b]),
                    b'\\' => Some(vec![0x1c]),
                    b']' => Some(vec![0x1d]),
                    _ => None,
                };
            }
            let text = if m.alt && key.len() == 1 { key } else { key_char? };
            let mut v = Vec::new();
            if m.alt {
                v.push(0x1b);
            }
            v.extend_from_slice(text.as_bytes());
            v
        }
    };
    Some(b)
}

/// The 256 colors, and the named ones: foreground, background, cursor.
fn palette(i: usize, dark: bool) -> Rgb {
    const BASE: [(u8, u8, u8); 16] = [
        (0x1d, 0x1f, 0x24),
        (0xd2, 0x3d, 0x3d),
        (0x2e, 0x9d, 0x4f),
        (0xb5, 0x89, 0x00),
        (0x2f, 0x6b, 0xd8),
        (0xa8, 0x4f, 0xc7),
        (0x1f, 0x9c, 0xa8),
        (0xc8, 0xcb, 0xd2),
        (0x6b, 0x70, 0x80),
        (0xf0, 0x5a, 0x5a),
        (0x4f, 0xc2, 0x6c),
        (0xe0, 0xb1, 0x2a),
        (0x5a, 0x8c, 0xfa),
        (0xc7, 0x7d, 0xe6),
        (0x4f, 0xc7, 0xd2),
        (0xf4, 0xf5, 0xf7),
    ];
    let (r, g, b) = match i {
        0..16 => BASE[i],
        16..232 => {
            let n = i - 16;
            let v = |x: usize| if x == 0 { 0 } else { (55 + x * 40) as u8 };
            (v(n / 36), v((n / 6) % 6), v(n % 6))
        }
        232..256 => {
            let v = (8 + (i - 232) * 10) as u8;
            (v, v, v)
        }
        // Foreground, background, cursor.
        256 | 258 => {
            if dark {
                (0xe8, 0xe9, 0xed)
            } else {
                (0x1d, 0x1f, 0x24)
            }
        }
        _ => {
            if dark {
                (0x14, 0x16, 0x1b)
            } else {
                (0xfb, 0xfb, 0xfc)
            }
        }
    };
    Rgb { r, g, b }
}

pub fn background(dark: bool) -> Hsla {
    to_hsla(palette(257, dark))
}

fn to_hsla(c: Rgb) -> Hsla {
    Hsla::from(gpui::Rgba { r: c.r as f32 / 255., g: c.g as f32 / 255., b: c.b as f32 / 255., a: 1. })
}

fn resolve(c: Color, colors: &alacritty_terminal::term::color::Colors, dark: bool, bold: bool) -> Rgb {
    match c {
        Color::Spec(rgb) => rgb,
        Color::Indexed(i) => {
            let i = if bold && i < 8 { i + 8 } else { i };
            colors[i as usize].unwrap_or_else(|| palette(i as usize, dark))
        }
        Color::Named(n) => {
            let i = n as usize;
            let i = match n {
                NamedColor::Foreground | NamedColor::BrightForeground => 256,
                NamedColor::Background => 257,
                NamedColor::Cursor => 258,
                NamedColor::DimForeground => 256,
                _ if i < 16 && bold && i < 8 => i + 8,
                _ if i < 16 => i,
                _ => {
                    // Dim colors: the normal ones.
                    if (NamedColor::DimBlack as usize..=NamedColor::DimWhite as usize).contains(&i) {
                        i - NamedColor::DimBlack as usize
                    } else {
                        256
                    }
                }
            };
            if i < 256 { colors[i].unwrap_or_else(|| palette(i, dark)) } else { palette(i, dark) }
        }
    }
}

impl gpui::EntityInputHandler for Terminal {
    fn text_for_range(
        &mut self,
        _: std::ops::Range<usize>,
        _: &mut Option<std::ops::Range<usize>>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<String> {
        None
    }

    fn selected_text_range(&mut self, _: bool, _: &mut Window, _: &mut Context<Self>) -> Option<gpui::UTF16Selection> {
        Some(gpui::UTF16Selection { range: 0..0, reversed: false })
    }

    fn marked_text_range(&self, _: &mut Window, _: &mut Context<Self>) -> Option<std::ops::Range<usize>> {
        self.marked.as_ref().map(|m| 0..m.encode_utf16().count())
    }

    fn unmark_text(&mut self, _: &mut Window, cx: &mut Context<Self>) {
        self.marked = None;
        cx.notify();
    }

    /// Committed text: the composition is over, and the text goes to the shell.
    fn replace_text_in_range(
        &mut self,
        _: Option<std::ops::Range<usize>>,
        text: &str,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.marked = None;
        if !text.is_empty() {
            self.term.lock().scroll_display(Scroll::Bottom);
            self.write(text.as_bytes().to_vec());
        }
        cx.notify();
    }

    /// Composing text: drawn at the cursor, not sent until it is committed.
    fn replace_and_mark_text_in_range(
        &mut self,
        _: Option<std::ops::Range<usize>>,
        text: &str,
        _: Option<std::ops::Range<usize>>,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.marked = (!text.is_empty()).then(|| text.to_string());
        cx.notify();
    }

    fn bounds_for_range(
        &mut self,
        _: std::ops::Range<usize>,
        _: Bounds<Pixels>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>> {
        self.ime_cursor.clone()
    }

    fn character_index_for_point(
        &mut self,
        _: gpui::Point<Pixels>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<usize> {
        None
    }
}

impl Render for Terminal {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .id("terminal")
            .key_context("Terminal")
            .track_focus(&self.focus)
            .size_full()
            .on_key_down(cx.listener(Self::on_key))
            .on_scroll_wheel(cx.listener(Self::on_scroll))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|t, _: &MouseDownEvent, window, cx| {
                    window.focus(&t.focus);
                    cx.stop_propagation();
                }),
            )
            .child(GridElement { term: cx.entity() })
    }
}

struct GridElement {
    term: Entity<Terminal>,
}

impl IntoElement for GridElement {
    type Element = Self;
    fn into_element(self) -> Self {
        self
    }
}

impl Element for GridElement {
    type RequestLayoutState = ();
    type PrepaintState = ();

    fn id(&self) -> Option<ElementId> {
        None
    }
    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&gpui::InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, ()) {
        let mut style = Style::default();
        style.size.width = relative(1.).into();
        style.size.height = relative(1.).into();
        (window.request_layout(style, [], cx), ())
    }

    fn prepaint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&gpui::InspectorElementId>,
        _: Bounds<Pixels>,
        _: &mut (),
        _: &mut Window,
        _: &mut App,
    ) {
    }

    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&gpui::InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut (),
        _: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) {
        let dark = matches!(window.appearance(), gpui::WindowAppearance::Dark | gpui::WindowAppearance::VibrantDark);
        let t = self.term.read(cx);
        let focused = t.focus.is_focused(window);
        let focus = t.focus.clone();
        let marked = t.marked.clone();
        let font_px = px(t.font_size);
        let line_h = (t.font_size * LINE_HEIGHT).round().max(1.);
        let font = Font { family: FONT_FAMILY.into(), ..window.text_style().font() };
        let font_id = window.text_system().resolve_font(&font);
        let cell_w = window
            .text_system()
            .advance(font_id, font_px, 'm')
            .map(|s| f32::from(s.width))
            .unwrap_or(t.font_size * CELL_W_PER_PX);
        let term = t.term.clone();
        let term = term.lock();
        let content = term.renderable_content();
        let colors = content.colors;
        let origin = bounds.origin + point(px(t.font_size * 0.6), px(t.font_size * 0.45));
        let display_offset = content.display_offset as i32;

        struct Seg {
            col: usize,
            text: String,
            runs: Vec<TextRun>,
            wide: bool,
        }

        window.handle_input(&focus, gpui::ElementInputHandler::new(bounds, self.term.clone()), cx);
        let mut ime_cursor = None;
        window.with_content_mask(Some(gpui::ContentMask { bounds }), |window| {
            let mut line_no = i32::MIN;
            let mut segs: Vec<Seg> = Vec::new();
            let paint_line = |segs: &mut Vec<Seg>, line: i32, window: &mut Window, cx: &mut App| {
                let y = origin.y + px((line + display_offset) as f32 * line_h);
                for s in segs.drain(..) {
                    let x = origin.x + px(s.col as f32 * cell_w);
                    let force = (!s.wide).then_some(px(cell_w));
                    let shaped =
                        window.text_system().shape_line(SharedString::from(s.text), font_px, &s.runs, force);
                    let _ = shaped.paint(point(x, y), px(line_h), window, cx);
                }
            };
            for cell in content.display_iter {
                let line = cell.point.line.0;
                if line != line_no {
                    if line_no != i32::MIN {
                        paint_line(&mut segs, line_no, window, cx);
                    }
                    line_no = line;
                }
                let flags = cell.flags;
                if flags.contains(Flags::WIDE_CHAR_SPACER) || flags.contains(Flags::LEADING_WIDE_CHAR_SPACER) {
                    continue;
                }
                let col = cell.point.column.0;
                let bold = flags.contains(Flags::BOLD);
                let mut fg = resolve(cell.fg, colors, dark, bold);
                let mut bg = resolve(cell.bg, colors, dark, false);
                if flags.contains(Flags::INVERSE) {
                    std::mem::swap(&mut fg, &mut bg);
                }
                let wide = flags.contains(Flags::WIDE_CHAR);
                let y = origin.y + px((line + display_offset) as f32 * line_h);
                let x = origin.x + px(col as f32 * cell_w);
                let default_bg = palette(257, dark);
                if bg != default_bg {
                    let w = if wide { 2. } else { 1. } * cell_w;
                    window.paint_quad(fill(Bounds::new(point(x, y), size(px(w + 0.5), px(line_h))), to_hsla(bg)));
                }
                if cell.c == ' ' || cell.c == '\t' {
                    continue;
                }
                let mut color = to_hsla(fg);
                if flags.contains(Flags::DIM) {
                    color = color.opacity(0.6);
                }
                let run = TextRun {
                    len: cell.c.len_utf8(),
                    font: Font {
                        weight: if bold { FontWeight::BOLD } else { FontWeight::NORMAL },
                        style: if flags.contains(Flags::ITALIC) { FontStyle::Italic } else { FontStyle::Normal },
                        ..font.clone()
                    },
                    color,
                    background_color: None,
                    underline: flags.contains(Flags::UNDERLINE).then_some(gpui::UnderlineStyle {
                        thickness: px(1.),
                        color: Some(color),
                        wavy: false,
                    }),
                    strikethrough: None,
                };
                // A run of narrow cells side by side shapes as one.
                let extend = segs.last_mut().filter(|s| {
                    !s.wide && !wide && s.col + s.text.chars().count() == col
                });
                match extend {
                    Some(s) => {
                        s.text.push(cell.c);
                        match s.runs.last_mut() {
                            Some(r) if r.font == run.font && r.color == run.color && r.underline == run.underline => {
                                r.len += run.len
                            }
                            _ => s.runs.push(run),
                        }
                    }
                    None => segs.push(Seg { col, text: cell.c.to_string(), runs: vec![run], wide }),
                }
            }
            if line_no != i32::MIN {
                paint_line(&mut segs, line_no, window, cx);
            }

            // The cursor: a block while focused, an outline otherwise. The composition, if any,
            // sits on it, and the input method is told where it is so candidates pop up there.
            let cur = content.cursor;
            if display_offset == 0 {
                let x = origin.x + px(cur.point.column.0 as f32 * cell_w);
                let y = origin.y + px(cur.point.line.0 as f32 * line_h);
                let b = Bounds::new(point(x, y), size(px(cell_w), px(line_h)));
                ime_cursor = Some(b.clone());
                if let Some(text) = &marked {
                    let color = to_hsla(palette(256, dark));
                    let run = TextRun {
                        len: text.len(),
                        font: font.clone(),
                        color,
                        background_color: None,
                        underline: Some(gpui::UnderlineStyle { color: Some(color), thickness: px(1.), wavy: false }),
                        strikethrough: None,
                    };
                    let shaped =
                        window.text_system().shape_line(SharedString::from(text.clone()), font_px, &[run], None);
                    let at = point(x, y);
                    window.paint_quad(fill(Bounds::new(at, size(shaped.width, px(line_h))), background(dark)));
                    let _ = shaped.paint(at, px(line_h), window, cx);
                } else if content.mode.contains(TermMode::SHOW_CURSOR) {
                    let color = to_hsla(palette(258, dark));
                    if focused {
                        window.paint_quad(fill(b, color.opacity(0.55)));
                    } else {
                        window.paint_quad(gpui::outline(b, color.opacity(0.6), gpui::BorderStyle::Solid));
                    }
                }
            }
        });
        self.term.update(cx, |t, _| t.ime_cursor = ime_cursor);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_send_what_xterm_sends() {
        let none = gpui::Modifiers::default();
        let ctrl = gpui::Modifiers { control: true, ..Default::default() };
        let alt = gpui::Modifiers { alt: true, ..Default::default() };
        assert_eq!(key_bytes("c", Some("c"), ctrl, false), Some(vec![3]));
        assert_eq!(key_bytes("up", None, none, false), Some(b"\x1b[A".to_vec()));
        assert_eq!(key_bytes("up", None, none, true), Some(b"\x1bOA".to_vec()));
        assert_eq!(key_bytes("b", Some("∫"), alt, false), Some(b"\x1bb".to_vec()));
        assert_eq!(key_bytes("shift", None, none, false), None);
    }

    #[test]
    fn typed_text_goes_to_the_input_method() {
        let none = gpui::Modifiers::default();
        let ctrl = gpui::Modifiers { control: true, ..Default::default() };
        let alt = gpui::Modifiers { alt: true, ..Default::default() };
        assert!(is_text_key(none, Some("中")));
        assert!(is_text_key(none, Some(" ")));
        assert!(!is_text_key(none, Some("\r")));
        assert!(!is_text_key(none, None));
        assert!(!is_text_key(ctrl, Some("c")));
        assert!(!is_text_key(alt, Some("b")));
    }

    #[test]
    fn grid_fits_the_body() {
        let (c, r) = grid_for(560.0, 340.0 - 26.0);
        assert!(c > 60 && r > 15);
    }

}
