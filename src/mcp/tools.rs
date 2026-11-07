//! The workspace's MCP tools and resources, over the deck's model. Every
//! write goes through the model's own add/remove/undo path, so what an
//! agent does undoes as the user's edits do. The block field table is the
//! single source of what may be read and written.

use std::collections::HashSet;
use std::path::Path;

use serde_json::{Map, Value, json};

use crate::lifespan::NOTE_LIFE_MS;
use crate::memo;
use crate::model::{BlockKind, Block, MIN_BLOCK_H, MIN_BLOCK_W, Model};
use crate::preview;

pub const SCHEMA_VERSION: i32 = 4;
pub const SERVER_NAME: &str = "boundlessdeck";

pub type Args = Map<String, Value>;

/// One field of a block as agents see it.
pub struct Field {
    pub name: &'static str,
    pub ty: &'static str,
    pub mutable: bool,
    pub on_create: bool,
    /// Empty: every kind.
    pub kinds: &'static [BlockKind],
    pub min: Option<f64>,
    pub enum_: &'static [&'static str],
    /// Kept in no block: worked out from one that is.
    pub derived: bool,
    pub description: &'static str,
}

const ALL: &[BlockKind] = &[];
const KIND_NAMES: &[&str] = &["note", "memo", "file", "terminal", "browser", "chart", "stack"];
const CHART_NAMES: &[&str] = &["bar", "hbar", "line", "area", "pie"];

pub const FIELDS: &[Field] = &[
    Field { name: "id", ty: "string", mutable: false, on_create: false, kinds: ALL, min: None, enum_: &[], derived: false,
        description: "block 的稳定 ID（b 加 6 位十六进制），创建后不可改；所有写操作都用它指名 block。" },
    Field { name: "kind", ty: "string", mutable: false, on_create: true, kinds: ALL, min: None, enum_: KIND_NAMES, derived: false,
        description: "block 类型：note（QuickLook 便签，正文存在 deck 里）、memo（笔记：工作目录里的一个 Markdown 文件）、file（链接的本地文件）、terminal（终端）、browser（浏览器）、chart（图表：画它 text 里的表格）、stack（块堆：收纳其他 block 的钱包卡片，收进去的 block 不在画布上显示）。只在创建时给出，之后不可改。" },
    Field { name: "title", ty: "string", mutable: true, on_create: true, kinds: ALL, min: None, enum_: &[], derived: false,
        description: "标题。写非空值会同时把它标记为用户命名（named=true）；写空串则回到 kind 或文件的默认标题。" },
    Field { name: "named", ty: "boolean", mutable: true, on_create: true, kinds: ALL, min: None, enum_: &[], derived: false,
        description: "标题是否由用户命名；false 时终端 shell 或网页可以自动改标题。" },
    Field { name: "x", ty: "number", mutable: true, on_create: true, kinds: ALL, min: None, enum_: &[], derived: false,
        description: "世界坐标 X（画布坐标，不是屏幕像素）。省略时新 block 落在视口中央附近的空位。" },
    Field { name: "y", ty: "number", mutable: true, on_create: true, kinds: ALL, min: None, enum_: &[], derived: false,
        description: "世界坐标 Y。" },
    Field { name: "w", ty: "number", mutable: true, on_create: true, kinds: ALL, min: Some(MIN_BLOCK_W as f64), enum_: &[], derived: false,
        description: "世界坐标宽度，最小 140。" },
    Field { name: "h", ty: "number", mutable: true, on_create: true, kinds: ALL, min: Some(MIN_BLOCK_H as f64), enum_: &[], derived: false,
        description: "世界坐标高度，最小 90。" },
    Field { name: "path", ty: "string", mutable: true, on_create: true, kinds: &[BlockKind::File, BlockKind::Memo], min: None, enum_: &[], derived: false,
        description: "文件路径。工作区内的相对路径（相对 deck 根），工作区外用绝对路径；写绝对路径会尽量转成相对路径。改路径会重新加载预览。memo 创建时可省略：按标题在 deck 根下新建 <标题>.md（重名加序号）；给出的 .md 不存在时会新建空文件。" },
    Field { name: "text", ty: "string", mutable: true, on_create: true, kinds: &[BlockKind::Note, BlockKind::Memo, BlockKind::Chart], min: None, enum_: &[], derived: false,
        description: "正文（Markdown）。note 存在 deck 里；memo 直接写进它的文件（覆盖整个文件），读 memo 的内容请读 absPath 指向的文件。chart 的 text 是它画的表格：每行一项，第一列是名称，其后每列一组数值，用逗号或 Tab 分隔（也可用 Markdown 表格）；第一行含非数字时作为列名（图例）。最多 8 组、200 行；饼图只画第一组。" },
    Field { name: "chart", ty: "string", mutable: true, on_create: true, kinds: &[BlockKind::Chart], min: None, enum_: CHART_NAMES, derived: false,
        description: "图表类型：bar 柱状图、hbar 条形图、line 折线图、area 面积图、pie 饼图。省略为 bar。" },
    Field { name: "expires", ty: "integer", mutable: true, on_create: true, kinds: &[BlockKind::Note], min: Some(0.), enum_: &[], derived: false,
        description: "QuickLook 自动消失的时刻，Unix 毫秒；0 为永久保留。新建 note 不给时默认 1 小时后消失。到时它会被删除（用户可 ⌘Z 找回）；关闭期间到期的在打开 deck 时直接移除。只想说“多久后消失”时用 ttlMinutes。" },
    Field { name: "ttlMinutes", ty: "number", mutable: true, on_create: true, kinds: &[BlockKind::Note], min: Some(0.), enum_: &[], derived: true,
        description: "从现在起多少分钟后消失（可带小数），写入时换算成 expires；0 为永久保留。读出的是剩余分钟数（保留一位小数，永久的为 0）。不能与 expires 同时给。" },
    Field { name: "page", ty: "integer", mutable: true, on_create: true, kinds: &[BlockKind::File], min: Some(0.), enum_: &[], derived: false,
        description: "PDF 显示的页，从 0 开始；对非 PDF 文件无意义，越界会由预览夹到有效范围。" },
    Field { name: "url", ty: "string", mutable: true, on_create: true, kinds: &[BlockKind::Browser], min: None, enum_: &[], derived: false,
        description: "浏览器显示的地址。http/https/file 原样使用；裸域名或搜索词会补上 https://；javascript: 与 data: 会被拒绝。空串清空页面。" },
    Field { name: "tabs", ty: "array", mutable: false, on_create: false, kinds: &[BlockKind::Browser], min: None, enum_: &[], derived: false,
        description: "浏览器的标签页，每项 {url, title}，按从左到右的顺序；只读。用户点开新窗口的链接时会在当前标签后新开一个。url 字段始终是打开的那个标签的地址，写 url 就是让打开的标签跳转。" },
    Field { name: "tab", ty: "integer", mutable: false, on_create: false, kinds: &[BlockKind::Browser], min: Some(0.), enum_: &[], derived: false,
        description: "打开的是 tabs 中的第几个，从 0 开始；只读。" },
    Field { name: "items", ty: "array", mutable: true, on_create: true, kinds: &[BlockKind::Stack], min: None, enum_: &[], derived: false,
        description: "块堆里的 block ID，按从上到下的卡片顺序。收进堆的 block 仍留在 deck 里、可用 get_block 读写，但不再画在画布上；一个 block 只属于一个堆，块堆不能装块堆。" },
];

