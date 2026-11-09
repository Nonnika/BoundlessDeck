//! Word documents (.docx, and .doc through macOS's textutil) read as
//! Markdown for their preview: text, headings (also hand-sized bold ones),
//! bold and italic, lists by numbering.xml, links, tables and pictures.

use std::collections::HashMap;
use std::io::{Cursor, Read as _};
use std::path::Path;

use quick_xml::events::{BytesStart, Event};

/// A Word document read for its preview: its Markdown, and the pictures
/// it shows, by the names its Markdown gives them (`word/media/…`).
#[derive(Debug, Default)]
pub struct Doc {
    pub markdown: String,
    pub media: HashMap<String, Vec<u8>>,
}

/// What the Markdown's picture links start with.
pub const MEDIA_SCHEME: &str = "docx-media:";
const MAX_PART: u64 = 64 << 20;

/// Reads a .docx, or a .doc by way of textutil.
pub fn read(file: &Path) -> Result<Doc, String> {
    let is_doc = file.extension().and_then(|e| e.to_str()).is_some_and(|e| e.eq_ignore_ascii_case("doc"));
    if is_doc {
        return read_doc(file);
    }
    let data = std::fs::read(file).map_err(|e| e.to_string())?;
    read_docx(&data)
}

fn read_doc(file: &Path) -> Result<Doc, String> {
    let dir = tempfile::tempdir().map_err(|e| e.to_string())?;
    let out = dir.path().join("doc.docx");
    let res = std::process::Command::new("textutil")
        .args(["-convert", "docx", "-output"])
        .arg(&out)
        .arg(file)
        .output()
        .map_err(|_| ".doc 预览需要 macOS 的 textutil".to_string())?;
    if !res.status.success() {
        return Err(format!("无法读取这个 .doc：{}", String::from_utf8_lossy(&res.stderr).trim()));
    }
    read_docx(&std::fs::read(&out).map_err(|e| e.to_string())?)
}

/// Reads a .docx from its bytes.
pub fn read_docx(data: &[u8]) -> Result<Doc, String> {
    let bad = || "不是可以打开的 Word 文档".to_string();
    let mut zip = zip::ZipArchive::new(Cursor::new(data)).map_err(|_| bad())?;
    let mut part = |name: &str| -> Option<Vec<u8>> {
        let f = zip.by_name(name).ok()?;
        let mut buf = Vec::new();
        f.take(MAX_PART + 1).read_to_end(&mut buf).ok()?;
        (buf.len() as u64 <= MAX_PART).then_some(buf)
    };
    let body = part("word/document.xml").ok_or_else(bad)?;
    let mut targets = HashMap::new();
    if let Some(rels) = part("word/_rels/document.xml.rels") {
        for el in starts(&rels) {
            if el.0 == "Relationship" {
                let (id, target) = (el.attr("Id"), el.attr("Target"));
                let t = if el.attr("TargetMode") == "External" { target } else { clean_join("word", &target) };
                targets.insert(id, t);
            }
        }
    }
    let numbered = part("word/numbering.xml").map(|n| numbering(&n)).unwrap_or_default();
    let mut doc = Doc::default();
    let mut pictures = Vec::new();
    doc.markdown = markdown(&body, &targets, &mut |id| {
        let name = targets.get(id)?.clone();
        pictures.push(name.clone());
        Some(format!("{MEDIA_SCHEME}{name}"))
    }, &|num, lvl| numbered.get(num).and_then(|l| l.get(&lvl)).copied().unwrap_or(false))?;
    for name in pictures {
        if !doc.media.contains_key(&name)
            && let Some(b) = part(&name)
        {
            doc.media.insert(name, b);
        }
    }
    Ok(doc)
}

fn clean_join(base: &str, rel: &str) -> String {
    let mut parts: Vec<&str> = base.split('/').filter(|s| !s.is_empty()).collect();
    for seg in rel.split('/') {
        match seg {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            s => parts.push(s),
        }
    }
    parts.join("/")
}

/// A start or empty element: its local name and attributes by local name.
struct El(String, Vec<(String, String)>);

impl El {
    fn of(e: &BytesStart) -> El {
        let name = String::from_utf8_lossy(e.local_name().as_ref()).into_owned();
        let attrs = e
            .attributes()
            .flatten()
            .map(|a| {
                (
                    String::from_utf8_lossy(a.key.local_name().as_ref()).into_owned(),
                    a.unescape_value().map(|v| v.into_owned()).unwrap_or_default(),
                )
            })
            .collect();
        El(name, attrs)
    }
    fn attr(&self, name: &str) -> String {
        self.1.iter().find(|(k, _)| k == name).map(|(_, v)| v.clone()).unwrap_or_default()
    }
    /// A toggle of a run's style, as <w:b/> or <w:b w:val="0"/>.
    fn on(&self) -> bool {
        !matches!(self.attr("val").as_str(), "0" | "false" | "off" | "none")
    }
}

