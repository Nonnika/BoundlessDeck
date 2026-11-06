package main

import (
	"encoding/json"
	"errors"
	"fmt"
	"math/rand/v2"
	"os"
	"path/filepath"
	"sync"
	"time"

	"github.com/egoist/mygo"
	"github.com/egoist/mygo/plugins/terminal"
	"github.com/egoist/mygo/ui"
	"github.com/egoist/mygo/ui/editor"
	"github.com/egoist/mygo/ui/markdown"
)

type blockKind string

const (
	kindNote     blockKind = "note"
	kindMemo     blockKind = "memo" // 笔记: a Markdown file of the deck's folder (memo.go)
	kindFile     blockKind = "file"
	kindTerminal blockKind = "terminal"
	kindBrowser  blockKind = "browser"
	kindStack    blockKind = "stack" // 块堆: a wallet of blocks held off the canvas (stack.go)
)

// blockKinds is every kind, in the order the toolbar and the MCP list them.
var blockKinds = []blockKind{kindNote, kindMemo, kindFile, kindTerminal, kindBrowser, kindStack}

// kindNames is blockKinds as strings.
func kindNames() []string {
	out := make([]string, len(blockKinds))
	for i, k := range blockKinds {
		out[i] = string(k)
	}
	return out
}

// errNoWindow is why a block that needs the window's native views shows
// nothing in tests.
var errNoWindow = errors.New("需要一个窗口")

// block is one card of the canvas. Positions and sizes are in world
// coordinates; the camera turns them into DIPs of the window.
type block struct {
	ID    string    `json:"id"`
	Kind  blockKind `json:"kind"`
	Title string    `json:"title"`
	// Named says the user named the block: the titles a terminal's shell
	// or a browser's page give it no longer replace its title.
	Named bool    `json:"named,omitempty"`
	X     float32 `json:"x"`
	Y     float32 `json:"y"`
	W     float32 `json:"w"`
	H     float32 `json:"h"`

	// Path is the file a file block links to: relative to the workspace
	// root when it is inside it.
	Path string `json:"path,omitempty"`
	// Text is the body of a note.
	Text string `json:"text,omitempty"`
	// URL is the page a browser block shows.
	URL string `json:"url,omitempty"`
	// Page is the page of a PDF a file block shows, from 0.
	Page int `json:"page,omitempty"`
	// Term is what a terminal block kept of its terminal as the deck
	// closed, to start a new one from.
	Term *termState `json:"term,omitempty"`
	// Expires is when a QuickLook block goes by itself, in Unix ms; 0
	// keeps it (lifespan.go).
	Expires int64 `json:"expires,omitempty"`
	// Items are the blocks a 块堆 holds, by ID, the top card first. A
	// block in a stack stays in the deck but is not drawn on the canvas.
	Items []string `json:"items,omitempty"`

	// rt is what the block keeps between frames: never saved.
	rt *runtime
}