fn field(name: &str) -> Option<&'static Field> {
    FIELDS.iter().find(|f| f.name == name)
}

fn for_kind(f: &Field, k: BlockKind) -> bool {
    f.kinds.is_empty() || f.kinds.contains(&k)
}

fn mutable_names(k: BlockKind) -> Vec<&'static str> {
    FIELDS.iter().filter(|f| f.mutable && for_kind(f, k)).map(|f| f.name).collect()
}

fn read_only_names(k: BlockKind) -> Vec<&'static str> {
    FIELDS.iter().filter(|f| !f.mutable && for_kind(f, k)).map(|f| f.name).collect()
}

/// A new block's title and size when an agent gives none.
pub fn kind_defaults(k: BlockKind) -> (&'static str, f32, f32) {
    match k {
        BlockKind::Note => ("QuickLook", 360., 260.),
        BlockKind::Memo => (memo::MEMO_TITLE, 440., 520.),
        BlockKind::File => ("文件", 560., 420.),
        BlockKind::Terminal => ("终端", 560., 340.),
        BlockKind::Browser => ("浏览器", 760., 540.),
        BlockKind::Stack => ("块堆", 380., 300.),
        BlockKind::Chart => ("图表", 480., 320.),
    }
}

fn field_json(f: &Field) -> Value {
    let mut o = json!({"name": f.name, "type": f.ty, "mutable": f.mutable, "description": f.description});
    let m = o.as_object_mut().unwrap();
    if f.on_create {
        m.insert("onCreate".into(), true.into());
    }
    if !f.kinds.is_empty() {
        m.insert("kinds".into(), f.kinds.iter().map(|k| k.as_str()).collect::<Vec<_>>().into());
    }
    if let Some(min) = f.min {
        m.insert("min".into(), min.into());
    }
    if !f.enum_.is_empty() {
        m.insert("enum".into(), f.enum_.to_vec().into());
    }
    if f.derived {
        m.insert("derived".into(), true.into());
    }
    o
}

