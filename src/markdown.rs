//! Markdown rendered as elements: headings, paragraphs with bold, italic,
//! code and links, lists, quotes, code blocks, rules and tables.

use std::ops::Range;
use std::sync::Arc;

use std::rc::Rc;

use gpui::{
    AnyElement, App, FontStyle, FontWeight, HighlightStyle, InteractiveElement, IntoElement, MouseButton,
    ParentElement, SharedString, Styled, StyledText, UnderlineStyle, Window, div, px,
};
use pulldown_cmark::{Event, HeadingLevel, Options, Parser, Tag, TagEnd};

use crate::theme::MdColors;

/// One run of a paragraph being gathered: its text and highlights.
#[derive(Clone, Default)]
struct Inline {
    text: String,
    highlights: Vec<(Range<usize>, HighlightStyle)>,
}

impl Inline {
    fn is_empty(&self) -> bool {
        self.text.trim().is_empty()
    }
}

#[derive(Clone, Copy)]
enum Container {
    List { ordered: Option<u64> },
    Quote,
}

/// A run as kept for drawing: text and highlights in order, shared, so a
/// frame only bumps reference counts.
struct Text {
    text: SharedString,
    highlights: Arc<[(Range<usize>, HighlightStyle)]>,
}

impl From<Inline> for Text {
    fn from(inl: Inline) -> Self {
        let mut h = inl.highlights;
        h.sort_by_key(|(r, _)| r.start);
        Text { text: inl.text.into(), highlights: h.into() }
    }
}

/// A task item's checkbox: whether it is ticked, and the byte of the source
/// between its brackets (` ` or `x`), which a click flips.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Task {
    pub done: bool,
    pub at: usize,
}

/// What a click on a checkbox does, given its `Task::at`.
pub type OnToggle = Rc<dyn Fn(usize, &mut Window, &mut App)>;

/// The source with the checkbox whose bracketed byte is at `at` flipped, or
/// none when `at` is not one (the text changed meanwhile).
pub fn toggle_task(src: &str, at: usize) -> Option<String> {
    let b = src.as_bytes();
    if at == 0 || b.get(at - 1) != Some(&b'[') || b.get(at + 1) != Some(&b']') {
        return None;
    }
    let next = match b[at] {
        b' ' => "x",
        b'x' | b'X' => " ",
        _ => return None,
    };
    let mut s = src.to_string();
    s.replace_range(at..at + 1, next);
    Some(s)
}

/// A block of the document as parsed, before it becomes an element; its runs
/// are `Inline` while gathered and `Text` once kept.
enum Node<I = Text> {
    Heading(u8, I),
    Para(I),
    Item { depth: usize, marker: SharedString, body: I, quote: usize, task: Option<Task> },
    Code(SharedString),
    Rule,
    Table(Vec<Vec<I>>),
    QuotePara(usize, I),
    Image(String),
}

impl Node<Inline> {
    fn freeze(self) -> Node {
        match self {
            Node::Heading(l, i) => Node::Heading(l, i.into()),
            Node::Para(i) => Node::Para(i.into()),
            Node::Item { depth, marker, body, quote, task } => Node::Item { depth, marker, body: body.into(), quote, task },
            Node::Code(c) => Node::Code(c),
            Node::Rule => Node::Rule,
            Node::Table(rows) => Node::Table(rows.into_iter().map(|r| r.into_iter().map(Text::from).collect()).collect()),
            Node::QuotePara(d, i) => Node::QuotePara(d, i.into()),
            Node::Image(u) => Node::Image(u),
        }
    }
}

/// A parsed Markdown document whose nodes can be rendered across frames without re-parsing.
#[derive(Clone)]
pub struct Parsed {
    nodes: Arc<[Node]>,
    dark: bool,
}

impl Parsed {
    pub fn is_dark(&self) -> bool {
        self.dark
    }
}

fn parse(src: &str, c: &MdColors) -> Vec<Node<Inline>> {
    parse_with(src, c, false)
}

