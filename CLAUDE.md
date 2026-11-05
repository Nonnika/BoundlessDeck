# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## What this is

BoundlessDeck is an infinite-canvas workbench (local file links/previews, code, terminals) written in pure Go on **MyGo** (`github.com/egoist/mygo`, pinned in `go.mod`). The UI is native: MyGo's `ui` package draws it directly — no WebView page, no npm, no JS bindings. The project used to be a Vite/TypeScript web frontend; that was removed on purpose, so don't reintroduce it. macOS 13+ (terminals use libghostty-vt).

Product/architecture docs (Chinese): `docs/requirements.md` (v0.2 native architecture, requirement IDs and status, verified MyGo API references) and `docs/agent-workbench.md` (v0.3 agent-driven product requirements).

## Commands

```sh
go tool mygo dev        # run in dev mode; restarts on .go changes
go tool mygo build      # package into build/<os>-<arch>/ (config in mygo.json)
go test ./...           # canvas + persistence unit tests, headless (no window)
go test -race ./...
go test -run TestZoomKeepsThePointUnderThePointer .   # single test
go tool mygo vet .      # build-time lifecycle checks for native UI (stored elements, goroutine captures)
DECK_SNAPSHOT=1 go test -run TestSnapshot .           # re-render docs/canvas-snapshot.png
```

When changing UI code, also follow `.agents/skills/mygo-maintenance/SKILL.md` (MyGo lifetime/identity/threading rules; it suggests `CGO_ENABLED=0 go test ./...` too). Check APIs against the pinned MyGo source: `go list -m -f '{{.Dir}}' github.com/egoist/mygo`.

## Terminology

Every element on the deck is a **block** (note, file, terminal… are block *kinds*). Never name things `node` or `card` in code; the data file key is `"blocks"`.

## Architecture

Single `package main`:

- `main.go` — creates the `app`, opens the window with `ui.View(a.view)`, wires file drop and save-on-close.
- `home.go` — `app`: the home screen (open a folder, recent folders kept in `recents.json` in MyGo's user-data dir) or the `deck` of the folder open. Each deck is a folder; switching decks, going home or closing the window keeps the state of each terminal the user typed or pasted into (`runtime.termUsed`; one left alone keeps what its block had) in its block (`termstate.go`: `History()` of what it showed, at most 256 KB, and the shell's dir, saved in deck.json as `term`), saves the deck and hangs the shells up — nothing keeps running. Opened again, a terminal block starts a new shell in that dir with the old output fed in above it under a dim "已恢复" line. A terminal that finishes starting after its block was hung up closes itself (`runtime.termGen`).
- `homescene.go` — the home screen's background: one dotted canvas plane leaning back in 3D (`Element.Transform` + `Perspective`), blocks on it blurred by depth (`Element.Blur`: near blurry, far sharp; only ≥ 960 wide), the deck mark. The scene and the column are both `Absolute` (absolute elements paint above in-flow siblings). Render with `DECK_SNAPSHOT=1 go test -run TestHomeSnapshot .`
- `termstate.go` — what a terminal block keeps of its terminal across closing the deck (history + dir), and where its next shell starts.
- `deck.go` — state and model: `deck` (camera, blocks, selection, drag/pan/marquee state, undo/redo stacks), `block` (serialized, world coordinates), `runtime` (per-block non-persisted state: loaded text, `editor.State`, bitmap, `terminal.Terminal`), camera math (`toScreen`/`toWorld`/`zoomAt`), add/remove/undo/redo, and `deck.json` load/save.
- `canvas.go` — the per-frame view: grid, canvas input (pan/zoom/marquee), block input (select, title-bar drag), toolbar, status bar, fit-all.
- `blocks.go` — one block's view and its body per kind (text via `ui/editor`, image, terminal via `plugins/terminal`), plus off-thread loaders. Images that may be see-through (PNG alpha/tRNS, WebP alpha, SVG) show on a checkerboard.
- `preview.go` — how a file block previews by extension (`previewOf`): `.md` and Word rendered with `ui/markdown` in a scroller (a `hasScroller` body), PDFs a page at a time with a ‹ n / N › pager (`block.Page` is saved), re-rendered in 256 px steps as the zoom changes. Controls inside a block body must take presses with their own `HandleInput`: the card's handler takes them before a `ui.Button`.
- `browser.go` — the browser block (`kindBrowser`, `block.URL` saved): a native `ui.WebView` under a bar (back/forward/reload/address). The page zooms with the canvas via `Page.SetZoomFactor(cam.S)`, but only once the zoom rests: WebKit's page zoom re-lays out and redraws, lagging behind the block, so while the canvas zooms the page shows as a `CapturePage` snapshot scaled with the block (`zoomPage`; held at its old size until the snapshot arrives), and the live page returns after it has redrawn at the new zoom; pages are `Untrusted`, which alone would block every cross-origin navigation (even a redirect such as www.bing.com → cn.bing.com), so an `OnWillNavigate` listener allows web schemes (`webScheme`) and the web view starts blank and loads its URL only after the listeners are on; a user-clicked new-window link opens a browser block beside it. The card *and* the canvas decline presses in a browser's body (`inBrowserBody`) so its `ui.Button`s and `TextInput` get them; the `bodyCover` starts below the bar. In tests there is no window, so the page shows `errNoWindow`.
- `pdf_darwin.go` — PDF pages rendered by CoreGraphics through purego (no cgo); `pdf_other.go` stubs it elsewhere.
- `word.go` — `.docx` → Markdown in pure Go (headings, also hand-sized bold ones; bold/italic; bullet and numbered lists via numbering.xml; links; tables; pictures kept by name); `.doc` goes through macOS `textutil -convert docx` first. Literal `[`, `]`, `$` are written as character references: `\[…\]` would read as math.

Key design constraints (explained in `docs/requirements.md` §2):

- **Real zoom, not transforms.** `ui.WebView` can't be scaled, so the canvas multiplies block X/Y/W/H (and font sizes) by `cam.S` and re-lays out each frame instead of using a Scale transform. Text/editors/terminals stay crisp.
- **Immediate mode.** The whole tree rebuilds every frame. Only blocks in view (plus a margin) are built (`visible`), and below `lodScale` (0.3) a block shows just its frame and title — no editor/terminal/bitmap. Heavy state lives in `block.rt`, never in view locals.
- **One handler per press.** A press goes to the innermost handler (editor/terminal take clicks inside them); wheel/zoom events walk the chain, so blocks forward wheel and pinch back to the canvas so scrolling over a block still moves the canvas.
- **Threading.** File reads, image decoding, terminal start and dialogs (`mygo.Dialog.Open`) run in goroutines; results come back via `win.Update`. No I/O inside view code.
- **Undo.** `addBlock`/`removeBlock` are the only ways a block enters or leaves the deck, so every addition/removal is undoable. Removing a block hangs up its terminal; an undone-back block starts a new one.
- **Persistence.** A deck lives in its folder at `.boundlessdeck/deck.json`, written atomically (tmp + rename) when the deck closes (going home, opening another folder, closing the window). The folder is the deck's root: terminals start there and file paths inside it are stored relative to it (`linkPath`), so the folder can move.

Tests drive the real view headlessly with `ui.NewTester` (`Pinch`, `Wheel`, `Press`/`Move`/`Release`, `Image`); `testDeck(t)` in `canvas_test.go` gives a deck with a temp root and two notes.