pub fn block_schema() -> Vec<Value> {
    BlockKind::ALL
        .iter()
        .map(|&k| {
            let (title, w, h) = kind_defaults(k);
            let fields: Vec<&Field> = FIELDS.iter().filter(|f| for_kind(f, k)).collect();
            json!({
                "kind": k.as_str(),
                "defaults": {"title": title, "w": w, "h": h},
                "create": fields.iter().filter(|f| f.on_create).map(|f| f.name).collect::<Vec<_>>(),
                "mutable": mutable_names(k),
                "readOnly": read_only_names(k),
                "fields": fields.iter().map(|f| field_json(f)).collect::<Vec<_>>(),
            })
        })
        .collect()
}

pub fn schema_digest() -> Value {
    json!({
        "schemaVersion": SCHEMA_VERSION,
        "blockVersion": 1,
        "coordinateUnit": "画布世界坐标；x/y 是左上角，w/h 是尺寸，屏幕位置 = (x - camX) * camS",
        "kinds": block_schema(),
    })
}

fn field_prop(f: &Field) -> Value {
    let mut desc = f.description.to_string();
    if !f.kinds.is_empty() {
        desc += &format!("（仅 {}）", f.kinds.iter().map(|k| k.as_str()).collect::<Vec<_>>().join("、"));
    }
    let mut p = json!({"type": f.ty, "description": desc});
    if let Some(min) = f.min {
        p["minimum"] = min.into();
    }
    if !f.enum_.is_empty() {
        p["enum"] = f.enum_.to_vec().into();
    }
    if f.ty == "array" {
        p["items"] = json!({"type": "string"});
    }
    p
}

fn object_schema(props: Value, required: &[&str]) -> Value {
    let mut s = json!({"type": "object", "properties": props, "additionalProperties": false});
    if !required.is_empty() {
        s["required"] = required.to_vec().into();
    }
    s
}

fn ids_prop(what: &str) -> Value {
    json!({"type": "array", "items": {"type": "string"}, "description": what})
}

