//! The workspace's MCP tools and resources, over the deck's model. Every
//! write goes through the model's own add/remove/undo path, so what an
//! agent does undoes as the user's edits do. The block field table is the
//! single source of what may be read and written.

use std::collections::HashSet;
use std::path::Path;

use serde_json::{Map, Value, json};

use crate::lifespan::NOTE_LIFE_MS;
use crate::memo;
use crate::model::{self, BlockKind, Block, Camera, Model};
use crate::preview;

mod arrange;
pub mod layout;
mod patch;
mod schema;
mod style;

pub use schema::{block_schema, kind_defaults, schema_digest, tool_list, tool_names};
use schema::{KIND_NAMES, mutable_names, read_only_names};

pub const SCHEMA_VERSION: i32 = 5;
pub const SERVER_NAME: &str = "boundlessdeck";

pub type Args = Map<String, Value>;

// ---- argument helpers ----

fn arg_str(a: &Args, name: &str) -> Result<Option<String>, String> {
    match a.get(name) {
        None => Ok(None),
        Some(Value::String(s)) => Ok(Some(s.clone())),
        Some(_) => Err(format!("{name} 要是字符串")),
    }
}

fn arg_num(a: &Args, name: &str) -> Result<Option<f64>, String> {
    match a.get(name) {
        None => Ok(None),
        Some(v) => v.as_f64().filter(|f| f.is_finite()).map(Some).ok_or_else(|| format!("{name} 要是数字")),
    }
}

fn arg_bool(a: &Args, name: &str) -> Result<Option<bool>, String> {
    match a.get(name) {
        None => Ok(None),
        Some(Value::Bool(b)) => Ok(Some(*b)),
        Some(_) => Err(format!("{name} 要是布尔值")),
    }
}

fn arg_int(a: &Args, name: &str) -> Result<Option<i64>, String> {
    match a.get(name) {
        None => Ok(None),
        Some(v) => v.as_i64().map(Some).ok_or_else(|| format!("{name} 要是整数")),
    }
}

fn arg_strings(a: &Args, name: &str) -> Result<Vec<String>, String> {
    match a.get(name) {
        None => Ok(Vec::new()),
        Some(Value::Array(v)) => v
            .iter()
            .map(|x| x.as_str().map(str::to_string).ok_or_else(|| format!("{name} 要是字符串数组")))
            .collect(),
        Some(_) => Err(format!("{name} 要是字符串数组")),
    }
}

fn val_str(v: &Value, name: &str) -> Result<String, String> {
    v.as_str().map(str::to_string).ok_or_else(|| format!("{name} 要是字符串"))
}

fn val_num(v: &Value, name: &str) -> Result<f64, String> {
    v.as_f64().filter(|f| f.is_finite()).ok_or_else(|| format!("{name} 要是数字"))
}

fn round(v: f32) -> f64 {
    (f64::from(v) * 100.).round() / 100.
}

/// What a tool works on: the model, the view, and what the view must catch up with.
/// Where the call comes from, and where the user is: for `current_terminal`.
#[derive(Clone, Debug, Default)]
pub struct Here {
    /// The terminal the agent runs in, as its bridge says.
    pub caller: crate::mcp::Caller,
    /// The block the user works in or edits, if any.
    pub focused: Option<String>,
}

pub struct Ctx<'a> {
    pub m: &'a mut Model,
    pub view: (f32, f32),
    /// About the MCP endpoint, for deck_info.
    pub mcp: Value,
    pub here: Here,
    /// Something was written: the deck saves and redraws.
    pub dirty: bool,
    /// 笔记 whose file a tool wrote, with what it wrote.
    pub memos: Vec<(String, String)>,
    /// Blocks whose preview must load again (a path changed).
    pub reload: Vec<String>,
    /// The view moved (`set_view`): from where, and whether to glide there.
    pub view_from: Option<(Camera, bool)>,
}

