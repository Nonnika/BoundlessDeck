//! A 笔记 block is a Markdown file of the deck's folder: its name is the
//! block's title, renaming the block renames it, and what else writes to it
//! shows in the block. Taking the block off the canvas leaves the file.

use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

pub const MEMO_TITLE: &str = "笔记";
/// What is typed is written once typing rests this long.
pub const SAVE_IDLE: Duration = Duration::from_millis(800);
/// How often a note shown, and not being edited, looks whether its file changed.
pub const POLL: Duration = Duration::from_secs(2);
pub const MAX_BYTES: u64 = 4 << 20;

/// The file name, without .md, a title gives a note.
pub fn name_for(title: &str) -> String {
    let name: String =
        title.trim().chars().map(|c| if matches!(c, '/' | '\\' | ':' | '\0') { '-' } else { c }).collect();
    let name = name.trim_matches(|c| c == '.' || c == ' ');
    if name.is_empty() { MEMO_TITLE.to_string() } else { name.to_string() }
}

/// The title of a note at path: its file's name without the extension.
pub fn title_of(path: &str) -> String {
    Path::new(path).file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| path.to_string())
}

/// Makes an empty Markdown file in dir named after title, or "title 2"… when
/// that is taken, and returns its name.
pub fn create_file(dir: &Path, title: &str) -> std::io::Result<String> {
    let base = name_for(title);
    for i in 1..1000 {
        let name = if i == 1 { format!("{base}.md") } else { format!("{base} {i}.md") };
        match fs::OpenOptions::new().write(true).create_new(true).open(dir.join(&name)) {
            Ok(_) => return Ok(name),
            Err(e) if e.kind() == ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e),
        }
    }
    Err(std::io::Error::other(format!("{} 下已有太多同名笔记", dir.display())))
}

/// Renames the note at rel, inside root, after title; never over a file there already.
pub fn rename_file(root: &Path, rel: &str, title: &str) -> Result<String, String> {
    let rel_path = Path::new(rel);
    let next = rel_path.parent().unwrap_or(Path::new("")).join(format!("{}.md", name_for(title)));
    let next_s = next.to_string_lossy().into_owned();
    if next_s == rel {
        return Ok(next_s);
    }
    let abs = |p: &Path| -> PathBuf { if p.is_absolute() { p.to_path_buf() } else { root.join(p) } };
    // A rename that only changes case is the same file on macOS.
    if abs(&next).exists() && !next_s.eq_ignore_ascii_case(rel) {
        return Err(format!("已有文件 {}", title_of(&next_s) + ".md"));
    }
    fs::rename(abs(rel_path), abs(&next)).map_err(|e| e.to_string())?;
    Ok(next_s)
}

/// Reads a note's file and its modification time.
pub fn read(path: &Path) -> Result<(String, SystemTime), String> {
    let meta = fs::metadata(path).map_err(|e| e.to_string())?;
    if meta.len() > MAX_BYTES {
        return Err(format!("笔记太大（{} MB 以上）", MAX_BYTES >> 20));
    }
    let text = fs::read_to_string(path).map_err(|e| e.to_string())?;
    Ok((text, meta.modified().unwrap_or(SystemTime::UNIX_EPOCH)))
}

/// Writes a note's text and returns the file's new modification time.
pub fn write(path: &Path, text: &str) -> Result<SystemTime, String> {
    fs::write(path, text).map_err(|e| e.to_string())?;
    fs::metadata(path).and_then(|m| m.modified()).map_err(|e| e.to_string())
}

/// The file's modification time, if it changed from known; None when it did not.
pub fn changed_since(path: &Path, known: SystemTime) -> Result<Option<(String, SystemTime)>, String> {
    let meta = fs::metadata(path).map_err(|e| e.to_string())?;
    if meta.modified().ok() == Some(known) {
        return Ok(None);
    }
    read(path).map(Some)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_avoid_what_a_file_name_cannot_hold() {
        assert_eq!(name_for(" a/b:c "), "a-b-c");
        assert_eq!(name_for(" .. "), MEMO_TITLE);
        assert_eq!(title_of("dir/会议.md"), "会议");
    }

    #[test]
    fn create_numbers_taken_names_and_rename_never_overwrites() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(create_file(dir.path(), "笔记").unwrap(), "笔记.md");
        assert_eq!(create_file(dir.path(), "笔记").unwrap(), "笔记 2.md");
        assert!(rename_file(dir.path(), "笔记 2.md", "笔记").is_err());
        assert_eq!(rename_file(dir.path(), "笔记 2.md", "计划").unwrap(), "计划.md");
        assert!(dir.path().join("计划.md").exists());
    }

    #[test]
    fn changed_since_sees_another_writer() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("n.md");
        let mod1 = write(&p, "one").unwrap();
        assert!(changed_since(&p, mod1).unwrap().is_none());
        std::thread::sleep(Duration::from_millis(20));
        fs::write(&p, "two").unwrap();
        let filetime_moved = fs::metadata(&p).unwrap().modified().unwrap() != mod1;
        if filetime_moved {
            assert_eq!(changed_since(&p, mod1).unwrap().unwrap().0, "two");
        }
    }
}
