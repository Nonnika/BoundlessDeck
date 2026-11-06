package main

import (
	"fmt"
	"os"
	"path/filepath"
	"testing"

	"github.com/egoist/mygo/ui"
)

// gridDeck is a deck of 5×5 cards all alike, text files 200×150 apart by
// 40, and the id of the one in the middle.
func gridDeck(t *testing.T) (*deck, string) {
	t.Helper()
	d := testDeck(t)
	d.blocks = nil
	for row := range 5 {
		for col := range 5 {
			id := fmt.Sprintf("r%dc%d", row, col)
			name := id + ".txt"
			if err := os.WriteFile(filepath.Join(d.root, name), []byte(id), 0o644); err != nil {
				t.Fatal(err)
			}
			d.blocks = append(d.blocks, &block{ID: id, Kind: kindFile, Title: name, Path: name,
				X: float32(col) * 240, Y: float32(row) * 190, W: 200, H: 150})
		}
	}
	return d, "r2c2"
}

// lookAt centers the view, w×h, on the middle of block id.
func lookAt(d *deck, id string, w, h float32) {
	n := d.blockByID(id)
	d.cam.X = n.X + n.W/2 - w/2/d.cam.S
	d.cam.Y = n.Y + n.H/2 - h/2/d.cam.S
}

// Cards alike load the middlemost first, then outward.
func TestLoadsStartFromTheMiddleOfTheView(t *testing.T) {
	d, middle := gridDeck(t)
	d.cam.S = 1
	lookAt(d, middle, 1600, 1200)
	tt := ui.NewTester(d.view, 1600, 1200)
	tt.Frame()

	started := d.loads.started
	if len(started) != 25 {
		t.Fatalf("%d loads started, want 25: %v", len(started), started)
	}
	if started[0] != "text:"+middle {
		t.Fatalf("the first load is %s, not the middle card's", started[0])
	}
	mx, my := d.cam.toWorld(800, 600)
	last := -1.0
	for _, key := range started {
		n := d.blockByID(key[len("text:"):])
		_, dist := loadPriority(n, mx, my)
		if dist < last-1e-6 {
			t.Fatalf("%s loads after a card farther from the middle: %v", key, started)
		}
		last = dist
	}
}

// While the loads run at their limit, the others wait; those whose card
// leaves the view are dropped, and, back, the middlemost of the view as it
// is then starts first.
func TestWaitingLoadsFollowTheView(t *testing.T) {
	d, middle := gridDeck(t)
	d.cam.S = 1
	lookAt(d, middle, 1600, 1200)
	// Loads at their limit, before the first frame (NewTester builds one).
	d.loads.running = map[string]bool{}
	for i := range loadLimit {
		d.loads.running[fmt.Sprint("busy", i)] = true
	}
	tt := ui.NewTester(d.view, 1600, 1200)
	tt.Frame()
	if len(d.loads.started) != 0 || len(d.loads.queued) != 25 {
		t.Fatalf("at the limit: %d started, %d waiting", len(d.loads.started), len(d.loads.queued))
	}

	d.cam.X, d.cam.Y = 50000, 50000 // nothing in view
	tt.Frame()
	if len(d.loads.queued) != 0 {
		t.Fatalf("%d loads still wait for cards out of view", len(d.loads.queued))
	}

	lookAt(d, "r0c0", 1600, 1200)
	clear(d.loads.running)
	tt.Frame()
	if len(d.loads.started) == 0 || d.loads.started[0] != "text:r0c0" {
		t.Fatalf("looking at the corner, loads started %v", d.loads.started)
	}
}
