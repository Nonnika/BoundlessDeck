//! The block field table, the single source of what agents may read and
//! write, and the tool list and schemas made from it.

use serde_json::{Map, Value, json};

use super::SCHEMA_VERSION;
use crate::memo;
use crate::model::{self, BlockKind, MIN_BLOCK_H, MIN_BLOCK_W};

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
pub(super) const KIND_NAMES: &[&str] = &["quicklook", "note", "file", "terminal", "browser", "chart", "stack", "text", "shape"];
const BARE: &[BlockKind] = &[BlockKind::Text, BlockKind::Shape];
pub(super) const STYLE_KEYS: &[&str] = &["fill", "stroke", "color", "font", "size", "weight", "italic", "radius", "corners", "align", "valign"];
pub(super) const CHART_NAMES: &[&str] = &["bar", "hbar", "line", "area", "pie"];

pub const FIELDS: &[Field] = &[
    Field { name: "id", ty: "string", mutable: false, on_create: false, kinds: ALL, min: None, enum_: &[], derived: false,
        description: "block 的稳定 ID（b 加 6 位十六进制），创建后不可改；所有写操作都用它指名 block。" },
    Field { name: "kind", ty: "string", mutable: false, on_create: true, kinds: ALL, min: None, enum_: KIND_NAMES, derived: false,
        description: "block 类型：note（笔记：工作目录里的一个 Markdown 文件；旧名 memo 仍可用）、quicklook（QuickLook 便签，正文存在 deck 里，默认 1 小时后消失）、file（链接的本地文件）、terminal（终端）、browser（浏览器）、chart（图表：画它 text 里的表格）、stack（块堆：收纳其他 block 的钱包卡片，收进去的 block 不在画布上显示）、text（文字：画布上的一段字，按 Markdown 显示、每个换行都是一行，没有卡片，平时不显示标题）、shape（图形：矩形、椭圆、菱形、三角或箭头，里面可以写字）。只在创建时给出，之后不可改。" },
    Field { name: "title", ty: "string", mutable: true, on_create: true, kinds: ALL, min: None, enum_: &[], derived: false,
        description: "标题。写非空值会同时把它标记为用户命名（named=true）；写空串则回到 kind 或文件的默认标题。" },
    Field { name: "named", ty: "boolean", mutable: true, on_create: true, kinds: ALL, min: None, enum_: &[], derived: false,
        description: "标题是否由用户命名；false 时终端 shell 或网页可以自动改标题。" },
    Field { name: "x", ty: "number", mutable: true, on_create: true, kinds: ALL, min: None, enum_: &[], derived: false,
        description: "世界坐标 X（画布坐标，不是屏幕像素）。省略时新 block 以视野中心为中心。" },
    Field { name: "y", ty: "number", mutable: true, on_create: true, kinds: ALL, min: None, enum_: &[], derived: false,
        description: "世界坐标 Y。" },
    Field { name: "w", ty: "number", mutable: true, on_create: true, kinds: ALL, min: Some(MIN_BLOCK_W as f64), enum_: &[], derived: false,
        description: "世界坐标宽度，最小 140。" },
    Field { name: "h", ty: "number", mutable: true, on_create: true, kinds: ALL, min: Some(MIN_BLOCK_H as f64), enum_: &[], derived: false,
        description: "世界坐标高度，最小 90。" },
    Field { name: "path", ty: "string", mutable: true, on_create: true, kinds: &[BlockKind::File, BlockKind::Memo], min: None, enum_: &[], derived: false,
        description: "文件路径。工作区内的相对路径（相对 deck 根），工作区外用绝对路径；写绝对路径会尽量转成相对路径。改路径会重新加载预览。note 创建时可省略：按标题在 deck 根下新建 <标题>.md（重名加序号）；给出的 .md 不存在时会新建空文件。" },
    Field { name: "text", ty: "string", mutable: true, on_create: true, kinds: &[BlockKind::Note, BlockKind::Memo, BlockKind::Chart, BlockKind::Text, BlockKind::Shape], min: None, enum_: &[], derived: false,
        description: "正文（Markdown）。quicklook 存在 deck 里；note 直接写进它的文件（覆盖整个文件），读 note 的内容请读 absPath 指向的文件。text（文字）按 Markdown 显示，每个换行都是一行；shape 里的字原样显示。chart 的 text 是它画的表格：每行一项，第一列是名称，其后每列一组数值，用逗号或 Tab 分隔（也可用 Markdown 表格）；第一行含非数字时作为列名（图例）。最多 8 组、200 行；饼图只画第一组。" },
    Field { name: "chart", ty: "string", mutable: true, on_create: true, kinds: &[BlockKind::Chart], min: None, enum_: CHART_NAMES, derived: false,
        description: "图表类型：bar 柱状图、hbar 条形图、line 折线图、area 面积图、pie 饼图。省略为 bar。" },
    Field { name: "expires", ty: "integer", mutable: true, on_create: true, kinds: &[BlockKind::Note], min: Some(0.), enum_: &[], derived: false,
        description: "quicklook 自动消失的时刻，Unix 毫秒；0 为永久保留。新建 quicklook 不给时默认 1 小时后消失。到时它会被删除（用户可 ⌘Z 找回）；关闭期间到期的在打开 deck 时直接移除。只想说“多久后消失”时用 ttlMinutes。" },
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
    Field { name: "shape", ty: "string", mutable: true, on_create: true, kinds: &[BlockKind::Shape], min: None, enum_: model::SHAPES, derived: false,
        description: "图形的形状：rect（矩形，默认）、ellipse（椭圆）、diamond（菱形）、triangle（三角）、arrow（向右的箭头，字写在箭身里）。" },
    Field { name: "style", ty: "object", mutable: true, on_create: true, kinds: BARE, min: None, enum_: &[], derived: false,
        description: "文字与图形的样式，按键合并（只改给出的键，值为 null 恢复默认）：fill（填充）、stroke（边框）、color（文字颜色）——颜色可写调色板名 ink/gray/red/orange/yellow/green/teal/blue/purple/pink（随浅色/深色变化，作填充时是柔和底色）、#rrggbb 或 none；font（系统已安装的字族名，空为系统字体）；size（字号，6–400）；weight（100–900 的整百）；italic（布尔）；radius（圆角，≥0，只对矩形和文字有效）；corners（四个角各自的圆角 [左上, 右上, 右下, 左下]，≥0，设了就代替 radius）；align（left/center/right）；valign（top/middle/bottom）。文字默认无填充无边框、24 号、左上对齐；图形默认蓝色柔和填充与蓝边、16 号、居中。" },
];