/// With `breaks`, a single newline breaks the line, as typed, instead of
/// joining the lines into one paragraph.
fn parse_with(src: &str, c: &MdColors, breaks: bool) -> Vec<Node<Inline>> {
    let mut nodes = Vec::new();
    let mut stack: Vec<Container> = Vec::new();
    let mut cur: Option<Inline> = None;
    let mut heading: Option<u8> = None;
    let (mut bold, mut italic, mut strike, mut link) = (0, 0, 0, 0);
    let mut code: Option<String> = None;
    let mut table: Option<Vec<Vec<Inline>>> = None;
    let mut item_marker: Option<String> = None;
    let mut item_task: Option<Task> = None;
    let mut image: Option<String> = None;

    let push_text = |cur: &mut Option<Inline>, t: &str, style: HighlightStyle| {
        let inl = cur.get_or_insert_with(Inline::default);
        let start = inl.text.len();
        inl.text.push_str(t);
        if style != HighlightStyle::default() {
            inl.highlights.push((start..inl.text.len(), style));
        }
    };

    let opts = Options::ENABLE_TABLES | Options::ENABLE_STRIKETHROUGH | Options::ENABLE_TASKLISTS;
    for (ev, range) in Parser::new_ext(src, opts).into_offset_iter() {
        let style = HighlightStyle {
            font_weight: (bold > 0).then_some(FontWeight::BOLD),
            font_style: (italic > 0).then_some(FontStyle::Italic),
            color: (link > 0).then_some(c.link),
            underline: (link > 0).then_some(UnderlineStyle { thickness: px(1.), color: Some(c.link), wavy: false }),
            strikethrough: (strike > 0).then_some(gpui::StrikethroughStyle { thickness: px(1.), color: Some(c.muted) }),
            ..Default::default()
        };
        let depth = stack.iter().filter(|s| matches!(s, Container::List { .. })).count();
        let quotes = stack.iter().filter(|s| matches!(s, Container::Quote)).count();
        match ev {
            Event::Start(tag) => match tag {
                Tag::Heading { level, .. } => {
                    heading = Some(match level {
                        HeadingLevel::H1 => 1,
                        HeadingLevel::H2 => 2,
                        HeadingLevel::H3 => 3,
                        _ => 4,
                    })
                }
                Tag::Strong => bold += 1,
                Tag::Emphasis => italic += 1,
                Tag::Strikethrough => strike += 1,
                Tag::Link { .. } => link += 1,
                Tag::CodeBlock(_) => code = Some(String::new()),
                Tag::Image { dest_url, .. } => {
                    // A picture stands on a line of its own.
                    if let Some(inl) = cur.take().filter(|i| !i.is_empty()) {
                        nodes.push(Node::Para(inl));
                    }
                    image = Some(dest_url.to_string());
                }
                Tag::List(start) => {
                    // The text of the item the list is nested in comes first.
                    if let Some(inl) = cur.take() {
                        flush_item(&mut nodes, &mut item_marker, &mut item_task, depth, quotes, inl);
                    }
                    stack.push(Container::List { ordered: start })
                }
                Tag::BlockQuote(_) => stack.push(Container::Quote),
                Tag::Item => {
                    // A paragraph left open in the parent item ends here.
                    if let Some(inl) = cur.take() {
                        flush_item(&mut nodes, &mut item_marker, &mut item_task, depth, quotes, inl);
                    }
                    let marker = match stack.iter_mut().rev().find(|s| matches!(s, Container::List { .. })) {
                        Some(Container::List { ordered: Some(n) }) => {
                            let m = format!("{n}.");
                            *n += 1;
                            m
                        }
                        _ => "•".to_string(),
                    };
                    item_marker = Some(marker);
                    item_task = None;
                }
                Tag::Table(_) => table = Some(Vec::new()),
                Tag::TableHead | Tag::TableRow => {
                    if let Some(t) = table.as_mut() {
                        t.push(Vec::new());
                    }
                }
                _ => {}
            },
            Event::End(tag) => match tag {
                TagEnd::Heading(_) => {
                    nodes.push(Node::Heading(heading.take().unwrap_or(1), cur.take().unwrap_or_default()));
                }
                TagEnd::Paragraph => {
                    if let Some(inl) = cur.take() {
                        if depth > 0 {
                            flush_item(&mut nodes, &mut item_marker, &mut item_task, depth, quotes, inl);
                        } else if quotes > 0 {
                            nodes.push(Node::QuotePara(quotes, inl));
                        } else {
                            nodes.push(Node::Para(inl));
                        }
                    }
                }
                TagEnd::Item => {
                    if let Some(inl) = cur.take() {
                        flush_item(&mut nodes, &mut item_marker, &mut item_task, depth, quotes, inl);
                    } else if let Some(m) = item_marker.take() {
                        let task = item_task.take();
                        nodes.push(Node::Item { depth, marker: m.into(), body: Inline::default(), quote: quotes, task });
                    }
                }
                TagEnd::Strong => bold -= 1,
                TagEnd::Emphasis => italic -= 1,
                TagEnd::Strikethrough => strike -= 1,
                TagEnd::Link => link -= 1,
                TagEnd::CodeBlock => {
                    nodes.push(Node::Code(code.take().unwrap_or_default().trim_end_matches('\n').to_string().into()))
                }
                TagEnd::Image => {
                    if let Some(url) = image.take() {
                        nodes.push(Node::Image(url));
                    }
                }
                TagEnd::List(_) | TagEnd::BlockQuote(_) => {
                    stack.pop();
                }
                TagEnd::TableCell => {
                    let inl = cur.take().unwrap_or_default();
                    if let Some(row) = table.as_mut().and_then(|t| t.last_mut()) {
                        row.push(inl);
                    }
                }
                TagEnd::Table => nodes.push(Node::Table(table.take().unwrap_or_default())),
                _ => {}
            },
            Event::Text(_) if image.is_some() => {} // a picture's alt text
            Event::Text(t) => {
                if let Some(code) = code.as_mut() {
                    code.push_str(&t);
                } else {
                    push_text(&mut cur, &t, style);
                }
            }
            Event::Code(t) => {
                let s = HighlightStyle { background_color: Some(c.code_bg), ..style };
                push_text(&mut cur, &format!("\u{2009}{t}\u{2009}"), s);
            }
            Event::SoftBreak => push_text(&mut cur, if breaks { "\n" } else { " " }, HighlightStyle::default()),
            Event::HardBreak => push_text(&mut cur, "\n", HighlightStyle::default()),
            Event::TaskListMarker(done) => {
                // The event spans `[ ]`; inside a quote the source may differ, so look.
                let at = src[range.clone()].find('[').map(|i| range.start + i + 1);
                match at {
                    Some(at) if toggle_task(src, at).is_some() => item_task = Some(Task { done, at }),
                    _ => push_text(&mut cur, if done { "☑ " } else { "☐ " }, HighlightStyle::default()),
                }
            }
            Event::Rule => nodes.push(Node::Rule),
            Event::Html(t) | Event::InlineHtml(t) => push_text(&mut cur, &t, HighlightStyle::default()),
            _ => {}
        }
    }
    if let Some(inl) = cur.take().filter(|i| !i.is_empty()) {
        nodes.push(Node::Para(inl));
    }
    nodes
}