impl Ctx<'_> {
    fn block(&self, id: &str) -> Result<&Block, String> {
        self.m.block(id).ok_or_else(|| format!("没有 id 为 {id:?} 的 block"))
    }

    fn selected_ids(&self) -> Vec<String> {
        self.m.blocks.iter().filter(|b| self.m.sel.contains(&b.id)).map(|b| b.id.clone()).collect()
    }

    pub fn fields_json(&self, n: &Block) -> Value {
        let mut out = json!({
            "id": n.id, "kind": n.kind.mcp_name(), "title": n.title, "named": n.named,
            "x": round(n.x), "y": round(n.y), "w": round(n.w), "h": round(n.h),
            "selected": self.m.sel.contains(&n.id),
            "editable": mutable_names(n.kind), "readOnly": read_only_names(n.kind),
        });
        let o = out.as_object_mut().unwrap();
        let abs = |p: &str| (!p.is_empty() && !Path::new(p).is_absolute()).then(|| self.m.path_of(p).to_string_lossy().into_owned());
        match n.kind {
            BlockKind::Note => {
                o.insert("text".into(), n.text.clone().into());
                o.insert("expires".into(), n.expires.into());
                let mut ttl = 0.;
                if n.expires > 0 {
                    if let Some(at) = chrono::DateTime::from_timestamp_millis(n.expires) {
                        o.insert("expiresAt".into(), at.with_timezone(&chrono::Local).to_rfc3339_opts(chrono::SecondsFormat::Secs, true).into());
                    }
                    let left = (n.expires - self.m.now_ms()) as f64 / 60_000.;
                    ttl = ((left * 10.).ceil() / 10.).max(0.1);
                }
                o.insert("ttlMinutes".into(), ttl.into());
            }
            BlockKind::Memo => {
                o.insert("path".into(), n.path.clone().into());
                if let Some(a) = abs(&n.path) {
                    o.insert("absPath".into(), a.into());
                }
            }
            BlockKind::File => {
                o.insert("path".into(), n.path.clone().into());
                o.insert("page".into(), n.page.into());
                let kind = match preview::of(&n.path) {
                    preview::Kind::Text => "text",
                    preview::Kind::Image => "image",
                    preview::Kind::Markdown => "markdown",
                    preview::Kind::Word => "word",
                    preview::Kind::Pdf => "pdf",
                    preview::Kind::Video => "video",
                };
                o.insert("preview".into(), kind.into());
                if let Some(a) = abs(&n.path) {
                    o.insert("absPath".into(), a.into());
                }
            }
            BlockKind::Browser => {
                o.insert("url".into(), n.url.clone().into());
                // One tab is the url alone: it is listed all the same.
                let mut b = n.clone();
                b.ensure_tabs();
                let tabs: Vec<Value> = b.tabs.iter().map(|t| json!({"url": t.url, "title": t.title})).collect();
                o.insert("tabs".into(), tabs.into());
                o.insert("tab".into(), b.tab.into());
            }
            BlockKind::Terminal => {}
            BlockKind::Stack => {
                o.insert("items".into(), n.items.clone().into());
            }
            BlockKind::Chart => {
                o.insert("text".into(), n.text.clone().into());
                o.insert("chart".into(), if n.chart.is_empty() { "bar".into() } else { n.chart.clone().into() });
            }
            BlockKind::Text | BlockKind::Shape => {
                o.insert("text".into(), n.text.clone().into());
                if n.kind == BlockKind::Shape {
                    o.insert("shape".into(), if n.shape.is_empty() { "rect".into() } else { n.shape.clone().into() });
                }
                o.insert("style".into(), serde_json::to_value(&n.style).unwrap_or_else(|_| json!({})));
            }
        }
        out
    }

    fn list(&self, kind: Option<BlockKind>) -> Vec<Value> {
        self.m.blocks.iter().filter(|b| kind.is_none_or(|k| b.kind == k)).map(|b| self.fields_json(b)).collect()
    }

    /// The world point at the middle of the window.
    fn view_center(&self) -> Value {
        let (x, y) = self.m.cam.to_world(self.view.0 / 2., self.view.1 / 2.);
        json!({"x": round(x), "y": round(y)})
    }

    /// Moves the view: its center to a block or a point, its scale as asked.
    fn set_view(&mut self, a: &Args) -> Result<Value, String> {
        let (vw, vh) = self.view;
        let from = self.m.cam;
        let (mut cx, mut cy) = from.to_world(vw / 2., vh / 2.);
        let mut s = match arg_num(a, "scale")? {
            Some(v) if v.is_finite() && v > 0. => v as f32,
            Some(_) => return Err("scale 要是正数".into()),
            None => from.s,
        };
        let (x, y) = (arg_num(a, "x")?, arg_num(a, "y")?);
        match arg_str(a, "id")? {
            Some(_) if x.is_some() || y.is_some() => return Err("id 与 x/y 只能给一种".into()),
            Some(id) => {
                let b = self.block(&id)?;
                (cx, cy) = (b.x + b.w / 2., b.y + b.h / 2.);
                if arg_bool(a, "fit")?.unwrap_or(false) && vw > 0. {
                    s = (vw * 0.8 / b.w).min(vh * 0.8 / b.h);
                }
            }
            None => {
                if x.is_none() && y.is_none() && arg_num(a, "scale")?.is_none() {
                    return Err("set_view 需要 id、x/y 或 scale 之一".into());
                }
                cx = x.map_or(cx, |v| v as f32);
                cy = y.map_or(cy, |v| v as f32);
            }
        }
        if !cx.is_finite() || !cy.is_finite() {
            return Err("x/y 要是有限的数".into());
        }
        // On a zoom level, so the view rests where it was sent.
        let s = model::zoom_level(s.clamp(model::MIN_SCALE, model::MAX_SCALE)).clamp(model::MIN_SCALE, model::MAX_SCALE);
        self.m.cam = Camera { x: cx - vw / 2. / s, y: cy - vh / 2. / s, s };
        self.view_from = Some((from, arg_bool(a, "animate")?.unwrap_or(true)));
        self.dirty = true;
        Ok(json!({"center": {"x": round(cx), "y": round(cy)}, "scale": round(s), "camera": {"x": round(self.m.cam.x), "y": round(self.m.cam.y), "s": round(s)}}))
    }

    pub fn deck_info(&self) -> Value {
        json!({
            "workspace": self.m.root.to_string_lossy(),
            "deckFile": self.m.deck_file().map(|p| p.to_string_lossy().into_owned()),
            "blocks": self.m.blocks.len(),
            "selected": self.selected_ids(),
            "camera": {"x": round(self.m.cam.x), "y": round(self.m.cam.y), "s": round(self.m.cam.s)},
            "center": self.view_center(),
            "view": {"w": round(self.view.0), "h": round(self.view.1)},
            "kinds": KIND_NAMES,
            "schemaVersion": SCHEMA_VERSION,
            "callerTerminal": self.caller_terminal().ok(),
            "mcp": self.mcp,
        })
    }

    /// The terminal block the caller runs in, or why there is none.
    fn caller_terminal(&self) -> Result<String, String> {
        let c = &self.here.caller;
        let Some(id) = &c.terminal else {
            return Err("你不是从 BoundlessDeck 的终端里启动的（没有 BOUNDLESSDECK_TERMINAL 环境变量）".into());
        };
        let same = |a: &std::path::Path, b: &std::path::Path| {
            a == b || std::fs::canonicalize(a).ok().zip(std::fs::canonicalize(b).ok()).is_some_and(|(x, y)| x == y)
        };
        if let Some(deck) = &c.deck
            && !same(std::path::Path::new(deck), &self.m.root)
        {
            return Err(format!("你运行在另一个工作区的终端里：{deck}"));
        }
        match self.m.block(id) {
            Some(b) if b.kind == BlockKind::Terminal => Ok(id.clone()),
            Some(_) => Err(format!("{id} 不是终端 block")),
            None => Err(format!("启动你的终端 {id} 已不在画布上")),
        }
    }

    fn current_terminal(&self) -> Value {
        let (me, why) = match self.caller_terminal() {
            Ok(id) => (self.m.block(&id).map(|b| self.fields_json(b)), None),
            Err(e) => (None, Some(e)),
        };
        let focused = self.here.focused.as_ref().and_then(|id| self.m.block(id));
        let focused_why = match focused {
            None => Some("人现在没有在操作任何 block"),
            Some(b) if b.kind != BlockKind::Terminal => Some("人正在操作的不是终端"),
            Some(_) => None,
        };
        let focused = focused.filter(|b| b.kind == BlockKind::Terminal).map(|b| self.fields_json(b));
        let mut out = json!({"self": me, "focused": focused});
        if let Some(w) = why {
            out["selfReason"] = w.into();
        }
        if let Some(w) = focused_why {
            out["focusedReason"] = w.into();
        }
        out
    }

    // ---- tools ----

    fn create_block(&mut self, args: &Args) -> Result<Value, String> {
        let kind_s = arg_str(args, "kind")?.ok_or_else(|| format!("create_block 需要 kind：{}", KIND_NAMES.join("、")))?;
        let kind = BlockKind::parse_mcp(&kind_s).ok_or_else(|| format!("没有 kind {kind_s:?}：可用 {}", KIND_NAMES.join("、")))?;
        let mut patch = args.clone();
        patch.remove("kind");
        if kind == BlockKind::File && !patch.contains_key("path") {
            return Err("创建 file block 需要 path".into());
        }
        let id = self.m.new_id();
        self.validate_patch(kind, &id, &patch)?;
        let (title, mut w, mut h) = kind_defaults(kind);
        // A file starts at its kind's size (a page, 16:9 for a video).
        if kind == BlockKind::File
            && let Some(p) = patch.get("path").and_then(Value::as_str)
        {
            (w, h) = crate::model::file_block_size(std::path::Path::new(p));
        }
        let mut b = Block::new(kind);
        b.id = id.clone();
        b.title = title.into();
        b.w = w;
        b.h = h;
        if kind == BlockKind::Chart {
            b.chart = "bar".into();
        }
        // Built off the canvas first, then added as one undoable addition.
        let stack_items = patch.remove("items");
        self.m.blocks.push(b);
        let applied = self.apply_patch(&id, &patch);
        let mut b = self.m.blocks.pop().unwrap();
        let mut applied = applied?;
        if kind == BlockKind::Note && !patch.contains_key("expires") && !patch.contains_key("ttlMinutes") {
            b.expires = self.m.now_ms() + NOTE_LIFE_MS;
        }
        if kind == BlockKind::Memo && b.path.is_empty() {
            let name = memo::create_file(&self.m.root, &b.title).map_err(|e| e.to_string())?;
            b.title = memo::title_of(&name);
            b.path = name;
        }
        if !patch.contains_key("x") || !patch.contains_key("y") {
            let (cx, cy) = if self.view.0 > 0. { self.m.cam.to_world(self.view.0 / 2., self.view.1 / 2.) } else { (0., 0.) };
            let (x, y) = (cx - b.w / 2., cy - b.h / 2.);
            if !patch.contains_key("x") {
                b.x = x;
            }
            if !patch.contains_key("y") {
                b.y = y;
            }
        }
        self.m.add_block(b);
        if let Some(items) = stack_items {
            self.apply_field(&id, "items", &items)?;
            applied.push("items");
        }
        self.dirty = true;
        let mut out = self.fields_json(self.m.block(&id).unwrap());
        out["applied"] = applied.into();
        Ok(out)
    }

    fn update_block(&mut self, args: &Args) -> Result<Value, String> {
        let id = arg_str(args, "id")?.ok_or("update_block 需要 id")?;
        let kind = self.block(&id)?.kind;
        let mut patch = args.clone();
        patch.remove("id");
        if let Some(fields) = patch.remove("fields") {
            let Value::Object(fields) = fields else { return Err("fields 要是对象".into()) };
            for (k, v) in fields {
                patch.entry(k).or_insert(v);
            }
        }
        self.validate_patch(kind, &id, &patch)?;
        let applied = self.apply_patch(&id, &patch)?;
        if !applied.is_empty() {
            self.dirty = true;
        }
        let mut out = self.fields_json(self.m.block(&id).unwrap());
        out["applied"] = applied.into();
        Ok(out)
    }

    fn update_blocks(&mut self, args: &Args) -> Result<Value, String> {
        let ids = arg_strings(args, "ids")?;
        let Some(Value::Object(fields)) = args.get("fields") else {
            return Err("update_blocks 需要 fields（对象）".into());
        };
        if ids.is_empty() {
            return Err("update_blocks 需要 ids".into());
        }
        for id in &ids {
            let kind = self.block(id)?.kind;
            self.validate_patch(kind, id, fields).map_err(|e| format!("{id}：{e}"))?;
        }
        let mut updated = Vec::new();
        for id in &ids {
            self.apply_patch(id, fields)?;
            updated.push(self.fields_json(self.m.block(id).unwrap()));
        }
        self.dirty = true;
        Ok(json!({"updated": updated}))
    }

    fn check_ids(&self, ids: &[String]) -> Result<(), String> {
        let missing: Vec<&str> = ids.iter().filter(|id| self.m.block(id).is_none()).map(String::as_str).collect();
        if missing.is_empty() { Ok(()) } else { Err(format!("没有这些 block：{}", missing.join(", "))) }
    }

    fn delete_blocks(&mut self, args: &Args) -> Result<Value, String> {
        let ids = arg_strings(args, "ids")?;
        if ids.is_empty() {
            return Err("delete_blocks 需要 ids".into());
        }
        self.check_ids(&ids)?;
        let mut deleted = Vec::new();
        for id in &ids {
            if self.m.block(id).is_some() {
                self.m.remove_block(id);
                deleted.push(id.clone());
            }
        }
        self.dirty = true;
        Ok(json!({"deleted": deleted, "blocks": self.m.blocks.len()}))
    }

    fn move_blocks(&mut self, args: &Args) -> Result<Value, String> {
        let ids = arg_strings(args, "ids")?;
        if ids.is_empty() {
            return Err("move_blocks 需要 ids".into());
        }
        self.check_ids(&ids)?;
        let dx = arg_num(args, "dx")?.unwrap_or(0.) as f32;
        let dy = arg_num(args, "dy")?.unwrap_or(0.) as f32;
        let mut seen = HashSet::new();
        let ps: Vec<(String, f32, f32)> = ids
            .iter()
            .filter(|id| seen.insert(*id))
            .map(|id| {
                let b = self.m.block(id).unwrap();
                (id.clone(), b.x + dx, b.y + dy)
            })
            .collect();
        let moved = self.m.apply_placements(&ps);
        self.dirty |= moved > 0;
        let out: Vec<Value> = ps
            .iter()
            .map(|(id, _, _)| {
                let b = self.m.block(id).unwrap();
                json!({"id": id, "x": round(b.x), "y": round(b.y)})
            })
            .collect();
        Ok(json!({"moved": out, "count": moved}))
    }

    fn select_blocks(&mut self, args: &Args) -> Result<Value, String> {
        let ids = arg_strings(args, "ids")?;
        let mode = arg_str(args, "mode")?.unwrap_or_default();
        if !matches!(mode.as_str(), "" | "replace" | "set" | "add" | "toggle") {
            return Err(format!("mode 要是 replace、add 或 toggle，收到 {mode:?}"));
        }
        self.check_ids(&ids)?;
        if matches!(mode.as_str(), "" | "replace" | "set") {
            self.m.sel.clear();
        }
        for id in ids {
            if mode == "toggle" && self.m.sel.contains(&id) {
                self.m.sel.remove(&id);
            } else {
                self.m.sel.insert(id);
            }
        }
        self.dirty = true;
        Ok(json!({"selected": self.selected_ids()}))
    }

    pub fn read_resource(&self, uri: &str) -> Result<Value, String> {
        let path = uri.strip_prefix("boundlessdeck://").ok_or_else(|| format!("只认 boundlessdeck:// 开头的资源：{uri}"))?;
        match path.trim_end_matches('/') {
            "deck" => Ok(self.deck_info()),
            "blocks" => Ok(json!({"blocks": self.list(None)})),
            "schema" => Ok(schema_digest()),
            p => match p.strip_prefix("blocks/") {
                Some(id) => Ok(self.fields_json(self.block(id)?)),
                None => Err(format!("没有资源 {uri}")),
            },
        }
    }

    /// Runs a tool.
    pub fn call(&mut self, name: &str, args: &Args) -> Result<Value, String> {
        match name {
            "deck_info" => Ok(self.deck_info()),
            "block_schema" => {
                let kinds = block_schema();
                match arg_str(args, "kind")?.filter(|k| !k.is_empty()) {
                    None => Ok(json!({"schemaVersion": SCHEMA_VERSION, "kinds": kinds})),
                    Some(k) => {
                        let k = BlockKind::parse_mcp(&k).map(|x| x.mcp_name()).unwrap_or(k.as_str()).to_string();
                        kinds.into_iter().find(|x| x["kind"] == k.as_str()).ok_or_else(|| format!("没有 kind {k:?}"))
                    }
                }
            }
            "list_blocks" => {
                let kind = arg_str(args, "kind")?.filter(|k| !k.is_empty());
                let kind = match kind {
                    Some(k) => Some(BlockKind::parse_mcp(&k).ok_or_else(|| format!("没有 kind {k:?}"))?),
                    None => None,
                };
                Ok(Value::Array(self.list(kind)))
            }
            "get_block" => {
                let id = arg_str(args, "id")?.unwrap_or_default();
                Ok(self.fields_json(self.block(&id)?))
            }
            "current_terminal" => Ok(self.current_terminal()),
            "read_selection" => {
                let blocks: Vec<Value> =
                    self.m.blocks.iter().filter(|b| self.m.sel.contains(&b.id)).map(|b| self.fields_json(b)).collect();
                Ok(json!({"ids": self.selected_ids(), "blocks": blocks}))
            }
            "create_block" => self.create_block(args),
            "update_block" => self.update_block(args),
            "update_blocks" => self.update_blocks(args),
            "delete_blocks" => self.delete_blocks(args),
            "move_blocks" => self.move_blocks(args),
            "align_blocks" => self.align_blocks(args),
            "distribute_blocks" => self.distribute_blocks(args),
            "arrange_blocks" => self.arrange_blocks(args),
            "select_blocks" => self.select_blocks(args),
            "set_view" => self.set_view(args),
            "undo" => {
                self.m.undo_last();
                self.dirty = true;
                Ok(json!({"ok": true, "blocks": self.m.blocks.len()}))
            }
            "redo" => {
                self.m.redo_last();
                self.dirty = true;
                Ok(json!({"ok": true, "blocks": self.m.blocks.len()}))
            }
            "save_deck" => {
                self.m.save().map_err(|e| e.to_string())?;
                Ok(json!({"saved": self.m.deck_file().map(|p| p.to_string_lossy().into_owned()), "blocks": self.m.blocks.len()}))
            }
            _ => Err(format!("没有工具 {name}")),
        }
    }

}

