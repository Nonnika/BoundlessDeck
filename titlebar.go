package main

import "github.com/egoist/mygo/ui"

// The window has no title bar of its own (mygo.TitleBarHidden): the
// canvas and the home screen run up to its top edge, under the traffic
// lights, which sit level with the deck's top row (trafficLights).
//
// A band along the top, as tall as the lights' room (ui.Context.TitleBar,
// zero in full screen and in tests), drags the window where nothing else
// is: on the deck, empty canvas there drags it, and blocks over it take
// their presses as anywhere else.

// chromeTop is how far from the top the deck's top row of chrome (the
// home pill, the toolbar) is, and chromeHeight how tall it is.
const (
	chromeTop    = 12
	chromeHeight = 37
)

// trafficLights is where the close button's top-left corner goes, from
// the window's: level with the deck's top row, the lights being 14 tall.
var trafficLights = struct{ X, Y int }{20, chromeTop + (chromeHeight-14)/2}

// dragBand builds the band that drags the window, painted under what
// follows it. A press drags the window only when the band is the topmost
// element under it: every element is a hit target, its children aside,
// unless PassThrough or IgnoreInput, so on the deck the block layer over
// it must stay PassThrough (canvas.go). Tests cannot see a window drag
// start (the headless host ignores it): check by hand after changing what
// lies over the band.
func dragBand(c *ui.Context, height float32) {
	if height <= 0 {
		return
	}
	ui.Box(c.Key("drag-band")).Absolute().Left(0).Top(0).FillWidth().Height(height).DragWindow()
}

// chromeLeft is how far from the left the deck's top-left chrome starts:
// clear of the traffic lights.
func chromeLeft(c *ui.Context) float32 {
	if bar := c.TitleBar(); bar.Left > 0 {
		return bar.Left + 12
	}
	return 12
}
