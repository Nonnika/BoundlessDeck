//! A plain text editor: one line (a block's title) or many, wrapped (a
//! QuickLook's or 笔记's Markdown). GPUI ships no text input, so this is
//! one, after its input example: IME through `EntityInputHandler`, the
//! selection, the clipboard, and the keys an editor is expected to know.

use std::ops::Range;

use gpui::{
    App, Bounds, ClipboardItem, Context, CursorStyle, Element, ElementId, ElementInputHandler, Entity,
    EntityInputHandler, EventEmitter, FocusHandle, Focusable, GlobalElementId, Hsla, KeyBinding, LayoutId,
    MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, PaintQuad, Pixels, Point, ScrollWheelEvent,
    SharedString, Style, TextRun, UTF16Selection, UnderlineStyle, Window, WrappedLine, actions, div, fill,
    point, prelude::*, px, relative, size,
};
use unicode_segmentation::UnicodeSegmentation as _;

actions!(
    editor,
    [
        Backspace,
        Delete,
        Left,
        Right,
        Up,
        Down,
        SelectLeft,
        SelectRight,
        SelectUp,
        SelectDown,
        WordLeft,
        WordRight,
        SelectAll,
        Home,
        End,
        Newline,
        Done,
        Tab,
        Paste,
        Cut,
        Copy,
    ]
);

pub fn bind_keys(cx: &mut App) {
    let c = Some("Editor");
    cx.bind_keys([
        KeyBinding::new("backspace", Backspace, c),
        KeyBinding::new("delete", Delete, c),
        KeyBinding::new("left", Left, c),
        KeyBinding::new("right", Right, c),
        KeyBinding::new("up", Up, c),
        KeyBinding::new("down", Down, c),
        KeyBinding::new("shift-left", SelectLeft, c),
        KeyBinding::new("shift-right", SelectRight, c),
        KeyBinding::new("shift-up", SelectUp, c),
        KeyBinding::new("shift-down", SelectDown, c),
        KeyBinding::new("alt-left", WordLeft, c),
        KeyBinding::new("alt-right", WordRight, c),
        KeyBinding::new("cmd-left", Home, c),
        KeyBinding::new("cmd-right", End, c),
        KeyBinding::new("home", Home, c),
        KeyBinding::new("end", End, c),
        KeyBinding::new("cmd-a", SelectAll, c),
        KeyBinding::new("cmd-v", Paste, c),
        KeyBinding::new("cmd-c", Copy, c),
        KeyBinding::new("cmd-x", Cut, c),
        KeyBinding::new("enter", Newline, c),
        KeyBinding::new("tab", Tab, c),
        KeyBinding::new("escape", Done, c),
        KeyBinding::new("cmd-enter", Done, c),
    ]);
}

/// What an editor tells its owner.
#[derive(Clone, Debug, PartialEq)]
pub enum EditorEvent {
    /// The text changed.
    Changed,
    /// Esc or ⌘↵, or ↵ in a single line: done editing.
    Done,
    /// It lost the keyboard focus.
    Blurred,
}

pub struct Editor {
    focus: FocusHandle,
    pub text: String,
    pub placeholder: SharedString,
    pub multiline: bool,
    pub font_size: f32,
    pub color: Hsla,
    pub muted: Hsla,
    pub accent: Hsla,
    pub mono: bool,
    selected: Range<usize>,
    reversed: bool,
    marked: Option<Range<usize>>,
    selecting: bool,
    /// How far the text is scrolled up, in pixels.
    scroll_y: f32,
    /// The x the up/down keys keep to.
    goal_x: Option<Pixels>,
    layout: Option<Layout>,
}

/// The text as last painted: its lines, where each starts, and where they lie.
struct Layout {
    lines: Vec<WrappedLine>,
    starts: Vec<usize>,
    tops: Vec<Pixels>,
    bounds: Bounds<Pixels>,
    line_height: Pixels,
    scroll_y: f32,
}

