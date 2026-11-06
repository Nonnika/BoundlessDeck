package main

import (
	"cmp"
	"encoding/json"
	"fmt"
	"math"
	"os"
	"path/filepath"
	"strings"
	"time"
)

// mcpArgs is the arguments object of a tool call, kept raw so that a field
// of the wrong type is an error with the field's name in it, not a zero
// value silently written.
type mcpArgs map[string]json.RawMessage

func (a mcpArgs) has(name string) bool {
	_, ok := a[name]
	return ok
}

func (a mcpArgs) without(names ...string) mcpArgs {
	out := make(mcpArgs, len(a))
	for k, v := range a {
		out[k] = v
	}
	for _, name := range names {
		delete(out, name)
	}
	return out
}

func rawString(raw json.RawMessage, name string) (string, error) {
	var s string
	if err := json.Unmarshal(raw, &s); err != nil {
		return "", fmt.Errorf("%s 要是字符串：%w", name, err)
	}
	return s, nil
}

func rawFloat(raw json.RawMessage, name string) (float64, error) {
	var f float64
	if err := json.Unmarshal(raw, &f); err != nil {
		return 0, fmt.Errorf("%s 要是数字：%w", name, err)
	}
	if math.IsNaN(f) || math.IsInf(f, 0) {
		return 0, fmt.Errorf("%s 要是有限的数字", name)
	}
	return f, nil
}

func rawBool(raw json.RawMessage, name string) (bool, error) {
	var b bool
	if err := json.Unmarshal(raw, &b); err != nil {
		return false, fmt.Errorf("%s 要是布尔值：%w", name, err)
	}
	return b, nil
}

func rawInt(raw json.RawMessage, name string) (int, error) {
	var n int
	if err := json.Unmarshal(raw, &n); err != nil {
		return 0, fmt.Errorf("%s 要是整数：%w", name, err)
	}
	return n, nil
}

func (a mcpArgs) str(name string) (string, bool, error) {
	raw, ok := a[name]
	if !ok {
		return "", false, nil
	}
	s, err := rawString(raw, name)
	return s, true, err
}

func (a mcpArgs) num(name string) (float64, bool, error) {
	raw, ok := a[name]
	if !ok {
		return 0, false, nil
	}
	f, err := rawFloat(raw, name)
	return f, true, err
}

func (a mcpArgs) boolean(name string) (bool, bool, error) {
	raw, ok := a[name]
	if !ok {
		return false, false, nil
	}
	b, err := rawBool(raw, name)
	return b, true, err
}

func (a mcpArgs) integer(name string) (int, bool, error) {
	raw, ok := a[name]
	if !ok {
		return 0, false, nil
	}
	n, err := rawInt(raw, name)
	return n, true, err
}

func (a mcpArgs) strings(name string) ([]string, error) {
	raw, ok := a[name]
	if !ok {
		return nil, nil
	}
	var out []string
	if err := json.Unmarshal(raw, &out); err != nil {
		return nil, fmt.Errorf("%s 要是字符串数组：%w", name, err)
	}
	return out, nil
}

