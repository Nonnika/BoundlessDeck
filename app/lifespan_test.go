package main

import (
	"encoding/json"
	"os"
	"path/filepath"
	"testing"
	"time"

	"github.com/egoist/mygo/ui"
)

// clockAt makes d's clock read from *now.
func clockAt(d *deck, now *time.Time) { d.clock = func() time.Time { return *now } }

func TestNewQuickLookGoesAfterAnHour(t *testing.T) {
	d := testDeck(t)
	now := time.Date(2026, 11, 6, 9, 0, 0, 0, time.Local)
	clockAt(d, &now)
	n := d.addAt(kindNote, 300, 200)
	d.stopEditing()
	if want := now.Add(noteLife).UnixMilli(); n.Expires != want {
		t.Fatalf("a new QuickLook expires at %d, want an hour on (%d)", n.Expires, want)
	}
	if other := d.addAt(kindTerminal, 600, 400); other.Expires != 0 {
		t.Fatal("a terminal was given a time to go")
	}

	tt := ui.NewTester(d.view, 900, 600)
	tt.Frame()
	if d.indexOf(n) < 0 {
		t.Fatal("the QuickLook went before its time")
	}
	now = now.Add(noteLife)
	tt.Frame()
	if d.indexOf(n) >= 0 {
		t.Fatal("the QuickLook stayed past its time")
	}
	if d.blockByID("a") == nil {
		t.Fatal("a QuickLook kept for good went")
	}

	// ⌘Z brings it back, with time to save it.
	d.undoLast()
	if d.indexOf(n) < 0 {
		t.Fatal("undo did not bring the QuickLook back")
	}
	if at, _ := expiresAt(n); at.Sub(now) != noteRescue {
		t.Fatalf("brought back with %v left, want %v", at.Sub(now), noteRescue)
	}
	tt.Frame()
	if d.indexOf(n) < 0 {
		t.Fatal("the QuickLook brought back went again at once")
	}
}

func TestQuickLookBeingWrittenWaitsForTheEditToEnd(t *testing.T) {
	d := testDeck(t)
	now := time.Now()
	clockAt(d, &now)
	n := d.blockByID("a")
	n.Expires = now.Add(time.Minute).UnixMilli()
	d.startEditing(n, editBody)
	now = now.Add(2 * time.Minute)
	d.expireNotes()
	if d.indexOf(n) < 0 {
		t.Fatal("the QuickLook went while it was written in")
	}
	d.stopEditing()
	d.expireNotes()
	if d.indexOf(n) >= 0 {
		t.Fatal("the QuickLook stayed past its time once the edit ended")
	}
}

func TestSetLifeAndTimeLeft(t *testing.T) {
	d := testDeck(t)
	now := time.Now().Truncate(time.Millisecond)
	clockAt(d, &now)
	n := d.blockByID("a")
	d.setLife(n, 3*time.Hour)
	if at, ok := expiresAt(n); !ok || at.Sub(now) != 3*time.Hour {
		t.Fatalf("setLife 3h: %v %v", at, ok)
	}
	d.setLife(n, 0)
	if _, ok := expiresAt(n); ok || n.Expires != 0 {
		t.Fatal("setLife 0 did not keep the block")
	}

	for left, want := range map[time.Duration]string{
		30 * time.Second:             "< 1 分钟",
		90 * time.Second:             "2 分钟",
		time.Hour:                    "60 分钟",
		90*time.Minute + time.Second: "2 小时",
		47 * time.Hour:               "47 小时",
		7 * 24 * time.Hour:           "7 天",
	} {
		if got := lifeLeft(left); got != want {
			t.Errorf("lifeLeft(%v) = %q, want %q", left, got, want)
		}
	}

	n.Expires = now.Add(time.Hour).UnixMilli()
	if o := d.noteOpacity(n); o != 1 {
		t.Fatalf("an hour left shows at %v, want 1", o)
	}
	n.Expires = now.Add(noteFade / 2).UnixMilli()
	if o := d.noteOpacity(n); o <= noteFadeTo || o >= 1 {
		t.Fatalf("halfway through its fade it shows at %v", o)
	}
}

