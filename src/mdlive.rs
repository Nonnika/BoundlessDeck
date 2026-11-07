//! Typora-style Markdown for the 笔记 editor. Each source line splits into
//! pieces that show as written (text), go away (markup: `**`, `# `, a link's
//! url), or show as something else (`- ` as a bullet). The lines the cursor
//! or selection touch show all their source, markup dimmed; the others show
//! rendered. A `Map` turns offsets in the source line into offsets in what is
//! shown and back, so the editor keeps editing the source.

use std::ops::Range;

/// How a piece of text is styled.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Sty {
    pub bold: bool,
    pub italic: bool,
    pub code: bool,
    pub strike: bool,
    pub link: bool,
    /// Dimmed: markup shown on the cursor's line, an image's alt text.
    pub dim: bool,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Show {
    /// Shown as written.
    Text,
    /// Markup: hidden when rendered.
    Mark,
    /// Shown as this when rendered.
    As(&'static str),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Piece {
    /// Its bytes in the line.
    pub src: Range<usize>,
    pub show: Show,
    pub sty: Sty,
}

/// What a whole line is.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum Block {
    #[default]
    Para,
    Heading(u8),
    Quote,
    /// A line inside a fenced code block.
    Code,
    /// A fence (```).
    Fence,
    /// A thematic break (---).
    Rule,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Line {
    pub block: Block,
    /// They cover the line, in order.
    pub pieces: Vec<Piece>,
}

/// Each line of text (split at '\n'), parsed.
pub fn lines(text: &str) -> Vec<Line> {
    let mut fence = false;
    text.split('\n').map(|s| line(s, &mut fence)).collect()
}

fn piece(src: Range<usize>, show: Show, sty: Sty) -> Piece {
    Piece { src, show, sty }
}

fn line(s: &str, fence: &mut bool) -> Line {
    let t = s.trim_start();
    let ind = s.len() - t.len();
    let code = Sty { code: true, ..Sty::default() };
    let all = 0..s.len();
    if t.starts_with("```") || t.starts_with("~~~") {
        *fence = !*fence;
        return Line { block: Block::Fence, pieces: vec![piece(all, Show::Mark, code)] };
    }
    if *fence {
        let pieces = if s.is_empty() { vec![] } else { vec![piece(all, Show::Text, code)] };
        return Line { block: Block::Code, pieces };
    }
    if s.is_empty() {
        return Line::default();
    }
    if is_rule(t) {
        return Line { block: Block::Rule, pieces: vec![piece(all, Show::Mark, Sty::default())] };
    }
    let mut pieces = Vec::new();
    let b = t.as_bytes();
    // A heading: up to six #, then a space or the end.
    let hashes = b.iter().take_while(|&&c| c == b'#').count();
    if ind <= 3 && (1..=6).contains(&hashes) && (b.len() == hashes || b[hashes] == b' ') {
        let body = ind + hashes + t[hashes..].len() - t[hashes..].trim_start().len();
        pieces.push(piece(0..body, Show::Mark, Sty::default()));
        inline(s, body, s.len(), Sty::default(), &mut pieces);
        return Line { block: Block::Heading(hashes as u8), pieces };
    }
    if b[0] == b'>' {
        let body = ind + if b.get(1) == Some(&b' ') { 2 } else { 1 };
        pieces.push(piece(0..body, Show::Mark, Sty::default()));
        inline(s, body, s.len(), Sty::default(), &mut pieces);
        return Line { block: Block::Quote, pieces };
    }
    if ind > 0 {
        pieces.push(piece(0..ind, Show::Text, Sty::default()));
    }
    let bullet = matches!(b[0], b'-' | b'*' | b'+') && b.get(1) == Some(&b' ');
    if bullet && b.len() >= 6 && b[2] == b'[' && b[4] == b']' && b[5] == b' ' && matches!(b[3], b' ' | b'x' | b'X') {
        let shown = if b[3] == b' ' { "☐ " } else { "☑ " };
        pieces.push(piece(ind..ind + 6, Show::As(shown), Sty::default()));
        inline(s, ind + 6, s.len(), Sty::default(), &mut pieces);
    } else if bullet {
        pieces.push(piece(ind..ind + 2, Show::As("• "), Sty::default()));
        inline(s, ind + 2, s.len(), Sty::default(), &mut pieces);
    } else {
        let digits = b.iter().take_while(|c| c.is_ascii_digit()).count();
        let ordered = (1..=9).contains(&digits) && matches!(b.get(digits), Some(b'.' | b')')) && b.get(digits + 1) == Some(&b' ');
        let body = if ordered { ind + digits + 2 } else { ind };
        if body > ind {
            pieces.push(piece(ind..body, Show::Text, Sty::default()));
        }
        inline(s, body, s.len(), Sty::default(), &mut pieces);
    }
    Line { block: Block::Para, pieces }
}

/// `---`, `***` or `___`: three or more of one, spaces between allowed.
fn is_rule(t: &str) -> bool {
    let t = t.trim_end();
    let Some(c) = t.chars().next().filter(|c| matches!(c, '-' | '*' | '_')) else { return false };
    t.chars().all(|x| x == c || x == ' ') && t.chars().filter(|&x| x == c).count() >= 3
}

/// How many times byte c repeats from i.
fn run(b: &[u8], i: usize, c: u8) -> usize {
    b[i..].iter().take_while(|&&x| x == c).count()
}

/// Inline Markdown in s[from..to]: emphasis, code, strikethrough, links,
/// images and escapes. What is not markup is text in `sty`.
fn inline(s: &str, from: usize, to: usize, sty: Sty, out: &mut Vec<Piece>) {
    let b = s.as_bytes();
    let mut text = from;
    let mut i = from;
    let flush = |out: &mut Vec<Piece>, a: usize, z: usize| {
        if z > a {
            out.push(piece(a..z, Show::Text, sty));
        }
    };
    while i < to {
        let c = b[i];
        match c {
            b'\\' if i + 1 < to && b[i + 1].is_ascii_punctuation() => {
                flush(out, text, i);
                out.push(piece(i..i + 1, Show::Mark, sty));
                out.push(piece(i + 1..i + 2, Show::Text, sty));
                i += 2;
                text = i;
            }
            b'`' => {
                let n = run(b, i, b'`');
                let close = (i + n..to).find(|&j| b[j] == b'`' && run(b, j, b'`') == n && (j == 0 || b[j - 1] != b'`'));
                match close {
                    Some(j) if j > i + n => {
                        flush(out, text, i);
                        out.push(piece(i..i + n, Show::Mark, sty));
                        out.push(piece(i + n..j, Show::Text, Sty { code: true, ..sty }));
                        out.push(piece(j..j + n, Show::Mark, sty));
                        i = j + n;
                        text = i;
                    }
                    _ => i += n,
                }
            }
            b'*' | b'_' | b'~' => {
                let n = run(b, i, c);
                let d = n;
                let ok_len = if c == b'~' { d == 2 } else { d <= 3 };
                // An opener: not after a word character for `_`, and followed by a non-space.
                let opens = ok_len
                    && i + d < to
                    && b[i + d] != b' '
                    && !(c == b'_' && i > 0 && b[i - 1].is_ascii_alphanumeric());
                let close = opens
                    .then(|| {
                        let mut j = i + d;
                        while j < to {
                            if b[j] == c {
                                let m = run(b, j, c).min(to - j);
                                // A longer run closes with its last d: `**a *b***`.
                                let fits = m >= d
                                    && b[j - 1] != b' '
                                    && !(c == b'_' && j + m < to && b[j + m].is_ascii_alphanumeric());
                                if fits && j + m - d > i + d {
                                    return Some(j + m - d);
                                }
                                j += m;
                            } else if b[j] == b'`' {
                                // Skip a code span: markup inside it is not ours.
                                let k = run(b, j, b'`');
                                j = (j + k..to).find(|&x| run(b, x, b'`') == k).map_or(j + k, |x| x + k);
                            } else {
                                j += 1;
                            }
                        }
                        None
                    })
                    .flatten();
                match close {
                    Some(j) => {
                        flush(out, text, i);
                        let inner = match (c, d) {
                            (b'~', _) => Sty { strike: true, ..sty },
                            (_, 1) => Sty { italic: true, ..sty },
                            (_, 2) => Sty { bold: true, ..sty },
                            _ => Sty { bold: true, italic: true, ..sty },
                        };
                        out.push(piece(i..i + d, Show::Mark, sty));
                        inline(s, i + d, j, inner, out);
                        out.push(piece(j..j + d, Show::Mark, sty));
                        i = j + d;
                        text = i;
                    }
                    None => i += n,
                }
            }
            b'[' | b'!' => {
                let image = c == b'!';
                let open = if image { i + 1 } else { i };
                let link = (open < to && b[open] == b'[')
                    .then(|| {
                        let close = (open + 1..to).find(|&j| b[j] == b']')?;
                        (close + 1 < to && b[close + 1] == b'(').then_some(())?;
                        let end = (close + 2..to).find(|&j| b[j] == b')')?;
                        Some((close, end))
                    })
                    .flatten();
                match link {
                    Some((close, end)) => {
                        flush(out, text, i);
                        out.push(piece(i..open + 1, Show::Mark, sty));
                        if image {
                            if close > open + 1 {
                                out.push(piece(open + 1..close, Show::Text, Sty { dim: true, italic: true, ..sty }));
                            }
                        } else {
                            inline(s, open + 1, close, Sty { link: true, ..sty }, out);
                        }
                        out.push(piece(close..end + 1, Show::Mark, sty));
                        i = end + 1;
                        text = i;
                    }
                    None => i += 1,
                }
            }
            _ => i += 1,
        }
    }
    flush(out, text, to);
}

/// One piece of a line's map: `src` shows as `disp`; a verbatim one maps
/// byte for byte, another one as a whole.
#[derive(Clone, Debug, PartialEq)]
struct Seg {
    src: Range<usize>,
    disp: Range<usize>,
    verbatim: bool,
}

/// Offsets between a source line and what it shows.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Map {
    segs: Vec<Seg>,
    src_len: usize,
    disp_len: usize,
}