fn starts(xml: &[u8]) -> Vec<El> {
    let mut r = quick_xml::Reader::from_reader(xml);
    let mut out = Vec::new();
    let mut buf = Vec::new();
    loop {
        match r.read_event_into(&mut buf) {
            Ok(Event::Start(e) | Event::Empty(e)) => out.push(El::of(&e)),
            Ok(Event::Eof) | Err(_) => break,
            _ => {}
        }
        buf.clear();
    }
    out
}

/// Which levels of which lists count 1. 2. 3. rather than showing bullets.
fn numbering(xml: &[u8]) -> HashMap<String, HashMap<i32, bool>> {
    let mut abstract_: HashMap<String, HashMap<i32, bool>> = HashMap::new();
    let mut nums: Vec<(String, String)> = Vec::new();
    let (mut cur_abs, mut cur_lvl, mut cur_num) = (String::new(), -1, String::new());
    for el in starts(xml) {
        match el.0.as_str() {
            "abstractNum" => cur_abs = el.attr("abstractNumId"),
            "lvl" => cur_lvl = el.attr("ilvl").parse().unwrap_or(-1),
            "numFmt" => {
                let v = el.attr("val");
                abstract_
                    .entry(cur_abs.clone())
                    .or_default()
                    .insert(cur_lvl, !v.is_empty() && v != "bullet" && v != "none");
            }
            "num" => cur_num = el.attr("numId"),
            "abstractNumId" => nums.push((cur_num.clone(), el.attr("val"))),
            _ => {}
        }
    }
    nums.into_iter().map(|(n, a)| (n, abstract_.get(&a).cloned().unwrap_or_default())).collect()
}

#[derive(Clone, Default)]
struct Run {
    text: String,
    bold: bool,
    italic: bool,
    link: String,
    image: String,
    /// In half points; 0 where the style says.
    size: i32,
}

fn heading_level(style: &str) -> usize {
    let s = style.to_ascii_lowercase();
    match s.as_str() {
        "title" => 1,
        "subtitle" => 2,
        _ => s.strip_prefix("heading").and_then(|n| n.parse().ok()).filter(|n| (1..=6).contains(n)).unwrap_or(0),
    }
}

/// The level a paragraph styled by hand reads as: all bold, at 16 pt or more.
fn sized_heading(runs: &[Run]) -> usize {
    let (mut size, mut text) = (0, false);
    for r in runs {
        if r.text.trim().is_empty() {
            continue;
        }
        if !r.bold || r.size == 0 {
            return 0;
        }
        size = size.max(r.size);
        text = true;
    }
    match (text, size) {
        (false, _) => 0,
        (_, s) if s >= 40 => 1,
        (_, s) if s >= 32 => 2,
        _ => 0,
    }
}