/// A browser address as it is kept.
pub fn normalize_url(s: &str) -> Result<String, String> {
    let s = s.trim();
    if s.is_empty() {
        return Ok(String::new());
    }
    let lower = s.to_ascii_lowercase();
    for bad in ["javascript:", "data:", "vbscript:"] {
        if lower.starts_with(bad) {
            return Err(format!("不接受 {} 开头的地址", bad.trim_end_matches(':')));
        }
    }
    if s.contains("://") || lower.starts_with("about:") { Ok(s.to_string()) } else { Ok(format!("https://{s}")) }
}

#[cfg(test)]
mod tests {
    use super::schema::field;
    use super::*;

    fn ctx_model() -> (tempfile::TempDir, Model) {
        let dir = tempfile::tempdir().unwrap();
        let mut m = Model::new(dir.path());
        m.clock = Some(1_000_000);
        (dir, m)
    }

    fn call(m: &mut Model, name: &str, args: Value) -> Result<Value, String> {
        call_from(m, Here::default(), name, args)
    }

    fn call_from(m: &mut Model, here: Here, name: &str, args: Value) -> Result<Value, String> {
        let mut c = Ctx { m, view: (1000., 800.), mcp: Value::Null, here, dirty: false, memos: vec![], reload: vec![], view_from: None };
        c.call(name, args.as_object().unwrap())
    }