impl EventEmitter<EditorEvent> for Editor {}

impl Focusable for Editor {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Editor {
    pub fn new(text: String, multiline: bool, window: &mut Window, cx: &mut Context<Self>) -> Editor {
        let focus = cx.focus_handle();
        cx.on_blur(&focus, window, |_, _, cx| cx.emit(EditorEvent::Blurred)).detach();
        let end = text.len();
        Editor {
            focus,
            text,
            placeholder: SharedString::default(),
            multiline,
            font_size: 14.0,
            color: gpui::black(),
            muted: gpui::black().opacity(0.4),
            accent: gpui::blue(),
            mono: false,
            selected: end..end,
            reversed: false,
            marked: None,
            selecting: false,
            scroll_y: 0.0,
            goal_x: None,
            layout: None,
        }
    }

    pub fn set_text(&mut self, text: String, cx: &mut Context<Self>) {
        if text != self.text {
            let end = text.len();
            self.text = text;
            self.selected = end..end;
            self.marked = None;
            cx.notify();
        }
    }

    pub fn select_all_text(&mut self) {
        self.selected = 0..self.text.len();
        self.reversed = false;
    }

    fn cursor(&self) -> usize {
        if self.reversed { self.selected.start } else { self.selected.end }
    }

    fn move_to(&mut self, offset: usize, cx: &mut Context<Self>) {
        self.selected = offset..offset;
        self.reversed = false;
        cx.notify();
    }

    fn select_to(&mut self, offset: usize, cx: &mut Context<Self>) {
        if self.reversed {
            self.selected.start = offset;
        } else {
            self.selected.end = offset;
        }
        if self.selected.end < self.selected.start {
            self.reversed = !self.reversed;
            self.selected = self.selected.end..self.selected.start;
        }
        cx.notify();
    }

    fn prev_boundary(&self, offset: usize) -> usize {
        self.text.grapheme_indices(true).rev().find_map(|(i, _)| (i < offset).then_some(i)).unwrap_or(0)
    }

    fn next_boundary(&self, offset: usize) -> usize {
        self.text.grapheme_indices(true).find_map(|(i, _)| (i > offset).then_some(i)).unwrap_or(self.text.len())
    }

    fn prev_word(&self, offset: usize) -> usize {
        self.text
            .unicode_word_indices()
            .rev()
            .find_map(|(i, _)| (i < offset).then_some(i))
            .unwrap_or(0)
    }

    fn next_word(&self, offset: usize) -> usize {
        self.text
            .unicode_word_indices()
            .find_map(|(i, w)| (i + w.len() > offset).then_some(i + w.len()))
            .unwrap_or(self.text.len())
    }

    fn line_start(&self, offset: usize) -> usize {
        self.text[..offset].rfind('\n').map_or(0, |i| i + 1)
    }

    fn line_end(&self, offset: usize) -> usize {
        self.text[offset..].find('\n').map_or(self.text.len(), |i| offset + i)
    }

    /// Where the offset is painted, relative to the text's top-left (unscrolled).
    fn point_for(&self, offset: usize) -> Option<Point<Pixels>> {
        let l = self.layout.as_ref()?;
        let i = l.starts.partition_point(|&s| s <= offset).saturating_sub(1);
        let p = l.lines.get(i)?.position_for_index(offset - l.starts[i], l.line_height)?;
        Some(point(p.x, p.y + l.tops[i]))
    }

    /// The offset nearest a point relative to the text's top-left (unscrolled).
    fn offset_at(&self, p: Point<Pixels>) -> usize {
        let Some(l) = self.layout.as_ref() else { return self.text.len() };
        if l.lines.is_empty() {
            return 0;
        }
        if p.y < px(0.) {
            return 0;
        }
        let i = l.tops.partition_point(|&t| t <= p.y).saturating_sub(1);
        let line = &l.lines[i];
        let local = point(p.x.max(px(0.)), p.y - l.tops[i]);
        let idx = match line.closest_index_for_position(local, l.line_height) {
            Ok(v) | Err(v) => v,
        };
        (l.starts[i] + idx).min(self.text.len())
    }