fn flush_item(
    nodes: &mut Vec<Node<Inline>>,
    marker: &mut Option<String>,
    task: &mut Option<Task>,
    depth: usize,
    quote: usize,
    body: Inline,
) {
    let m = marker.take().unwrap_or_default();
    nodes.push(Node::Item { depth, marker: m.into(), body, quote, task: task.take() });
}

fn styled(t: &Text) -> StyledText {
    StyledText::new(t.text.clone()).with_highlights(t.highlights.iter().cloned())
}

/// Parses `src` once for the given palette so frames during zoom or scroll reuse the AST.
pub fn parse_doc(src: &str, c: &MdColors, dark: bool) -> Parsed {
    Parsed { nodes: parse(src, c).into_iter().map(Node::freeze).collect(), dark }
}

/// Like `parse_doc`, but each newline typed breaks its line: for short text
/// set on the canvas, where a line is meant as written.
pub fn parse_lines(src: &str, c: &MdColors, dark: bool) -> Parsed {
    Parsed { nodes: parse_with(src, c, true).into_iter().map(Node::freeze).collect(), dark }
}

/// Renders a pre-parsed document at font size `size`, in colors `c`.
pub fn render_parsed(doc: &Parsed, size: f32, c: &MdColors) -> AnyElement {
    render_nodes(&doc.nodes, size, c, &local_image, None)
}

/// A picture named by an absolute path; others are not shown.
pub fn local_image(url: &str) -> Option<gpui::ImageSource> {
    let p = std::path::Path::new(url);
    p.is_absolute().then(|| gpui::ImageSource::from(p.to_path_buf()))
}

fn render_nodes(
    nodes: &[Node],
    size: f32,
    c: &MdColors,
    images: &dyn Fn(&str) -> Option<gpui::ImageSource>,
    toggle: Option<&OnToggle>,
) -> AnyElement {
    let gap = size * 0.6;
    let children: Vec<AnyElement> = nodes.iter().map(|n| node_element(n, size, c, images, toggle)).collect();
    root(div().flex().flex_col().gap(px(gap)), size, c).children(children).into_any_element()
}

/// The text style every node inherits.
fn root(d: gpui::Div, size: f32, c: &MdColors) -> gpui::Div {
    d.text_size(px(size)).line_height(px(size * 1.5)).text_color(c.text)
}

