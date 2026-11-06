package main

import (
	"fmt"
	"strings"
	"time"

	"github.com/egoist/mygo/ui"
	"github.com/egoist/mygo/ui/markdown"
)

// A 块堆 (kind stack) is a wallet of blocks: blocks dropped on it leave the
// canvas and live in the pile, and the stack shows them as the cards of a
// wallet, each header peeking out above the next. A click opens it: then
// hovering its middle pushes the card under the pointer out, hovering its
// top or bottom edge (or the wheel) scrolls the cards, and space opens a
// long preview of the card out, which stays until space or escape. Losing
// the focus or the selection puts the card back (watchStack).
//
// The blocks of a stack stay in the deck, by ID (block.Items): they keep
// their files, notes and saved terminal state, but nothing of theirs is
// built while they are in the pile. Its card index is runtime state, not
// saved.

// stackCardFan is how many cards of a stack show at once: the top one and
// the edges of those behind it.
const stackCardFan = 3

func copyIDs(ids []string) []string {
	return append([]string(nil), ids...)
}

func containsID(ids []string, id string) bool {
	for _, v := range ids {
		if v == id {
			return true
		}
	}
	return false
}

func withoutID(ids []string, id string) []string {
	out := make([]string, 0, len(ids))
	for _, v := range ids {
		if v != id {
			out = append(out, v)
		}
	}
	return out
}

// stackedIDs is every block ID held by a stack, this frame.
func (d *deck) stackedIDs() map[string]bool {
	out := map[string]bool{}
	for _, n := range d.blocks {
		if n.Kind != kindStack {
			continue
		}
		for _, id := range n.Items {
			out[id] = true
		}
	}
	return out
}

// stackMembers lists the blocks a stack holds, in card order, skipping an
// ID whose block is gone (a block deleted while it was hidden).
func (d *deck) stackMembers(st *block) []*block {
	if st == nil || st.Kind != kindStack {
		return nil
	}
	out := make([]*block, 0, len(st.Items))
	for _, id := range st.Items {
		if n := d.blockByID(id); n != nil && n != st {
			out = append(out, n)
		}
	}
	return out
}

// stackIDs is what the MCP reports of a stack: its items that still exist.
func (d *deck) stackIDs(st *block) []string {
	members := d.stackMembers(st)
	out := make([]string, len(members))
	for i, n := range members {
		out[i] = n.ID
	}
	return out
}

// stackPicked is the card of st picked (pushed out, or last out), kept
// within its items.
func (d *deck) stackPicked(st *block) (int, []*block) {
	members := d.stackMembers(st)
	if len(members) == 0 {
		return 0, members
	}
	rt := d.ensureRuntime(st)
	rt.stackPick = ((rt.stackPick % len(members)) + len(members)) % len(members)
	return rt.stackPick, members
}

// stepStack turns a stack by delta cards.
func (d *deck) stepStack(st *block, delta int) {
	idx, members := d.stackPicked(st)
	if len(members) == 0 {
		return
	}
	d.ensureRuntime(st).stackPick = ((idx+delta)%len(members) + len(members)) % len(members)
}

// openStackPreview opens st's long preview: its card comes out of the
// pile and grows to the middle of the window. One still going back into
// st turns around where it is.
func (d *deck) openStackPreview(st *block) {
	if d.previewFresh = d.previewLeaving != st; d.previewFresh {
		d.previewFrom = ui.Rect{} // the pile says where its card is
	}
	d.previewStack, d.previewLeaving = st, nil
}

// closeStackPreview closes the long preview: it shrinks back into its
// card (stackPreviewView), and the pile closes up behind it.
func (d *deck) closeStackPreview() {
	if d.previewStack != nil {
		d.previewLeaving, d.previewStack = d.previewStack, nil
	}
}

// previewShown is the stack whose long preview shows, open or going back
// into its card.
func (d *deck) previewShown() *block {
	if d.previewStack != nil {
		return d.previewStack
	}
	return d.previewLeaving
}

// selectedStack is the stack the selection names, if it names exactly one.
func (d *deck) selectedStack() *block {
	if len(d.sel) != 1 {
		return nil
	}
	for id := range d.sel {
		if n := d.blockByID(id); n != nil && n.Kind == kindStack {
			return n
		}
	}
	return nil
}

// stackAt is the stack under a point of the window, in DIPs: where a block
// dropped there goes.
func (d *deck) stackAt(x, y float32, except *block) *block {
	hidden := d.stackedIDs()
	for i := len(d.blocks) - 1; i >= 0; i-- {
		st := d.blocks[i]
		if st.Kind != kindStack || st == except || hidden[st.ID] {
			continue
		}
		sx, sy := d.cam.toScreen(st.X, st.Y)
		w, h := st.W*d.cam.S, st.H*d.cam.S
		if x >= sx && x < sx+w && y >= sy && y < sy+h {
			return st
		}
	}
	return nil
}

// stackAdd puts n in st, taking it from any other stack it was in. The
// change is one undoable one, and the stack is selected.
func (d *deck) stackAdd(st, n *block) {
	if st == nil || n == nil || st == n || n.Kind == kindStack {
		return // no nesting: a stack never holds a stack
	}
	if containsID(st.Items, n.ID) {
		d.sel = map[string]bool{st.ID: true}
		return
	}
	edits := []stackEdit{}
	for _, other := range d.blocks {
		if other.Kind != kindStack || other == st || !containsID(other.Items, n.ID) {
			continue
		}
		edits = append(edits, stackEdit{
			stack:  other,
			before: copyIDs(other.Items),
			after:  withoutID(other.Items, n.ID),
		})
	}
	edits = append(edits, stackEdit{
		stack:  st,
		before: copyIDs(st.Items),
		after:  append(copyIDs(st.Items), n.ID),
	})
	d.applyStackEdits(edits)
	rt := d.ensureRuntime(st)
	rt.stackPick = len(d.stackMembers(st)) - 1
	rt.stackScroll, rt.stackOut = float32(rt.stackPick), false // in view; clamped as it is drawn
	d.sel = map[string]bool{st.ID: true}
	d.focusBlock = ""
}