func TestQuickLookThatWentWhileClosedIsGoneOnOpen(t *testing.T) {
	root := t.TempDir()
	past := time.Now().Add(-time.Minute).UnixMilli()
	later := time.Now().Add(time.Hour).UnixMilli()
	w := workspace{Version: 1, CamS: 1, Blocks: []*block{
		{ID: "gone", Kind: kindNote, Title: "过期", W: 200, H: 120, Expires: past},
		{ID: "stays", Kind: kindNote, Title: "还在", W: 200, H: 120, Expires: later},
		{ID: "kept", Kind: kindNote, Title: "永久", W: 200, H: 120},
		{ID: "st", Kind: kindStack, Title: "块堆", W: 380, H: 300, Items: []string{"gone", "stays"}},
	}}
	data, _ := json.Marshal(w)
	if err := os.MkdirAll(filepath.Join(root, deckDir), 0o755); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(filepath.Join(root, deckDir, "deck.json"), data, 0o644); err != nil {
		t.Fatal(err)
	}
	d := newDeck()
	d.root = root
	d.load()
	if d.blockByID("gone") != nil {
		t.Fatal("a QuickLook whose time ran out while the deck was closed is still there")
	}
	if d.blockByID("stays") == nil || d.blockByID("kept") == nil {
		t.Fatal("a QuickLook with time left went on open")
	}
	if items := d.blockByID("st").Items; len(items) != 1 || items[0] != "stays" {
		t.Fatalf("the stack still holds %v", items)
	}
}

func TestMCPQuickLookExpires(t *testing.T) {
	s, d := mcpTestServer(t)
	out := toolCall(t, s, "create_block", map[string]any{"kind": "note", "text": "临时"})
	n := d.blockByID(out["id"].(string))
	if at, ok := expiresAt(n); !ok || time.Until(at) < 59*time.Minute || time.Until(at) > noteLife {
		t.Fatalf("an MCP note expires at %v (%v), want in an hour", at, ok)
	}
	if out["expiresAt"] == nil {
		t.Fatalf("no expiresAt in %v", out)
	}
	out = toolCall(t, s, "create_block", map[string]any{"kind": "note", "expires": 0})
	if n := d.blockByID(out["id"].(string)); n.Expires != 0 {
		t.Fatal("expires 0 did not keep the note")
	}
	if _, isErr, _ := toolCallRaw(t, s, "create_block", map[string]any{"kind": "note", "expires": -5}); !isErr {
		t.Fatal("a negative expires was taken")
	}
	if _, isErr, _ := toolCallRaw(t, s, "create_block", map[string]any{"kind": "terminal", "expires": 5}); !isErr {
		t.Fatal("a terminal took expires")
	}
}

func TestMCPQuickLookTTLMinutes(t *testing.T) {
	s, d := mcpTestServer(t)
	now := time.Now().Truncate(time.Millisecond)
	clockAt(d, &now)
	out := toolCall(t, s, "create_block", map[string]any{"kind": "note", "ttlMinutes": 30})
	id := out["id"].(string)
	n := d.blockByID(id)
	if at, ok := expiresAt(n); !ok || at.Sub(now) != 30*time.Minute {
		t.Fatalf("ttlMinutes 30 expires at %v (%v), want 30 minutes on", at, ok)
	}
	if out["ttlMinutes"] != 30.0 {
		t.Fatalf("ttlMinutes reads %v, want 30", out["ttlMinutes"])
	}

	now = now.Add(10*time.Minute + 30*time.Second)
	out = toolCall(t, s, "update_block", map[string]any{"id": id, "title": "改个名"})
	if out["ttlMinutes"] != 19.5 {
		t.Fatalf("ttlMinutes reads %v after 10.5 minutes, want 19.5", out["ttlMinutes"])
	}

	out = toolCall(t, s, "update_block", map[string]any{"id": id, "ttlMinutes": 1.5})
	if at, _ := expiresAt(n); at.Sub(now) != 90*time.Second {
		t.Fatalf("ttlMinutes 1.5 gave %v", at.Sub(now))
	}
	toolCall(t, s, "update_block", map[string]any{"id": id, "ttlMinutes": 0})
	if n.Expires != 0 {
		t.Fatal("ttlMinutes 0 did not keep the note")
	}

	for _, bad := range []map[string]any{
		{"kind": "note", "ttlMinutes": -1},
		{"kind": "note", "ttlMinutes": 5, "expires": 0},
		{"kind": "terminal", "ttlMinutes": 5},
	} {
		if _, isErr, _ := toolCallRaw(t, s, "create_block", bad); !isErr {
			t.Fatalf("create_block %v was taken", bad)
		}
	}
}
