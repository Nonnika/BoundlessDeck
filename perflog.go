package main

import (
	"fmt"
	"log"
	"os"
	"path/filepath"
	goruntime "runtime"
	"runtime/pprof"
	"sort"
	"strings"
	"time"
)

// A performance log, for finding what makes a frame slow: with
// DECK_PERFLOG=file, each frame of the deck writes how long its view took
// to build, how long since the last, the zoom's state and the blocks in
// view; zoom events and the zoom's rests are written as they come. MyGo's
// own frame statistics (MYGO_FRAME_STATS=all: build, layout, paint and
// present of each frame) go to the same file, by the standard logger.
// Unset, nothing is measured.

var perfOn bool

// perfProfile is the CPU profile DECK_CPUPROFILE asked for, nil without;
// perfProfiled says it was taken.
var (
	perfProfile  *os.File
	perfProfiled bool
)

// perfProfileTime is how long the profile runs from the first zoom.
const perfProfileTime = 15 * time.Second

// perfStart starts the CPU profile of DECK_CPUPROFILE, if set, as the
// first zoom comes; perfProfileTime later perfStop ends it and writes what
// was allocated meanwhile beside it (.allocs).
func perfStart() {
	path := os.Getenv("DECK_CPUPROFILE")
	if path == "" || perfProfiled {
		return
	}
	perfProfiled = true
	f, err := os.Create(path)
	if err != nil {
		log.Printf("DECK_CPUPROFILE: %v", err)
		return
	}
	goruntime.MemProfileRate = 64 << 10
	if err := pprof.StartCPUProfile(f); err != nil {
		log.Printf("DECK_CPUPROFILE: %v", err)
		f.Close()
		return
	}
	perfProfile = f
	perff("profiling for %v", perfProfileTime)
	time.AfterFunc(perfProfileTime, perfStop)
}

func perfStop() {
	if perfProfile == nil {
		return
	}
	pprof.StopCPUProfile()
	perfProfile.Close()
	if f, err := os.Create(perfProfile.Name() + ".allocs"); err == nil {
		pprof.Lookup("allocs").WriteTo(f, 0)
		f.Close()
	}
	perff("profile written to %s", perfProfile.Name())
	perfProfile = nil
}

// perfInit sends the standard logger to DECK_PERFLOG, if set.
func perfInit() {
	path := os.Getenv("DECK_PERFLOG")
	if path == "" {
		return
	}
	f, err := os.Create(path)
	if err != nil {
		log.Printf("DECK_PERFLOG: %v", err)
		return
	}
	log.SetOutput(f)
	log.SetFlags(log.Ltime | log.Lmicroseconds)
	perfOn = true
}

// perff writes a line to the performance log, when it is on.
func perff(format string, args ...any) {
	if perfOn {
		log.Printf("deck: "+format, args...)
	}
}

// perfFrame is what the deck's view measures of a frame.
type perfFrame struct {
	start, last time.Time
	loads       int // loads started before it
}

// perfBegin starts measuring a frame of the deck's view.
func (d *deck) perfBegin() {
	if !perfOn {
		return
	}
	d.perf.start = time.Now()
	d.perf.loads = len(d.loads.started)
}

// perfEnd writes the frame: how long the view took, the gap since the
// last frame, the cameras and the blocks built, by kind.
func (d *deck) perfEnd(built []*block) {
	if !perfOn {
		return
	}
	now := time.Now()
	gap := float64(0)
	if !d.perf.last.IsZero() {
		gap = ms(d.perf.start.Sub(d.perf.last))
	}
	d.perf.last = d.perf.start
	state := "rest"
	switch {
	case d.snap.on:
		state = "snap"
	case now.Before(d.zoomSettle):
		state = "gesture"
	}
	kinds := map[string]int{}
	for _, n := range built {
		k := string(n.Kind)
		if n.Kind == kindFile {
			k = "file:" + strings.TrimPrefix(filepath.Ext(n.Path), ".")
		}
		if d.lay.S < lodScale {
			k += "(far)"
		}
		kinds[k]++
	}
	names := make([]string, 0, len(kinds))
	for k, v := range kinds {
		names = append(names, fmt.Sprintf("%s×%d", k, v))
	}
	sort.Strings(names)
	started := d.loads.started[d.perf.loads:]
	cached := ""
	if d.layerMoved() {
		cached = " moved"
		if d.layerCached(built) {
			cached = " cached"
		}
	}
	perff("view %.1f ms, gap %.1f ms, %s, cam %.3f lay %.3f layer ×%.3f%s, %d built [%s], loads started %v",
		ms(now.Sub(d.perf.start)), gap, state, d.cam.S, d.lay.S, d.layerScale(), cached,
		len(built), strings.Join(names, " "), started)
}

func ms(t time.Duration) float64 { return t.Seconds() * 1000 }