impl Parsed {
    /// How many blocks the document has, each one item of `render_item`.
    pub fn len(&self) -> usize {
        self.nodes.len()
    }

    /// Whether `other` is this very parse (not merely equal text).
    pub fn same(&self, other: &Parsed) -> bool {
        Arc::ptr_eq(&self.nodes, &other.nodes)
    }

    /// The document's block `ix` alone, styled as in `render_parsed_with`, with
    /// the gap after it, for a list that lays out only what shows. With
    /// `toggle`, its checkboxes can be clicked.
    pub fn render_item(
        &self,
        ix: usize,
        size: f32,
        c: &MdColors,
        images: &dyn Fn(&str) -> Option<gpui::ImageSource>,
        toggle: Option<&OnToggle>,
    ) -> AnyElement {
        let Some(n) = self.nodes.get(ix) else { return div().into_any_element() };
        let last = ix + 1 == self.nodes.len();
        root(div(), size, c).when(!last, |d| d.pb(px(size * 0.6))).child(node_element(n, size, c, images, toggle)).into_any_element()
    }
}

fn node_element(
    n: &Node,
    size: f32,
    c: &MdColors,
    images: &dyn Fn(&str) -> Option<gpui::ImageSource>,
    toggle: Option<&OnToggle>,
) -> AnyElement {
    match n {
        Node::Heading(level, inl) => {
            let k = match *level {
                1 => 1.6,
                2 => 1.35,
                3 => 1.15,
                _ => 1.0,
            };
            div()
                .text_size(px(size * k))
                .line_height(px(size * k * 1.3))
                .font_weight(FontWeight::BOLD)
                .pt(px(size * 0.3))
                .child(styled(inl))
                .into_any_element()
        }
        Node::Para(inl) => div().child(styled(inl)).into_any_element(),
        Node::QuotePara(depth, inl) => div()
            .pl(px(size * 0.8 * *depth as f32))
            .border_l(px((size / 6.).max(2.)))
            .border_color(c.rule)
            .text_color(c.muted)
            .child(styled(inl))
            .into_any_element(),
        Node::Item { depth, marker, body, quote, task } => {
            let lead = match task {
                Some(task) => checkbox(*task, size, c, toggle),
                None => div().text_color(c.muted).child(marker.clone()).into_any_element(),
            };
            let done = task.is_some_and(|t| t.done);
            div()
                .flex()
                .flex_row()
                .pl(px(size * 1.2 * (depth.saturating_sub(1)) as f32 + size * 0.8 * *quote as f32))
                .gap(px(size * 0.4))
                .child(div().flex_none().min_w(px(size * 0.9)).child(lead))
                .child(div().flex_1().min_w_0().when(done, |d| d.text_color(c.muted)).child(styled(body)))
                .into_any_element()
        }
        Node::Code(code) => div()
            .bg(c.code_bg)
            .rounded(px(size * 0.35))
            .px(px(size * 0.7))
            .py(px(size * 0.5))
            .font_family(crate::font::mono())
            .text_size(px(size * 0.88))
            .child(code.clone())
            .into_any_element(),
        Node::Rule => div().h(px(1.)).bg(c.rule).my(px(size * 0.3)).into_any_element(),
        Node::Image(url) => match images(url) {
            Some(src) => gpui::img(src).max_w_full().into_any_element(),
            None => div().text_color(c.muted).child(format!("［图片：{url}］")).into_any_element(),
        },
        Node::Table(rows) => div()
            .flex()
            .flex_col()
            .border_1()
            .border_color(c.rule)
            .children(rows.iter().enumerate().map(|(i, row)| {
                div()
                    .flex()
                    .flex_row()
                    .when(i > 0, |d| d.border_t_1().border_color(c.rule))
                    .when(i == 0, |d| d.font_weight(FontWeight::BOLD))
                    .children(row.iter().map(|cell| {
                        div().flex_1().min_w_0().px(px(size * 0.5)).py(px(size * 0.25)).child(styled(cell))
                    }))
            }))
            .into_any_element(),
    }
}