    #[test]
    fn current_terminal_names_the_callers_and_the_users() {
        let (dir, mut m) = ctx_model();
        let mut t = Block::new(BlockKind::Terminal);
        t.id = "b0000t1".into();
        m.add_block(t);
        let mut n = Block::new(BlockKind::Note);
        n.id = "b0000n1".into();
        m.add_block(n);
        let root = dir.path().to_string_lossy().into_owned();

        let out = call(&mut m, "current_terminal", json!({})).unwrap();
        assert!(out["self"].is_null() && out["selfReason"].as_str().unwrap().contains("BOUNDLESSDECK_TERMINAL"));
        assert!(out["focused"].is_null());

        let here = |t: &str, deck: &str, f: Option<&str>| Here {
            caller: crate::mcp::Caller { terminal: Some(t.into()), deck: Some(deck.into()) },
            focused: f.map(str::to_string),
        };
        let out = call_from(&mut m, here("b0000t1", &root, Some("b0000t1")), "current_terminal", json!({})).unwrap();
        assert_eq!(out["self"]["id"], "b0000t1");
        assert_eq!(out["focused"]["id"], "b0000t1");
        let info = call_from(&mut m, here("b0000t1", &root, None), "deck_info", json!({})).unwrap();
        assert_eq!(info["callerTerminal"], "b0000t1");

        let out = call_from(&mut m, here("b0000t1", "/elsewhere", Some("b0000n1")), "current_terminal", json!({})).unwrap();
        assert!(out["self"].is_null() && out["selfReason"].as_str().unwrap().contains("另一个工作区"));
        assert!(out["focused"].is_null() && out["focusedReason"].as_str().unwrap().contains("不是终端"));

        let out = call_from(&mut m, here("b0000gone", &root, None), "current_terminal", json!({})).unwrap();
        assert!(out["selfReason"].as_str().unwrap().contains("不在画布上"));
    }