// runtime is a block's state between frames: loaded text, a bitmap, an
// editor, a terminal.
type runtime struct {
	loading bool
	text    string
	textErr error
	editor  *editor.State

	source  ui.ImageSource
	imgErr  error
	imgDone bool
	// imgAlpha says the picture may be see-through: a checkerboard shows
	// under it.
	imgAlpha bool

	// doc is a .md or Word file read for its preview, and mdOpts what it
	// renders with, at mdSize in the dark theme or not.
	doc    *document
	docErr error
	mdOpts *markdown.Options
	mdSize float32
	mdDark bool

	// pdf is a PDF open for its preview; pdfImage its page pdfPage as
	// last rendered, pdfPx pixels wide. pdfGen counts its closings: what
	// opens or renders after one is dropped.
	pdf          *pdfDoc
	pdfErr       error
	pdfImage     *ui.Bitmap
	pdfPage      int
	pdfPx        int
	pdfOpening   bool
	pdfRendering bool
	pdfGen       int

	// web is a browser block's web view, at page zoom webZoom; address is
	// what its address bar holds. webGen counts its closings: a web view
	// that finishes starting after one is destroyed.
	web         *mygo.WebView
	webErr      error
	webStarting bool
	webZoom     float32
	webGen      int
	// While the canvas zooms, webSnap is a picture of the page, shown in
	// its place (zoomPage); webSeenS the zoom of the last frame, and
	// webZoomRest and webRedrawn when the page takes the zoom and shows.
	webSnap *ui.Bitmap
	// webSnapOld is the picture the last zoom took, shown while the next
	// is taken: the layer can show as a bitmap then (layerCached).
	webSnapOld   *ui.Bitmap
	webCapturing bool
	webSeenS     float32
	webZoomRest  time.Time
	webRedrawn   time.Time
	webLoading   bool
	webFailed    string
	address      string
	canBack      bool
	canForward   bool

	term     *terminal.Terminal
	termErr  error
	starting bool
	// termGen counts hang-ups: a terminal that finishes starting after one
	// is closed at once, not left running unseen.
	termGen int
	// termUsed says the user typed or pasted into the terminal: only then
	// does closing the deck keep what it shows (keepTerminals).
	termUsed bool
	termFont float32 // the font size the terminal was last given
	// termEpoch is the fontEpoch the terminal was last given its font in.
	termEpoch int

	// A 笔记's text as edited (memo) and as its file holds it (memoSaved),
	// read once memoLoaded; memoMod is the file's time as last read or
	// written, memoCheck when it was last looked at, memoSeen and
	// memoEdited the text as the last frame saw it and when it changed.
	memo        string
	memoSaved   string
	memoSeen    string
	memoLoaded  bool
	memoErr     error
	memoSaveErr error
	memoSaving  bool
	memoMod     time.Time
	memoCheck   time.Time
	memoEdited  time.Time

	// liveScroll is how far a QuickLook's or 笔记's live preview is
	// scrolled (livePreview).
	liveScroll ui.ScrollState

	// stackPick is the card of a 块堆 pushed out, or last out: the index
	// into its items this frame; stackOut says it is out now. stackScroll
	// is how far its cards are scrolled, in cards. stackFocus is its card,
	// which takes the focus as it is clicked; stackHadFocus says it has
	// had it since it opened (losing it puts the card back).
	stackPick     int
	stackOut      bool
	stackScroll   float32
	stackFocus    ui.Handle
	stackHadFocus bool
}

func newRuntime() *runtime { return &runtime{} }

// workspace is what deck.json holds.
type workspace struct {
	Version int      `json:"version"`
	Root    string   `json:"root"`
	CamX    float32  `json:"camX"`
	CamY    float32  `json:"camY"`
	CamS    float32  `json:"camS"`
	Blocks  []*block `json:"blocks"`
}

// camera shows the world point (X, Y) at the window's top-left corner, S
// times as large.
type camera struct {
	X float32 `json:"x"`
	Y float32 `json:"y"`
	S float32 `json:"s"`
}

func (c camera) toScreen(wx, wy float32) (float32, float32) {
	return (wx - c.X) * c.S, (wy - c.Y) * c.S
}

func (c camera) toWorld(sx, sy float32) (float32, float32) {
	return sx/c.S + c.X, sy/c.S + c.Y
}

type point struct{ X, Y float32 }

const (
	minScale = 0.08
	maxScale = 4
	// lodScale is the scale below which a block shows its frame alone: no
	// children, no editor, no terminal.
	lodScale = 0.3
)