/// A task's box, on the first line of its item; ticked, it is filled with a check.
/// Clicking it (with `toggle`) flips the task and does nothing else.
fn checkbox(task: Task, size: f32, c: &MdColors, toggle: Option<&OnToggle>) -> AnyElement {
    let side = (size * 0.85).round().max(6.);
    let check = || {
        gpui::svg()
            .path("icons/check.svg")
            .size(px(side * 0.8))
            .text_color(gpui::white())
    };
    let mut b = div()
        .size(px(side))
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(side * 0.22))
        .border(px((size / 12.).max(1.)))
        .when(task.done, |d| d.bg(c.link).border_color(c.link).child(check()))
        .when(!task.done, |d| d.border_color(c.muted));
    if let Some(f) = toggle {
        let f = f.clone();
        b = b.cursor_pointer().on_mouse_down(MouseButton::Left, move |_, window, cx| {
            cx.stop_propagation();
            f(task.at, window, cx);
        });
    }
    // Centered on the first line, whose height is the root's line height.
    div().h(px(size * 1.5)).flex().items_center().child(b).into_any_element()
}

use gpui::prelude::FluentBuilder as _;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::{MdColors, Theme};

    fn kinds(src: &str) -> Vec<String> {
        let c = MdColors::of_theme(&Theme::light());
        parse(src, &c)
            .into_iter()
            .map(|n| match n {
                Node::Heading(l, i) => format!("h{l}:{}", i.text),
                Node::Para(i) => format!("p:{}", i.text),
                Node::QuotePara(d, i) => format!("q{d}:{}", i.text),
                Node::Item { depth, marker, body, .. } => format!("li{depth}{marker}:{}", body.text),
                Node::Code(s) => format!("code:{}", s.trim()),
                Node::Rule => "hr".into(),
                Node::Table(r) => format!("table:{}x{}", r.len(), r[0].len()),
                Node::Image(u) => format!("img:{u}"),
            })
            .collect()
    }

    #[test]
    fn parses_the_blocks_notes_use() {
        let got = kinds("# 标题\n\n段落 **粗** *斜*\n\n- 一\n- 二\n  1. 甲\n\n> 引用\n\n```\nx\n```\n\n---\n\n|a|b|\n|-|-|\n|1|2|\n");
        assert_eq!(
            got,
            vec![
                "h1:标题",
                "p:段落 粗 斜",
                "li1•:一",
                "li1•:二",
                "li21.:甲",
                "q1:引用",
                "code:x",
                "hr",
                "table:2x2"
            ]
        );
    }

    #[test]
    fn task_boxes_point_at_their_source() {
        let c = MdColors::of_theme(&Theme::light());
        let src = "- [ ] 买菜\n- [x] 洗衣\n  - [ ] 嵌套\n\n> - [X] 引用里\n";
        let tasks: Vec<(String, Task)> = parse(src, &c)
            .into_iter()
            .filter_map(|n| match n {
                Node::Item { body, task: Some(t), .. } => Some((body.text, t)),
                _ => None,
            })
            .collect();
        let names: Vec<&str> = tasks.iter().map(|(b, _)| b.as_str()).collect();
        assert_eq!(names, ["买菜", "洗衣", "嵌套", "引用里"]);
        assert_eq!(tasks.iter().map(|(_, t)| t.done).collect::<Vec<_>>(), [false, true, false, true]);
        for (_, t) in &tasks {
            assert_eq!(&src[t.at - 1..t.at + 2].to_ascii_lowercase(), if t.done { "[x]" } else { "[ ]" });
        }
        let ticked = toggle_task(src, tasks[0].1.at).unwrap();
        assert!(ticked.starts_with("- [x] 买菜\n- [x] 洗衣"));
        let unticked = toggle_task(src, tasks[3].1.at).unwrap();
        assert!(unticked.ends_with("> - [ ] 引用里\n"));
        // Not a checkbox: nothing happens.
        assert_eq!(toggle_task(src, 0), None);
        assert_eq!(toggle_task(src, src.len()), None);
    }

    #[test]
    fn lines_can_break_where_typed() {
        let c = MdColors::of_theme(&Theme::light());
        let text = |n: &Node<Inline>| match n {
            Node::Para(i) => i.text.clone(),
            _ => String::new(),
        };
        assert_eq!(text(&parse("一\n二", &c)[0]), "一 二");
        assert_eq!(text(&parse_with("一\n二", &c, true)[0]), "一\n二");
    }

    #[test]
    fn pictures_stand_alone() {
        assert_eq!(kinds("前 ![alt](a.png) 后"), vec!["p:前 ", "img:a.png", "p: 后"]);
    }

    #[test]
    fn highlights_are_in_order_and_inside_the_text() {
        let c = MdColors::of_theme(&Theme::light());
        for n in parse("a **b *c* d** `e` [f](g)", &c) {
            if let Node::Para(i) = n {
                let mut last = 0;
                for (r, _) in &i.highlights {
                    assert!(r.start >= last && r.end <= i.text.len());
                    last = r.end;
                }
            }
        }
    }
}
