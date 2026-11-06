package main

import "boundlessdeck/internal/chart"

// The block schema is what the MCP exposes of a block: every field a client
// may write, of which kinds, and everything it may only read. It is the one
// place that knows what a block is made of; the create/update tools, the
// JSON schemas they advertise and .boundlessdeck/block-schema.json all come
// from this table. TestMCPSchemaCoversEveryBlockField checks it against the
// block struct, so a field added there fails the build until it is
// described here.

// mcpSchemaVersion is the version of this schema: bumped when a field's
// name, type or meaning changes, so that a client that cached the schema
// knows to read it again.
const mcpSchemaVersion = 4

// mcpField is one field of a block as the MCP sees it. Mutable fields are
// writable through update_block (and create_block when OnCreate); the rest
// are read-only.
type mcpField struct {
	Name     string      `json:"name"`
	Type     string      `json:"type"` // string, number, integer, boolean, object
	Mutable  bool        `json:"mutable"`
	OnCreate bool        `json:"onCreate,omitempty"`
	Kinds    []blockKind `json:"kinds,omitempty"` // empty: every kind
	Min      *float64    `json:"min,omitempty"`
	Enum     []string    `json:"enum,omitempty"`
	// Derived fields are not kept in a block: they read and write another
	// field in other terms (ttlMinutes is expires counted from now).
	Derived     bool   `json:"derived,omitempty"`
	Description string `json:"description"`
}

func mcpNum(v float64) *float64 { return &v }