impl Map {
    /// Where source offset o shows.
    pub fn to_disp(&self, o: usize) -> usize {
        if let Some(s) = self.segs.iter().find(|s| s.verbatim && s.src.start <= o && o <= s.src.end) {
            return s.disp.start + (o - s.src.start);
        }
        if let Some(s) = self.segs.iter().find(|s| s.src.start <= o && o <= s.src.end) {
            return if o == s.src.start { s.disp.start } else { s.disp.end };
        }
        self.disp_len
    }

    /// The source offset behind shown offset d.
    pub fn to_src(&self, d: usize) -> usize {
        if d >= self.disp_len {
            return self.src_len;
        }
        if let Some(s) = self.segs.iter().find(|s| s.verbatim && s.disp.start <= d && d <= s.disp.end) {
            return s.src.start + (d - s.disp.start);
        }
        if let Some(s) = self.segs.iter().find(|s| s.disp.start <= d && d <= s.disp.end) {
            return if d == s.disp.start { s.src.start } else { s.src.end };
        }
        self.src_len
    }
}

/// A line as it shows: its text, styled spans over it, and its map (none
/// when it shows its source as it is).
#[derive(Clone, Debug, Default)]
pub struct Shown {
    pub text: String,
    pub spans: Vec<(Range<usize>, Sty)>,
    pub map: Option<Map>,
}

