//! How a file block previews, by its extension.

use std::fs;
use std::io::Read as _;
use std::path::Path;

use gpui::SharedString;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Kind {
    Text,
    Image,
    Markdown,
    Word,
    Pdf,
}

pub fn of(path: &str) -> Kind {
    let ext = Path::new(path).extension().and_then(|e| e.to_str()).map(str::to_ascii_lowercase);
    match ext.as_deref() {
        Some("png" | "jpg" | "jpeg" | "gif" | "webp" | "bmp" | "svg" | "ico" | "tif" | "tiff") => Kind::Image,
        Some("md" | "markdown" | "mdown") => Kind::Markdown,
        Some("docx" | "doc") => Kind::Word,
        Some("pdf") => Kind::Pdf,
        _ => Kind::Text,
    }
}

/// The most of a file a text preview reads.
pub const MAX_TEXT_BYTES: usize = 4 << 20;

fn read_capped(path: &Path) -> Result<String, String> {
    let f = fs::File::open(path).map_err(|e| e.to_string())?;
    let mut buf = Vec::new();
    f.take(MAX_TEXT_BYTES as u64).read_to_end(&mut buf).map_err(|e| e.to_string())?;
    if buf.iter().take(8000).any(|&b| b == 0) {
        return Err("二进制文件，无法以文本预览".into());
    }
    Ok(String::from_utf8_lossy(&buf).into_owned())
}

/// A text file's lines, tabs as four spaces.
pub fn read_lines(path: &Path) -> Result<Vec<SharedString>, String> {
    let text = read_capped(path)?;
    Ok(text.lines().map(|l| SharedString::from(l.replace('\t', "    "))).collect())
}

/// A Markdown file's text.
pub fn read_doc(path: &Path) -> Result<String, String> {
    read_capped(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn previews_by_extension() {
        assert_eq!(of("a/b.PNG"), Kind::Image);
        assert_eq!(of("readme.md"), Kind::Markdown);
        assert_eq!(of("x.pdf"), Kind::Pdf);
        assert_eq!(of("main.rs"), Kind::Text);
        assert_eq!(of("Makefile"), Kind::Text);
    }

    #[test]
    fn binary_files_are_not_shown_as_text() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("b.bin");
        fs::write(&p, [1u8, 0, 2]).unwrap();
        assert!(read_lines(&p).is_err());
        fs::write(&p, "a\tb\nc").unwrap();
        assert_eq!(read_lines(&p).unwrap(), vec![SharedString::from("a    b"), SharedString::from("c")]);
    }
}