// mcpFieldsJSON is the JSON shape of a block as every tool returns it: its
// current values, the fields its kind lets a client write, and whether it
// is selected.
func (d *deck) mcpFieldsJSON(n *block) map[string]any {
	out := map[string]any{
		"id":       n.ID,
		"kind":     string(n.Kind),
		"title":    n.Title,
		"named":    n.Named,
		"x":        roundCoord(n.X),
		"y":        roundCoord(n.Y),
		"w":        roundCoord(n.W),
		"h":        roundCoord(n.H),
		"selected": d.sel[n.ID],
		"editable": mcpMutableNames(n.Kind),
		"readOnly": mcpReadOnlyNames(n.Kind),
	}
	switch n.Kind {
	case kindNote:
		out["text"] = n.Text
		out["expires"] = n.Expires
		out["ttlMinutes"] = 0.0
		if at, ok := expiresAt(n); ok {
			out["expiresAt"] = at.Format(time.RFC3339)
			// Rounded up, so a block with any time left never reads as kept.
			left := at.Sub(d.now()).Minutes()
			out["ttlMinutes"] = max(math.Ceil(left*10)/10, 0.1)
		}
	case kindMemo:
		out["path"] = n.Path
		if n.Path != "" && !filepath.IsAbs(n.Path) {
			out["absPath"] = d.path(n)
		}
	case kindFile:
		out["path"] = n.Path
		out["page"] = n.Page
		out["preview"] = previewOf(n.Path).String()
		if n.Path != "" && !filepath.IsAbs(n.Path) {
			out["absPath"] = d.path(n)
		}
	case kindBrowser:
		out["url"] = n.URL
	case kindTerminal:
		out["term"] = mcpTermJSON(n.Term)
	case kindStack:
		out["items"] = d.stackIDs(n)
	}
	return out
}

func mcpTermJSON(st *termState) any {
	if st == nil {
		return nil
	}
	return map[string]any{
		"historyBytes": len(st.History),
		"dir":          st.Dir,
		"savedAt":      st.SavedAt,
	}
}

func roundCoord(v float32) float64 {
	return math.Round(float64(v)*100) / 100
}

// mcpValidatePatch checks a patch against a kind without touching any
// block, so a batch is checked whole before its first write.
func (d *deck) mcpValidatePatch(kind blockKind, patch mcpArgs) error {
	for name := range patch {
		if !mcpKnownField(name) {
			return fmt.Errorf("未知字段 %q；用 block_schema 看每个 kind 的可写字段", name)
		}
		if !mcpFieldMutable(kind, name) {
			return fmt.Errorf("%s block 不能写字段 %q（可写：%s）",
				kind, name, strings.Join(mcpMutableNames(kind), ", "))
		}
	}
	if patch.has("ttlMinutes") && patch.has("expires") {
		return fmt.Errorf("expires 与 ttlMinutes 只能给一个")
	}
	probe := &block{Kind: kind}
	for _, f := range mcpFields {
		raw, ok := patch[f.Name]
		if !ok {
			continue
		}
		if err := d.mcpApplyField(probe, f.Name, raw, true); err != nil {
			return fmt.Errorf("%s：%w", f.Name, err)
		}
	}
	return nil
}

// mcpPatchBlock writes patch fields onto n in the table's order: an
// explicit title lands before the path that would otherwise rename it, and
// a size before the PDF page it re-renders. It returns the fields it wrote.
func (d *deck) mcpPatchBlock(n *block, patch mcpArgs) ([]string, error) {
	if len(patch) == 0 {
		return nil, nil
	}
	// Check everything first: a patch is all or nothing, as an agent that
	// mistypes one field would otherwise leave half a write behind.
	if err := d.mcpValidatePatch(n.Kind, patch); err != nil {
		return nil, err
	}
	applied := make([]string, 0, len(patch))
	for _, f := range mcpFields {
		raw, ok := patch[f.Name]
		if !ok {
			continue
		}
		if err := d.mcpApplyField(n, f.Name, raw, false); err != nil {
			return applied, fmt.Errorf("%s：%w", f.Name, err)
		}
		applied = append(applied, f.Name)
	}
	return applied, nil
}

