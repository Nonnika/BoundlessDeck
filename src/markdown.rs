//! Markdown rendered as elements: headings, paragraphs with bold, italic,
//! code and links, lists, quotes, code blocks, rules and tables.

use std::ops::Range;
use std::sync::Arc;

use gpui::{
    AnyElement, FontStyle, FontWeight, HighlightStyle, IntoElement, ParentElement, SharedString, Styled,
    StyledText, UnderlineStyle, div, px,
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

/// A block of the document as parsed, before it becomes an element; its runs
/// are `Inline` while gathered and `Text` once kept.
enum Node<I = Text> {
    Heading(u8, I),
    Para(I),
    Item { depth: usize, marker: SharedString, body: I, quote: usize },
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
            Node::Item { depth, marker, body, quote } => Node::Item { depth, marker, body: body.into(), quote },
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
    let mut nodes = Vec::new();
    let mut stack: Vec<Container> = Vec::new();
    let mut cur: Option<Inline> = None;
    let mut heading: Option<u8> = None;
    let (mut bold, mut italic, mut strike, mut link) = (0, 0, 0, 0);
    let mut code: Option<String> = None;
    let mut table: Option<Vec<Vec<Inline>>> = None;
    let mut item_marker: Option<String> = None;
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
    for ev in Parser::new_ext(src, opts) {
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
                        flush_item(&mut nodes, &mut item_marker, depth, quotes, inl);
                    }
                    stack.push(Container::List { ordered: start })
                }
                Tag::BlockQuote(_) => stack.push(Container::Quote),
                Tag::Item => {
                    // A paragraph left open in the parent item ends here.
                    if let Some(inl) = cur.take() {
                        flush_item(&mut nodes, &mut item_marker, depth, quotes, inl);
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
                            flush_item(&mut nodes, &mut item_marker, depth, quotes, inl);
                        } else if quotes > 0 {
                            nodes.push(Node::QuotePara(quotes, inl));
                        } else {
                            nodes.push(Node::Para(inl));
                        }
                    }
                }
                TagEnd::Item => {
                    if let Some(inl) = cur.take() {
                        flush_item(&mut nodes, &mut item_marker, depth, quotes, inl);
                    } else if let Some(m) = item_marker.take() {
                        nodes.push(Node::Item { depth, marker: m.into(), body: Inline::default(), quote: quotes });
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
            Event::SoftBreak => push_text(&mut cur, " ", HighlightStyle::default()),
            Event::HardBreak => push_text(&mut cur, "\n", HighlightStyle::default()),
            Event::TaskListMarker(done) => {
                push_text(&mut cur, if done { "☑ " } else { "☐ " }, HighlightStyle::default())
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

fn flush_item(nodes: &mut Vec<Node<Inline>>, marker: &mut Option<String>, depth: usize, quote: usize, body: Inline) {
    let m = marker.take().unwrap_or_default();
    nodes.push(Node::Item { depth, marker: m.into(), body, quote });
}

fn styled(t: &Text) -> StyledText {
    StyledText::new(t.text.clone()).with_highlights(t.highlights.iter().cloned())
}

/// Parses `src` once for the given palette so frames during zoom or scroll reuse the AST.
pub fn parse_doc(src: &str, c: &MdColors, dark: bool) -> Parsed {
    Parsed { nodes: parse(src, c).into_iter().map(Node::freeze).collect(), dark }
}

/// Renders a pre-parsed document at font size `size`, in colors `c`.
pub fn render_parsed(doc: &Parsed, size: f32, c: &MdColors) -> AnyElement {
    render_nodes(&doc.nodes, size, c, &local_image)
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
) -> AnyElement {
    let gap = size * 0.6;
    let children: Vec<AnyElement> = nodes.iter().map(|n| node_element(n, size, c, images)).collect();
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
    /// the gap after it, for a list that lays out only what shows.
    pub fn render_item(
        &self,
        ix: usize,
        size: f32,
        c: &MdColors,
        images: &dyn Fn(&str) -> Option<gpui::ImageSource>,
    ) -> AnyElement {
        let Some(n) = self.nodes.get(ix) else { return div().into_any_element() };
        let last = ix + 1 == self.nodes.len();
        root(div(), size, c).when(!last, |d| d.pb(px(size * 0.6))).child(node_element(n, size, c, images)).into_any_element()
    }
}

fn node_element(n: &Node, size: f32, c: &MdColors, images: &dyn Fn(&str) -> Option<gpui::ImageSource>) -> AnyElement {
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
        Node::Item { depth, marker, body, quote } => div()
            .flex()
            .flex_row()
            .pl(px(size * 1.2 * (depth.saturating_sub(1)) as f32 + size * 0.8 * *quote as f32))
            .gap(px(size * 0.4))
            .child(div().flex_none().min_w(px(size * 0.9)).text_color(c.muted).child(marker.clone()))
            .child(div().flex_1().min_w_0().child(styled(body)))
            .into_any_element(),
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