    fn offset_for_mouse(&self, pos: Point<Pixels>) -> usize {
        let Some(l) = self.layout.as_ref() else { return self.text.len() };
        let local = pos - l.bounds.origin;
        self.offset_at(point(local.x, local.y + px(l.scroll_y)))
    }

    fn vertical(&mut self, dir: f32, select: bool, cx: &mut Context<Self>) {
        let cur = self.cursor();
        let (Some(p), Some(lh)) = (self.point_for(cur), self.layout.as_ref().map(|l| l.line_height)) else {
            return;
        };
        let x = *self.goal_x.get_or_insert(p.x);
        let target = point(x, p.y + lh * dir + lh / 2.);
        let to = if dir < 0. && p.y < lh / 2. {
            0
        } else {
            self.offset_at(target)
        };
        let goal = self.goal_x;
        if select { self.select_to(to, cx) } else { self.move_to(to, cx) }
        self.goal_x = goal;
    }

    fn left(&mut self, _: &Left, _: &mut Window, cx: &mut Context<Self>) {
        if self.selected.is_empty() {
            self.move_to(self.prev_boundary(self.cursor()), cx);
        } else {
            self.move_to(self.selected.start, cx);
        }
    }
    fn right(&mut self, _: &Right, _: &mut Window, cx: &mut Context<Self>) {
        if self.selected.is_empty() {
            self.move_to(self.next_boundary(self.cursor()), cx);
        } else {
            self.move_to(self.selected.end, cx);
        }
    }
    fn up(&mut self, _: &Up, _: &mut Window, cx: &mut Context<Self>) {
        self.vertical(-1., false, cx);
    }
    fn down(&mut self, _: &Down, _: &mut Window, cx: &mut Context<Self>) {
        self.vertical(1., false, cx);
    }
    fn select_left(&mut self, _: &SelectLeft, _: &mut Window, cx: &mut Context<Self>) {
        self.select_to(self.prev_boundary(self.cursor()), cx);
    }
    fn select_right(&mut self, _: &SelectRight, _: &mut Window, cx: &mut Context<Self>) {
        self.select_to(self.next_boundary(self.cursor()), cx);
    }
    fn select_up(&mut self, _: &SelectUp, _: &mut Window, cx: &mut Context<Self>) {
        self.vertical(-1., true, cx);
    }
    fn select_down(&mut self, _: &SelectDown, _: &mut Window, cx: &mut Context<Self>) {
        self.vertical(1., true, cx);
    }
    fn word_left(&mut self, _: &WordLeft, _: &mut Window, cx: &mut Context<Self>) {
        self.move_to(self.prev_word(self.cursor()), cx);
    }
    fn word_right(&mut self, _: &WordRight, _: &mut Window, cx: &mut Context<Self>) {
        self.move_to(self.next_word(self.cursor()), cx);
    }
    fn select_all(&mut self, _: &SelectAll, _: &mut Window, cx: &mut Context<Self>) {
        self.select_all_text();
        cx.notify();
    }
    fn home(&mut self, _: &Home, _: &mut Window, cx: &mut Context<Self>) {
        self.move_to(self.line_start(self.cursor()), cx);
    }
    fn end(&mut self, _: &End, _: &mut Window, cx: &mut Context<Self>) {
        self.move_to(self.line_end(self.cursor()), cx);
    }
    fn backspace(&mut self, _: &Backspace, window: &mut Window, cx: &mut Context<Self>) {
        if self.selected.is_empty() {
            self.select_to(self.prev_boundary(self.cursor()), cx);
        }
        self.replace_text_in_range(None, "", window, cx);
    }
    fn delete(&mut self, _: &Delete, window: &mut Window, cx: &mut Context<Self>) {
        if self.selected.is_empty() {
            self.select_to(self.next_boundary(self.cursor()), cx);
        }
        self.replace_text_in_range(None, "", window, cx);
    }
    fn newline(&mut self, _: &Newline, window: &mut Window, cx: &mut Context<Self>) {
        if self.multiline {
            self.replace_text_in_range(None, "\n", window, cx);
        } else {
            cx.emit(EditorEvent::Done);
        }
    }
    fn tab(&mut self, _: &Tab, window: &mut Window, cx: &mut Context<Self>) {
        if self.multiline {
            self.replace_text_in_range(None, "  ", window, cx);
        }
    }
    fn done(&mut self, _: &Done, _: &mut Window, cx: &mut Context<Self>) {
        cx.emit(EditorEvent::Done);
    }
    fn paste(&mut self, _: &Paste, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) {
            let text = if self.multiline { text } else { text.replace('\n', " ") };
            self.replace_text_in_range(None, &text, window, cx);
        }
    }
    fn copy(&mut self, _: &Copy, _: &mut Window, cx: &mut Context<Self>) {
        if !self.selected.is_empty() {
            cx.write_to_clipboard(ClipboardItem::new_string(self.text[self.selected.clone()].to_string()));
        }
    }
    fn cut(&mut self, _: &Cut, window: &mut Window, cx: &mut Context<Self>) {
        if !self.selected.is_empty() {
            cx.write_to_clipboard(ClipboardItem::new_string(self.text[self.selected.clone()].to_string()));
            self.replace_text_in_range(None, "", window, cx);
        }
    }