    #[test]
    fn every_block_field_is_in_the_table() {
        // Each field deck.json keeps is one an agent can see.
        let b = serde_json::to_value(Block::new(BlockKind::Note)).unwrap();
        let all = serde_json::to_value(Block {
            named: true,
            path: "p".into(),
            text: "t".into(),
            chart: "bar".into(),
            url: "u".into(),
            page: 1,
            expires: 1,
            items: vec!["x".into()],
            tabs: vec![crate::model::WebTab::new("a".into()), crate::model::WebTab::new("b".into())],
            tab: 1,
            ..Block::new(BlockKind::Note)
        })
        .unwrap();
        for key in b.as_object().unwrap().keys().chain(all.as_object().unwrap().keys()) {
            assert!(field(key).is_some(), "{key} is not in the MCP field table");
        }
    }

    #[test]
    fn writing_url_moves_the_open_tab() {
        let (_d, mut m) = ctx_model();
        let mut b = Block::new(BlockKind::Browser);
        b.id = "b00000a".into();
        b.url = "https://a".into();
        b.open_tab("https://b".into(), false);
        m.add_block(b);
        let out = call(&mut m, "update_block", json!({"id": "b00000a", "fields": {"url": "https://c"}})).unwrap();
        assert_eq!(out["tab"], 0);
        assert_eq!(out["tabs"][0]["url"], "https://c");
        assert_eq!(out["tabs"][1]["url"], "https://b", "the other tab is left alone");
        assert!(call(&mut m, "update_block", json!({"id": "b00000a", "tab": 1})).is_err(), "tab is read-only");
    }