// mcpApplyField sets one already-validated field. dry checks it on a probe
// block, touching no file: a 笔记's fields write its file.
func (d *deck) mcpApplyField(n *block, name string, raw json.RawMessage, dry bool) error {
	switch name {
	case "title":
		s, err := rawString(raw, name)
		if err != nil {
			return err
		}
		s = strings.TrimSpace(s)
		if n.Kind == kindMemo {
			// A note's title is its file's name.
			n.Title = cmp.Or(s, memoTitle)
			if n.Path == "" || dry {
				return nil
			}
			next, err := renameMemoFile(d.root, n.Path, n.Title)
			n.Path, n.Title = next, memoTitleOf(next)
			return err
		}
		if s == "" {
			n.Title, n.Named = defaultTitle(n), false
			return nil
		}
		n.Title, n.Named = s, true
	case "named":
		b, err := rawBool(raw, name)
		if err != nil {
			return err
		}
		n.Named = b
		if !b && strings.TrimSpace(n.Title) == "" {
			n.Title = defaultTitle(n)
		}
	case "x", "y":
		f, err := rawFloat(raw, name)
		if err != nil {
			return err
		}
		if name == "x" {
			n.X = float32(f)
		} else {
			n.Y = float32(f)
		}
	case "w", "h":
		f, err := rawFloat(raw, name)
		if err != nil {
			return err
		}
		if name == "w" {
			n.W = max(float32(f), minBlockW)
		} else {
			n.H = max(float32(f), minBlockH)
		}
	case "expires":
		ms, err := rawInt(raw, name)
		if err != nil {
			return err
		}
		if ms < 0 {
			return fmt.Errorf("expires 不能为负；0 为永久保留")
		}
		n.Expires = int64(ms)
	case "ttlMinutes":
		m, err := rawFloat(raw, name)
		if err != nil {
			return err
		}
		if m < 0 {
			return fmt.Errorf("ttlMinutes 不能为负；0 为永久保留")
		}
		n.Expires = 0
		if m > 0 {
			n.Expires = d.now().Add(time.Duration(m * float64(time.Minute))).UnixMilli()
		}
	case "text":
		s, err := rawString(raw, name)
		if err != nil {
			return err
		}
		if n.Kind == kindMemo {
			if dry {
				return nil
			}
			return d.mcpWriteMemo(n, s)
		}
		n.Text = s
	case "path":
		s, err := rawString(raw, name)
		if err != nil {
			return err
		}
		s = strings.TrimSpace(s)
		if s == "" {
			return fmt.Errorf("path 不能为空")
		}
		if filepath.IsAbs(s) {
			s = d.linkPath(filepath.Clean(s))
		} else {
			s = filepath.Clean(s)
		}
		if n.Kind == kindMemo {
			if err := mcpMemoPath(s); err != nil {
				return err
			}
			if !dry {
				d.flushMemo(n) // what was typed goes to the file it was typed for
				if err := mcpTouchMemo(d.path(&block{Path: s})); err != nil {
					return err
				}
			}
		}
		n.Path = s
		if !n.Named || n.Kind == kindMemo {
			n.Title = defaultTitle(n)
		}
		d.mcpResetPreview(n)
	case "page":
		page, err := rawInt(raw, name)
		if err != nil {
			return err
		}
		if page < 0 {
			return fmt.Errorf("page 从 0 开始，收到 %d", page)
		}
		n.Page = page
	case "url":
		s, err := rawString(raw, name)
		if err != nil {
			return err
		}
		url, err := mcpNormalizeURL(s)
		if err != nil {
			return err
		}
		n.URL = url
		d.mcpResetBrowser(n)
	case "items":
		ids, err := mcpArgs{name: raw}.strings(name)
		if err != nil {
			return err
		}
		if err := d.validateStackItems(n.ID, ids); err != nil {
			return err
		}
		if dry {
			return nil
		}
		d.setStackItems(n, ids)
	default:
		return fmt.Errorf("字段 %q 不能写", name)
	}
	return nil
}

// mcpNormalizeURL turns what an agent typed into an address: a bare host
// or a search phrase gets https://, as the address bar does; scripts and
// data URLs are refused.
func mcpNormalizeURL(s string) (string, error) {
	s = strings.TrimSpace(s)
	if s == "" {
		return "", nil
	}
	lower := strings.ToLower(s)
	for _, bad := range []string{"javascript:", "data:", "vbscript:"} {
		if strings.HasPrefix(lower, bad) {
			return "", fmt.Errorf("不接受 %s 开头的地址", strings.TrimSuffix(bad, ":"))
		}
	}
	if strings.Contains(s, "://") || strings.HasPrefix(lower, "about:") {
		return s, nil
	}
	return "https://" + s, nil
}