// deck is the app's state: the window, the workspace and the camera.
type deck struct {
	win *mygo.Window
	// clock is the time QuickLook blocks go by (lifespan.go): the wall
	// clock when nil. lifeAsked is the block whose time-left menu was
	// last asked for.
	clock     func() time.Time
	lifeAsked *block
	root      string // the deck's folder: file blocks' paths are relative to it
	file      string
	// onHome closes the deck for the home screen; nil in tests.
	onHome func()
	// The workspace menu of the home pill: the folders opened lately,
	// the latest first (this one among them), opening one of them, and
	// asking for another. Nil in tests that leave them out.
	workspaces func() []string
	onOpen     func(path string)
	onPick     func()

	cam camera
	// lay is the camera the blocks were laid out with, the same as cam
	// but while a zoom goes on (zoomlayer.go); zoomSettle is when that
	// zoom rests.
	lay        camera
	zoomSettle time.Time
	// zoomAnchor is the point of the window the last zoom was about;
	// snap is the zoom's glide to a zoom level once it rests.
	zoomAnchor point
	snap       zoomSnap
	// perf is what the performance log measures of a frame (perflog.go).
	perf perfFrame
	// dragBand is the height of the band along the top whose empty canvas
	// drags the window (titlebar.go): zero without one.
	dragBand float32
	// loads are the blocks' loads, waiting or running (loadqueue.go).
	loads loadQueue
	// viewW and viewH are the window's size, in DIPs, as the last frame
	// built it.
	viewW, viewH float32
	blocks       []*block
	sel          map[string]bool

	panning bool
	panFrom point
	panCam  camera

	dragging  *block
	grab      point // the pointer in world coordinates when the press came
	dragOff   point // the block's top-left from the pointer at the press
	dragFrom  point // the pointer at the press, in window DIPs
	dragAt    point // the pointer now, in window DIPs
	dragMoved bool  // the pointer went further than a click's jitter

	// resizing is the block being resized by its corner: from its size
	// resizeStart, by the pointer's way from resizeFrom, in world units.
	resizing    *block
	resizeFrom  point
	resizeStart point

	// termZoomBlock is the terminal block ⌘ and the wheel last zoomed over,
	// whose cover takes the wheel for the canvas until termZoomUntil.
	termZoomBlock *block
	termZoomUntil time.Time

	// focusBlock is the ID of the block whose terminal or text has the
	// keyboard focus, "" when none has: only it takes the wheel.
	focusBlock string
	// active is the ID of the block the user works in: the one whose
	// focus (focusBlock) was last in it. It stays when the focus goes out
	// of the window's own content, as to a web page clicked, whose native
	// view then has the keyboard; only a press on the canvas or on another
	// block ends it. Its body takes the wheel (bodyCover).
	active string
	// canvasScrollUntil is when the canvas's last scroll stops being a
	// gesture under way (gestureHold).
	canvasScrollUntil time.Time

	marquee marquee

	// editing is the block whose title, or QuickLook body, shows an input,
	// nil when none does; editInput is that input. editHad says it has had
	// the focus (losing it ends the edit), editSeen that this frame built it;
	// editFrom is the title as the edit started.
	editing   *block
	editPart  editPart
	editFrom  string
	editInput ui.Handle
	editHad   bool
	editSeen  bool
	// previewStack is the 块堆 whose long preview (space) is open, nil
	// when none is.
	previewStack *block
	// stackOpen is the 块堆 a click opened: hovering it pushes its cards
	// out and scrolls them (stack.go), until it loses the selection or
	// the focus. Nil when none is.
	stackOpen *block

	// md shows the QuickLook bodies' Markdown, in a theme made for the
	// size and the light or dark mdSize and mdDark say.
	md     markdown.Options
	mdSize float32
	mdDark bool
	// previewMD is the same for a 块堆's long preview, at its own size.
	previewMD     markdown.Options
	previewMDSize float32
	previewMDDark bool

	undo []change
	redo []change

	// mcp is the workspace's MCP endpoint, nil while none runs; mcpProject
	// is the project-level config it wrote and loaded, mcpErr why it could
	// not start.
	mcp        *mcpServer
	mcpProject *mcpWorkspace
	mcpErr     error
	// projectMCP says to write the project-level MCP configs into the
	// folder (设置 → 项目级配置); without it agents come through the
	// global install.
	projectMCP bool

	// saveMu and saveSeq keep the saves in order: a snapshot marshalled on
	// the UI thread is written by a goroutine, and only the newest one is.
	saveMu  sync.Mutex
	saveSeq uint64
}