/// The tools, as tools/list describes them: name, title, description, input schema.
pub fn tool_list() -> Vec<(&'static str, &'static str, &'static str, Value)> {
    let kind_prop = json!({"type": "string", "enum": KIND_NAMES, "description": "只看这一种 block"});
    let mut mutable = Map::new();
    let mut create = Map::new();
    for f in FIELDS {
        if f.mutable {
            mutable.insert(f.name.into(), field_prop(f));
        }
        if f.on_create {
            create.insert(f.name.into(), field_prop(f));
        }
    }
    let mut update = mutable.clone();
    update.insert("id".into(), json!({"type": "string", "description": "block ID"}));
    update.insert(
        "fields".into(),
        json!({"type": "object", "description": "要写的字段，等价于把它们平铺在参数里", "additionalProperties": true}),
    );
    vec![
        ("deck_info", "工作区概况", "看当前工作区：路径、block 数、选择集、相机、MCP 端点。任何写操作前先调它。", object_schema(json!({}), &[])),
        ("block_schema", "block 字段 schema", "列出每种 block 的可写字段、只读字段、默认值与约束。改字段前用它确认字段名和类型；省略 kind 则返回全部种类。", object_schema(json!({"kind": kind_prop}), &[])),
        ("list_blocks", "列出 block", "列出画布上的 block，含每个的当前值与可写字段。可按 kind 过滤。", object_schema(json!({"kind": kind_prop}), &[])),
        ("get_block", "读一个 block", "按 id 读一个 block 的全部字段。", object_schema(json!({"id": {"type": "string", "description": "block ID"}}), &["id"])),
        ("read_selection", "读选择集", "读人当前选中的 block——「选择即上下文」。", object_schema(json!({}), &[])),
        ("current_terminal", "当前终端", "告诉你两个终端：self 是你自己运行在其中的终端 block（你从 BoundlessDeck 的终端里启动时才有，按启动你的那个终端识别）；focused 是人此刻正在操作的终端 block。想在自己旁边放东西、改自己的标题，或知道人在看哪个终端时用。不是终端时为 null，并在 selfReason / focusedReason 里说明原因。", object_schema(json!({}), &[])),
        ("create_block", "新建 block", "在画布上建一个 block 并写初始字段；kind 必填。file 需要 path。省略 x/y 时落在视口中央附近的空位。", object_schema(Value::Object(create), &["kind"])),
        ("update_block", "改一个 block", "按 id 写一个或多个可写字段；字段按 block 的 kind 校验，写入是原子的（有一个不合法就一个都不写）。也可以用 fields 对象一次传多个字段。", object_schema(Value::Object(update), &["id"])),
        ("update_blocks", "批量改 block", "把同一组字段写到多个 block；任何一个不合法就整批失败。", object_schema(json!({"ids": ids_prop("要改的 block ID"), "fields": {"type": "object", "additionalProperties": true, "description": "要写的字段"}}), &["ids", "fields"])),
        ("delete_blocks", "删除 block", "删除一个或多个 block；有任何一个 id 不存在就一个都不删。可撤销。", object_schema(json!({"ids": ids_prop("要删的 block ID")}), &["ids"])),
        ("move_blocks", "移动 block", "把一组 block 在世界坐标里平移 dx、dy。要落到位就用 update_block 的 x/y。", object_schema(json!({"ids": ids_prop("要移动的 block ID"), "dx": {"type": "number", "description": "水平位移"}, "dy": {"type": "number", "description": "垂直位移"}}), &["ids"])),
        ("align_blocks", "对齐 block", "把一组 block 对齐到同一条边或中线（左、右、上、下、水平居中、垂直居中）。默认以这组 block 的外框为参考，也可以对齐到第一个、最后一个或指定的 block；再给 spacing 就会沿另一轴按顺序排成一列/一行。省略 ids 则用当前选择集。",
            object_schema(json!({
                "ids": ids_prop("要对齐的 block；省略则用当前选择集"),
                "align": {"type": "string", "enum": ["left", "right", "top", "bottom", "centerX", "centerY"], "description": "对齐到哪条边或中线"},
                "to": {"type": "string", "description": "参考：bounds（默认，这组 block 的外框）、first、last，或一个 block ID"},
                "spacing": {"type": "number", "minimum": 0, "description": "给了就沿另一轴按当前顺序首尾排开，间距为这个值（world 单位）：对齐 left + spacing 得到整齐的一列"},
            }), &["align"])),
        ("distribute_blocks", "均匀分布 block", "沿 x 或 y 轴把三个以上 block 均匀排开，最外侧两个不动。mode=gaps 让相邻间隙相等，mode=centers 让中心间距相等。空间不够时默认报错，允许重叠传 allowOverlap=true。",
            object_schema(json!({
                "ids": ids_prop("要分布的 block；省略则用当前选择集"),
                "axis": {"type": "string", "enum": ["x", "y"], "description": "沿哪条轴分布"},
                "mode": {"type": "string", "enum": ["gaps", "centers"], "description": "gaps（默认）等间隙，centers 等中心距"},
                "allowOverlap": {"type": "boolean", "description": "间隙算出来是负数时也照做（block 会重叠）"},
            }), &[])),
        ("arrange_blocks", "排列 block", "把 block 排成一行、一列或网格，保持各自大小，默认外框左上角为起点、间距 24。网格按 ids 顺序填充，columns 省略时取 ceil(sqrt(n))；order=position 按画布上的位置（从上到下、从左到右）排。",
            object_schema(json!({
                "ids": ids_prop("要排列的 block；省略则用当前选择集"),
                "layout": {"type": "string", "enum": ["row", "column", "grid"], "description": "排成一行、一列或网格"},
                "columns": {"type": "integer", "minimum": 1, "description": "网格的列数；省略取 ceil(sqrt(n))"},
                "gapX": {"type": "number", "minimum": 0, "description": "列间距，默认 24"},
                "gapY": {"type": "number", "minimum": 0, "description": "行间距，默认 24"},
                "originX": {"type": "number", "description": "左上角 X；省略用这组 block 外框的 X"},
                "originY": {"type": "number", "description": "左上角 Y；省略用这组 block 外框的 Y"},
                "order": {"type": "string", "enum": ["ids", "position"], "description": "ids（默认）按 ids 顺序，position 按画布位置"},
            }), &[])),
        ("select_blocks", "设置选择集", "改选择集，让窗口里选中 agent 正在处理的 block；mode 为 replace（默认）、add 或 toggle。",
            object_schema(json!({"ids": ids_prop("要选中的 block ID"), "mode": {"type": "string", "enum": ["replace", "add", "toggle"], "description": "replace 换成这些，add 加上，toggle 取反"}}), &["ids"])),
        ("undo", "撤销", "撤销上一步画布操作（加、删 block），与窗口里的 ⌘Z 同一条撤销栈。", object_schema(json!({}), &[])),
        ("redo", "重做", "重做上一步撤销。", object_schema(json!({}), &[])),
        ("save_deck", "保存画布", "立刻把画布写进工作区的 .boundlessdeck/deck.json；写操作本来就会自动保存，这个用于确认。", object_schema(json!({}), &[])),
    ]
}