// mcpResetPreview drops what a file block loaded, so the new path is read
// as the body next builds.
func (d *deck) mcpResetPreview(n *block) {
	if n.rt == nil {
		return
	}
	n.rt.closePDF()
	n.rt.closeBrowser()
	n.rt = nil
}

// mcpResetBrowser closes the page a browser block showed; the address bar
// shows the new address and the page starts again.
func (d *deck) mcpResetBrowser(n *block) {
	rt := d.ensureRuntime(n)
	rt.closeBrowser()
	rt.webFailed = ""
	rt.address = n.URL
}

// mcpNewBlock is a block of the kind as the toolbar makes one.
func mcpNewBlock(kind blockKind) *block {
	def := mcpKindDefaults[kind]
	return &block{
		Kind:  kind,
		Title: def.Title,
		W:     float32(def.W),
		H:     float32(def.H),
	}
}

// mcpPlacement is where a block created without x/y lands: the middle of
// the view when there is one, or the world's origin, stepping aside from
// blocks already there.
func (d *deck) mcpPlacement(b *block) (float32, float32) {
	cx, cy := float32(0), float32(0)
	if d.viewW > 0 && d.viewH > 0 {
		cx, cy = d.cam.toWorld(d.viewW/2, d.viewH/2)
	}
	return d.freeSpot(cx-b.W/2, cy-b.H/2, b.W, b.H)
}

// mcpCreateBlock makes a block of args["kind"] and writes the rest of the
// arguments onto it, as update_block would.
func (d *deck) mcpCreateBlock(args mcpArgs) (map[string]any, error) {
	kindText, ok := args["kind"]
	if !ok {
		return nil, fmt.Errorf("create_block 需要 kind：%s", strings.Join(kindNames(), "、"))
	}
	kindString, err := rawString(kindText, "kind")
	if err != nil {
		return nil, err
	}
	kind := blockKind(kindString)
	if _, known := mcpKindDefaults[kind]; !known {
		return nil, fmt.Errorf("没有 kind %q：可用 %s", kindString, strings.Join(kindNames(), "、"))
	}
	patch := args.without("kind")
	if kind == kindFile && !patch.has("path") {
		return nil, fmt.Errorf("创建 file block 需要 path")
	}
	b := mcpNewBlock(kind)
	applied, err := d.mcpPatchBlock(b, patch)
	if err != nil {
		return nil, err
	}
	if kind == kindNote && !patch.has("expires") && !patch.has("ttlMinutes") {
		b.Expires = d.now().Add(noteLife).UnixMilli()
	}
	if kind == kindMemo {
		// Its file, unless a path or a text already made it.
		if err := d.mcpEnsureMemoFile(b); err != nil {
			return nil, err
		}
	}
	if !patch.has("x") || !patch.has("y") {
		x, y := d.mcpPlacement(b)
		if !patch.has("x") {
			b.X = x
		}
		if !patch.has("y") {
			b.Y = y
		}
	}
	d.addBlock(b)
	d.requestSave()
	out := d.mcpFieldsJSON(b)
	out["applied"] = applied
	return out, nil
}