// mcpFields describes every field of a block in a stable order: the order
// patches apply in, and the order block_schema lists them in.
var mcpFields = []mcpField{
	{
		Name: "id", Type: "string",
		Description: "block 的稳定 ID（b 加 6 位十六进制），创建后不可改；所有写操作都用它指名 block。",
	},
	{
		Name: "kind", Type: "string", OnCreate: true,
		Enum:        kindNames(),
		Description: "block 类型：note（QuickLook 便签，正文存在 deck 里）、memo（笔记：工作目录里的一个 Markdown 文件）、file（链接的本地文件）、terminal（终端）、browser（浏览器）、chart（图表：画它 text 里的表格）、stack（块堆：收纳其他 block 的钱包卡片，收进去的 block 不在画布上显示）。只在创建时给出，之后不可改。",
	},
	{
		Name: "title", Type: "string", Mutable: true, OnCreate: true,
		Description: "标题。写非空值会同时把它标记为用户命名（named=true）；写空串则回到 kind 或文件的默认标题。",
	},
	{
		Name: "named", Type: "boolean", Mutable: true, OnCreate: true,
		Description: "标题是否由用户命名；false 时终端 shell 或网页可以自动改标题。",
	},
	{
		Name: "x", Type: "number", Mutable: true, OnCreate: true,
		Description: "世界坐标 X（画布坐标，不是屏幕像素）。省略时新 block 落在视口中央附近的空位。",
	},
	{
		Name: "y", Type: "number", Mutable: true, OnCreate: true,
		Description: "世界坐标 Y。",
	},
	{
		Name: "w", Type: "number", Mutable: true, OnCreate: true, Min: mcpNum(minBlockW),
		Description: "世界坐标宽度，最小 140。",
	},
	{
		Name: "h", Type: "number", Mutable: true, OnCreate: true, Min: mcpNum(minBlockH),
		Description: "世界坐标高度，最小 90。",
	},
	{
		Name: "path", Type: "string", Mutable: true, OnCreate: true,
		Kinds:       []blockKind{kindFile, kindMemo},
		Description: "文件路径。工作区内的相对路径（相对 deck 根），工作区外用绝对路径；写绝对路径会尽量转成相对路径。改路径会重新加载预览。memo 创建时可省略：按标题在 deck 根下新建 <标题>.md（重名加序号）；给出的 .md 不存在时会新建空文件。",
	},
	{
		Name: "text", Type: "string", Mutable: true, OnCreate: true,
		Kinds:       []blockKind{kindNote, kindMemo, kindChart},
		Description: "正文（Markdown）。note 存在 deck 里；memo 直接写进它的文件（覆盖整个文件），读 memo 的内容请读 absPath 指向的文件。chart 的 text 是它画的表格：每行一项，第一列是名称，其后每列一组数值，用逗号或 Tab 分隔（也可用 Markdown 表格）；第一行含非数字时作为列名（图例）。最多 8 组、200 行；饼图只画第一组。",
	},
	{
		Name: "chart", Type: "string", Mutable: true, OnCreate: true,
		Kinds:       []blockKind{kindChart},
		Enum:        chart.TypeNames(),
		Description: "图表类型：bar 柱状图、hbar 条形图、line 折线图、area 面积图、pie 饼图。省略为 bar。",
	},
	{
		Name: "expires", Type: "integer", Mutable: true, OnCreate: true, Min: mcpNum(0),
		Kinds:       []blockKind{kindNote},
		Description: "QuickLook 自动消失的时刻，Unix 毫秒；0 为永久保留。新建 note 不给时默认 1 小时后消失。到时它会被删除（用户可 ⌘Z 找回）；关闭期间到期的在打开 deck 时直接移除。只想说“多久后消失”时用 ttlMinutes。",
	},
	{
		Name: "ttlMinutes", Type: "number", Mutable: true, OnCreate: true, Min: mcpNum(0), Derived: true,
		Kinds:       []blockKind{kindNote},
		Description: "从现在起多少分钟后消失（可带小数），写入时换算成 expires；0 为永久保留。读出的是剩余分钟数（保留一位小数，永久的为 0）。不能与 expires 同时给。",
	},
	{
		Name: "page", Type: "integer", Mutable: true, OnCreate: true, Min: mcpNum(0),
		Kinds:       []blockKind{kindFile},
		Description: "PDF 显示的页，从 0 开始；对非 PDF 文件无意义，越界会由预览夹到有效范围。",
	},
	{
		Name: "url", Type: "string", Mutable: true, OnCreate: true,
		Kinds:       []blockKind{kindBrowser},
		Description: "浏览器显示的地址。http/https/file 原样使用；裸域名或搜索词会补上 https://；javascript: 与 data: 会被拒绝。空串清空页面。",
	},
	{
		Name: "term", Type: "object",
		Kinds:       []blockKind{kindTerminal},
		Description: "终端关掉 deck 时留下的恢复状态（历史字节数、工作目录、保存时间），只读；终端本身在运行时起。",
	},
	{
		Name: "items", Type: "array", Mutable: true, OnCreate: true,
		Kinds:       []blockKind{kindStack},
		Description: "块堆里的 block ID，按从上到下的卡片顺序。收进堆的 block 仍留在 deck 里、可用 get_block 读写，但不再画在画布上；一个 block 只属于一个堆，块堆不能装块堆。",
	},
}

// mcpFieldNames is the set of every field mcpFields knows, for rejecting
// typos.
func mcpFieldNames() map[string]bool {
	out := make(map[string]bool, len(mcpFields))
	for _, f := range mcpFields {
		out[f.Name] = true
	}
	return out
}

func mcpKnownField(name string) bool { return mcpFieldNames()[name] }

// mcpFieldForKind says whether the field belongs to the kind at all.
func mcpFieldForKind(f mcpField, kind blockKind) bool {
	if len(f.Kinds) == 0 {
		return true
	}
	for _, k := range f.Kinds {
		if k == kind {
			return true
		}
	}
	return false
}

// mcpMutableNames lists the fields a client may write on a block of the
// kind, in table order.
func mcpMutableNames(kind blockKind) []string {
	out := []string{}
	for _, f := range mcpFields {
		if f.Mutable && mcpFieldForKind(f, kind) {
			out = append(out, f.Name)
		}
	}
	return out
}

// mcpFieldMutable says whether a patch may set the field on a block of
// the kind.
func mcpFieldMutable(kind blockKind, name string) bool {
	for _, f := range mcpFields {
		if f.Name == name {
			return f.Mutable && mcpFieldForKind(f, kind)
		}
	}
	return false
}