// stackRemove takes n out of st and puts it on the canvas beside the
// stack. The membership change undoes; where it lands does not, like any
// drag of a block.
func (d *deck) stackRemove(st, n *block) {
	if st == nil || n == nil || !containsID(st.Items, n.ID) {
		return
	}
	x, y := d.freeSpot(st.X+st.W+28, st.Y, n.W, n.H)
	d.stackTake(st, n, x, y)
}

// stackTake takes n out of st and puts it on the canvas at (x, y), in
// world coordinates, selected. The membership change undoes.
func (d *deck) stackTake(st, n *block, x, y float32) {
	if st == nil || n == nil || !containsID(st.Items, n.ID) {
		return
	}
	d.applyStackEdits([]stackEdit{{
		stack:  st,
		before: copyIDs(st.Items),
		after:  withoutID(st.Items, n.ID),
	}})
	n.X, n.Y = x, y
	d.sel = map[string]bool{n.ID: true}
	d.requestSave()
}

// setStackItems replaces a stack's items, as the MCP's items field does.
func (d *deck) setStackItems(st *block, ids []string) {
	if st == nil {
		return
	}
	if d.indexOf(st) < 0 {
		// Not on the canvas yet (create_block patches before adding it):
		// the addition's undo takes the items with it.
		st.Items = copyIDs(ids)
		d.ensureRuntime(st).stackPick = 0
		d.syncStacked()
		return
	}
	if stringSlicesEqual(st.Items, ids) {
		return
	}
	d.applyStackEdits([]stackEdit{{
		stack:  st,
		before: copyIDs(st.Items),
		after:  copyIDs(ids),
	}})
}

func stringSlicesEqual(a, b []string) bool {
	if len(a) != len(b) {
		return false
	}
	for i := range a {
		if a[i] != b[i] {
			return false
		}
	}
	return true
}

// applyStackEdits changes what stacks hold and records it as one undoable
// change.
func (d *deck) applyStackEdits(edits []stackEdit) {
	d.forgetRedo()
	d.undo = append(d.undo, change{kind: changeStack, stacks: edits})
	d.restoreStackEdits(edits, true)
	d.requestSave()
}

// restoreStackEdits puts each stack's before (or after) items back.
func (d *deck) restoreStackEdits(edits []stackEdit, after bool) {
	for i := len(edits) - 1; i >= 0; i-- {
		e := edits[i]
		next := e.before
		if after {
			next = e.after
		}
		e.stack.Items = copyIDs(next)
	}
	d.syncStacked()
}

// syncStacked hangs up what the stacks hide and drops a preview whose
// stack is gone.
func (d *deck) syncStacked() {
	hidden := d.stackedIDs()
	for _, n := range d.blocks {
		if !hidden[n.ID] {
			continue
		}
		d.keepTerminal(n)
		d.hangUp(n)
	}
	for _, st := range d.blocks {
		if st.Kind == kindStack && st.rt != nil {
			if len(d.stackMembers(st)) == 0 {
				st.rt.stackPick = 0
			}
		}
	}
	if d.previewStack != nil && d.indexOf(d.previewStack) < 0 {
		d.previewStack = nil
	}
	if d.previewLeaving != nil && d.indexOf(d.previewLeaving) < 0 {
		d.previewLeaving = nil
	}
	if d.stackOpen != nil && d.indexOf(d.stackOpen) < 0 {
		d.closeStack()
	}
}

// pruneStacks drops item IDs whose block is no longer in the deck. It runs
// as a deck loads: undo never needs a hidden block back then.
func (d *deck) pruneStacks() {
	for _, st := range d.blocks {
		if st.Kind != kindStack || len(st.Items) == 0 {
			continue
		}
		keep := make([]string, 0, len(st.Items))
		seen := map[string]bool{}
		for _, id := range st.Items {
			if seen[id] {
				continue
			}
			seen[id] = true
			if n := d.blockByID(id); n != nil && n != st {
				keep = append(keep, id)
			}
		}
		st.Items = keep
	}
}

// validateStackItems checks what a stack may hold: existing blocks, no
// stack, no duplicates, not itself.
func (d *deck) validateStackItems(selfID string, ids []string) error {
	seen := map[string]bool{}
	for _, id := range ids {
		if selfID != "" && id == selfID {
			return fmt.Errorf("块堆不能装自己")
		}
		if seen[id] {
			return fmt.Errorf("items 里有重复的 id %q", id)
		}
		seen[id] = true
		n := d.blockByID(id)
		if n == nil {
			return fmt.Errorf("没有 id 为 %q 的 block", id)
		}
		if n.Kind == kindStack {
			return fmt.Errorf("块堆不能嵌套：%q 也是块堆", id)
		}
	}
	return nil
}

// stackKeys handles space on a selected stack, and the keys of an open
// long preview: space or escape closes it, the arrows turn its cards.
// Space typed into a terminal or an editor belongs to it.
func (d *deck) stackKeys(c *ui.Context) {
	d.watchStack(c)
	if d.pluck != nil && d.pluck.active && c.Shortcut(0, ui.KeyEscape) {
		d.pluck = nil // the card out goes back
		return
	}
	if d.previewStack != nil {
		if c.Shortcut(0, ui.KeySpace) {
			d.closeStackPreview()
			return
		}
	} else if d.editing == nil && d.focusBlock == "" && c.Shortcut(0, ui.KeySpace) {
		if st := d.selectedStack(); st != nil && len(d.stackMembers(st)) > 0 {
			d.openStackPreview(st)
		}
	}
	st := d.previewStack
	if st == nil {
		return
	}
	if c.Shortcut(0, ui.KeyEscape) {
		d.closeStackPreview()
		return
	}
	if c.Shortcut(0, ui.KeyRight) {
		d.stepStack(st, 1)
	}
	if c.Shortcut(0, ui.KeyLeft) {
		d.stepStack(st, -1)
	}
}