// mcpUpdateBlock writes a patch onto the block id names.
func (d *deck) mcpUpdateBlock(id string, args mcpArgs) (map[string]any, error) {
	n := d.blockByID(id)
	if n == nil {
		return nil, fmt.Errorf("没有 id 为 %q 的 block", id)
	}
	patch := args.without("id", "fields")
	if raw, ok := args["fields"]; ok {
		var fields mcpArgs
		if err := json.Unmarshal(raw, &fields); err != nil {
			return nil, fmt.Errorf("fields 要是对象：%w", err)
		}
		for k, v := range fields {
			if _, top := patch[k]; !top {
				patch[k] = v
			}
		}
	}
	applied, err := d.mcpPatchBlock(n, patch)
	if err != nil {
		return nil, err
	}
	if len(applied) > 0 {
		d.requestSave()
	}
	out := d.mcpFieldsJSON(n)
	out["applied"] = applied
	return out, nil
}

// mcpDeleteBlocks removes blocks, all of them or none: an unknown id is an
// error before any block goes, so a typo cannot delete half a selection.
func (d *deck) mcpDeleteBlocks(ids []string) (map[string]any, error) {
	if len(ids) == 0 {
		return nil, fmt.Errorf("delete_blocks 需要 ids")
	}
	var missing []string
	seen := map[string]bool{}
	for _, id := range ids {
		if seen[id] {
			continue
		}
		seen[id] = true
		if d.blockByID(id) == nil {
			missing = append(missing, id)
		}
	}
	if len(missing) > 0 {
		return nil, fmt.Errorf("没有这些 block：%s", strings.Join(missing, ", "))
	}
	deleted := make([]string, 0, len(seen))
	for _, id := range ids {
		n := d.blockByID(id)
		if n == nil {
			continue
		}
		deleted = append(deleted, id)
		d.removeBlock(n)
	}
	d.requestSave()
	return map[string]any{"deleted": deleted, "blocks": len(d.blocks)}, nil
}

// mcpMoveBlocks moves blocks by dx, dy in world units.
func (d *deck) mcpMoveBlocks(ids []string, dx, dy float64) (map[string]any, error) {
	if len(ids) == 0 {
		return nil, fmt.Errorf("move_blocks 需要 ids")
	}
	var missing []string
	for _, id := range ids {
		if d.blockByID(id) == nil {
			missing = append(missing, id)
		}
	}
	if len(missing) > 0 {
		return nil, fmt.Errorf("没有这些 block：%s", strings.Join(missing, ", "))
	}
	ps := []placement{}
	seen := map[string]bool{}
	for _, id := range ids {
		if seen[id] {
			continue
		}
		seen[id] = true
		n := d.blockByID(id)
		ps = append(ps, placement{block: n, x: n.X + float32(dx), y: n.Y + float32(dy)})
	}
	moved := d.applyPlacements(ps)
	out := make([]map[string]any, len(ps))
	for i, p := range ps {
		out[i] = map[string]any{"id": p.block.ID, "x": roundCoord(p.block.X), "y": roundCoord(p.block.Y)}
	}
	return map[string]any{"moved": out, "count": moved}, nil
}

// mcpSelectBlocks sets the selection, as clicking and shift-clicking do:
// replace, add or toggle.
func (d *deck) mcpSelectBlocks(ids []string, mode string) (map[string]any, error) {
	switch mode {
	case "", "replace", "set", "add", "toggle":
	default:
		return nil, fmt.Errorf("mode 要是 replace、add 或 toggle，收到 %q", mode)
	}
	var missing []string
	for _, id := range ids {
		if d.blockByID(id) == nil {
			missing = append(missing, id)
		}
	}
	if len(missing) > 0 {
		return nil, fmt.Errorf("没有这些 block：%s", strings.Join(missing, ", "))
	}
	switch mode {
	case "", "replace", "set":
		d.sel = map[string]bool{}
	}
	for _, id := range ids {
		switch mode {
		case "toggle":
			d.sel[id] = !d.sel[id]
		default:
			d.sel[id] = true
		}
	}
	return map[string]any{"selected": d.mcpSelectedIDs()}, nil
}

// mcpSelectedIDs lists the selected blocks in deck order.
func (d *deck) mcpSelectedIDs() []string {
	out := []string{}
	for _, n := range d.blocks {
		if d.sel[n.ID] {
			out = append(out, n.ID)
		}
	}
	return out
}