type marquee struct {
	on   bool
	from point
	to   point
}

func newDeck() *deck {
	d := &deck{sel: map[string]bool{}, blocks: []*block{}}
	d.cam.S = 1
	return d
}

func (d *deck) setWindow(w *mygo.Window) { d.win = w }

func (c *camera) clamp() {
	c.S = min(max(c.S, minScale), maxScale)
}

// zoomAt zooms by factor, keeping the world point under (sx, sy) where it
// is on the screen.
func (d *deck) zoomAt(sx, sy float32, factor float32) {
	if factor <= 0 {
		return
	}
	d.zoomAnchor = point{sx, sy}
	wx, wy := d.cam.toWorld(sx, sy)
	d.cam.S *= factor
	d.cam.clamp()
	d.cam.X = wx - sx/d.cam.S
	d.cam.Y = wy - sy/d.cam.S
}

// changeKind says what a change did, for undo.
type changeKind uint8

const (
	changeAdd changeKind = iota
	changeRemove
	changeMoves
	changeStack
)

// change is one undoable edit of the deck: a block added, removed, a set
// of blocks moved or laid out at once, or what stacks hold.
type change struct {
	kind   changeKind
	block  *block
	index  int
	sel    map[string]bool
	moves  []moveChange
	stacks []stackEdit
}

// stackEdit is one 块堆's items before and after a change.
type stackEdit struct {
	stack  *block
	before []string
	after  []string
}

// geometry is where a block is, and how large: what an undo of a move puts
// back.
type geometry struct {
	X, Y, W, H float32
}

// moveChange is one block of a move, before and after.
type moveChange struct {
	block  *block
	before geometry
	after  geometry
}

// placement is where a layout puts a block, in world coordinates. Only the
// top-left moves; a layout never resizes.
type placement struct {
	block *block
	x, y  float32
}

// applyPlacements moves blocks as one undoable change, and saves when
// anything moved. It returns how many blocks moved.
func (d *deck) applyPlacements(ps []placement) int {
	moves := make([]moveChange, 0, len(ps))
	for _, p := range ps {
		b := p.block
		if b.X == p.x && b.Y == p.y {
			continue
		}
		moves = append(moves, moveChange{
			block:  b,
			before: geometry{X: b.X, Y: b.Y, W: b.W, H: b.H},
			after:  geometry{X: p.x, Y: p.y, W: b.W, H: b.H},
		})
	}
	if len(moves) == 0 {
		return 0
	}
	d.forgetRedo()
	d.undo = append(d.undo, change{kind: changeMoves, moves: moves})
	for _, m := range moves {
		m.block.X, m.block.Y = m.after.X, m.after.Y
	}
	d.requestSave()
	return len(moves)
}

// addBlock puts a block on the canvas and selects it. It is the only way a
// block joins the deck, so every addition undoes.
func (d *deck) addBlock(b *block) *block {
	if b.ID == "" {
		b.ID = d.newID()
	}
	if b.W <= 0 {
		b.W = 360
	}
	if b.H <= 0 {
		b.H = 260
	}
	d.forgetRedo()
	d.undo = append(d.undo, change{kind: changeAdd, block: b, index: len(d.blocks), sel: d.copySel()})
	d.blocks = append(d.blocks, b)
	d.sel = map[string]bool{b.ID: true}
	return b
}

// removeBlock takes a block off the canvas, and hangs up its terminal: a
// block undone back gets a new one when it shows again.
func (d *deck) removeBlock(b *block) {
	i := d.indexOf(b)
	if i < 0 {
		return
	}
	d.forgetRedo()
	d.undo = append(d.undo, change{kind: changeRemove, block: b, index: i, sel: d.copySel()})
	d.detach(i)
	delete(d.sel, b.ID)
}

