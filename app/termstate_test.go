package main

import (
	"bytes"
	"io"
	"os"
	"path/filepath"
	"strings"
	"testing"
	"time"

	"github.com/egoist/mygo/plugins/terminal"
)

// quietConn is a connection that never says anything, for a terminal
// that runs no program: what is fed to it is all it shows.
type quietConn struct{ r *io.PipeReader }

func newQuietConn() *quietConn {
	r, _ := io.Pipe()
	return &quietConn{r}
}
func (q *quietConn) Read(p []byte) (int, error)  { return q.r.Read(p) }
func (q *quietConn) Write(p []byte) (int, error) { return len(p), nil }
func (q *quietConn) Close() error                { return q.r.Close() }

// quietTerminal is a terminal running no program, or a skip where the
// terminal's library cannot load.
func quietTerminal(t *testing.T) *terminal.Terminal {
	t.Helper()
	if err := terminal.Load(); err != nil {
		t.Skipf("no libghostty-vt: %v", err)
	}
	term, err := terminal.New(terminal.Options{Conn: newQuietConn()})
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { term.Close() })
	term.Resize(80, 24)
	return term
}

func TestLeavingADeckKeepsWhatItsTerminalsShowed(t *testing.T) {
	a := testApp(t)
	dir := t.TempDir()
	a.openFolder(dir)
	b := a.deck.addAt(kindTerminal, 300, 200)
	term := quietTerminal(t)
	term.Feed([]byte("$ make build\r\n\x1b[32mok\x1b[0m  boundlessdeck\r\n$ "))
	rt := a.deck.ensureRuntime(b)
	rt.term, rt.termUsed = term, true // typed into
	a.goHome()

	if b.rt.term != nil {
		t.Fatal("leaving the deck did not hang the shell up")
	}
	// Opened again, from what was saved.
	a.openFolder(dir)
	var back *block
	for _, n := range a.deck.blocks {
		if n.ID == b.ID {
			back = n
		}
	}
	if back == nil || back.Term == nil {
		t.Fatal("the terminal block kept nothing in deck.json")
	}
	if back.rt != nil && back.rt.term != nil {
		t.Fatal("the deck came back with a terminal running")
	}

	// A new terminal fed what it kept shows it, and the line saying so.
	fresh := quietTerminal(t)
	fresh.Feed(restored(back.Term))
	shows := fresh.Text()
	for _, want := range []string{"$ make build", "ok  boundlessdeck", "已恢复"} {
		if !strings.Contains(shows, want) {
			t.Fatalf("the new terminal shows %q, missing %q", shows, want)
		}
	}
}

func TestANewShellStartsWhereTheOldOneWas(t *testing.T) {
	root := t.TempDir()
	sub := filepath.Join(root, "src")
	if err := os.Mkdir(sub, 0o755); err != nil {
		t.Fatal(err)
	}
	for _, c := range []struct {
		st   *termState
		want string
	}{
		{nil, root},
		{&termState{Dir: "src"}, sub},
		{&termState{Dir: sub}, sub},
		{&termState{Dir: "gone"}, root}, // removed since: the root
	} {
		if got := termDir(root, c.st); got != c.want {
			t.Errorf("termDir(%+v) = %s, want %s", c.st, got, c.want)
		}
	}
}

func TestKeptHistoryIsCappedAtARowsStart(t *testing.T) {
	var h bytes.Buffer
	for h.Len() < 3*maxHistory {
		h.WriteString("\x1b[1;34mrow of output\x1b[0m\r\n")
	}
	got := lastRows(h.Bytes(), maxHistory)
	if len(got) > maxHistory+8 {
		t.Fatalf("kept %d bytes, want about %d", len(got), maxHistory)
	}
	if !bytes.HasPrefix(got, []byte("\x1b[0m\x1b[1;34mrow")) {
		t.Fatalf("kept history starts %q, want a whole row", got[:24])
	}
	if short := []byte("a\r\nb"); !bytes.Equal(lastRows(short, maxHistory), short) {
		t.Fatal("a short history was cut")
	}
	if restored(&termState{SavedAt: time.Now()}) != nil {
		t.Fatal("a block that kept nothing shows a restored line")
	}
}

func TestATerminalLeftAloneKeepsNothingNew(t *testing.T) {
	a := testApp(t)
	dir := t.TempDir()
	a.openFolder(dir)
	b := a.deck.addAt(kindTerminal, 300, 200)
	term := quietTerminal(t)
	term.Feed([]byte("Last login: Thu Nov  5 17:00\r\n$ "))
	a.deck.ensureRuntime(b).term = term // never typed into
	a.goHome()
	a.openFolder(dir)
	if n := a.deck.blocks[len(a.deck.blocks)-1]; n.Term != nil {
		t.Fatalf("a terminal never used kept %q", n.Term.History)
	}

	// Restored, then left alone: it keeps what it was restored from, not
	// that plus its new prompt and another restored line.
	kept := &termState{History: []byte("$ make\r\nok\r\n"), SavedAt: time.Now()}
	n := a.deck.blocks[len(a.deck.blocks)-1]
	n.Term = kept
	again := quietTerminal(t)
	again.Feed(restored(kept))
	again.Feed([]byte("$ "))
	a.deck.ensureRuntime(n).term = again
	a.goHome()
	a.openFolder(dir)
	n = a.deck.blocks[len(a.deck.blocks)-1]
	if n.Term == nil || !bytes.Equal(n.Term.History, kept.History) {
		t.Fatalf("left alone, it kept %+v; want what it had", n.Term)
	}
}
