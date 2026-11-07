//! Markdown rendered as elements: headings, paragraphs with bold, italic,
//! code and links, lists, quotes, code blocks, rules and tables.

use std::ops::Range;

use gpui::{
    AnyElement, FontStyle, FontWeight, HighlightStyle, IntoElement, ParentElement, SharedString, Styled,
    StyledText, UnderlineStyle, div, px,
};
use pulldown_cmark::{Event, HeadingLevel, Options, Parser, Tag, TagEnd};

use crate::theme::MdColors;

/// One run of a paragraph being gathered: its text and highlights.
#[derive(Default)]
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

/// A block of the document as parsed, before it becomes an element.
enum Node {
    Heading(u8, Inline),
    Para(Inline),
    Item { depth: usize, marker: String, body: Inline, quote: usize },
    Code(String),
    Rule,
    Table(Vec<Vec<Inline>>),
    QuotePara(usize, Inline),
}

fn parse(src: &str, c: &MdColors) -> Vec<Node> {
    let mut nodes = Vec::new();
    let mut stack: Vec<Container> = Vec::new();
    let mut cur: Option<Inline> = None;
    let mut heading: Option<u8> = None;
    let (mut bold, mut italic, mut strike, mut link) = (0, 0, 0, 0);
    let mut code: Option<String> = None;
    let mut table: Option<Vec<Vec<Inline>>> = None;
    let mut item_marker: Option<String> = None;

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
                        nodes.push(Node::Item { depth, marker: m, body: Inline::default(), quote: quotes });
                    }
                }
                TagEnd::Strong => bold -= 1,
                TagEnd::Emphasis => italic -= 1,
                TagEnd::Strikethrough => strike -= 1,
                TagEnd::Link => link -= 1,
                TagEnd::CodeBlock => nodes.push(Node::Code(code.take().unwrap_or_default())),
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

fn flush_item(nodes: &mut Vec<Node>, marker: &mut Option<String>, depth: usize, quote: usize, body: Inline) {
    let m = marker.take().unwrap_or_default();
    nodes.push(Node::Item { depth, marker: m, body, quote });
}

fn styled(inl: Inline) -> StyledText {
    let mut h = inl.highlights;
    h.sort_by_key(|(r, _)| r.start);
    StyledText::new(SharedString::from(inl.text)).with_highlights(h)
}

/// Renders src at font size `size`, in colors `c`.
pub fn render(src: &str, size: f32, c: &MdColors) -> AnyElement {
    let gap = size * 0.6;
    let children: Vec<AnyElement> = parse(src, c)
        .into_iter()
        .map(|n| match n {
            Node::Heading(level, inl) => {
                let k = match level {
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
                .pl(px(size * 0.8 * depth as f32))
                .border_l(px((size / 6.).max(2.)))
                .border_color(c.rule)
                .text_color(c.muted)
                .child(styled(inl))
                .into_any_element(),
            Node::Item { depth, marker, body, quote } => div()
                .flex()
                .flex_row()
                .pl(px(size * 1.2 * (depth.saturating_sub(1)) as f32 + size * 0.8 * quote as f32))
                .gap(px(size * 0.4))
                .child(div().flex_none().min_w(px(size * 0.9)).text_color(c.muted).child(marker))
                .child(div().flex_1().min_w_0().child(styled(body)))
                .into_any_element(),
            Node::Code(code) => div()
                .bg(c.code_bg)
                .rounded(px(size * 0.35))
                .px(px(size * 0.7))
                .py(px(size * 0.5))
                .font_family("Menlo")
                .text_size(px(size * 0.88))
                .child(code.trim_end_matches('\n').to_string())
                .into_any_element(),
            Node::Rule => div().h(px(1.)).bg(c.rule).my(px(size * 0.3)).into_any_element(),
            Node::Table(rows) => div()
                .flex()
                .flex_col()
                .border_1()
                .border_color(c.rule)
                .children(rows.into_iter().enumerate().map(|(i, row)| {
                    div()
                        .flex()
                        .flex_row()
                        .when(i > 0, |d| d.border_t_1().border_color(c.rule))
                        .when(i == 0, |d| d.font_weight(FontWeight::BOLD))
                        .children(row.into_iter().map(|cell| {
                            div().flex_1().min_w_0().px(px(size * 0.5)).py(px(size * 0.25)).child(styled(cell))
                        }))
                }))
                .into_any_element(),
        })
        .collect();
    div()
        .flex()
        .flex_col()
        .gap(px(gap))
        .text_size(px(size))
        .line_height(px(size * 1.5))
        .text_color(c.text)
        .children(children)
        .into_any_element()
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
