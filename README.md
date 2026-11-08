# BoundlessDeck-RS

English · [简体中文](docs/README.zh-CN.md)

An infinite-canvas workbench, natively drawn with [GPUI](https://www.gpui.rs/). It is the Rust rewrite of BoundlessDeck (Go). A folder is a deck: the canvas is saved in `<folder>/.boundlessdeck/deck.json`, in the same format the Go version reads, so either can open the other's decks.

## Ideas

### 1. Everything is a block

Notes, Markdown files, documents, terminals, web pages, charts and stacks are all **blocks**: one model with a position, a size and a lifecycle. They move, resize, stack, select, undo and save the same way. They are also described to agents by the same field table, so anything you can put on the canvas, an agent can read and change too.

### 2. Agents drive the canvas

An open deck runs an MCP endpoint. Agents don't just answer in a chat: they work on the canvas. They create and edit blocks, lay them out, align and distribute them, select them, and move your view to show what they did. Every change an agent makes goes through the same undo stack as yours, so ⌘Z takes it back.

### 3. The environment is the context

You don't have to explain where you are. The deck folder is the workspace. A shell started in a terminal block knows which block it is (`BOUNDLESSDECK_TERMINAL`) and which deck it belongs to (`BOUNDLESSDECK_DECK`), and so does an agent launched from it. What you have selected, which terminal you are working in, and what sits next to what are all available to the agent (`read_selection`, `current_terminal`, `deck_info`). The way you arrange the canvas is the prompt.

## Getting started

```sh
cargo run                    # home screen
cargo run -- <folder>        # open that folder as a deck
cargo test
cargo bundle --release       # build the .app (needs cargo-bundle)
```

When trying things out, point `BOUNDLESSDECK_DATA_DIR` and `BOUNDLESSDECK_ACTIVE_FILE` at a temporary directory so your real data stays untouched.

## What's on the canvas

- **QuickLook** sticky notes (they expire), **notes** (Markdown files with live rendering and clickable checkboxes), and **file** previews (text, Markdown, images, PDF, Word, video)
- **Terminals** (alacritty_terminal, with input method support), **browsers** (WKWebView, with tabs), **charts**, and **stacks**
- **Text** and **shapes** (rectangle, ellipse, diamond, triangle) drawn right on the canvas, styled from a panel on the right: colors, any installed font through the system Fonts panel, corners and alignment
- Pan, pinch or ⌘-scroll to zoom, marquee select, undo and redo; text is re-laid out at every zoom level, so it stays sharp
- A first-run welcome that introduces the main features (open it again from Settings), and a choice of accent colors

## For agents

Opening a deck starts an MCP endpoint, and `boundlessdeck mcp-stdio` is the stdio bridge to it. There are 19 tools for reading, writing and arranging blocks. In Settings (⌘,) you can install the global config for Claude Code, Claude Desktop, Cursor, VS Code, Codex, OpenCode and oh-my-pi with one click.

## Windows

There is no Windows installer; build it yourself. Install [Rust](https://rustup.rs/) 1.85 or newer, then run `cargo build --release` in the repository. The binary is `target\release\boundlessdeck.exe`. Features that rely on macOS system APIs, such as PDF preview, pinch zoom, web page snapshots and blur, are missing or simplified on Windows.

## Not there yet

- Terminals can't select text with the mouse and don't report mouse events
- The text editor has no undo of its own

Development notes (in Chinese) are in `AGENTS.md`. Licensed under MIT.