fn markdown(
    body: &[u8],
    targets: &HashMap<String, String>,
    picture: &mut dyn FnMut(&str) -> Option<String>,
    is_numbered: &dyn Fn(&str, i32) -> bool,
) -> Result<String, String> {
    let mut out = String::new();
    let mut runs: Vec<Run> = Vec::new();
    let mut style = String::new();
    let mut list_level = -1;
    let mut list_num = String::new();
    let (mut bold, mut italic, mut size) = (false, false, 0);
    let (mut in_run, mut in_text, mut in_rpr, mut in_ppr) = (false, false, false, false);
    let mut link = String::new();
    let mut table_depth = 0;
    let mut row: Vec<String> = Vec::new();
    let mut rows: Vec<Vec<String>> = Vec::new();
    let mut cell = String::new();
    let mut in_cell = false;

    let mut r = quick_xml::Reader::from_reader(body);
    let mut buf = Vec::new();
    loop {
        let ev = r.read_event_into(&mut buf).map_err(|_| "Word 文档的内容读不懂".to_string())?;
        match ev {
            Event::Eof => break,
            Event::Start(ref e) | Event::Empty(ref e) => {
                let el = El::of(e);
                match el.0.as_str() {
                    "tbl" => table_depth += 1,
                    "tr" => row.clear(),
                    "tc" => {
                        in_cell = true;
                        cell.clear();
                    }
                    "pPr" => in_ppr = true,
                    "pStyle" if in_ppr => style = el.attr("val"),
                    "ilvl" if in_ppr => {
                        if let Ok(n) = el.attr("val").parse() {
                            list_level = n;
                        }
                    }
                    "numPr" if in_ppr && list_level < 0 => list_level = 0,
                    "numId" if in_ppr => list_num = el.attr("val"),
                    "hyperlink" => link = targets.get(&el.attr("id")).cloned().unwrap_or_default(),
                    "r" => {
                        in_run = true;
                        bold = false;
                        italic = false;
                        size = 0;
                    }
                    "rPr" => in_rpr = true,
                    "b" if in_rpr => bold = el.on(),
                    "i" if in_rpr => italic = el.on(),
                    "sz" if in_rpr => size = el.attr("val").parse().unwrap_or(0),
                    "t" => in_text = in_run && matches!(ev, Event::Start(_)),
                    "tab" if in_run => runs.push(Run { text: " ".into(), bold, italic, link: link.clone(), ..Default::default() }),
                    "br" | "cr" if in_run => runs.push(Run { text: "\n".into(), bold, italic, ..Default::default() }),
                    "blip" => {
                        if let Some(name) = picture(&el.attr("embed")) {
                            runs.push(Run { image: name, ..Default::default() });
                        }
                    }
                    _ => {}
                }
                // An empty <w:p/> or <w:rPr/> ends as it starts.
                if matches!(ev, Event::Empty(_)) {
                    match el.0.as_str() {
                        "pPr" => in_ppr = false,
                        "rPr" => in_rpr = false,
                        _ => {}
                    }
                }
            }
            Event::End(ref e) => match e.local_name().as_ref() {
                b"p" => {
                    let paragraph = std::mem::take(&mut runs);
                    flush_paragraph(
                        &paragraph,
                        &style,
                        list_level,
                        &list_num,
                        in_cell,
                        &mut cell,
                        &mut out,
                        is_numbered,
                    );
                    style.clear();
                    list_level = -1;
                    list_num.clear();
                }
                b"pPr" => in_ppr = false,
                b"rPr" => in_rpr = false,
                b"r" => in_run = false,
                b"t" => in_text = false,
                b"hyperlink" => link.clear(),
                b"tc" => {
                    row.push(cell.trim().to_string());
                    in_cell = false;
                }
                b"tr" => rows.push(std::mem::take(&mut row)),
                b"tbl" => {
                    table_depth -= 1;
                    if table_depth == 0 {
                        flush_table(&mut rows, &mut out);
                    }
                }
                _ => {}
            },
            Event::Text(t) if in_text => {
                let text = t.unescape().map(|c| c.into_owned()).unwrap_or_default();
                runs.push(Run { text, bold, italic, link: link.clone(), size, ..Default::default() });
            }
            _ => {}
        }
        buf.clear();
    }
    Ok(out.trim().to_string() + "\n")
}

#[allow(clippy::too_many_arguments)]
fn flush_paragraph(
    runs: &[Run],
    style: &str,
    mut list_level: i32,
    list_num: &str,
    in_cell: bool,
    cell: &mut String,
    out: &mut String,
    is_numbered: &dyn Fn(&str, i32) -> bool,
) {
    let mut text = runs_markdown(runs);
    if in_cell {
        if !text.trim().is_empty() {
            if !cell.is_empty() {
                cell.push_str("<br>");
            }
            cell.push_str(&text);
        }
        return;
    }
    if text.trim().is_empty() {
        return;
    }
    let mut level = heading_level(style);
    if level == 0 {
        level = sized_heading(runs);
    }
    // A bullet typed as text, as textutil and Pages write them.
    if list_level < 0
        && let Some(rest) = text.strip_prefix('•')
    {
        text = rest.trim().to_string();
        list_level = 0;
    }
    // A list ends with a blank line, or what follows would continue its last item.
    if list_level < 0 && out.ends_with('\n') && !out.ends_with("\n\n") {
        out.push('\n');
    }
    if level > 0 {
        let t = text.strip_prefix("**").unwrap_or(&text);
        let t = t.strip_suffix("**").unwrap_or(t);
        out.push_str(&format!("{} {t}\n\n", "#".repeat(level)));
    } else if list_level >= 0 {
        let marker = if is_numbered(list_num, list_level) { "1. " } else { "- " };
        out.push_str(&format!("{}{marker}{text}\n", "  ".repeat(list_level as usize)));
    } else {
        out.push_str(&text);
        out.push_str("\n\n");
    }
}