pub fn tool_names() -> Vec<&'static str> {
    tool_list().into_iter().map(|t| t.0).collect()
}

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
            "id": n.id, "kind": n.kind.as_str(), "title": n.title, "named": n.named,
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
        }
        out
    }

    fn list(&self, kind: Option<BlockKind>) -> Vec<Value> {
        self.m.blocks.iter().filter(|b| kind.is_none_or(|k| b.kind == k)).map(|b| self.fields_json(b)).collect()
    }

    pub fn deck_info(&self) -> Value {
        json!({
            "workspace": self.m.root.to_string_lossy(),
            "deckFile": self.m.deck_file().map(|p| p.to_string_lossy().into_owned()),
            "blocks": self.m.blocks.len(),
            "selected": self.selected_ids(),
            "camera": {"x": round(self.m.cam.x), "y": round(self.m.cam.y), "s": round(self.m.cam.s)},
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

    // ---- patches ----

    fn validate_patch(&self, kind: BlockKind, self_id: &str, patch: &Args) -> Result<(), String> {
        for name in patch.keys() {
            let Some(f) = field(name) else {
                return Err(format!("未知字段 {name:?}；用 block_schema 看每个 kind 的可写字段"));
            };
            if !f.mutable || !for_kind(f, kind) {
                return Err(format!(
                    "{} block 不能写字段 {name:?}（可写：{}）",
                    kind.as_str(),
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
    fn check_field(&self, n: &mut Block, name: &str, v: &Value) -> Result<(), String> {
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
    fn apply_patch(&mut self, id: &str, patch: &Args) -> Result<Vec<&'static str>, String> {
        let mut applied = Vec::new();
        for f in FIELDS {
            let Some(v) = patch.get(f.name) else { continue };
            self.apply_field(id, f.name, v).map_err(|e| format!("{}：{e}", f.name))?;
            applied.push(f.name);
        }
        Ok(applied)
    }

    fn apply_field(&mut self, id: &str, name: &str, v: &Value) -> Result<(), String> {
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

    fn ensure_memo_file(&mut self, id: &str) -> Result<(), String> {
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

    // ---- tools ----

    fn create_block(&mut self, args: &Args) -> Result<Value, String> {
        let kind_s = arg_str(args, "kind")?.ok_or_else(|| format!("create_block 需要 kind：{}", KIND_NAMES.join("、")))?;
        let kind = BlockKind::parse(&kind_s).ok_or_else(|| format!("没有 kind {kind_s:?}：可用 {}", KIND_NAMES.join("、")))?;
        let mut patch = args.clone();
        patch.remove("kind");
        if kind == BlockKind::File && !patch.contains_key("path") {
            return Err("创建 file block 需要 path".into());
        }
        let id = self.m.new_id();
        self.validate_patch(kind, &id, &patch)?;
        let (title, w, h) = kind_defaults(kind);
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
            let (x, y) = self.m.free_spot(cx - b.w / 2., cy - b.h / 2., b.w, b.h);
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
                    Some(k) => kinds.into_iter().find(|x| x["kind"] == k.as_str()).ok_or_else(|| format!("没有 kind {k:?}")),
                }
            }
            "list_blocks" => {
                let kind = arg_str(args, "kind")?.filter(|k| !k.is_empty());
                let kind = match kind {
                    Some(k) => Some(BlockKind::parse(&k).ok_or_else(|| format!("没有 kind {k:?}"))?),
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

    // ---- layout ----

    fn layout_ids(&self, args: &Args) -> Result<Vec<String>, String> {
        let mut ids = arg_strings(args, "ids")?;
        if ids.is_empty() {
            ids = self.selected_ids();
        }
        if ids.is_empty() {
            return Err("没有指定 block：给 ids，或先在画布上选中几个".into());
        }
        let mut seen = HashSet::new();
        ids.retain(|id| seen.insert(id.clone()));
        self.check_ids(&ids)?;
        Ok(ids)
    }

    fn rects(&self, ids: &[String]) -> Vec<(String, f32, f32, f32, f32)> {
        ids.iter()
            .map(|id| {
                let b = self.m.block(id).unwrap();
                (id.clone(), b.x, b.y, b.w, b.h)
            })
            .collect()
    }

    fn geometry_list(&self, ids: &[String]) -> Vec<Value> {
        ids.iter()
            .map(|id| {
                let b = self.m.block(id).unwrap();
                json!({"id": id, "kind": b.kind.as_str(), "title": b.title, "x": round(b.x), "y": round(b.y), "w": round(b.w), "h": round(b.h)})
            })
            .collect()
    }

    fn align_blocks(&mut self, args: &Args) -> Result<Value, String> {
        let ids = self.layout_ids(args)?;
        let align = arg_str(args, "align")?.ok_or("align_blocks 需要 align：left、right、top、bottom、centerX 或 centerY")?;
        let align = match align.trim().to_ascii_lowercase().as_str() {
            "left" | "start" => "left",
            "right" | "end" => "right",
            "top" => "top",
            "bottom" => "bottom",
            "centerx" | "center-x" | "hc" | "hcenter" | "horizontal-center" | "xcenter" => "centerX",
            "centery" | "center-y" | "vc" | "vcenter" | "vertical-center" | "middle" | "ycenter" => "centerY",
            _ => return Err(format!("align 要是 left、right、top、bottom、centerX 或 centerY，收到 {align:?}")),
        };
        let to = arg_str(args, "to")?.unwrap_or_default();
        let spacing = arg_num(args, "spacing")?;
        if spacing.is_some_and(|s| s < 0.) {
            return Err("spacing 不能是负数".into());
        }
        let rs = self.rects(&ids);
        let bounds = layout::bounds(&rs);
        let (ref_rect, ref_id) = match to.trim().to_ascii_lowercase().as_str() {
            "" | "bounds" | "set" => {
                if rs.len() < 2 {
                    return Err("按 bounds 对齐需要至少两个 block；对齐到一个 block 用 to".into());
                }
                (bounds, None)
            }
            "first" | "last" => {
                if rs.len() < 2 {
                    return Err(format!("按 {to} 对齐需要至少两个 block"));
                }
                let r = if to.eq_ignore_ascii_case("first") { &rs[0] } else { &rs[rs.len() - 1] };
                ((r.1, r.2, r.3, r.4), Some(r.0.clone()))
            }
            _ => {
                let b = self.m.block(&to).ok_or_else(|| format!("to 要是 bounds、first、last 或一个 block ID，收到 {to:?}"))?;
                ((b.x, b.y, b.w, b.h), Some(to.clone()))
            }
        };
        let ps = layout::align(&rs, align, ref_rect, ref_id.as_deref(), spacing.map(|s| s as f32));
        let moved = self.m.apply_placements(&ps);
        self.dirty |= moved > 0;
        let mut out = json!({"align": align, "to": ref_id.unwrap_or("bounds".into()), "moved": moved, "blocks": self.geometry_list(&ids)});
        if let Some(s) = spacing {
            out["spacing"] = s.into();
        }
        Ok(out)
    }

    fn distribute_blocks(&mut self, args: &Args) -> Result<Value, String> {
        let ids = self.layout_ids(args)?;
        let axis = match arg_str(args, "axis")?.unwrap_or_default().trim().to_ascii_lowercase().as_str() {
            "" | "x" | "h" | "horizontal" | "row" => "x",
            "y" | "v" | "vertical" | "column" => "y",
            a => return Err(format!("axis 要是 x 或 y，收到 {a:?}")),
        };
        let mode = match arg_str(args, "mode")?.unwrap_or_default().trim().to_ascii_lowercase().as_str() {
            "" | "gaps" | "gap" | "space" => "gaps",
            "centers" | "center" => "centers",
            m => return Err(format!("mode 要是 gaps 或 centers，收到 {m:?}")),
        };
        let allow = arg_bool(args, "allowOverlap")?.unwrap_or(false);
        if ids.len() < 3 {
            return Err(format!("distribute_blocks 需要至少三个 block（首尾保持不动），收到 {} 个", ids.len()));
        }
        let rs = self.rects(&ids);
        let (order, ps, gap_or_step) = layout::distribute(&rs, axis == "x", mode == "centers", allow)?;
        let moved = self.m.apply_placements(&ps);
        self.dirty |= moved > 0;
        let mut out = json!({"axis": axis, "mode": mode, "moved": moved, "blocks": self.geometry_list(&order)});
        out[if mode == "gaps" { "gap" } else { "step" }] = round(gap_or_step).into();
        Ok(out)
    }

    fn arrange_blocks(&mut self, args: &Args) -> Result<Value, String> {
        let ids = self.layout_ids(args)?;
        let layout_ = match arg_str(args, "layout")?.unwrap_or_default().trim().to_ascii_lowercase().as_str() {
            "" | "grid" => "grid",
            "row" | "line" | "horizontal" => "row",
            "column" | "col" | "vertical" | "stack" => "column",
            l => return Err(format!("layout 要是 row、column 或 grid，收到 {l:?}")),
        };
        let gap_x = arg_num(args, "gapX")?.unwrap_or(24.);
        let gap_y = arg_num(args, "gapY")?.unwrap_or(24.);
        if gap_x < 0. || gap_y < 0. {
            return Err("gapX 与 gapY 不能是负数".into());
        }
        let columns = arg_int(args, "columns")?.unwrap_or(0).max(0) as usize;
        let origin = (arg_num(args, "originX")?.map(|v| v as f32), arg_num(args, "originY")?.map(|v| v as f32));
        let by_position = match arg_str(args, "order")?.unwrap_or_default().trim().to_ascii_lowercase().as_str() {
            "" | "ids" => false,
            "position" => true,
            o => return Err(format!("order 要是 ids 或 position，收到 {o:?}")),
        };
        let rs = self.rects(&ids);
        let (order, ps, cols, rows) =
            layout::arrange(&rs, layout_, columns, gap_x as f32, gap_y as f32, origin, by_position);
        let moved = self.m.apply_placements(&ps);
        self.dirty |= moved > 0;
        Ok(json!({"layout": layout_, "columns": cols, "rows": rows, "gapX": gap_x, "gapY": gap_y, "moved": moved, "blocks": self.geometry_list(&order)}))
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

/// Layouts work out where blocks go first; the model then moves them as one change.
pub mod layout {
    pub type Rect = (String, f32, f32, f32, f32);

    pub fn bounds(rs: &[Rect]) -> (f32, f32, f32, f32) {
        let x0 = rs.iter().map(|r| r.1).fold(f32::MAX, f32::min);
        let y0 = rs.iter().map(|r| r.2).fold(f32::MAX, f32::min);
        let x1 = rs.iter().map(|r| r.1 + r.3).fold(f32::MIN, f32::max);
        let y1 = rs.iter().map(|r| r.2 + r.4).fold(f32::MIN, f32::max);
        (x0, y0, x1 - x0, y1 - y0)
    }

    pub fn align(
        rs: &[Rect],
        align: &str,
        r: (f32, f32, f32, f32),
        ref_id: Option<&str>,
        spacing: Option<f32>,
    ) -> Vec<(String, f32, f32)> {
        let horizontal = matches!(align, "left" | "right" | "centerX");
        let mut out: Vec<(String, f32, f32)> = rs
            .iter()
            .map(|b| {
                if Some(b.0.as_str()) == ref_id {
                    return (b.0.clone(), b.1, b.2);
                }
                let (mut x, mut y) = (b.1, b.2);
                match align {
                    "left" => x = r.0,
                    "right" => x = r.0 + r.2 - b.3,
                    "centerX" => x = r.0 + r.2 / 2. - b.3 / 2.,
                    "top" => y = r.1,
                    "bottom" => y = r.1 + r.3 - b.4,
                    _ => y = r.1 + r.3 / 2. - b.4 / 2.,
                }
                (b.0.clone(), x, y)
            })
            .collect();
        if let Some(sp) = spacing {
            let mut order: Vec<usize> = (0..rs.len()).collect();
            order.sort_by(|&i, &j| {
                let (a, b) = (&rs[i], &rs[j]);
                let k = |r: &Rect| if horizontal { (r.2, r.1) } else { (r.1, r.2) };
                k(a).partial_cmp(&k(b)).unwrap_or(std::cmp::Ordering::Equal)
            });
            let mut cursor = if horizontal { r.1 } else { r.0 };
            if let Some(rb) = ref_id.and_then(|id| rs.iter().find(|b| b.0 == id)) {
                cursor = if horizontal { rb.2 } else { rb.1 };
            }
            for i in order {
                if horizontal {
                    out[i].2 = cursor;
                    cursor += rs[i].4 + sp;
                } else {
                    out[i].1 = cursor;
                    cursor += rs[i].3 + sp;
                }
            }
        }
        out
    }

    #[allow(clippy::type_complexity)]
    pub fn distribute(
        rs: &[Rect],
        horizontal: bool,
        centers: bool,
        allow_overlap: bool,
    ) -> Result<(Vec<String>, Vec<(String, f32, f32)>, f32), String> {
        let key = |r: &Rect| match (horizontal, centers) {
            (true, true) => r.1 + r.3 / 2.,
            (true, false) => r.1,
            (false, true) => r.2 + r.4 / 2.,
            (false, false) => r.2,
        };
        let cross = |r: &Rect| if horizontal { r.2 } else { r.1 };
        let mut o: Vec<&Rect> = rs.iter().collect();
        o.sort_by(|a, b| (key(a), cross(a)).partial_cmp(&(key(b), cross(b))).unwrap_or(std::cmp::Ordering::Equal));
        let (first, last) = (o[0], o[o.len() - 1]);
        let n = o.len() as f32;
        let mut ps = Vec::new();
        let value;
        if !centers {
            let span = if horizontal { last.1 + last.3 - first.1 } else { last.2 + last.4 - first.2 };
            let sizes: f32 = o.iter().map(|r| if horizontal { r.3 } else { r.4 }).sum();
            let gap = (span - sizes) / (n - 1.);
            if gap < 0. && !allow_overlap {
                return Err(format!(
                    "按当前首尾排不下：block 共占 {sizes:.0}，首尾跨度 {span:.0}；允许重叠传 allowOverlap=true"
                ));
            }
            let mut cursor = if horizontal { first.1 } else { first.2 };
            for r in &o {
                if horizontal {
                    ps.push((r.0.clone(), cursor, r.2));
                    cursor += r.3 + gap;
                } else {
                    ps.push((r.0.clone(), r.1, cursor));
                    cursor += r.4 + gap;
                }
            }
            value = gap;
        } else {
            let (c0, c1) = (key(first), key(last));
            let step = (c1 - c0) / (n - 1.);
            for (i, r) in o.iter().enumerate() {
                let c = c0 + step * i as f32;
                if horizontal {
                    ps.push((r.0.clone(), c - r.3 / 2., r.2));
                } else {
                    ps.push((r.0.clone(), r.1, c - r.4 / 2.));
                }
            }
            value = step;
        }
        Ok((o.iter().map(|r| r.0.clone()).collect(), ps, value))
    }

    #[allow(clippy::type_complexity)]
    pub fn arrange(
        rs: &[Rect],
        layout: &str,
        columns: usize,
        gap_x: f32,
        gap_y: f32,
        origin: (Option<f32>, Option<f32>),
        by_position: bool,
    ) -> (Vec<String>, Vec<(String, f32, f32)>, usize, usize) {
        let mut o: Vec<&Rect> = rs.iter().collect();
        if by_position {
            let row = layout == "row";
            o.sort_by(|a, b| {
                let k = |r: &Rect| if row { (r.1, r.2) } else { (r.2, r.1) };
                k(a).partial_cmp(&k(b)).unwrap_or(std::cmp::Ordering::Equal)
            });
        }
        let n = o.len();
        let cols = match layout {
            "row" => n,
            "column" => 1,
            _ if columns > 0 => columns,
            _ => (n as f64).sqrt().ceil() as usize,
        }
        .clamp(1, n.max(1));
        let rows = n.div_ceil(cols);
        let mut col_w = vec![0f32; cols];
        let mut row_h = vec![0f32; rows];
        for (i, r) in o.iter().enumerate() {
            col_w[i % cols] = col_w[i % cols].max(r.3);
            row_h[i / cols] = row_h[i / cols].max(r.4);
        }
        let mut col_x = vec![0f32; cols];
        let mut row_y = vec![0f32; rows];
        for c in 1..cols {
            col_x[c] = col_x[c - 1] + col_w[c - 1] + gap_x;
        }
        for r in 1..rows {
            row_y[r] = row_y[r - 1] + row_h[r - 1] + gap_y;
        }
        let owned: Vec<Rect> = o.iter().map(|r| (*r).clone()).collect();
        let b = bounds(&owned);
        let (ox, oy) = (origin.0.unwrap_or(b.0), origin.1.unwrap_or(b.1));
        let ps = o
            .iter()
            .enumerate()
            .map(|(i, r)| (r.0.clone(), ox + col_x[i % cols], oy + row_y[i / cols]))
            .collect();
        (o.iter().map(|r| r.0.clone()).collect(), ps, cols, rows)
    }
}

#[cfg(test)]
mod tests {
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
        let mut c = Ctx { m, view: (1000., 800.), mcp: Value::Null, here, dirty: false, memos: vec![], reload: vec![] };
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
        let out = call(&mut m, "create_block", json!({"kind": "note", "title": "想法", "text": "# hi"})).unwrap();
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
    fn memo_text_goes_to_its_file() {
        let (d, mut m) = ctx_model();
        let out = call(&mut m, "create_block", json!({"kind": "memo", "title": "计划", "text": "- a"})).unwrap();
        assert_eq!(out["path"], "计划.md");
        assert_eq!(std::fs::read_to_string(d.path().join("计划.md")).unwrap(), "- a");
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
        let c = Ctx { m: &mut m, view: (0., 0.), mcp: Value::Null, here: Here::default(), dirty: false, memos: vec![], reload: vec![] };
        assert!(c.read_resource("boundlessdeck://schema").unwrap()["kinds"].as_array().unwrap().len() == 7);
        assert!(c.read_resource("other://x").is_err());
    }
}