pub(super) fn field(name: &str) -> Option<&'static Field> {
    FIELDS.iter().find(|f| f.name == name)
}

pub(super) fn for_kind(f: &Field, k: BlockKind) -> bool {
    f.kinds.is_empty() || f.kinds.contains(&k)
}

pub(super) fn mutable_names(k: BlockKind) -> Vec<&'static str> {
    FIELDS.iter().filter(|f| f.mutable && for_kind(f, k)).map(|f| f.name).collect()
}

pub(super) fn read_only_names(k: BlockKind) -> Vec<&'static str> {
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
        BlockKind::Text => ("文字", 280., 90.),
        BlockKind::Shape => ("图形", 220., 160.),
    }
}

fn field_json(f: &Field) -> Value {
    let mut o = json!({"name": f.name, "type": f.ty, "mutable": f.mutable, "description": f.description});
    let m = o.as_object_mut().unwrap();
    if f.on_create {
        m.insert("onCreate".into(), true.into());
    }
    if !f.kinds.is_empty() {
        m.insert("kinds".into(), f.kinds.iter().map(|k| k.mcp_name()).collect::<Vec<_>>().into());
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
                "kind": k.mcp_name(),
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
        desc += &format!("（仅 {}）", f.kinds.iter().map(|k| k.mcp_name()).collect::<Vec<_>>().join("、"));
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
        ("screenshot", "截取画面", "截取窗口里画布现在的样子（相机看到的内容），作为 PNG 图片返回。想确认自己的改动在画布上看起来如何时用；图片随 MCP 图像内容返回，文字部分带相机、视野中心与像素尺寸。", object_schema(json!({}), &[])),
        ("create_block", "新建 block", "在画布上建一个 block 并写初始字段；kind 必填。file 需要 path。省略 x/y 时以视野中心为中心（可能盖住已有的 block）。", object_schema(Value::Object(create), &["kind"])),
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
        ("set_view", "移动视野", "把窗口的视野移到画布上的某处，默认平滑滑过去（远距离时会先拉远再推近）。给 id 就以那个 block 为中心，再加 fit=true 会缩放到它刚好占满视野的八成；否则用 x/y 指定视野中心的世界坐标（省略的那一维保持不动）。scale 改缩放（0.08–4，会取最近的缩放档位）。不进撤销栈。给人看你新建或改动的 block 时用。",
            object_schema(json!({
                "id": {"type": "string", "description": "以这个 block 为视野中心"},
                "fit": {"type": "boolean", "description": "与 id 一起用：缩放到这个 block 刚好占满视野的八成"},
                "x": {"type": "number", "description": "视野中心的世界坐标 X"},
                "y": {"type": "number", "description": "视野中心的世界坐标 Y"},
                "scale": {"type": "number", "minimum": model::MIN_SCALE, "maximum": model::MAX_SCALE, "description": "缩放，1 为 100%；省略保持当前"},
                "animate": {"type": "boolean", "description": "平滑滑过去（默认 true）；false 立即跳过去"},
            }), &[])),
        ("process", "报告任务进度", "告诉人你在做什么、做到哪了：窗口底部中间会显示一个胶囊。每项任务都必须用它报告：开始时 status=preparing（title 写任务名，如「整理调研笔记」），动手时 status=running（detail 写当前这一步，能估算时带 progress 0–1，可多次调用更新），结束时 status=done（胶囊显示「已完成」后自己消失）；中途放弃也要报 done 并在 detail 里说明。一个终端里的 agent 同时只有一项任务，新的报告接替旧的；没给的字段沿用上次。不改画布，不进撤销栈。",
            object_schema(json!({
                "status": {"type": "string", "enum": crate::mcp::tools::Stage::NAMES, "description": "preparing（准备中）、running（进行中）或 done（已完成）"},
                "title": {"type": "string", "description": "任务名，简短（80 字以内）；省略沿用上次"},
                "detail": {"type": "string", "description": "当前这一步在做什么；省略沿用上次"},
                "progress": {"type": "number", "minimum": 0, "maximum": 1, "description": "完成了多少，0–1；不确定就省略"},
            }), &["status"])),
        ("undo", "撤销", "撤销上一步画布操作（加、删 block），与窗口里的 ⌘Z 同一条撤销栈。", object_schema(json!({}), &[])),
        ("redo", "重做", "重做上一步撤销。", object_schema(json!({}), &[])),
        ("save_deck", "保存画布", "立刻把画布写进工作区的 .boundlessdeck/deck.json；写操作本来就会自动保存，这个用于确认。", object_schema(json!({}), &[])),
    ]
}

pub fn tool_names() -> Vec<&'static str> {
    tool_list().into_iter().map(|t| t.0).collect()
}
