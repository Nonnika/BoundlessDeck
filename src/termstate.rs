//! What a terminal block keeps of its terminal while the deck is closed:
//! what it showed and where its shell was. Opened again, the block starts a
//! new shell there, under what the old one showed.

use std::path::{Path, PathBuf};

use base64::Engine as _;
use chrono::{DateTime, FixedOffset, Local};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// How much of a terminal's history a block keeps, in bytes.
pub const MAX_HISTORY: usize = 256 << 10;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TermState {
    /// What the terminal showed, as rows of VT sequences; base64 in JSON,
    /// as Go writes a []byte.
    #[serde(default, skip_serializing_if = "Vec::is_empty", serialize_with = "ser_b64", deserialize_with = "de_b64")]
    pub history: Vec<u8>,
    /// Where the shell was: relative to the root when inside it.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub dir: String,
    #[serde(rename = "savedAt", default = "epoch")]
    pub saved_at: DateTime<FixedOffset>,
}

fn epoch() -> DateTime<FixedOffset> {
    DateTime::UNIX_EPOCH.fixed_offset()
}

fn ser_b64<S: Serializer>(v: &[u8], s: S) -> Result<S::Ok, S::Error> {
    s.serialize_str(&base64::engine::general_purpose::STANDARD.encode(v))
}

fn de_b64<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<u8>, D::Error> {
    let s = Option::<String>::deserialize(d)?.unwrap_or_default();
    base64::engine::general_purpose::STANDARD.decode(s).map_err(serde::de::Error::custom)
}

impl TermState {
    pub fn new(history: Vec<u8>, dir: String) -> TermState {
        TermState { history: last_rows(&history, MAX_HISTORY), dir, saved_at: Local::now().fixed_offset() }
    }
}

/// The end of history, at most max bytes, cut at the start of a row so
/// that no escape sequence is cut in two.
pub fn last_rows(history: &[u8], max: usize) -> Vec<u8> {
    if history.len() <= max {
        return history.to_vec();
    }
    let mut tail = &history[history.len() - max..];
    if let Some(i) = tail.iter().position(|&b| b == b'\n') {
        tail = &tail[i + 1..];
    }
    let mut out = b"\x1b[0m".to_vec();
    out.extend_from_slice(tail);
    out
}

/// Where a terminal block's new shell starts: where the old one was if that
/// folder is still there, the root otherwise. Reads the disk.
pub fn term_dir(root: &Path, st: Option<&TermState>) -> PathBuf {
    let Some(st) = st.filter(|s| !s.dir.is_empty()) else {
        return root.to_path_buf();
    };
    let dir = Path::new(&st.dir);
    let dir = if dir.is_absolute() { dir.to_path_buf() } else { root.join(dir) };
    if dir.is_dir() { dir } else { root.to_path_buf() }
}

/// What a block's new terminal shows first: what the old one showed, then a
/// faint line saying when it was left.
pub fn restored(st: Option<&TermState>) -> Vec<u8> {
    let Some(st) = st.filter(|s| !s.history.is_empty()) else {
        return Vec::new();
    };
    let mut b = st.history.clone();
    b.extend_from_slice(b"\x1b[0m\r\n\x1b[2m\xe2\x94\x80\xe2\x94\x80 ");
    b.extend_from_slice(
        format!("已恢复 · {} ──\x1b[0m\r\n", st.saved_at.with_timezone(&Local).format("%Y-%m-%d %H:%M")).as_bytes(),
    );
    b
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn last_rows_cuts_at_a_row() {
        let h = b"aaaa\nbbbb\ncccc".to_vec();
        assert_eq!(last_rows(&h, 100), h);
        assert_eq!(last_rows(&h, 7), b"\x1b[0mcccc".to_vec());
    }

    #[test]
    fn term_dir_falls_back_to_the_root() {
        let root = tempfile::tempdir().unwrap();
        std::fs::create_dir(root.path().join("src")).unwrap();
        let mut st = TermState::new(vec![], "src".into());
        assert_eq!(term_dir(root.path(), Some(&st)), root.path().join("src"));
        st.dir = "gone".into();
        assert_eq!(term_dir(root.path(), Some(&st)), root.path());
        assert_eq!(term_dir(root.path(), None), root.path());
    }

    #[test]
    fn restored_puts_a_line_under_the_history() {
        assert!(restored(None).is_empty());
        let st = TermState::new(b"$ ls".to_vec(), String::new());
        let out = String::from_utf8(restored(Some(&st))).unwrap();
        assert!(out.starts_with("$ ls") && out.contains("已恢复"));
    }
}