// mcpSelectedBlocks returns the selected blocks, whole.
func (d *deck) mcpSelectedBlocks() []map[string]any {
	out := []map[string]any{}
	for _, n := range d.blocks {
		if d.sel[n.ID] {
			out = append(out, d.mcpFieldsJSON(n))
		}
	}
	return out
}

// mcpListBlocks lists blocks, optionally only those of a kind.
func (d *deck) mcpListBlocks(kind blockKind) []map[string]any {
	out := []map[string]any{}
	for _, n := range d.blocks {
		if kind != "" && n.Kind != kind {
			continue
		}
		out = append(out, d.mcpFieldsJSON(n))
	}
	return out
}

// mcpDeckInfo says what workspace this is and what drives it.
func (d *deck) mcpDeckInfo() map[string]any {
	info := map[string]any{
		"workspace": d.root,
		"deckFile":  d.deckFile(),
		"blocks":    len(d.blocks),
		"selected":  d.mcpSelectedIDs(),
		"camera": map[string]any{
			"x": roundCoord(d.cam.X), "y": roundCoord(d.cam.Y), "s": roundCoord(d.cam.S),
		},
		"view": map[string]any{
			"w": roundCoord(d.viewW), "h": roundCoord(d.viewH),
		},
		"kinds":         kindNames(),
		"schemaVersion": mcpSchemaVersion,
	}
	mcp := map[string]any{"running": d.mcp != nil}
	if d.mcp != nil {
		mcp["socket"] = d.mcp.sock
		mcp["startedAt"] = d.mcp.started
		mcp["tools"] = len(mcpToolList)
	}
	if d.mcpProject != nil {
		mcp["projectServers"] = d.mcpProject.Servers
		mcp["configFile"] = filepath.Join(deckDir, "mcp.json")
		mcp["warnings"] = d.mcpProject.Warnings
	}
	if d.mcpErr != nil {
		mcp["error"] = d.mcpErr.Error()
	}
	info["mcp"] = mcp
	return info
}

// mcpNow is the clock the reports stamp: a variable so tests can pin it.
var mcpNow = time.Now

// mcpEnsureMemoFile gives a 笔记 made through the MCP its file, named
// after its title, unless it has one.
func (d *deck) mcpEnsureMemoFile(n *block) error {
	if n.Path != "" {
		return nil
	}
	name, err := createMemoFile(d.root, n.Title)
	if err != nil {
		return err
	}
	n.Path, n.Title = name, memoTitleOf(name)
	return nil
}

// mcpTouchMemo makes an empty .md at path for a 笔记 to show, unless one is
// there.
func mcpTouchMemo(path string) error {
	f, err := os.OpenFile(path, os.O_WRONLY|os.O_CREATE, 0o644)
	if err != nil {
		return err
	}
	return f.Close()
}

// mcpWriteMemo writes text to the 笔记's file, all of it, and shows it in
// the block at once: an agent's write wins over what was being typed.
func (d *deck) mcpWriteMemo(n *block, text string) error {
	if err := d.mcpEnsureMemoFile(n); err != nil {
		return err
	}
	if err := writeFileAtomic(d.path(n), []byte(text)); err != nil {
		return err
	}
	if rt := n.rt; rt != nil && rt.memoLoaded {
		rt.memo, rt.memoSaved, rt.memoSeen = text, text, text
		if info, err := os.Stat(d.path(n)); err == nil {
			rt.memoMod = info.ModTime()
		}
	}
	return nil
}

// mcpMemoPath checks that a 笔记's path is a Markdown file's.
func mcpMemoPath(path string) error {
	switch strings.ToLower(filepath.Ext(path)) {
	case ".md", ".markdown":
		return nil
	}
	return fmt.Errorf("笔记是 Markdown 文件，路径要以 .md 结尾：%s", path)
}