/// How src, parsed as line, shows: rendered, or as source when revealed.
pub fn show(src: &str, line: &Line, reveal: bool) -> Shown {
    let mut text = String::new();
    let mut spans = Vec::new();
    let mut segs = Vec::new();
    for p in &line.pieces {
        let s = &src[p.src.clone()];
        let at = text.len();
        let (shown, sty, verbatim) = match (p.show, reveal) {
            (Show::Text, _) => (s, p.sty, true),
            (_, true) => (s, Sty { dim: true, ..p.sty }, true),
            (Show::Mark, false) => ("", p.sty, false),
            (Show::As(r), false) => (r, p.sty, false),
        };
        text.push_str(shown);
        if !shown.is_empty() {
            spans.push((at..text.len(), sty));
        }
        segs.push(Seg { src: p.src.clone(), disp: at..text.len(), verbatim });
    }
    let map = (!reveal).then_some(Map { segs, src_len: src.len(), disp_len: text.len() });
    Shown { text, spans, map }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rendered(s: &str) -> String {
        let ls = lines(s);
        s.split('\n').zip(&ls).map(|(src, l)| show(src, l, false).text).collect::<Vec<_>>().join("\n")
    }

    #[test]
    fn markup_goes_when_rendered() {
        assert_eq!(rendered("# 计划"), "计划");
        assert_eq!(rendered("写 **测试** 与 *斜体* 和 `code`"), "写 测试 与 斜体 和 code");
        assert_eq!(rendered("- 一\n  * 二\n- [ ] 待办\n- [x] 完成"), "• 一\n  • 二\n☐ 待办\n☑ 完成");
        assert_eq!(rendered("> 引用"), "引用");
        assert_eq!(rendered("看 [文档](https://a.b/c) 吧"), "看 文档 吧");
        assert_eq!(rendered("![图](x.png)"), "图");
        assert_eq!(rendered("~~删~~ \\*不是\\*"), "删 *不是*");
        assert_eq!(rendered("---"), "");
        assert_eq!(rendered("1. 第一"), "1. 第一");
    }

    #[test]
    fn what_is_not_markup_stays() {
        assert_eq!(rendered("a * b * c"), "a * b * c");
        assert_eq!(rendered("snake_case_name"), "snake_case_name");
        assert_eq!(rendered("**未闭合"), "**未闭合");
        assert_eq!(rendered("[不是链接]"), "[不是链接]");
        assert_eq!(rendered("#标签"), "#标签");
    }

    #[test]
    fn code_fences_hold_their_lines() {
        let ls = lines("```rust\nlet **a** = 1;\n```\n**b**");
        assert_eq!(ls[0].block, Block::Fence);
        assert_eq!(ls[1].block, Block::Code);
        assert_eq!(ls[2].block, Block::Fence);
        assert_eq!(rendered("```rust\nlet **a** = 1;\n```\n**b**"), "\nlet **a** = 1;\n\nb");
    }

    #[test]
    fn styles_follow_the_markup() {
        let l = &lines("**粗 *斜***")[0];
        let sh = show("**粗 *斜***", l, false);
        assert_eq!(sh.text, "粗 斜");
        let bold_italic = sh.spans.iter().find(|(r, _)| &sh.text[r.clone()] == "斜").unwrap().1;
        assert!(bold_italic.bold && bold_italic.italic);
    }

    #[test]
    fn revealed_lines_show_their_source() {
        for s in ["# 计划", "- [x] **完成** [链接](u)", "> `x`", "---", "```"] {
            let l = &lines(s)[0];
            let sh = show(s, l, true);
            assert_eq!(sh.text, s);
            assert!(sh.map.is_none());
            assert_eq!(sh.spans.iter().map(|(r, _)| r.len()).sum::<usize>(), s.len());
        }
    }

    #[test]
    fn offsets_map_both_ways() {
        let s = "# 写 **测试**";
        let sh = show(s, &lines(s)[0], false);
        let m = sh.map.unwrap();
        assert_eq!(sh.text, "写 测试");
        // The start of what shows is the start of the heading's text.
        assert_eq!(m.to_src(0), 2);
        assert_eq!(m.to_disp(2), 0);
        // Inside the bold text, byte for byte.
        let t = s.find("测").unwrap();
        assert_eq!(m.to_disp(t), sh.text.find("测").unwrap());
        assert_eq!(m.to_src(sh.text.find("试").unwrap()), s.find("试").unwrap());
        // The end of the line is the end of the source.
        assert_eq!(m.to_src(sh.text.len()), s.len());
        assert_eq!(m.to_disp(s.len()), sh.text.len());
        // Inside hidden markup: to its edge.
        assert_eq!(m.to_disp(1), 0);
        let b = "- 项";
        let m = show(b, &lines(b)[0], false).map.unwrap();
        assert_eq!(m.to_src("• ".len()), 2);
    }
}
