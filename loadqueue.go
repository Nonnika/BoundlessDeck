package main

import (
	"cmp"
	"math"
	"slices"
)

// The blocks' loads (reading a file, decoding a picture, opening and
// rendering a PDF, starting a shell or a web view) do not all start at
// once, as a zoom that brings many cards into view would have them do:
// they wait in the deck's loadQueue, and loadLimit of them run at a time,
// those of the cards nearest the middle of the view first, however alike
// the cards' sizes. The order is made anew each frame, from where the view
// is then; a load whose card left the view (or went below lodScale) before
// it started is dropped, and wanted again when the card comes back.

// loadLimit is how many loads run at once.
const loadLimit = 3

// loadJob is a load waiting to start. start runs on the main thread as it
// starts, to mark it under way; work runs off it and returns what to do
// back on the main thread with its result.
type loadJob struct {
	key   string
	n     *block
	start func()
	work  func() (apply func())
	frame int // the frame it was last wanted in
}

type loadQueue struct {
	frame   int
	queued  map[string]*loadJob
	running map[string]bool
	// started lists the keys in the order they started; tests read it.
	started []string
}

// wantLoad asks for a load of n, by key, unless one by that key waits or
// runs: a load waiting takes the newest work, as a PDF page to render
// at the newest size. View code asks for it each frame it still wants it.
func (d *deck) wantLoad(key string, n *block, start func(), work func() (apply func())) {
	q := &d.loads
	if q.running[key] {
		return
	}
	if q.queued == nil {
		q.queued = map[string]*loadJob{}
	}
	if j := q.queued[key]; j != nil {
		j.start, j.work, j.frame = start, work, q.frame
		return
	}
	q.queued[key] = &loadJob{key: key, n: n, start: start, work: work, frame: q.frame}
}

// loadPriority orders the loads: the distance from the middle of the
// view, in world units, to the card, nothing for the card under it; then
// to the card's middle, between cards the middle is in or as far from.
func loadPriority(n *block, cx, cy float32) (edge, middle float64) {
	dx := max(n.X-cx, 0, cx-(n.X+n.W))
	dy := max(n.Y-cy, 0, cy-(n.Y+n.H))
	mx, my := n.X+n.W/2-cx, n.Y+n.H/2-cy
	return math.Hypot(float64(dx), float64(dy)), math.Hypot(float64(mx), float64(my))
}

// runLoads drops the loads the frame no longer wanted and starts the
// nearest of the others while fewer than loadLimit run, reporting whether
// it started any. The view calls it once its cards are built, with the
// middle of the view in the world.
func (d *deck) runLoads(cx, cy float32) (started bool) {
	q := &d.loads
	defer func() { q.frame++ }()
	if len(q.queued) == 0 {
		return false
	}
	waiting := make([]*loadJob, 0, len(q.queued))
	for key, j := range q.queued {
		if j.frame != q.frame {
			delete(q.queued, key) // its card went: wanted again if it comes back
			continue
		}
		waiting = append(waiting, j)
	}
	// A block in a 块堆 shows where its stack is, not where it was left.
	holder := map[string]*block{}
	for _, st := range d.blocks {
		if st.Kind == kindStack {
			for _, id := range st.Items {
				holder[id] = st
			}
		}
	}
	at := func(n *block) *block {
		if st := holder[n.ID]; st != nil {
			return st
		}
		return n
	}
	slices.SortFunc(waiting, func(a, b *loadJob) int {
		ae, am := loadPriority(at(a.n), cx, cy)
		be, bm := loadPriority(at(b.n), cx, cy)
		if ae != be {
			return cmp.Compare(ae, be)
		}
		if am != bm {
			return cmp.Compare(am, bm)
		}
		return cmp.Compare(a.key, b.key)
	})
	for _, j := range waiting {
		if len(q.running) >= loadLimit {
			break
		}
		delete(q.queued, j.key)
		d.startLoad(j)
		started = true
	}
	return started
}

// startLoad runs a load: off the main thread, its result back on it; here
// and now in tests, which have no window to hand results to.
func (d *deck) startLoad(j *loadJob) {
	q := &d.loads
	q.started = append(q.started, j.key)
	if j.start != nil {
		j.start()
	}
	if d.win == nil {
		j.work()()
		return
	}
	if q.running == nil {
		q.running = map[string]bool{}
	}
	q.running[j.key] = true
	win := d.win
	go func() {
		apply := j.work()
		win.Update(func() {
			delete(q.running, j.key)
			apply()
		})
	}()
}