// kindLabel names a kind as the interface writes it.
func kindLabel(k blockKind) string {
	switch k {
	case kindNote:
		return "QuickLook"
	case kindMemo:
		return "笔记"
	case kindFile:
		return "文件"
	case kindTerminal:
		return "终端"
	case kindBrowser:
		return "浏览器"
	case kindStack:
		return "块堆"
	case kindChart:
		return "图表"
	}
	return string(k)
}

// cutText cuts s to at most max runes, at a rune boundary, for a card's
// snippet or a preview.
func cutText(s string, max int) string {
	r := []rune(s)
	if len(r) <= max {
		return s
	}
	return string(r[:max]) + "…"
}

// stackSnippet is the line under a card's title: what the block holds.
func (d *deck) stackSnippet(n *block) string {
	switch n.Kind {
	case kindNote:
		return stackTextSnippet(n.Text)
	case kindMemo:
		if n.rt != nil && n.rt.memoLoaded {
			return stackTextSnippet(n.rt.memo)
		}
		return n.Path
	case kindFile:
		return n.Path
	case kindTerminal:
		if n.Term != nil && n.Term.Dir != "" {
			return n.Term.Dir
		}
		return "终端"
	case kindBrowser:
		if n.URL != "" {
			return n.URL
		}
		return "空白页面"
	case kindChart:
		return chartSummary(n, d.chartDataOf(n))
	}
	return ""
}

// stackTextSnippet is the first line of Markdown as a card can show it,
// without the marks that only mean something rendered.
func stackTextSnippet(s string) string {
	s = strings.TrimSpace(s)
	if s == "" {
		return ""
	}
	line := strings.TrimSpace(strings.SplitN(s, "\n", 2)[0])
	line = strings.TrimSpace(strings.TrimLeft(line, "#>*-+`"))
	return cutText(line, 120)
}

// stackGeom is where a stack's cards lie in its body, in layer pixels:
// like the cards of a wallet, each in front of the one before it, the
// headers of those behind peeking out above. The cards scroll by whole
// headers: scroll 2 has card 2 at the top, 0 and 1 tucked behind it.
type stackGeom struct {
	w, h   float32 // the body
	pad    float32 // around the cards
	peek   float32 // how much of a card shows above the next: its header
	cardH  float32
	reveal float32 // how far the cards after one pushed out move down
	lift   float32 // how far the card pushed out rises from its slot
	band   float32 // the edges where hovering scrolls
	count  int
	max    float32 // how far the cards scroll
}

func newStackGeom(w, h, s float32, count int) stackGeom {
	g := stackGeom{w: w, h: h, pad: 12 * s, peek: 32 * s, lift: 8 * s, count: count}
	g.cardH = max(h-2*g.pad-2*g.peek, 2*g.peek)
	g.reveal = g.cardH - g.peek + 6*s
	g.band = min(30*s, h/5)
	// No further than the last card shows whole at the bottom.
	g.max = max(0, float32(count-1)-(h-2*g.pad-g.cardH)/g.peek)
	return g
}

// stackGeomOf is the geometry of n's body as the layer lays it out.
func (d *deck) stackGeomOf(n *block, count int) stackGeom {
	s := d.lay.S
	return newStackGeom(n.W*s, n.H*s-d.titleHeight(n), s, count)
}

// slot is the top of card i with the cards scrolled to sc, before any
// card is pushed out: those scrolled past tuck up behind the first.
func (g stackGeom) slot(i int, sc float32) float32 {
	at := float32(i) - sc
	if at < 0 {
		return g.pad + at*g.peek*0.3
	}
	return g.pad + at*g.peek
}

// cardAt is the card under y, the front one where they overlap, with
// card out (-1 for none) pushed out: its following cards make room.
func (g stackGeom) cardAt(y, sc float32, out int) int {
	for i := g.count - 1; i >= 0; i-- {
		top := g.slot(i, sc)
		switch {
		case out >= 0 && i > out:
			top += g.reveal
		case i == out:
			top -= g.lift
		}
		if y >= top {
			return i
		}
	}
	return 0
}

// openStack makes st the stack the pointer works, as a click on its body
// does: hovering it then scrolls its cards and pushes them out. (x, y) is
// the click in its body.
func (d *deck) openStack(st *block, x, y float32) {
	d.sel = map[string]bool{st.ID: true}
	d.focusBlock = ""
	rt := d.ensureRuntime(st)
	if d.stackOpen != st {
		d.stackOpen = st
		rt.stackHadFocus = false
	}
	_, members := d.stackPicked(st)
	if len(members) == 0 {
		return
	}
	g := d.stackGeomOf(st, len(members))
	if y < g.band && rt.stackScroll > 0 || y > g.h-g.band {
		return // an edge: hovering it scrolls
	}
	out := -1
	if rt.stackOut {
		out = rt.stackPick
	}
	rt.stackPick, rt.stackOut = g.cardAt(y, rt.stackScroll, out), true
}

// closeStack puts the card pushed out of st back in its slot.
func (d *deck) closeStack() {
	if st := d.stackOpen; st != nil && st.rt != nil {
		st.rt.stackOut, st.rt.stackHadFocus = false, false
	}
	d.stackOpen = nil
}

// watchStack closes the stack the pointer works once it is no longer
// the one thing selected, or has had the keyboard focus and lost it: the
// card it pushed out goes back. An open long preview keeps it.
func (d *deck) watchStack(c *ui.Context) {
	st := d.stackOpen
	if st == nil || d.previewShown() != nil {
		return
	}
	if d.indexOf(st) < 0 || d.stackedIDs()[st.ID] || len(d.sel) != 1 || !d.sel[st.ID] ||
		d.editing != nil || d.focusBlock != "" {
		d.closeStack()
		return
	}
	rt := d.ensureRuntime(st)
	if rt.stackFocus.Focused(c) {
		rt.stackHadFocus = true
	} else if rt.stackHadFocus {
		d.closeStack()
	}
}