// mcpReadOnlyNames lists what a block of the kind has that a client only
// reads.
func mcpReadOnlyNames(kind blockKind) []string {
	out := []string{}
	for _, f := range mcpFields {
		if !f.Mutable && mcpFieldForKind(f, kind) {
			out = append(out, f.Name)
		}
	}
	return out
}

// mcpKindDefaults is how a block of each kind starts: its title, and the
// size the toolbar gives it.
var mcpKindDefaults = map[blockKind]struct {
	Title string
	W, H  float64
}{
	kindNote:     {"QuickLook", 360, 260},
	kindMemo:     {memoTitle, 440, 520},
	kindFile:     {"文件", 560, 420},
	kindTerminal: {"终端", 560, 340},
	kindBrowser:  {"浏览器", 760, 540},
	kindStack:    {"块堆", 380, 300},
	kindChart:    {"图表", 480, 320},
}

// mcpBlockSchema is the per-kind view of the table: what a client may
// create, what it may write afterwards, and what it only reads.
func mcpBlockSchema() []map[string]any {
	kinds := blockKinds
	out := make([]map[string]any, 0, len(kinds))
	for _, kind := range kinds {
		create := []string{}
		fields := []mcpField{}
		for _, f := range mcpFields {
			if !mcpFieldForKind(f, kind) {
				continue
			}
			fields = append(fields, f)
			if f.OnCreate {
				create = append(create, f.Name)
			}
		}
		def := mcpKindDefaults[kind]
		out = append(out, map[string]any{
			"kind":     string(kind),
			"defaults": map[string]any{"title": def.Title, "w": def.W, "h": def.H},
			"create":   create,
			"mutable":  mcpMutableNames(kind),
			"readOnly": mcpReadOnlyNames(kind),
			"fields":   fields,
		})
	}
	return out
}

// mcpSchemaDigest is the whole schema as one JSON-able value: what
// block_schema returns and block-schema.json holds.
func mcpSchemaDigest() map[string]any {
	return map[string]any{
		"schemaVersion":  mcpSchemaVersion,
		"blockVersion":   1,
		"coordinateUnit": "画布世界坐标；x/y 是左上角，w/h 是尺寸，屏幕位置 = (x - camX) * camS",
		"kinds":          mcpBlockSchema(),
	}
}

// mcpFieldProp is a field as a JSON Schema property, telling which kinds
// it applies to in its description.
func mcpFieldProp(f mcpField) map[string]any {
	desc := f.Description
	if len(f.Kinds) > 0 {
		names := make([]string, len(f.Kinds))
		for i, k := range f.Kinds {
			names[i] = string(k)
		}
		desc += "（仅 " + joinWords(names, "、") + "）"
	}
	p := map[string]any{"type": f.Type, "description": desc}
	if f.Min != nil {
		p["minimum"] = *f.Min
	}
	if len(f.Enum) > 0 {
		enum := make([]any, len(f.Enum))
		for i, v := range f.Enum {
			enum[i] = v
		}
		p["enum"] = enum
	}
	return p
}

// mcpMutableProps is the properties object of update_block: every mutable
// field of any kind. The tool checks each against the block's own kind.
func mcpMutableProps() map[string]any {
	props := map[string]any{}
	for _, f := range mcpFields {
		if f.Mutable {
			props[f.Name] = mcpFieldProp(f)
		}
	}
	return props
}

// mcpCreateProps is the properties object of create_block.
func mcpCreateProps() map[string]any {
	props := map[string]any{}
	for _, f := range mcpFields {
		if f.OnCreate {
			props[f.Name] = mcpFieldProp(f)
		}
	}
	return props
}

// joinWords joins words with sep, as the schema's descriptions do.
func joinWords(words []string, sep string) string {
	out := ""
	for i, w := range words {
		if i > 0 {
			out += sep
		}
		out += w
	}
	return out
}
