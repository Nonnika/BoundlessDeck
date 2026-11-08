//! Where a deck's files are, and reading and writing deck.json: atomically,
//! keeping blocks of kinds this build does not know as they were.

use std::fs;
use std::io::Write as _;
use std::path::{Component, Path, PathBuf};

use serde::{Deserialize, Serialize, Serializer};

use super::{Block, BlockKind, Camera, DECK_DIR, Model};

impl Model {
    /// A block's path, resolved against the root.
    pub fn path_of(&self, p: &str) -> PathBuf {
        let p = Path::new(p);
        if p.is_absolute() { p.to_path_buf() } else { self.root.join(p) }
    }

    /// p relative to the root when it is inside it, so the folder can move.
    pub fn link_path(&self, p: &Path) -> String {
        match p.strip_prefix(&self.root) {
            Ok(rel) if !rel.as_os_str().is_empty() && !rel.components().any(|c| c == Component::ParentDir) => {
                rel.to_string_lossy().into_owned()
            }
            _ => p.to_string_lossy().into_owned(),
        }
    }

    // ---- deck.json ----

    pub fn deck_file(&self) -> Option<PathBuf> {
        if let Some(f) = &self.file {
            return Some(f.clone());
        }
        if self.root.as_os_str().is_empty() {
            return None;
        }
        Some(self.root.join(DECK_DIR).join("deck.json"))
    }

    /// Reads the deck saved in its folder; a folder with none gets a new deck.
    /// A deck.json there that cannot be read is an error, and nothing is
    /// loaded, so saving does not write over it. Blocks in it this build does
    /// not know are kept aside (`foreign`) and saved back.
    pub fn load(&mut self) -> Result<(), String> {
        let Some(file) = self.deck_file() else {
            self.seed();
            return Ok(());
        };
        let data = match fs::read(&file) {
            Ok(d) => d,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                self.seed();
                return Ok(());
            }
            Err(e) => return Err(e.to_string()),
        };
        let ws: RawWorkspace = serde_json::from_slice(&data).map_err(|e| e.to_string())?;
        if self.root.as_os_str().is_empty() {
            self.root = PathBuf::from(ws.root);
        }
        let mut blocks = Vec::new();
        for v in ws.blocks {
            match serde_json::from_value::<Block>(v.clone()) {
                Ok(b) => blocks.push(b),
                Err(_) => self.foreign.push(v),
            }
        }
        self.blocks = blocks;
        // The Go build may have changed a browser's url alone: its open tab follows.
        for b in self.blocks.iter_mut().filter(|b| b.kind == BlockKind::Browser) {
            b.ensure_tabs();
        }
        self.cam = Camera { x: ws.cam_x, y: ws.cam_y, s: if ws.cam_s == 0.0 { 1.0 } else { ws.cam_s } };
        self.drop_expired();
        self.prune_stacks();
        Ok(())
    }

    pub fn snapshot(&self) -> Vec<u8> {
        let ws = WorkspaceRef {
            version: 1,
            root: self.root.to_string_lossy().into_owned(),
            cam_x: self.cam.x,
            cam_y: self.cam.y,
            cam_s: self.cam.s,
            blocks: (&self.blocks, &self.foreign),
        };
        serde_json::to_vec_pretty(&ws).unwrap_or_default()
    }

    /// Writes deck.json atomically, here and now.
    pub fn save(&self) -> std::io::Result<()> {
        let Some(path) = self.deck_file() else { return Ok(()) };
        write_file_atomic(&path, &self.snapshot())
    }

    fn seed(&mut self) {
        let mut b = Block::new(BlockKind::Note);
        b.title = "开始".into();
        b.x = 80.0;
        b.y = 80.0;
        b.w = 420.0;
        b.h = 250.0;
        b.text = "- 双指滑动平移，⌘ + 滚轮缩放，⌘0 显示全部\n\
                  - 拖顶部的胶带移动，拖右下角等比缩放\n\
                  - 点胶带、框选或 Shift + 点击来选中，⌘D 删除\n\
                  - 右上角新建 QuickLook、笔记、链接文件、打开终端\n\
                  - 左上角回到主页，画布会保存在这个文件夹里"
            .into();
        self.add_block(b);
        self.sel.clear();
    }
}

#[derive(Serialize)]
struct WorkspaceRef<'a> {
    version: i32,
    root: String,
    #[serde(rename = "camX")]
    cam_x: f32,
    #[serde(rename = "camY")]
    cam_y: f32,
    #[serde(rename = "camS")]
    cam_s: f32,
    #[serde(serialize_with = "blocks_then_foreign")]
    blocks: (&'a [Block], &'a [serde_json::Value]),
}

/// deck.json's blocks: those read, then those kept aside as they were.
fn blocks_then_foreign<S: Serializer>(b: &(&[Block], &[serde_json::Value]), s: S) -> Result<S::Ok, S::Error> {
    use serde::ser::SerializeSeq;
    let mut seq = s.serialize_seq(Some(b.0.len() + b.1.len()))?;
    for x in b.0 {
        seq.serialize_element(x)?;
    }
    for x in b.1 {
        seq.serialize_element(x)?;
    }
    seq.end()
}

/// deck.json as read: its blocks still JSON, each read on its own.
#[derive(Deserialize)]
struct RawWorkspace {
    #[serde(default)]
    root: String,
    #[serde(default, rename = "camX")]
    cam_x: f32,
    #[serde(default, rename = "camY")]
    cam_y: f32,
    #[serde(default, rename = "camS")]
    cam_s: f32,
    #[serde(default)]
    blocks: Vec<serde_json::Value>,
}

/// The app's own data folder (recents, settings), unless
/// `BOUNDLESSDECK_DATA_DIR` puts it elsewhere, as tests and trial runs do.
pub fn data_dir() -> Option<PathBuf> {
    match std::env::var_os("BOUNDLESSDECK_DATA_DIR") {
        Some(d) => Some(PathBuf::from(d)),
        None => dirs::data_dir().map(|d| d.join("BoundlessDeck")),
    }
}

/// Writes data through a temporary file and a rename: a reader never sees half a file.
pub fn write_file_atomic(path: &Path, data: &[u8]) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension(format!(
        "{}.tmp",
        path.extension().map(|e| e.to_string_lossy().into_owned()).unwrap_or_default()
    ));
    let mut f = fs::File::create(&tmp)?;
    f.write_all(data)?;
    f.sync_all()?;
    fs::rename(&tmp, path)
}