    fn on_mouse_down(&mut self, ev: &MouseDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        window.focus(&self.focus);
        self.selecting = true;
        self.goal_x = None;
        let at = self.offset_for_mouse(ev.position);
        if ev.click_count >= 2 {
            let start = self.prev_word(self.next_boundary(at).min(self.text.len()));
            let end = self.next_word(start);
            self.selected = start..end.max(start);
            self.reversed = false;
            cx.notify();
        } else if ev.modifiers.shift {
            self.select_to(at, cx);
        } else {
            self.move_to(at, cx);
        }
        cx.stop_propagation();
    }
    fn on_mouse_up(&mut self, _: &MouseUpEvent, _: &mut Window, _: &mut Context<Self>) {
        self.selecting = false;
    }
    fn on_mouse_move(&mut self, ev: &MouseMoveEvent, _: &mut Window, cx: &mut Context<Self>) {
        if self.selecting {
            self.select_to(self.offset_for_mouse(ev.position), cx);
        }
    }
    fn on_scroll(&mut self, ev: &ScrollWheelEvent, _: &mut Window, cx: &mut Context<Self>) {
        if !self.multiline || ev.modifiers.platform || ev.modifiers.control {
            return; // ⌘ and the wheel zoom the canvas
        }
        let lh = self.layout.as_ref().map_or(px(20.), |l| l.line_height);
        let dy = f32::from(ev.delta.pixel_delta(lh).y);
        self.scroll_y = (self.scroll_y - dy).max(0.);
        cx.notify();
        cx.stop_propagation();
    }

    fn utf16_to(&self, offset: usize) -> usize {
        let mut out = 0;
        let mut n = 0;
        for ch in self.text.chars() {
            if n >= offset {
                break;
            }
            n += ch.len_utf8();
            out += ch.len_utf16();
        }
        out
    }
    fn utf16_from(&self, offset: usize) -> usize {
        let mut out = 0;
        let mut n = 0;
        for ch in self.text.chars() {
            if n >= offset {
                break;
            }
            n += ch.len_utf16();
            out += ch.len_utf8();
        }
        out
    }
    fn range_to_utf16(&self, r: &Range<usize>) -> Range<usize> {
        self.utf16_to(r.start)..self.utf16_to(r.end)
    }
    fn range_from_utf16(&self, r: &Range<usize>) -> Range<usize> {
        self.utf16_from(r.start)..self.utf16_from(r.end)
    }
}