    #[test]
    fn create_update_delete_and_undo() {
        let (_d, mut m) = ctx_model();
        let out = call(&mut m, "create_block", json!({"kind": "quicklook", "title": "想法", "text": "# hi"})).unwrap();
        let id = out["id"].as_str().unwrap().to_string();
        assert_eq!(out["named"], true);
        assert_eq!(out["expires"], 1_000_000 + NOTE_LIFE_MS, "a note goes in an hour by default");
        let out = call(&mut m, "update_block", json!({"id": id, "fields": {"w": 10, "ttlMinutes": 0}})).unwrap();
        assert_eq!(out["w"], 140.0);
        assert_eq!(out["expires"], 0);
        // Atomic: one bad field and nothing is written.
        let err = call(&mut m, "update_block", json!({"id": id, "x": 5, "url": "x"})).unwrap_err();
        assert!(err.contains("url"));
        assert_ne!(m.block(&id).unwrap().x, 5.);
        assert!(call(&mut m, "delete_blocks", json!({"ids": [id, "nope"]})).is_err());
        call(&mut m, "delete_blocks", json!({"ids": [id]})).unwrap();
        assert!(m.blocks.is_empty());
        call(&mut m, "undo", json!({})).unwrap();
        assert_eq!(m.blocks.len(), 1);
    }

