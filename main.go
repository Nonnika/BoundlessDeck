// BoundlessDeck is an infinite canvas workbench in native UI: MyGo draws it
// itself, from Go, with no web page. Each deck is a folder: the home
// screen opens one, and the deck is saved inside it.
package main

import (
	"log"

	"github.com/egoist/mygo"
	"github.com/egoist/mygo/ui"
)

func main() {
	perfInit() // DECK_PERFLOG, DECK_CPUPROFILE (perflog.go)
	a := newApp()
	mygo.App.WhenReady(func() {
		win := mygo.NewWindow(mygo.WindowOptions{
			Title:           "BoundlessDeck",
			Width:           1280,
			Height:          800,
			MinWidth:        680,
			MinHeight:       420,
			BackgroundColor: "light-dark(#f6f7f9, #0f1115)",
			StateKey:        "main",
			// The canvas runs up under the traffic lights (titlebar.go).
			TitleBarStyle:        mygo.TitleBarHidden,
			TrafficLightPosition: &mygo.Point{X: trafficLights.X, Y: trafficLights.Y},
			Content:              ui.View(a.view),
		})
		a.win = win
		a.loadRecents()
		// A folder dropped on the home screen opens as a deck; files
		// dropped on a deck become blocks where they landed.
		win.OnFileDrop(func(e *mygo.FileDropEvent) {
			win.Update(func() { a.dropped(e.Paths, float32(e.X), float32(e.Y)) })
		})
		win.OnClosed(a.closeDeck)
	})
	if err := mygo.App.Run(); err != nil {
		log.Fatal(err)
	}
}