impl EntityInputHandler for Editor {
    fn text_for_range(
        &mut self,
        range: Range<usize>,
        actual: &mut Option<Range<usize>>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<String> {
        let r = self.range_from_utf16(&range);
        actual.replace(self.range_to_utf16(&r));
        Some(self.text[r].to_string())
    }

    fn selected_text_range(&mut self, _: bool, _: &mut Window, _: &mut Context<Self>) -> Option<UTF16Selection> {
        Some(UTF16Selection { range: self.range_to_utf16(&self.selected), reversed: self.reversed })
    }

    fn marked_text_range(&self, _: &mut Window, _: &mut Context<Self>) -> Option<Range<usize>> {
        self.marked.as_ref().map(|r| self.range_to_utf16(r))
    }

    fn unmark_text(&mut self, _: &mut Window, _: &mut Context<Self>) {
        self.marked = None;
    }

    fn replace_text_in_range(
        &mut self,
        range: Option<Range<usize>>,
        new_text: &str,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let new_text = if self.multiline { new_text.to_string() } else { new_text.replace('\n', " ") };
        let r = range
            .as_ref()
            .map(|r| self.range_from_utf16(r))
            .or(self.marked.clone())
            .unwrap_or(self.selected.clone());
        self.text.replace_range(r.clone(), &new_text);
        let at = r.start + new_text.len();
        self.selected = at..at;
        self.reversed = false;
        self.marked = None;
        self.goal_x = None;
        cx.emit(EditorEvent::Changed);
        cx.notify();
    }

    fn replace_and_mark_text_in_range(
        &mut self,
        range: Option<Range<usize>>,
        new_text: &str,
        new_selected: Option<Range<usize>>,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let r = range
            .as_ref()
            .map(|r| self.range_from_utf16(r))
            .or(self.marked.clone())
            .unwrap_or(self.selected.clone());
        self.text.replace_range(r.clone(), new_text);
        self.marked = (!new_text.is_empty()).then(|| r.start..r.start + new_text.len());
        self.selected = new_selected
            .as_ref()
            .map(|s| self.range_from_utf16(s))
            .map(|s| r.start + s.start..r.start + s.end)
            .unwrap_or_else(|| r.start + new_text.len()..r.start + new_text.len());
        cx.emit(EditorEvent::Changed);
        cx.notify();
    }

    fn bounds_for_range(
        &mut self,
        range: Range<usize>,
        _: Bounds<Pixels>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>> {
        let r = self.range_from_utf16(&range);
        let l = self.layout.as_ref()?;
        let (origin, lh, sy) = (l.bounds.origin, l.line_height, l.scroll_y);
        let a = self.point_for(r.start)?;
        let b = self.point_for(r.end)?;
        Some(Bounds::from_corners(
            origin + point(a.x, a.y - px(sy)),
            origin + point(b.x.max(a.x + px(1.)), b.y - px(sy) + lh),
        ))
    }

    fn character_index_for_point(
        &mut self,
        p: Point<Pixels>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<usize> {
        Some(self.utf16_to(self.offset_for_mouse(p)))
    }
}

impl Render for Editor {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .id("editor")
            .key_context("Editor")
            .track_focus(&self.focus)
            .cursor(CursorStyle::IBeam)
            .size_full()
            .on_action(cx.listener(Self::backspace))
            .on_action(cx.listener(Self::delete))
            .on_action(cx.listener(Self::left))
            .on_action(cx.listener(Self::right))
            .on_action(cx.listener(Self::up))
            .on_action(cx.listener(Self::down))
            .on_action(cx.listener(Self::select_left))
            .on_action(cx.listener(Self::select_right))
            .on_action(cx.listener(Self::select_up))
            .on_action(cx.listener(Self::select_down))
            .on_action(cx.listener(Self::word_left))
            .on_action(cx.listener(Self::word_right))
            .on_action(cx.listener(Self::select_all))
            .on_action(cx.listener(Self::home))
            .on_action(cx.listener(Self::end))
            .on_action(cx.listener(Self::newline))
            .on_action(cx.listener(Self::tab))
            .on_action(cx.listener(Self::done))
            .on_action(cx.listener(Self::paste))
            .on_action(cx.listener(Self::cut))
            .on_action(cx.listener(Self::copy))
            .on_mouse_down(MouseButton::Left, cx.listener(Self::on_mouse_down))
            .on_mouse_up(MouseButton::Left, cx.listener(Self::on_mouse_up))
            .on_mouse_up_out(MouseButton::Left, cx.listener(Self::on_mouse_up))
            .on_mouse_move(cx.listener(Self::on_mouse_move))
            .on_scroll_wheel(cx.listener(Self::on_scroll))
            .child(TextElement { editor: cx.entity() })
    }
}

struct TextElement {
    editor: Entity<Editor>,
}

struct Prepaint {
    lines: Vec<WrappedLine>,
    starts: Vec<usize>,
    tops: Vec<Pixels>,
    line_height: Pixels,
    selection: Vec<PaintQuad>,
    cursor: Option<PaintQuad>,
    scroll_y: f32,
}

impl IntoElement for TextElement {
    type Element = Self;
    fn into_element(self) -> Self {
        self
    }
}

impl Element for TextElement {
    type RequestLayoutState = ();
    type PrepaintState = Prepaint;

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
        let e = self.editor.read(cx);
        let mut style = Style::default();
        style.size.width = relative(1.).into();
        style.size.height = if e.multiline { relative(1.).into() } else { px(e.font_size * 1.35).into() };
        (window.request_layout(style, [], cx), ())
    }

    fn prepaint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&gpui::InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) -> Prepaint {
        let e = self.editor.read(cx);
        let size_px = px(e.font_size);
        let line_height = px(e.font_size * if e.multiline { 1.5 } else { 1.35 });
        let mut font = window.text_style().font();
        if e.mono {
            font.family = "Menlo".into();
        }
        let (text, color) = if e.text.is_empty() {
            (e.placeholder.to_string(), e.muted)
        } else {
            (e.text.clone(), e.color)
        };
        let base = TextRun { len: 0, font, color, background_color: None, underline: None, strikethrough: None };

        let mut lines = Vec::new();
        let mut starts = Vec::new();
        let mut tops = Vec::new();
        let mut y = px(0.);
        let wrap = e.multiline.then_some(bounds.size.width);
        let mut start = 0;
        for piece in text.split('\n') {
            let mut runs = Vec::new();
            let end = start + piece.len();
            match e.marked.as_ref().filter(|_| !e.text.is_empty()) {
                Some(m) if m.start < end && m.end > start => {
                    let (a, b) = (m.start.max(start) - start, m.end.min(end) - start);
                    runs.push(TextRun { len: a, ..base.clone() });
                    runs.push(TextRun {
                        len: b - a,
                        underline: Some(UnderlineStyle { color: Some(color), thickness: px(1.), wavy: false }),
                        ..base.clone()
                    });
                    runs.push(TextRun { len: piece.len() - b, ..base.clone() });
                }
                _ => runs.push(TextRun { len: piece.len(), ..base.clone() }),
            }
            runs.retain(|r| r.len > 0);
            let shaped = window
                .text_system()
                .shape_text(SharedString::from(piece.to_string()), size_px, &runs, wrap, None)
                .ok()
                .and_then(|mut v| v.pop())
                .unwrap_or_default();
            let h = shaped.size(line_height).height.max(line_height);
            starts.push(start);
            tops.push(y);
            lines.push(shaped);
            y += h;
            start = end + 1;
        }
        let content_h = f32::from(y);
        let focused = e.focus.is_focused(window);
        let selected = e.selected.clone();
        let cursor_off = e.cursor();
        let has_text = !e.text.is_empty();
        let accent = e.accent;
        let mut scroll_y = e.scroll_y;

        let point_for = |off: usize| -> Point<Pixels> {
            let i = starts.partition_point(|&s| s <= off).saturating_sub(1);
            lines[i]
                .position_for_index(off - starts[i], line_height)
                .map(|p| point(p.x, p.y + tops[i]))
                .unwrap_or(point(px(0.), tops[i]))
        };

        let view_h = f32::from(bounds.size.height);
        let mut cursor_pt = point(px(0.), px(0.));
        if has_text {
            cursor_pt = point_for(cursor_off);
        }
        if e.multiline && focused {
            // Keep the cursor in view.
            let cy = f32::from(cursor_pt.y);
            let lh = f32::from(line_height);
            if cy < scroll_y {
                scroll_y = cy;
            } else if cy + lh > scroll_y + view_h {
                scroll_y = cy + lh - view_h;
            }
        }
        scroll_y = scroll_y.min((content_h - view_h).max(0.)).max(0.);
        let origin = bounds.origin - point(px(0.), px(scroll_y));

        let mut selection = Vec::new();
        if has_text && !selected.is_empty() {
            let a = point_for(selected.start);
            let b = point_for(selected.end);
            let tint = accent.opacity(0.25);
            if a.y == b.y {
                selection.push(fill(Bounds::from_corners(origin + a, origin + point(b.x, b.y + line_height)), tint));
            } else {
                let right = bounds.size.width;
                selection.push(fill(
                    Bounds::from_corners(origin + a, origin + point(right, a.y + line_height)),
                    tint,
                ));
                if b.y > a.y + line_height {
                    selection.push(fill(
                        Bounds::from_corners(origin + point(px(0.), a.y + line_height), origin + point(right, b.y)),
                        tint,
                    ));
                }
                selection.push(fill(Bounds::from_corners(origin + point(px(0.), b.y), origin + point(b.x, b.y + line_height)), tint));
            }
        }
        let cursor = (focused && selected.is_empty()).then(|| {
            fill(
                Bounds::new(origin + cursor_pt, size(px((e.font_size / 10.).max(1.5)), line_height)),
                accent,
            )
        });
        Prepaint { lines, starts, tops, line_height, selection, cursor, scroll_y }
    }

    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&gpui::InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut (),
        pp: &mut Prepaint,
        window: &mut Window,
        cx: &mut App,
    ) {
        let focus = self.editor.read(cx).focus.clone();
        window.handle_input(&focus, ElementInputHandler::new(bounds, self.editor.clone()), cx);
        let origin = bounds.origin - point(px(0.), px(pp.scroll_y));
        window.with_content_mask(Some(gpui::ContentMask { bounds }), |window| {
            for q in pp.selection.drain(..) {
                window.paint_quad(q);
            }
            for (line, top) in pp.lines.iter().zip(&pp.tops) {
                let at = origin + point(px(0.), *top);
                if at.y > bounds.bottom() || at.y + line.size(pp.line_height).height < bounds.top() {
                    continue;
                }
                let _ = line.paint(at, pp.line_height, gpui::TextAlign::Left, None, window, cx);
            }
            if let Some(c) = pp.cursor.take() {
                window.paint_quad(c);
            }
        });
        let lines = std::mem::take(&mut pp.lines);
        let starts = std::mem::take(&mut pp.starts);
        let tops = std::mem::take(&mut pp.tops);
        let (line_height, scroll_y) = (pp.line_height, pp.scroll_y);
        self.editor.update(cx, |e, _| {
            e.scroll_y = scroll_y;
            e.layout = Some(Layout { lines, starts, tops, bounds, line_height, scroll_y });
        });
    }
}