    #[test]
    fn a_new_block_centers_on_the_view() {
        let (_d, mut m) = ctx_model();
        // The window is 1000 × 800; moved so its middle is at (2000, -300).
        call(&mut m, "set_view", json!({"x": 2000, "y": -300, "scale": 1})).unwrap();
        for _ in 0..2 {
            let b = call(&mut m, "create_block", json!({"kind": "chart", "w": 400, "h": 200})).unwrap();
            assert_eq!((b["x"].as_f64(), b["y"].as_f64()), (Some(1800.), Some(-400.)), "the same spot, even taken");
        }
        let b = call(&mut m, "create_block", json!({"kind": "chart", "w": 400, "h": 200, "x": 5})).unwrap();
        assert_eq!((b["x"].as_f64(), b["y"].as_f64()), (Some(5.), Some(-400.)));
    }

    #[test]
    fn set_view_centers_the_window_where_asked() {
        let (_d, mut m) = ctx_model();
        let out = call(&mut m, "set_view", json!({"x": 500, "y": -200, "scale": 2})).unwrap();
        assert_eq!(out["center"], json!({"x": 500.0, "y": -200.0}));
        assert_eq!(out["scale"], 2.0);
        // The window is 1000 × 800: at 2×, its middle is 250 × 200 world units from the corner.
        assert_eq!((m.cam.x, m.cam.y, m.cam.s), (250., -400., 2.));
        let b = call(&mut m, "create_block", json!({"kind": "chart", "x": 1000, "y": 1000, "w": 400, "h": 200})).unwrap();
        let id = b["id"].as_str().unwrap();
        let out = call(&mut m, "set_view", json!({"id": id, "fit": true})).unwrap();
        assert_eq!(out["center"], json!({"x": 1200.0, "y": 1100.0}));
        let s = out["scale"].as_f64().unwrap();
        assert!(s > 1.8 && s < 2.2, "about 0.8 × 1000 / 400: {s}");
        assert!(call(&mut m, "set_view", json!({"id": id, "x": 1})).is_err());
        assert!(call(&mut m, "set_view", json!({})).is_err());
        assert!(call(&mut m, "set_view", json!({"id": "nope"})).is_err());
        // Out of range scales are held to the range.
        assert_eq!(call(&mut m, "set_view", json!({"scale": 100})).unwrap()["scale"], 4.0);
    }