// stackScrollBy scrolls a stack's cards by dy layer pixels, as the wheel
// does over a stack the pointer works.
func (d *deck) stackScrollBy(st *block, dy float32) bool {
	_, members := d.stackPicked(st)
	g := d.stackGeomOf(st, len(members))
	if g.max <= 0 {
		return false
	}
	rt := d.ensureRuntime(st)
	rt.stackScroll = min(max(rt.stackScroll+dy/g.peek, 0), g.max)
	return true
}

// How fast hovering an edge scrolls the cards, in cards a second, and how
// the wheel's scrolling and a card coming out move.
const stackEdgeSpeed = 5

var (
	stackScrollAnim = ui.Animation{Duration: 160 * time.Millisecond, Ease: ui.EaseOut}
	stackCardAnim   = ui.Animation{Spring: ui.SpringOptions{Duration: 380 * time.Millisecond, Bounce: 0.18}}
	// The long preview growing out of its card and the pile moving apart.
	stackPreviewAnim = ui.Animation{Spring: ui.SpringOptions{Duration: 420 * time.Millisecond, Bounce: 0.1}}
	stackSpreadAnim  = ui.Animation{Spring: ui.SpringOptions{Duration: 460 * time.Millisecond, Bounce: 0.14}}
)

// stackTint is the color a card of kind k is marked with.
func stackTint(k blockKind) ui.Color {
	switch k {
	case kindNote:
		return ui.Hex("#E3A92B")
	case kindMemo:
		return ui.Hex("#2F9E6E")
	case kindFile:
		return ui.Hex("#3B7FE0")
	case kindTerminal:
		return ui.Hex("#6B7280")
	case kindBrowser:
		return ui.Hex("#8B5CF6")
	case kindChart:
		return ui.Hex("#D9480F")
	}
	return ui.Hex("#6B7280")
}

// stackFace is what a card's face shows under its header: the start of a
// note, a file's path, a terminal's directory, a page's address.
func (d *deck) stackFace(n *block) string {
	switch n.Kind {
	case kindNote:
		return stackTextFace(n.Text)
	case kindMemo:
		if n.rt != nil && n.rt.memoLoaded {
			if s := stackTextFace(n.rt.memo); s != "" {
				return s
			}
		}
		return n.Path
	}
	return d.stackSnippet(n)
}

// stackFaceMarkdown is the Markdown a card's face shows rendered — a
// QuickLook's text, a 笔记's file, a .md or Word file — asking for what
// is still to load; "" when it holds none, or none yet, and its face is
// stackFace's line.
func (d *deck) stackFaceMarkdown(n *block) string {
	var src string
	switch n.Kind {
	case kindNote:
		src = n.Text
	case kindMemo:
		rt := d.ensureRuntime(n)
		d.loadMemo(n, rt)
		if rt.memoLoaded {
			src = rt.memo
		}
	case kindFile:
		if p := previewOf(n.Path); p != previewMarkdown && p != previewWord {
			return ""
		}
		rt := d.ensureRuntime(n)
		d.loadDocument(n, rt)
		if rt.doc != nil {
			src = rt.doc.markdown
		}
	}
	if strings.TrimSpace(src) == "" {
		return ""
	}
	// No more than a face can show.
	return cutText(src, 2000)
}

// stackTextFace is the first lines of Markdown, without the marks that
// only mean something rendered.
func stackTextFace(s string) string {
	var lines []string
	for _, line := range strings.Split(strings.TrimSpace(s), "\n") {
		line = strings.TrimSpace(strings.TrimLeft(strings.TrimSpace(line), "#>*-+`"))
		if line == "" {
			continue
		}
		lines = append(lines, line)
		if len(lines) == 8 {
			break
		}
	}
	return cutText(strings.Join(lines, "\n"), 360)
}

// stackCard is one card as a stack's body paints it this frame.
type stackCard struct {
	n           *block
	index       int
	y           float32
	lifted      float32 // 0 in its slot, 1 pushed out
	apart       float32 // 0 in the pile, 1 out of the long preview's way
	title, face string
	md          string     // Markdown the face shows rendered, rather than face
	chart       *chartData // a 图表's table, drawn small on its face
}

// faceShows says whether the face of cards[i], below its header, shows
// in the body: the card in front of it (the next one drawn) leaves some
// of it uncovered.
func (g stackGeom) faceShows(cards []stackCard, i int) bool {
	k := cards[i]
	bottom := min(k.y+g.cardH, g.h)
	if i+1 < len(cards) {
		bottom = min(bottom, cards[i+1].y)
	}
	return bottom-(k.y+g.peek) > g.peek/3
}

