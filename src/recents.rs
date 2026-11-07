//! The folders opened lately, as the home screen lists them, kept in
//! recents.json in the app's data folder.

use std::path::{Path, PathBuf};

use chrono::{DateTime, FixedOffset, Local};
use serde::{Deserialize, Serialize};

use crate::model::write_file_atomic;

/// How many folders the home screen remembers.
pub const MAX_RECENTS: usize = 12;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Recent {
    pub path: String,
    #[serde(default)]
    pub blocks: usize,
    #[serde(rename = "openedAt")]
    pub opened_at: DateTime<FixedOffset>,
    /// The folder is no longer there.
    #[serde(skip)]
    pub missing: bool,
}

#[derive(Default)]
pub struct Recents {
    pub list: Vec<Recent>,
    /// Where they are kept; None keeps them in memory only (tests).
    pub file: Option<PathBuf>,
}

impl Recents {
    /// The recents kept in the app's data folder.
    pub fn open() -> Recents {
        let file = dirs::data_dir().map(|d| d.join("BoundlessDeck").join("recents.json"));
        let mut r = Recents { list: Vec::new(), file };
        r.load();
        r
    }

    fn load(&mut self) {
        let Some(f) = &self.file else { return };
        if let Ok(data) = std::fs::read(f)
            && let Ok(list) = serde_json::from_slice::<Vec<Recent>>(&data)
        {
            self.list = list;
        }
        self.check();
    }

    /// Marks the recents whose folders are gone.
    pub fn check(&mut self) {
        for r in &mut self.list {
            r.missing = !Path::new(&r.path).is_dir();
        }
    }

    /// Puts the folder first, with its block count.
    pub fn touch(&mut self, path: &str, blocks: usize) {
        self.list.retain(|r| r.path != path);
        self.list.insert(
            0,
            Recent { path: path.to_string(), blocks, opened_at: Local::now().fixed_offset(), missing: false },
        );
        self.list.truncate(MAX_RECENTS);
        self.save();
    }

    pub fn forget(&mut self, path: &str) {
        self.list.retain(|r| r.path != path);
        self.save();
    }

    pub fn mark_missing(&mut self, path: &str) {
        for r in self.list.iter_mut().filter(|r| r.path == path) {
            r.missing = true;
        }
    }

    /// The folders still there, the latest first.
    pub fn paths(&self) -> Vec<String> {
        self.list.iter().filter(|r| !r.missing).map(|r| r.path.clone()).collect()
    }

    fn save(&self) {
        let Some(f) = &self.file else { return };
        if let Ok(data) = serde_json::to_vec_pretty(&self.list) {
            let _ = write_file_atomic(f, &data);
        }
    }
}

/// A path under the home folder written from ~.
pub fn short_path(p: &str) -> String {
    if let Some(home) = dirs::home_dir() {
        let home = home.to_string_lossy();
        if p == home {
            return "~".into();
        }
        if let Some(rest) = p.strip_prefix(&*home)
            && rest.starts_with('/')
        {
            return format!("~{rest}");
        }
    }
    p.to_string()
}

/// How long ago t was, as the recents show it.
pub fn ago(now: DateTime<FixedOffset>, t: DateTime<FixedOffset>) -> String {
    let d = now.signed_duration_since(t);
    if d.num_minutes() < 1 {
        "刚刚".into()
    } else if d.num_hours() < 1 {
        format!("{} 分钟前", d.num_minutes())
    } else if d.num_hours() < 24 {
        format!("{} 小时前", d.num_hours())
    } else if d.num_days() < 7 {
        format!("{} 天前", d.num_days())
    } else {
        t.format("%Y-%m-%d").to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Duration;

    #[test]
    fn touch_puts_a_folder_first_once() {
        let mut r = Recents::default();
        r.touch("/a", 1);
        r.touch("/b", 2);
        r.touch("/a", 3);
        assert_eq!(r.paths(), vec!["/a", "/b"]);
        assert_eq!(r.list[0].blocks, 3);
        for i in 0..20 {
            r.touch(&format!("/x{i}"), 0);
        }
        assert_eq!(r.list.len(), MAX_RECENTS);
        r.mark_missing("/x19");
        assert!(!r.paths().contains(&"/x19".to_string()));
    }

    #[test]
    fn ago_reads_coarsely() {
        let now = Local::now().fixed_offset();
        assert_eq!(ago(now, now), "刚刚");
        assert_eq!(ago(now, now - Duration::minutes(5)), "5 分钟前");
        assert_eq!(ago(now, now - Duration::hours(3)), "3 小时前");
        assert_eq!(ago(now, now - Duration::days(2)), "2 天前");
    }

    #[test]
    fn reads_go_recents() {
        let data = r#"[{"path":"/p","blocks":4,"openedAt":"2026-11-01T09:30:00.123456+08:00"}]"#;
        let list: Vec<Recent> = serde_json::from_str(data).unwrap();
        assert_eq!(list[0].blocks, 4);
    }
}
