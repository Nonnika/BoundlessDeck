//! Writing fields an agent sent: every patch is checked against the field
//! table on a copy of the block first, then applied.

use std::collections::HashSet;
use std::path::Path;

use serde_json::Value;

use super::{Args, Ctx, normalize_url, val_num, val_str};
use super::schema::{CHART_NAMES, FIELDS, field, for_kind, mutable_names};
use super::style::style_patch;
use crate::memo;
use crate::model::{self, Block, BlockKind, MIN_BLOCK_H, MIN_BLOCK_W};

impl Ctx<'_> {
    pub(super) fn validate_patch(&self, kind: BlockKind, self_id: &str, patch: &Args) -> Result<(), String> {
        for name in patch.keys() {
            let Some(f) = field(name) else {
                return Err(format!("未知字段 {name:?}；用 block_schema 看每个 kind 的可写字段"));
            };
            if !f.mutable || !for_kind(f, kind) {
                return Err(format!(
                    "{} block 不能写字段 {name:?}（可写：{}）",
                    kind.mcp_name(),
                    mutable_names(kind).join(", ")
                ));
            }
        }
        if patch.contains_key("ttlMinutes") && patch.contains_key("expires") {
            return Err("expires 与 ttlMinutes 只能给一个".into());
        }
        let mut probe = Block::new(kind);
        probe.id = self_id.to_string();
        for f in FIELDS {
            if let Some(v) = patch.get(f.name) {
                self.check_field(&mut probe, f.name, v).map_err(|e| format!("{}：{e}", f.name))?;
            }
        }
        Ok(())
    }

    /// Checks a field's value, applying what needs no file or model change to `n`.
    pub(super) fn check_field(&self, n: &mut Block, name: &str, v: &Value) -> Result<(), String> {
        match name {
            "title" => n.title = val_str(v, name)?,
            "named" => n.named = v.as_bool().ok_or("named 要是布尔值")?,
            "x" | "y" | "w" | "h" => {
                val_num(v, name)?;
            }
            "expires" => {
                let ms = v.as_i64().ok_or("expires 要是整数")?;
                if ms < 0 {
                    return Err("expires 不能为负；0 为永久保留".into());
                }
            }
            "ttlMinutes" => {
                if val_num(v, name)? < 0. {
                    return Err("ttlMinutes 不能为负；0 为永久保留".into());
                }
            }
            "text" => {
                val_str(v, name)?;
            }
            "path" => {
                let s = val_str(v, name)?;
                if s.trim().is_empty() {
                    return Err("path 不能为空".into());
                }
                if n.kind == BlockKind::Memo {
                    let ext = Path::new(s.trim()).extension().and_then(|e| e.to_str()).map(str::to_ascii_lowercase);
                    if !matches!(ext.as_deref(), Some("md" | "markdown")) {
                        return Err(format!("笔记是 Markdown 文件，路径要以 .md 结尾：{}", s.trim()));
                    }
                }
            }
            "page" => {
                let p = v.as_i64().ok_or("page 要是整数")?;
                if p < 0 {
                    return Err(format!("page 从 0 开始，收到 {p}"));
                }
            }
            "url" => {
                normalize_url(&val_str(v, name)?)?;
            }
            "chart" => {
                let s = val_str(v, name)?;
                if !CHART_NAMES.contains(&s.as_str()) {
                    return Err(format!("没有图表类型 {s:?}：可用 {}", CHART_NAMES.join("、")));
                }
            }
            "shape" => {
                let s = val_str(v, name)?;
                if !model::SHAPES.contains(&s.as_str()) {
                    return Err(format!("没有形状 {s:?}：可用 {}", model::SHAPES.join("、")));
                }
            }
            "style" => n.style = style_patch(&n.style, v)?,
            "items" => {
                let ids: Vec<String> = v
                    .as_array()
                    .ok_or("items 要是字符串数组")?
                    .iter()
                    .map(|x| x.as_str().map(str::to_string).ok_or("items 要是字符串数组".to_string()))
                    .collect::<Result<_, _>>()?;
                let mut seen = HashSet::new();
                for id in &ids {
                    if !seen.insert(id) {
                        return Err(format!("items 里 {id} 重复了"));
                    }
                    if id == &n.id {
                        return Err("块堆不能装自己".into());
                    }
                    let b = self.block(id)?;
                    if b.kind == BlockKind::Stack {
                        return Err(format!("{id} 是块堆：块堆不能装块堆"));
                    }
                }
            }
            _ => return Err(format!("字段 {name:?} 不能写")),
        }
        Ok(())
    }

    /// Applies a validated patch to the block with this id, in the field table's order.
    pub(super) fn apply_patch(&mut self, id: &str, patch: &Args) -> Result<Vec<&'static str>, String> {
        let mut applied = Vec::new();
        for f in FIELDS {
            let Some(v) = patch.get(f.name) else { continue };
            self.apply_field(id, f.name, v).map_err(|e| format!("{}：{e}", f.name))?;
            applied.push(f.name);
        }
        Ok(applied)
    }

    pub(super) fn apply_field(&mut self, id: &str, name: &str, v: &Value) -> Result<(), String> {
        let now = self.m.now_ms();
        let root = self.m.root.clone();
        match name {
            "title" => {
                let s = val_str(v, name)?.trim().to_string();
                let b = self.m.block_mut(id).unwrap();
                if b.kind == BlockKind::Memo {
                    b.title = if s.is_empty() { memo::MEMO_TITLE.into() } else { s };
                    if !b.path.is_empty() {
                        let next = memo::rename_file(&root, &b.path, &b.title)?;
                        b.title = memo::title_of(&next);
                        b.path = next;
                        self.reload.push(id.to_string());
                    }
                } else if s.is_empty() {
                    b.title = b.default_title();
                    b.named = false;
                } else {
                    b.title = s;
                    b.named = true;
                }
            }
            "named" => {
                let b = self.m.block_mut(id).unwrap();
                b.named = v.as_bool().unwrap_or(false);
                if !b.named && b.title.trim().is_empty() {
                    b.title = b.default_title();
                }
            }
            "x" | "y" | "w" | "h" => {
                let f = val_num(v, name)? as f32;
                let b = self.m.block_mut(id).unwrap();
                match name {
                    "x" => b.x = f,
                    "y" => b.y = f,
                    "w" => b.w = f.max(MIN_BLOCK_W),
                    _ => b.h = f.max(MIN_BLOCK_H),
                }
            }
            "expires" => self.m.block_mut(id).unwrap().expires = v.as_i64().unwrap_or(0),
            "ttlMinutes" => {
                let m = val_num(v, name)?;
                self.m.block_mut(id).unwrap().expires = if m > 0. { now + (m * 60_000.) as i64 } else { 0 };
            }
            "text" => {
                let s = val_str(v, name)?;
                let kind = self.m.block(id).unwrap().kind;
                if kind == BlockKind::Memo {
                    self.ensure_memo_file(id)?;
                    let path = self.m.path_of(&self.m.block(id).unwrap().path);
                    crate::model::write_file_atomic(&path, s.as_bytes()).map_err(|e| e.to_string())?;
                    self.memos.push((id.to_string(), s));
                } else {
                    self.m.block_mut(id).unwrap().text = s;
                }
            }
            "path" => {
                let s = val_str(v, name)?.trim().to_string();
                let s = if Path::new(&s).is_absolute() { self.m.link_path(Path::new(&s)) } else { s };
                let kind = self.m.block(id).unwrap().kind;
                if kind == BlockKind::Memo {
                    let abs = self.m.path_of(&s);
                    std::fs::OpenOptions::new().create(true).append(true).open(&abs).map_err(|e| e.to_string())?;
                }
                let b = self.m.block_mut(id).unwrap();
                b.path = s;
                if !b.named || b.kind == BlockKind::Memo {
                    b.title = b.default_title();
                }
                self.reload.push(id.to_string());
            }
            "page" => self.m.block_mut(id).unwrap().page = v.as_i64().unwrap_or(0) as i32,
            "url" => {
                let b = self.m.block_mut(id).unwrap();
                b.url = normalize_url(&val_str(v, name)?)?;
                if b.kind == BlockKind::Browser {
                    b.ensure_tabs();
                }
                self.reload.push(id.to_string());
            }
            "chart" => self.m.block_mut(id).unwrap().chart = val_str(v, name)?,
            "shape" => self.m.block_mut(id).unwrap().shape = val_str(v, name)?,
            "style" => {
                let b = self.m.block_mut(id).unwrap();
                b.style = style_patch(&b.style, v)?;
            }
            "items" => {
                let ids: Vec<String> = v.as_array().unwrap().iter().filter_map(|x| x.as_str().map(str::to_string)).collect();
                if self.m.index_of(id).is_some() {
                    self.m.set_stack_items(id, &ids);
                } else {
                    self.m.block_mut(id).unwrap().items = ids;
                }
            }
            _ => return Err(format!("字段 {name:?} 不能写")),
        }
        Ok(())
    }

    pub(super) fn ensure_memo_file(&mut self, id: &str) -> Result<(), String> {
        let root = self.m.root.clone();
        let b = self.m.block_mut(id).unwrap();
        if !b.path.is_empty() {
            return Ok(());
        }
        let name = memo::create_file(&root, &b.title).map_err(|e| e.to_string())?;
        b.title = memo::title_of(&name);
        b.path = name;
        Ok(())
    }
}