// stackBody draws the pile as a wallet: the cards stand one in front of
// the other, their headers peeking out. Once a click opens it, hovering
// its middle pushes the card under the pointer out, as a card comes out
// of a wallet, and hovering its top or bottom edge scrolls the cards; the
// wheel scrolls them too. Space previews the card out at length. An empty
// pile says how to fill it.
func (d *deck) stackBody(c *ui.Context, n *block) {
	t := c.Theme()
	s := d.lay.S
	_, members := d.stackPicked(n)
	if d.pluck != nil && d.pluck.active && d.pluck.from == n {
		// The card plucked out is under the pointer, not in the pile.
		rest := make([]*block, 0, len(members))
		for _, m := range members {
			if !d.plucking(m) {
				rest = append(rest, m)
			}
		}
		members = rest
	}
	drop := d.dropHere(n)
	if len(members) == 0 {
		box := ui.Box(c).Grow(1).FillWidth().Padding(14*s).Border(1, t.Border).Radius(10)
		if drop {
			box = box.Border(2, t.Accent)
		}
		box.Children(func() {
			ui.Column(c).Fill().Justify(ui.Center).AlignItems(ui.Center).Gap(6 * s).Children(func() {
				ui.Icon(c, iconStack).FontSize(30 * s)
				ui.Text(c, "拖 block 的胶带或标题栏过来").FontSize(13 * s).TextColor(t.TextMuted)
				ui.Text(c, "松手即收纳 · 点击后悬停翻看 · 空格预览").FontSize(11 * s).TextColor(t.TextMuted)
			})
		})
		return
	}
	rt := d.ensureRuntime(n)
	g := d.stackGeomOf(n, len(members))
	open := d.stackOpen == n
	box := ui.Box(c.Key("stack-cards:" + n.ID)).Grow(1).FillWidth()

	// Where the cards scroll to: the edge hovered, or where the wheel left
	// them.
	rt.stackScroll = min(max(rt.stackScroll, 0), g.max)
	target, anim := rt.stackScroll, stackScrollAnim
	_, py, over := box.PointerPosition()
	if open && over && d.previewShown() == nil && d.dragging == nil && d.pluck == nil && !d.layerMoved() {
		switch {
		case py < g.band && rt.stackScroll > 0:
			target = 0
		case py > g.h-g.band:
			target = g.max
		default:
			out := -1
			if rt.stackOut {
				out = rt.stackPick
			}
			rt.stackPick, rt.stackOut = g.cardAt(py, rt.stackScroll, out), true
		}
		if target != rt.stackScroll {
			// An edge: the card out goes back, the cards run on.
			rt.stackOut = false
			anim = ui.Animation{Duration: time.Duration(abs(target-rt.stackScroll) / stackEdgeSpeed * float32(time.Second)), Ease: ui.Linear}
		}
	}
	shown := d.previewShown() == n
	if shown {
		// The long preview turns the cards: the one it shows is out, and
		// in view.
		rt.stackOut = true
		fit := (g.h - 2*g.pad - g.cardH) / g.peek
		rt.stackScroll = max(min(max(rt.stackScroll, float32(rt.stackPick)-fit), float32(rt.stackPick), g.max), 0)
		target = rt.stackScroll
	}
	sc := box.AnimateTo("scroll", target, anim)
	held, heldCard := d.pluckHeld(c, box, n)
	if target != rt.stackScroll {
		rt.stackScroll = sc
	}
	out := -1
	if rt.stackOut && (open || shown) {
		out = rt.stackPick
	}

	// The long preview open, the card it shows has come out of the pile
	// and the others move apart, out of its way, until it is back.
	spreadTo := float32(0)
	if d.previewStack == n {
		spreadTo = 1
	}
	spread := box.AnimateTo("spread", spreadTo, stackSpreadAnim)

	cards := make([]stackCard, 0, 8)
	for i, m := range members {
		reveal, lifted := float32(0), float32(0)
		if out >= 0 && i > out {
			reveal = g.reveal
		}
		if i == out {
			lifted = 1
		}
		reveal = box.AnimateTo("reveal:"+m.ID, reveal, stackCardAnim)
		lifted = box.AnimateTo("lift:"+m.ID, lifted, stackCardAnim)
		y := g.slot(i, sc) + reveal - lifted*g.lift
		var apart float32
		switch {
		case out < 0:
		case i < out:
			apart = spread
			y -= spread * (g.peek*1.5 + float32(out-i-1)*g.peek*0.6)
		case i > out:
			apart = spread
			y += spread * (g.peek*2 + float32(i-out-1)*g.peek*0.6)
		}
		if shown && i == out {
			// It is the preview now: where it lies is where the preview
			// grows from and goes back into.
			if b := box.Bounds(); b.W > 0 && g.w > 0 {
				k := b.W / g.w
				w := g.w - 2*g.pad + lifted*8*s
				d.previewFrom = ui.Rect{X: b.X + (g.pad-lifted*4*s)*k, Y: b.Y + y*k, W: w * k, H: g.cardH * k}
			}
			continue
		}
		if float32(i)-sc < -3 || y > g.h || y+g.cardH < 0 {
			continue
		}
		title := m.Title
		if strings.TrimSpace(title) == "" {
			title = defaultTitle(m)
		}
		cards = append(cards, stackCard{n: m, index: i, y: y, lifted: lifted, apart: apart, title: title})
	}
	// Only the cards whose face shows, not covered by the card in front,
	// load what they hold and lay it out: the others show their header.
	for i := range cards {
		if !g.faceShows(cards, i) {
			continue
		}
		k := &cards[i]
		switch {
		case k.n.Kind == kindChart:
			k.chart = d.chartDataOf(k.n)
		default:
			if k.md = d.stackFaceMarkdown(k.n); k.md == "" {
				k.face = d.stackFace(k.n)
			}
		}
	}

	count := len(members)
	var pill string
	switch {
	case drop:
		pill = "松手收进这一堆"
	case out >= 0:
		pill = fmt.Sprintf("%d / %d · 空格长预览 · 长按右上角取出", out+1, count)
	case open:
		pill = fmt.Sprintf("%d 张 · 悬停推出 · 上下边缘滚动", count)
	default:
		pill = fmt.Sprintf("%d 张 · 点击翻看", count)
	}
	shade := ui.RGBA(0, 0, 0, 0.16)
	if t.Dark {
		shade = ui.RGBA(0, 0, 0, 0.42)
	}
	scrollMax, scrolled := g.max, sc

	box.Draw(func(p *ui.Painter, r ui.Rect) {
		radius := 9 * s
		p.Clip(r, 0, func() {
			for _, k := range cards {
				l := k.lifted - k.apart*0.75 // apart, it steps back
				card := ui.Rect{
					X: r.X + g.pad - l*4*s,
					Y: r.Y + k.y,
					W: r.W - 2*g.pad + l*8*s,
					H: g.cardH,
				}
				tint := stackTint(k.n.Kind)
				p.Shadow(card, radius, 0, (2+6*l)*s, (8+16*l)*s, 0, shade)
				p.Fill(card, t.Surface.Mix(tint, 0.07+0.05*l), radius)
				p.Stroke(card, t.Border.Mix(tint, 0.35*l), radius, max(1*s, 0.5))
				p.Clip(card, radius, func() {
					// The header: what peeks out of the slot.
					p.Fill(ui.Rect{X: card.X, Y: card.Y, W: card.W, H: 3 * s}, tint.Alpha(0.85), 0)
					if k.n == heldCard {
						// Its corner held: the stripe fills from the
						// corner until the card comes out.
						w := card.W * held
						p.Fill(ui.Rect{X: card.X + card.W - w, Y: card.Y, W: w, H: 3 * s}, t.Accent, 0)
						p.Fill(ui.Rect{X: card.X + card.W - pluckCornerW*s, Y: card.Y + 3*s, W: pluckCornerW * s, H: g.peek - 3*s},
							t.Accent.Alpha(0.08+0.1*held), 0)
					}
					p.Fill(ui.Rect{X: card.X + 12*s, Y: card.Y + g.peek/2 - 3*s, W: 6 * s, H: 6 * s}, tint, 3*s)
					label := ui.Span{Text: kindLabel(k.n.Kind), Size: 10 * s, Color: t.TextMuted}
					if k.n == heldCard {
						label.Color = t.Accent
					}
					lw, _ := p.MeasureText(0, label)
					right := card.X + card.W - 12*s
					p.RichText(right-lw, card.Y+g.peek/2-6.5*s, 0, label)
					p.Clip(ui.Rect{X: card.X, Y: card.Y, W: max(right-lw-8*s-card.X, 0), H: g.peek}, 0, func() {
						p.RichText(card.X+24*s, card.Y+g.peek/2-8.5*s, 0,
							ui.Span{Text: k.title, Size: 12.5 * s, Weight: 600, Color: t.Text})
					})
					// The face: what shows once it is out.
					p.Line(card.X+12*s, card.Y+g.peek, card.X+card.W-12*s, card.Y+g.peek, max(1*s, 0.5), t.Border.Alpha(0.6))
					if k.face != "" {
						p.RichText(card.X+14*s, card.Y+g.peek+8*s, card.W-28*s,
							ui.Span{Text: k.face, Size: 11 * s, Color: t.TextMuted})
					}
					if k.chart != nil {
						chartView{typ: chartTypeOf(k.n), data: k.chart, s: s, t: t, compact: true}.draw(p,
							ui.Rect{X: card.X + 10*s, Y: card.Y + g.peek + 6*s, W: card.W - 20*s, H: g.cardH - g.peek - 12*s})
					}
				})
			}
		})
	})
	// The faces holding Markdown show it rendered, each clipped to what
	// the card in front leaves of it.
	box.Children(func() {
		for i, k := range cards {
			if k.md == "" {
				continue
			}
			l := k.lifted - k.apart*0.75
			top := k.y + g.peek + 1*s
			bottom := min(k.y+g.cardH-4*s, g.h)
			if i+1 < len(cards) {
				bottom = min(bottom, cards[i+1].y)
			}
			if bottom-top < 4*s {
				continue
			}
			w := g.w - 2*g.pad + l*8*s - 28*s
			ui.Box(c.Key("stack-face:"+k.n.ID)).Absolute().
				Left(g.pad-l*4*s+14*s).Top(top).Size(w, bottom-top).
				Clip().IgnoreInput(true).Children(func() {
				ui.Column(c).Width(w).Padding(7*s, 0, 0, 0).Children(func() {
					markdown.View(c, k.md, d.faceMarkdown(t, 11*s, t.Dark))
				})
			})
		}
	})
	box.DrawOver(func(p *ui.Painter, r ui.Rect) {
		p.Clip(r, 0, func() {
			// How far the cards are scrolled, when they scroll.
			if scrollMax > 0 {
				track := r.H - 2*g.pad
				thumb := max(track*(1-scrollMax/float32(count)), 16*s)
				ty := r.Y + g.pad + (track-thumb)*scrolled/scrollMax
				p.Fill(ui.Rect{X: r.X + r.W - 5*s, Y: ty, W: 2.5 * s, H: thumb}, t.TextMuted.Alpha(0.45), 1.25*s)
			}
			// What the pile does now, in a pill at its foot.
			span := ui.Span{Text: pill, Size: 10 * s, Color: t.TextMuted}
			if drop || out >= 0 {
				span.Color = t.Accent
			}
			pw, ph := p.MeasureText(0, span)
			pr := ui.Rect{X: r.X + (r.W-pw)/2 - 9*s, Y: r.Y + r.H - ph - 14*s, W: pw + 18*s, H: ph + 8*s}
			p.Shadow(pr, pr.H/2, 0, 1*s, 6*s, 0, shade)
			p.Fill(pr, t.Surface.Alpha(0.94), pr.H/2)
			p.Stroke(pr, t.Border, pr.H/2, max(1*s, 0.5))
			p.RichText(pr.X+9*s, pr.Y+4*s, 0, span)
		})
		if drop {
			p.Stroke(ui.Rect{X: r.X + 1.5*s, Y: r.Y + 1.5*s, W: r.W - 3*s, H: r.H - 3*s}, t.Accent, 10*s, 2*s)
		}
	})
}