// deleteSelected removes the selected blocks, the last first, so that the
// indices the changes keep stay true.
func (d *deck) deleteSelected() {
	for i := len(d.blocks) - 1; i >= 0; i-- {
		if d.sel[d.blocks[i].ID] {
			d.removeBlock(d.blocks[i])
		}
	}
}

// undoLast reverses the last addition or removal.
func (d *deck) undoLast() {
	n := len(d.undo)
	if n == 0 {
		return
	}
	c := d.undo[n-1]
	d.undo = d.undo[:n-1]
	switch c.kind {
	case changeAdd:
		if i := d.indexOf(c.block); i >= 0 {
			d.detach(i)
		}
		d.sel = c.sel
	case changeRemove:
		d.attach(c.index, c.block)
		d.sel = map[string]bool{c.block.ID: true}
		d.rescueNote(c.block) // a QuickLook that went gets more time
	case changeMoves:
		for i := len(c.moves) - 1; i >= 0; i-- {
			m := c.moves[i]
			m.block.X, m.block.Y = m.before.X, m.before.Y
			m.block.W, m.block.H = m.before.W, m.before.H
		}
	case changeStack:
		d.restoreStackEdits(c.stacks, false)
	}
	d.redo = append(d.redo, c)
}

// redoLast applies again what undoLast reversed.
func (d *deck) redoLast() {
	n := len(d.redo)
	if n == 0 {
		return
	}
	c := d.redo[n-1]
	d.redo = d.redo[:n-1]
	switch c.kind {
	case changeAdd:
		d.attach(c.index, c.block)
		d.sel = map[string]bool{c.block.ID: true}
	case changeRemove:
		if i := d.indexOf(c.block); i >= 0 {
			d.detach(i)
		}
		delete(d.sel, c.block.ID)
	case changeMoves:
		for _, m := range c.moves {
			m.block.X, m.block.Y = m.after.X, m.after.Y
			m.block.W, m.block.H = m.after.W, m.after.H
		}
	case changeStack:
		d.restoreStackEdits(c.stacks, true)
	}
	d.undo = append(d.undo, c)
}

// attach puts b back at index.
func (d *deck) attach(index int, b *block) {
	if index < 0 || index > len(d.blocks) {
		index = len(d.blocks)
	}
	d.blocks = append(d.blocks, nil)
	copy(d.blocks[index+1:], d.blocks[index:])
	d.blocks[index] = b
}

// detach takes the block at index off the canvas and returns it.
func (d *deck) detach(index int) *block {
	b := d.blocks[index]
	d.blocks = append(d.blocks[:index], d.blocks[index+1:]...)
	if d.previewStack == b {
		d.previewStack = nil
	}
	d.hangUp(b)
	return b
}

func (d *deck) indexOf(b *block) int {
	for i, other := range d.blocks {
		if other == b {
			return i
		}
	}
	return -1
}

// blockByID is the block with this ID, nil when the deck has none.
func (d *deck) blockByID(id string) *block {
	for _, n := range d.blocks {
		if n.ID == id {
			return n
		}
	}
	return nil
}

func (d *deck) copySel() map[string]bool {
	out := make(map[string]bool, len(d.sel))
	for id := range d.sel {
		out[id] = true
	}
	return out
}

// forgetRedo drops what redoLast would redo: the blocks it added stay off
// the canvas for good, so their terminals hang up.
func (d *deck) forgetRedo() {
	for _, c := range d.redo {
		if c.kind == changeAdd {
			d.hangUp(c.block)
		}
	}
	d.redo = nil
}

// hangUp lets go of what a block holds outside the app: its terminal's
// shell, its PDF, its web view.
func (d *deck) hangUp(b *block) {
	if b.rt == nil {
		return
	}
	d.flushMemo(b)
	b.rt.closePDF()
	b.rt.closeBrowser()
	b.rt.termGen++ // one still starting is closed as it comes
	b.rt.starting = false
	if b.rt.term != nil {
		b.rt.term.Close()
		b.rt.term = nil
	}
}

