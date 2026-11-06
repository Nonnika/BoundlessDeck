package main

import (
	"bytes"
	"os"
	"path/filepath"
	"time"
)

// termState is what a terminal block keeps of its terminal while the deck
// is closed: what it showed and where its shell was. The shell itself is
// hung up; opened again, the block starts a new one there, under what the
// old one showed, as a terminal restoring a session does.
type termState struct {
	// History is what the terminal showed, as rows of VT sequences
	// (terminal.History): its scrollback and screen, without the cursor
	// and the modes programs set. At most maxHistory bytes, the latest.
	History []byte `json:"history,omitempty"`
	// Dir is where the shell was: relative to the workspace root when it
	// is inside it.
	Dir     string    `json:"dir,omitempty"`
	SavedAt time.Time `json:"savedAt"`
}

// maxHistory is how much of a terminal's history a block keeps, in bytes:
// the deck saves it in deck.json.
const maxHistory = 256 << 10

// keepTerminals keeps what each terminal of the deck shows and where its
// shell is in its block, to save it, before the shells are hung up: of the
// terminals the user typed or pasted into. One left alone, showing just
// its shell's prompt or what it was restored to, keeps what its block had,
// so that opening and closing a deck piles up nothing.
func (d *deck) keepTerminals() {
	for _, b := range d.blocks {
		d.keepTerminal(b)
	}
}

// keepTerminal keeps what b's terminal shows and where its shell is, to
// start a new one from: of a terminal the user typed or pasted into. One
// left alone, showing just its shell's prompt or what it was restored to,
// keeps what its block had, so that opening and closing a deck piles up
// nothing.
func (d *deck) keepTerminal(b *block) {
	if b.Kind != kindTerminal || b.rt == nil || b.rt.term == nil || !b.rt.termUsed {
		return
	}
	st := &termState{History: lastRows(b.rt.term.History(), maxHistory), SavedAt: time.Now()}
	if dir := b.rt.term.Dir(); dir != "" {
		st.Dir = d.linkPath(dir)
	}
	b.Term = st
}

// lastRows returns the end of history, at most max bytes, cut at the start
// of a row so that no escape sequence is cut in two.
func lastRows(history []byte, max int) []byte {
	if len(history) <= max {
		return history
	}
	tail := history[len(history)-max:]
	if i := bytes.IndexByte(tail, '\n'); i >= 0 {
		tail = tail[i+1:]
	}
	// Rows start in the default style: what came before set none.
	return append([]byte("\x1b[0m"), tail...)
}

// termDir is where a terminal block's new shell starts, for a deck rooted
// at root: where the old one was (st) if that folder is still there, the
// root otherwise. It reads the disk: call it off the main thread.
func termDir(root string, st *termState) string {
	if st == nil || st.Dir == "" {
		return root
	}
	dir := st.Dir
	if !filepath.IsAbs(dir) {
		dir = filepath.Join(root, dir)
	}
	if info, err := os.Stat(dir); err != nil || !info.IsDir() {
		return root
	}
	return dir
}

// restored is what a block's new terminal shows first: what the old one
// showed, then a faint line saying when it was left, the new shell's
// prompt under it. Nothing for a block that kept nothing.
func restored(st *termState) []byte {
	if st == nil || len(st.History) == 0 {
		return nil
	}
	var b bytes.Buffer
	b.Write(st.History)
	b.WriteString("\x1b[0m\r\n\x1b[2m── 已恢复 · ")
	b.WriteString(st.SavedAt.Local().Format("2006-01-02 15:04"))
	b.WriteString(" ──\x1b[0m\r\n")
	return b.Bytes()
}