// dropHere says whether a block being dragged is over st, ready to be
// let go into it: what the pile highlights as it is dragged over.
func (d *deck) dropHere(st *block) bool {
	if p := d.pluck; p != nil && p.active {
		return d.stackAt(p.at.X, p.at.Y, nil) == st
	}
	if d.dragging == nil || d.dragging == st {
		return false
	}
	return d.stackAt(d.dragAt.X, d.dragAt.Y, d.dragging) == st
}

// stackPreviewView is the long preview space opens: a panel in the
// window, over the canvas, showing the card on top of a stack at a
// readable size. It stays until space or escape, or a click outside it.
// It grows out of the card's place in the pile to the middle of the
// window, and closed, shrinks back into it (previewLeaving) before the
// card shows in the pile again.
func (d *deck) stackPreviewView(c *ui.Context) {
	st := d.previewShown()
	if st == nil || d.indexOf(st) < 0 {
		d.previewStack, d.previewLeaving = nil, nil
		return
	}
	idx, members := d.stackPicked(st)
	if len(members) == 0 {
		d.previewStack, d.previewLeaving = nil, nil
		return
	}
	open := d.previewStack == st
	cw, ch := c.Size()
	w := min(float32(560), cw-48)
	h := min(float32(620), ch-96)
	x, y := (cw-w)/2, (ch-h)/2
	t := c.Theme()
	top := members[idx]

	backdrop := ui.Box(c.Key("stack-preview-backdrop")).Absolute().Left(0).Top(0).Size(cw, ch)
	// How far it has grown: 0 the card in the pile, 1 the panel. Just
	// opened, it starts at the card: the value starts at its first target.
	target := float32(0)
	if open && !d.previewFresh {
		target = 1
	}
	if d.previewFresh {
		d.previewFresh = false
		c.Invalidate()
	}
	k := backdrop.AnimateTo("grow", target, stackPreviewAnim)
	if !open && k <= 0.002 {
		d.previewLeaving = nil // back in its card
		return
	}
	backdrop.Background(ui.RGBA(0, 0, 0, 0.34*min(max(k, 0), 1))).
		HandleInput(func(ev ui.InputEvent) bool {
			if ev.Kind == ui.InputPointerDown && d.previewStack != nil {
				d.closeStackPreview()
				return true
			}
			return false
		})

	// From the card's box to the panel's, scaled and moved: the panel is
	// laid out once, at its size.
	from := d.previewFrom
	if from.W <= 0 || from.H <= 0 {
		// The card out of view: it grows from the middle.
		from = ui.Rect{X: x + w*0.1, Y: y + h*0.1, W: w * 0.8, H: h * 0.8}
	}
	cx, cy := from.X+(x-from.X)*k, from.Y+(y-from.Y)*k
	sx, sy := (from.W+(w-from.W)*k)/w, (from.H+(h-from.H)*k)/h
	shade := ui.RGBA(0, 0, 0, 0.20)
	if t.Dark {
		shade = ui.RGBA(0, 0, 0, 0.5)
	}
	panel := ui.Box(c.Key("stack-preview:"+st.ID)).Absolute().Left(x).Top(y).Size(w, h).
		Background(t.Surface.Mix(stackTint(top.Kind), 0.1*(1-min(max(k, 0), 1)))).
		Border(1, t.Border).Radius(14).Shadow(0, 10, 40, 0, shade).Clip()
	if k < 0.999 {
		panel = panel.TransformOrigin(0, 0).Scale(sx, sy).Translate(cx-x, cy-y)
	}
	if !open {
		panel = panel.IgnoreInput(true) // going back: the canvas has the pointer
	}
	// What it shows comes clear as it grows, and goes as it shrinks.
	panel.Children(func() {
		ui.Column(c).Fill().Opacity(min(max((k-0.35)/0.6, 0), 1)).Children(func() {
			ui.Row(c).FillWidth().Padding(14, 16, 4, 16).Gap(10).AlignItems(ui.Center).Children(func() {
				ui.Icon(c, kindIcon(top.Kind)).FontSize(18)
				title := top.Title
				if strings.TrimSpace(title) == "" {
					title = defaultTitle(top)
				}
				ui.Text(c, title).FontSize(16).FontWeight(600).SingleLine().Grow(1).TextColor(t.Text)
			})
			ui.Scroll(c.Key("stack-preview-body:"+st.ID)).Grow(1).FillWidth().
				Padding(12, 20, 16, 20).Children(func() {
				ui.Column(c).FillWidth().Gap(8).Children(func() {
					d.stackPreviewBody(c, top, w-40)
				})
			})
		})
	})
}