func (d *deck) closeAll() {
	for _, b := range d.blocks {
		d.hangUp(b)
	}
}

func (d *deck) newID() string {
	return fmt.Sprintf("b%06x", rand.IntN(1<<24))
}

// freeSpot returns a place near (x, y), in world coordinates, where a block
// of w×h overlaps no other: new blocks never land exactly on old ones.
func (d *deck) freeSpot(x, y, w, h float32) (float32, float32) {
	for range 64 {
		if !d.overlaps(x, y, w, h) {
			return x, y
		}
		x += 28
		y += 28
	}
	return x, y
}

func (d *deck) overlaps(x, y, w, h float32) bool {
	hidden := d.stackedIDs()
	for _, b := range d.blocks {
		if hidden[b.ID] {
			continue
		}
		if x < b.X+b.W && x+w > b.X && y < b.Y+b.H && y+h > b.Y {
			return true
		}
	}
	return false
}

// dropFiles adds a file block for each dropped file: where it was dropped,
// in window DIPs.
func (d *deck) dropFiles(paths []string, x, y float32) int {
	wx, wy := d.cam.toWorld(x, y)
	added := 0
	for _, p := range paths {
		abs, err := filepath.Abs(p)
		if err != nil {
			continue
		}
		fi, err := os.Stat(abs)
		if err != nil || fi.IsDir() {
			continue // a folder is no block yet
		}
		w, h := fileBlockSize(abs)
		// The block centers on the pointer, and steps aside only when
		// another one is already there.
		px, py := d.freeSpot(wx-w/2, wy-h/2, w, h)
		d.addBlock(&block{
			Kind:  kindFile,
			Title: filepath.Base(abs),
			Path:  d.linkPath(abs),
			X:     px,
			Y:     py,
			W:     w,
			H:     h,
		})
		wx += 28
		wy += 28
		added++
	}
	return added
}

// fileBlockSize is the size a block linking the file starts at: a page,
// upright, for a PDF or a Word document; wider for others.
func fileBlockSize(path string) (w, h float32) {
	switch previewOf(path) {
	case previewPDF, previewWord:
		return 440, 580
	}
	return 560, 420
}

// raise brings the block to the top, over the others: the canvas draws
// the blocks in order, the last on top, and saves them so.
func (d *deck) raise(n *block) {
	for i, b := range d.blocks {
		if b == n {
			if i < len(d.blocks)-1 {
				d.blocks = append(append(d.blocks[:i:i], d.blocks[i+1:]...), n)
			}
			return
		}
	}
}

// What of a block shows outside its box, at 100%: its washi tape above
// it (tapeOf: up to 0.8 of its 30 high, tilted up to 4°), its shadow
// around it.
const (
	blockReachTop  = 28
	blockReachSide = 10
)

// visible returns the blocks this frame builds, back to front: those the
// window shows some of, their tape and shadow included, and no other. A
// block out of view builds nothing and asks for no load (loadqueue.go);
// a 块堆's blocks are its own (stackBody).
func (d *deck) visible(vw, vh float32) []*block {
	hidden := d.stackedIDs()
	out := make([]*block, 0, len(d.blocks))
	for _, n := range d.blocks {
		if hidden[n.ID] {
			continue
		}
		sx, sy := d.cam.toScreen(n.X, n.Y)
		w, h := n.W*d.cam.S, n.H*d.cam.S
		top, side := blockReachTop*d.cam.S, blockReachSide*d.cam.S
		if sx+w+side <= 0 || sy+h+side <= 0 || sx-side >= vw || sy-top >= vh {
			continue
		}
		out = append(out, n)
	}
	return out
}

// path resolves a block's path against the workspace root.
func (d *deck) path(n *block) string {
	if filepath.IsAbs(n.Path) {
		return n.Path
	}
	return filepath.Join(d.root, n.Path)
}