fn flush_table(rows: &mut Vec<Vec<String>>, out: &mut String) {
    if rows.is_empty() {
        return;
    }
    if out.ends_with('\n') && !out.ends_with("\n\n") {
        out.push('\n');
    }
    let cols = rows.iter().map(Vec::len).max().unwrap_or(0);
    let line = |r: &[String], out: &mut String| {
        out.push('|');
        for i in 0..cols {
            out.push_str(&format!(" {} |", r.get(i).map(String::as_str).unwrap_or("")));
        }
        out.push('\n');
    };
    line(&rows[0], out);
    out.push('|');
    out.push_str(&" --- |".repeat(cols));
    out.push('\n');
    for r in &rows[1..] {
        line(r, out);
    }
    out.push('\n');
    rows.clear();
}

/// A paragraph's runs as Markdown, neighbors of one style together.
fn runs_markdown(runs: &[Run]) -> String {
    let mut b = String::new();
    let mut i = 0;
    while i < runs.len() {
        let r = &runs[i];
        if !r.image.is_empty() {
            b.push_str(&format!("![]({})", r.image));
            i += 1;
            continue;
        }
        let mut j = i;
        let mut text = String::new();
        while j < runs.len()
            && runs[j].image.is_empty()
            && runs[j].bold == r.bold
            && runs[j].italic == r.italic
            && runs[j].link == r.link
        {
            text.push_str(&runs[j].text);
            j += 1;
        }
        i = j;
        for (k, line) in text.split('\n').enumerate() {
            if k > 0 {
                b.push_str("  \n");
            }
            b.push_str(&styled(&escape(line), r));
        }
    }
    b.trim().to_string()
}

fn styled(text: &str, r: &Run) -> String {
    let core = text.trim();
    if core.is_empty() {
        return text.to_string();
    }
    let lead = &text[..text.find(core).unwrap_or(0)];
    let trail = &text[lead.len() + core.len()..];
    let mut core = match (r.bold, r.italic) {
        (true, true) => format!("***{core}***"),
        (true, false) => format!("**{core}**"),
        (false, true) => format!("*{core}*"),
        _ => core.to_string(),
    };
    if !r.link.is_empty() && !r.link.starts_with("word/") {
        core = format!("[{core}]({})", r.link);
    }
    format!("{lead}{core}{trail}")
}

/// Keeps text from reading as Markdown syntax; brackets and dollars as
/// character references, which would otherwise read as math.
fn escape(s: &str) -> String {
    let mut b = String::new();
    for (i, c) in s.char_indices() {
        match c {
            '[' => b.push_str("&#91;"),
            ']' => b.push_str("&#93;"),
            '$' => b.push_str("&#36;"),
            '\\' | '`' | '*' | '_' | '<' | '>' | '|' => {
                b.push('\\');
                b.push(c);
            }
            '#' | '-' | '+' if s[..i].trim().is_empty() => {
                b.push('\\');
                b.push(c);
            }
            _ => b.push(c),
        }
    }
    b
}

#[cfg(test)]
pub mod tests {
    use super::*;
    use std::io::Write as _;

    const DOCUMENT: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<w:document xmlns:w="w" xmlns:r="r" xmlns:a="a">
<w:body>
<w:p><w:pPr><w:pStyle w:val="Title"/></w:pPr><w:r><w:t>季度计划</w:t></w:r></w:p>
<w:p><w:pPr><w:pStyle w:val="Heading2"/></w:pPr><w:r><w:t>目标</w:t></w:r></w:p>
<w:p><w:r><w:t xml:space="preserve">这是 </w:t></w:r><w:r><w:rPr><w:b/></w:rPr><w:t>重点</w:t></w:r><w:r><w:rPr><w:b/></w:rPr><w:t>内容</w:t></w:r><w:r><w:t xml:space="preserve">，和 </w:t></w:r><w:r><w:rPr><w:i/></w:rPr><w:t>斜体</w:t></w:r><w:r><w:rPr><w:b w:val="0"/></w:rPr><w:t>。2*3 [x]</w:t></w:r></w:p>
<w:p><w:pPr><w:numPr><w:ilvl w:val="0"/><w:numId w:val="1"/></w:numPr></w:pPr><w:r><w:t>第一项</w:t></w:r></w:p>
<w:p><w:pPr><w:numPr><w:ilvl w:val="1"/><w:numId w:val="1"/></w:numPr></w:pPr><w:r><w:t>子项</w:t></w:r></w:p>
<w:p><w:pPr><w:numPr><w:ilvl w:val="0"/><w:numId w:val="2"/></w:numPr></w:pPr><w:r><w:t>步骤一</w:t></w:r></w:p>
<w:p><w:hyperlink r:id="rLink"><w:r><w:t>官网</w:t></w:r></w:hyperlink></w:p>
<w:tbl>
<w:tr><w:tc><w:p><w:r><w:t>名称</w:t></w:r></w:p></w:tc><w:tc><w:p><w:r><w:t>数量</w:t></w:r></w:p></w:tc></w:tr>
<w:tr><w:tc><w:p><w:r><w:t>a|b</w:t></w:r></w:p></w:tc><w:tc><w:p><w:r><w:t>3</w:t></w:r></w:p></w:tc></w:tr>
</w:tbl>
<w:p><w:r><w:drawing><a:blip r:embed="rPic"/></w:drawing></w:r></w:p>
</w:body>
</w:document>"#;