func kindIcon(k blockKind) *ui.SVG {
	switch k {
	case kindNote:
		return iconNote
	case kindMemo:
		return iconMemo
	case kindFile:
		return iconFile
	case kindTerminal:
		return iconTerminal
	case kindBrowser:
		return iconGlobe
	case kindStack:
		return iconStack
	case kindChart:
		return iconChart
	}
	return iconNote
}

// stackPreviewBody shows what the card holds, at a readable size: the
// note's text, the file's preview, a page of its PDF, the terminal's
// directory or the page's address. Loading stays off the UI thread, as
// every preview's does.
func (d *deck) stackPreviewBody(c *ui.Context, n *block, width float32) {
	t := c.Theme()
	rt := d.ensureRuntime(n)
	switch n.Kind {
	case kindNote:
		if strings.TrimSpace(n.Text) == "" {
			ui.Text(c, "（空 QuickLook）").FontSize(13).TextColor(t.TextMuted)
			return
		}
		markdown.View(c, cutText(n.Text, 20000), d.stackMarkdown(t, 14, t.Dark))
	case kindMemo:
		d.loadMemo(n, rt)
		switch {
		case rt.memoErr != nil:
			ui.Text(c, rt.memoErr.Error()).FontSize(13).TextColor(t.Danger)
		case !rt.memoLoaded:
			ui.Text(c, "加载中…").FontSize(13).TextColor(t.TextMuted)
		case strings.TrimSpace(rt.memo) == "":
			ui.Text(c, "（空笔记）").FontSize(13).TextColor(t.TextMuted)
		default:
			markdown.View(c, cutText(rt.memo, 20000), d.stackMarkdown(t, 14, t.Dark))
		}
		ui.Text(c, n.Path).FontSize(11).SingleLine().TextColor(t.TextMuted).Padding(8, 0, 0, 0)
	case kindFile:
		ui.Text(c, n.Path).FontSize(11).TextColor(t.TextMuted)
		switch previewOf(n.Path) {
		case previewImage:
			d.loadImage(n, rt)
			switch {
			case rt.imgErr != nil:
				ui.Text(c, rt.imgErr.Error()).FontSize(13).TextColor(t.Danger)
			case rt.source == nil:
				ui.Text(c, "加载中…").FontSize(13).TextColor(t.TextMuted)
			default:
				ui.Box(c).FillWidth().Height(320).Children(func() {
					ui.Image(c, rt.source).Fill().Fit(ui.Contain)
				})
			}
		case previewMarkdown, previewWord:
			d.loadDocument(n, rt)
			switch {
			case rt.docErr != nil:
				ui.Text(c, rt.docErr.Error()).FontSize(13).TextColor(t.Danger)
			case rt.doc == nil:
				ui.Text(c, "加载中…").FontSize(13).TextColor(t.TextMuted)
			default:
				markdown.View(c, cutText(rt.doc.markdown, 20000), d.stackMarkdown(t, 14, t.Dark))
			}
		case previewPDF:
			d.openPDFDoc(n, rt)
			switch {
			case rt.pdfErr != nil:
				ui.Text(c, rt.pdfErr.Error()).FontSize(13).TextColor(t.Danger)
			case rt.pdf == nil:
				ui.Text(c, "加载中…").FontSize(13).TextColor(t.TextMuted)
			default:
				pages := rt.pdf.Pages()
				page := min(max(n.Page, 0), max(pages-1, 0))
				d.renderPDFPage(n, rt, page, pdfWidth(width))
				if rt.pdfImage != nil {
					ui.Box(c).FillWidth().Height(400).Children(func() {
						ui.Image(c, rt.pdfImage).Fill().Fit(ui.Contain)
					})
				} else {
					ui.Textf(c, "第 %d / %d 页 · 渲染中…", page+1, pages).
						FontSize(13).TextColor(t.TextMuted)
				}
			}
		default:
			d.loadText(n, rt)
			switch {
			case rt.textErr != nil:
				ui.Text(c, rt.textErr.Error()).FontSize(13).TextColor(t.Danger)
			case rt.editor == nil && rt.text == "":
				ui.Text(c, "加载中…").FontSize(13).TextColor(t.TextMuted)
			default:
				ui.Text(c, cutText(rt.text, 20000)).FontSize(13).TextColor(t.Text)
			}
		}
	case kindTerminal:
		dir := d.root
		if n.Term != nil && n.Term.Dir != "" {
			dir = n.Term.Dir
		}
		ui.Text(c, "终端在堆里挂起，取出后从原目录重新启动。").FontSize(13).TextColor(t.TextMuted)
		ui.Text(c, "目录 · "+dir).FontSize(13).TextColor(t.Text).Padding(10, 0, 0, 0)
		if n.Term != nil && len(n.Term.History) > 0 {
			ui.Textf(c, "上次留下 %d KB 的输出", len(n.Term.History)>>10).
				FontSize(12).TextColor(t.TextMuted).Padding(6, 0, 0, 0)
		}
	case kindBrowser:
		ui.Text(c, "浏览器在堆里停着，取出后重新加载这一页。").FontSize(13).TextColor(t.TextMuted)
		ui.Text(c, cutText(n.URL, 300)).FontSize(13).TextColor(t.Text).Padding(10, 0, 0, 0)
	case kindStack:
		ui.Textf(c, "另一个块堆 · %d 个 block", len(d.stackMembers(n))).FontSize(13).TextColor(t.Text)
	case kindChart:
		cd, typ := d.chartDataOf(n), chartTypeOf(n)
		el := ui.Box(c.Key("stack-preview-chart:" + n.ID)).FillWidth().Height(380)
		hx, hy, over := el.PointerPosition()
		el.Draw(func(p *ui.Painter, r ui.Rect) {
			chartView{typ: typ, data: cd, s: 1, t: t,
				hover: ui.Point{X: r.X + hx, Y: r.Y + hy}, hovering: over}.draw(p, r)
		})
		ui.Text(c, chartSummary(n, cd)).FontSize(11).SingleLine().TextColor(t.TextMuted).Padding(8, 0, 0, 0)
	}
}