    #[test]
    fn a_video_file_starts_16_by_9_and_says_so() {
        let (d, mut m) = ctx_model();
        std::fs::write(d.path().join("clip.mp4"), b"x").unwrap();
        let out = call(&mut m, "create_block", json!({"kind": "file", "path": "clip.mp4"})).unwrap();
        assert_eq!(out["preview"], "video");
        assert_eq!((out["w"].as_f64(), out["h"].as_f64()), (Some(640.), Some(386.)));
        let out = call(&mut m, "create_block", json!({"kind": "file", "path": "clip.mp4", "w": 300, "h": 200})).unwrap();
        assert_eq!((out["w"].as_f64(), out["h"].as_f64()), (Some(300.), Some(200.)));
    }

    #[test]
    fn memo_text_goes_to_its_file() {
        let (d, mut m) = ctx_model();
        let out = call(&mut m, "create_block", json!({"kind": "note", "title": "计划", "text": "- a"})).unwrap();
        assert_eq!(out["path"], "计划.md");
        assert_eq!(out["kind"], "note", "笔记 is note over MCP");
        assert_eq!(std::fs::read_to_string(d.path().join("计划.md")).unwrap(), "- a");
        // deck.json keeps "memo", as the Go version reads it; "memo" still names it over MCP.
        let saved = serde_json::to_value(m.block(out["id"].as_str().unwrap()).unwrap()).unwrap();
        assert_eq!(saved["kind"], "memo");
        let old = call(&mut m, "create_block", json!({"kind": "memo", "title": "旧名"})).unwrap();
        assert_eq!(old["kind"], "note");
        let ql = call(&mut m, "list_blocks", json!({"kind": "quicklook"})).unwrap();
        assert_eq!(ql, json!([]));
        let id = out["id"].as_str().unwrap();
        let out = call(&mut m, "update_block", json!({"id": id, "title": "总结"})).unwrap();
        assert_eq!(out["path"], "总结.md");
        assert!(d.path().join("总结.md").exists());
        assert!(call(&mut m, "update_block", json!({"id": id, "path": "x.txt"})).is_err());
    }

    #[test]
    fn stacks_and_layouts() {
        let (_d, mut m) = ctx_model();
        let mut ids = Vec::new();
        for i in 0..3 {
            let out = call(&mut m, "create_block", json!({"kind": "chart", "x": i * 100, "y": i * 7, "w": 200})).unwrap();
            ids.push(out["id"].as_str().unwrap().to_string());
        }
        let st = call(&mut m, "create_block", json!({"kind": "stack", "items": [ids[0]]})).unwrap();
        assert_eq!(st["items"], json!([ids[0]]));
        assert!(m.stacked_ids().contains(&ids[0]));
        call(&mut m, "undo", json!({})).unwrap();
        let out = call(&mut m, "arrange_blocks", json!({"ids": [ids[1], ids[2]], "layout": "row", "originX": 0, "originY": 0})).unwrap();
        assert_eq!(out["moved"], 2);
        assert_eq!(m.block(&ids[2]).unwrap().x, 224.);
        let out = call(&mut m, "align_blocks", json!({"ids": [ids[1], ids[2]], "align": "top", "to": ids[1]})).unwrap();
        assert_eq!(out["to"], ids[1]);
        assert!(call(&mut m, "distribute_blocks", json!({"ids": [ids[1], ids[2]]})).is_err());
    }

    #[test]
    fn urls_are_normalized_and_resources_read() {
        assert_eq!(normalize_url("example.com").unwrap(), "https://example.com");
        assert!(normalize_url("javascript:alert(1)").is_err());
        let (_d, mut m) = ctx_model();
        let c = Ctx { m: &mut m, view: (0., 0.), mcp: Value::Null, here: Here::default(), dirty: false, memos: vec![], reload: vec![], view_from: None };
        assert_eq!(c.read_resource("boundlessdeck://schema").unwrap()["kinds"].as_array().unwrap().len(), BlockKind::ALL.len());
        assert!(c.read_resource("other://x").is_err());
    }
}