    const RELS: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<Relationships>
<Relationship Id="rLink" Target="https://example.com" TargetMode="External"/>
<Relationship Id="rPic" Target="media/image1.png"/>
</Relationships>"#;

    const NUMBERING: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<w:numbering xmlns:w="w">
<w:abstractNum w:abstractNumId="10"><w:lvl w:ilvl="0"><w:numFmt w:val="bullet"/></w:lvl><w:lvl w:ilvl="1"><w:numFmt w:val="bullet"/></w:lvl></w:abstractNum>
<w:abstractNum w:abstractNumId="20"><w:lvl w:ilvl="0"><w:numFmt w:val="decimal"/></w:lvl></w:abstractNum>
<w:num w:numId="1"><w:abstractNumId w:val="10"/></w:num>
<w:num w:numId="2"><w:abstractNumId w:val="20"/></w:num>
</w:numbering>"#;

    /// A small .docx with headings, emphasis, lists, a link, a table and a picture.
    pub fn docx() -> Vec<u8> {
        let mut buf = Cursor::new(Vec::new());
        let mut z = zip::ZipWriter::new(&mut buf);
        for (name, body) in [
            ("word/document.xml", DOCUMENT),
            ("word/_rels/document.xml.rels", RELS),
            ("word/numbering.xml", NUMBERING),
            ("word/media/image1.png", "PNGDATA"),
        ] {
            z.start_file(name, zip::write::SimpleFileOptions::default()).unwrap();
            z.write_all(body.as_bytes()).unwrap();
        }
        z.finish().unwrap();
        buf.into_inner()
    }

    #[test]
    fn a_docx_reads_as_markdown() {
        let doc = read_docx(&docx()).unwrap();
        let md = &doc.markdown;
        for want in [
            "# 季度计划\n",
            "## 目标\n",
            "这是 **重点内容**，和 *斜体*。2\\*3 &#91;x&#93;",
            "- 第一项\n  - 子项\n",
            "1. 步骤一\n",
            "[官网](https://example.com)",
            "| 名称 | 数量 |\n| --- | --- |\n| a\\|b | 3 |",
            "![](docx-media:word/media/image1.png)",
        ] {
            assert!(md.contains(want), "markdown lacks {want:?}:\n{md}");
        }
        assert_eq!(doc.media["word/media/image1.png"], b"PNGDATA");
    }

    #[test]
    fn a_list_ends_before_what_follows() {
        let mut out = String::new();
        let mut cell = String::new();
        let run = |t: &str| vec![Run { text: t.into(), ..Default::default() }];
        let no = |_: &str, _: i32| false;
        flush_paragraph(&run("一"), "", 0, "", false, &mut cell, &mut out, &no);
        flush_paragraph(&run("后文"), "", -1, "", false, &mut cell, &mut out, &no);
        assert_eq!(out, "- 一\n\n后文\n\n");
    }

    #[test]
    fn not_a_word_document() {
        assert!(read_docx(b"plain text").is_err());
    }

    #[test]
    fn word_files_macos_writes() {
        if std::process::Command::new("textutil").arg("-help").output().is_err() {
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let html = dir.path().join("in.html");
        std::fs::write(&html, r#"<html><head><meta charset="utf-8"></head><body><h1>会议纪要</h1><p>今天讨论了 <b>预算</b>。</p><ul><li>第一点</li><li>第二点</li></ul></body></html>"#).unwrap();
        for format in ["doc", "docx"] {
            let out = dir.path().join(format!("out.{format}"));
            let ok = std::process::Command::new("textutil")
                .args(["-convert", format, "-output"])
                .arg(&out)
                .arg(&html)
                .status()
                .unwrap()
                .success();
            assert!(ok);
            let doc = read(&out).unwrap();
            for want in ["# 会议纪要\n", "**预算**", "- 第一点\n", "- 第二点\n"] {
                assert!(doc.markdown.contains(want), ".{format} lacks {want:?}:\n{}", doc.markdown);
            }
        }
    }
}