// linkPath stores p relative to the workspace root when it is inside it, so
// a workspace moves with the files it links.
func (d *deck) linkPath(p string) string {
	rel, err := filepath.Rel(d.root, p)
	if err != nil || rel == ".." || (len(rel) > 2 && rel[:3] == ".."+string(filepath.Separator)) {
		return p
	}
	return rel
}

// deckFile is where the deck is saved: in its folder's .boundlessdeck, or
// where a test put it.
func (d *deck) deckFile() string {
	if d.file != "" {
		return d.file
	}
	if d.root == "" {
		return ""
	}
	return filepath.Join(d.root, deckDir, "deck.json")
}

// load reads the deck saved in its folder; a folder with none gets a new
// deck, with a note on how to use it.
func (d *deck) load() {
	data, err := os.ReadFile(d.deckFile())
	if err != nil {
		d.seed()
		return
	}
	var w workspace
	if err := json.Unmarshal(data, &w); err != nil {
		d.seed()
		return
	}
	// The folder the deck is in is its root, wherever it moved; the root
	// saved counts only for a deck kept outside one.
	if d.root == "" {
		d.root = w.Root
	}
	d.blocks = w.Blocks
	d.cam = camera{X: w.CamX, Y: w.CamY, S: w.CamS}
	if d.cam.S == 0 {
		d.cam.S = 1
	}
	for _, n := range d.blocks {
		if n.Kind == "" {
			n.Kind = kindNote
		}
	}
	d.dropExpired()
	d.pruneStacks()
}

// workspaceSnapshot is the deck as deck.json holds it.
func (d *deck) workspaceSnapshot() *workspace {
	return &workspace{
		Version: 1,
		Root:    d.root,
		CamX:    d.cam.X,
		CamY:    d.cam.Y,
		CamS:    d.cam.S,
		Blocks:  d.blocks,
	}
}

// save writes the workspace atomically, here and now: a temporary file,
// then a rename.
func (d *deck) save() { d.writeSave(true) }

// requestSave writes the workspace without holding the UI thread for the
// disk: the state is marshalled here, while it is steady, and written from
// a goroutine. Only the newest snapshot is written, and never two at once.
func (d *deck) requestSave() { d.writeSave(false) }

func (d *deck) writeSave(wait bool) {
	path := d.deckFile()
	if path == "" {
		return
	}
	data, err := json.MarshalIndent(d.workspaceSnapshot(), "", "  ")
	if err != nil {
		return
	}
	d.saveMu.Lock()
	d.saveSeq++
	seq := d.saveSeq
	d.saveMu.Unlock()
	write := func() {
		d.saveMu.Lock()
		defer d.saveMu.Unlock()
		if seq != d.saveSeq {
			return // a newer snapshot is there to write
		}
		_ = writeFileAtomic(path, data)
	}
	if wait || d.win == nil {
		write()
		return
	}
	go write()
}

// writeFileAtomic writes data to path through a temporary file and a
// rename, so that a reader never sees half a file.
func writeFileAtomic(path string, data []byte) error {
	if err := os.MkdirAll(filepath.Dir(path), 0o755); err != nil {
		return err
	}
	tmp := path + ".tmp"
	if err := os.WriteFile(tmp, data, 0o644); err != nil {
		return err
	}
	return os.Rename(tmp, path)
}

func (d *deck) seed() {
	d.addBlock(&block{
		Kind:  kindNote,
		Title: "开始",
		X:     80,
		Y:     80,
		W:     420,
		H:     250,
		Text: "- 双指滑动平移，⌘ + 滚轮或捏合缩放，⌘0 显示全部\n" +
			"- 拖顶部的胶带移动，拖右下角等比缩放\n" +
			"- 点胶带、框选或 Shift + 点击来选中，⌘D 删除\n" +
			"- 右上角新建 QuickLook、笔记、链接文件、打开终端\n" +
			"- 左上角回到主页，画布会保存在这个文件夹里",
	})
	d.sel = map[string]bool{}
}