// stackMarkdown is the options a stack's long preview renders Markdown
// with. It is separate from the QuickLook bodies' options: both are on
// screen at once, at different sizes.
func (d *deck) stackMarkdown(t *ui.Theme, size float32, dark bool) *markdown.Options {
	return stackMDOptions(&d.previewMD, &d.previewMDSize, &d.previewMDDark, t, size, dark, t.Text)
}

// faceMarkdown is the options a card's face renders Markdown with, in
// the muted color its face shows in.
func (d *deck) faceMarkdown(t *ui.Theme, size float32, dark bool) *markdown.Options {
	return stackMDOptions(&d.faceMD, &d.faceMDSize, &d.faceMDDark, t, size, dark, t.TextMuted)
}

// stackMDOptions makes o's theme anew when the size or the light or dark
// changed since it was made.
func stackMDOptions(o *markdown.Options, oSize *float32, oDark *bool, t *ui.Theme, size float32, dark bool, text ui.Color) *markdown.Options {
	if o.Theme == nil || *oSize != size || *oDark != dark {
		o.Theme = &markdown.Theme{
			FontSize:       size,
			Text:           text,
			Muted:          t.TextMuted,
			Strong:         t.Text,
			Link:           t.Accent,
			Code:           t.Text,
			CodeBackground: t.SurfacePressed,
			QuoteBar:       t.Border,
			QuoteText:      t.TextMuted,
		}
		*oSize, *oDark = size, dark
	}
	return o
}
