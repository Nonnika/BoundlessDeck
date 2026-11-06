package main

import (
	"fmt"
	"math"
	"sort"
	"strings"
)

// Layout is the canvas's own geometry, exposed to the MCP: aligning blocks
// on an edge or center line, spreading them evenly, and packing them into a
// row, a column or a grid. Every layout goes through applyPlacements, so it
// is one undoable change (⌘Z), like moving a block by hand.

// mcpRect is a block's box in world coordinates.
type mcpRect struct {
	X, Y, W, H float32
}

func rectOf(b *block) mcpRect { return mcpRect{X: b.X, Y: b.Y, W: b.W, H: b.H} }

func (r mcpRect) right() float32   { return r.X + r.W }
func (r mcpRect) bottom() float32  { return r.Y + r.H }
func (r mcpRect) centerX() float32 { return r.X + r.W/2 }
func (r mcpRect) centerY() float32 { return r.Y + r.H/2 }

// boundsOf is the box around every block.
func boundsOf(blocks []*block) mcpRect {
	if len(blocks) == 0 {
		return mcpRect{}
	}
	minX, minY := blocks[0].X, blocks[0].Y
	maxX, maxY := blocks[0].X+blocks[0].W, blocks[0].Y+blocks[0].H
	for _, b := range blocks[1:] {
		minX, minY = min(minX, b.X), min(minY, b.Y)
		maxX, maxY = max(maxX, b.X+b.W), max(maxY, b.Y+b.H)
	}
	return mcpRect{X: minX, Y: minY, W: maxX - minX, H: maxY - minY}
}

// mcpLayoutBlocks is the set a layout works on: the ids given, or the
// selection when none are: 选择即上下文. Duplicates are dropped, and an
// unknown id fails the call before anything moves.
func (d *deck) mcpLayoutBlocks(args mcpArgs) ([]*block, error) {
	ids, err := args.strings("ids")
	if err != nil {
		return nil, err
	}
	if len(ids) == 0 {
		ids = d.mcpSelectedIDs()
	}
	if len(ids) == 0 {
		return nil, fmt.Errorf("没有指定 block：给 ids，或先在画布上选中几个")
	}
	out := make([]*block, 0, len(ids))
	seen := map[string]bool{}
	var missing []string
	for _, id := range ids {
		if seen[id] {
			continue
		}
		seen[id] = true
		n := d.blockByID(id)
		if n == nil {
			missing = append(missing, id)
			continue
		}
		out = append(out, n)
	}
	if len(missing) > 0 {
		return nil, fmt.Errorf("没有这些 block：%s", strings.Join(missing, ", "))
	}
	return out, nil
}

// normalizeAlign names the alignment an agent asked for.
func normalizeAlign(s string) (string, error) {
	switch strings.ToLower(strings.TrimSpace(s)) {
	case "left", "start":
		return "left", nil
	case "right", "end":
		return "right", nil
	case "top":
		return "top", nil
	case "bottom":
		return "bottom", nil
	case "centerx", "center-x", "hc", "hcenter", "horizontal-center", "xcenter":
		return "centerX", nil
	case "centery", "center-y", "vc", "vcenter", "vertical-center", "middle", "ycenter":
		return "centerY", nil
	}
	return "", fmt.Errorf("align 要是 left、right、top、bottom、centerX 或 centerY，收到 %q", s)
}

// normalizeAxis names the axis distribute works along.
func normalizeAxis(s string) (string, error) {
	switch strings.ToLower(strings.TrimSpace(s)) {
	case "", "x", "h", "horizontal", "row":
		return "x", nil
	case "y", "v", "vertical", "column":
		return "y", nil
	}
	return "", fmt.Errorf("axis 要是 x 或 y，收到 %q", s)
}

// normalizeLayout names the arrangement arrange_blocks makes.
func normalizeLayout(s string) (string, error) {
	switch strings.ToLower(strings.TrimSpace(s)) {
	case "", "grid":
		return "grid", nil
	case "row", "line", "horizontal":
		return "row", nil
	case "column", "col", "vertical", "stack":
		return "column", nil
	}
	return "", fmt.Errorf("layout 要是 row、column 或 grid，收到 %q", s)
}

// mcpAlignBlocks puts the blocks on one edge or center line, optionally
// stacking them along the other axis with a spacing.
func (d *deck) mcpAlignBlocks(args mcpArgs) (map[string]any, error) {
	blocks, err := d.mcpLayoutBlocks(args)
	if err != nil {
		return nil, err
	}
	alignText, ok, err := args.str("align")
	if err != nil {
		return nil, err
	}
	if !ok {
		return nil, fmt.Errorf("align_blocks 需要 align：left、right、top、bottom、centerX 或 centerY")
	}
	align, err := normalizeAlign(alignText)
	if err != nil {
		return nil, err
	}
	to, _, err := args.str("to")
	if err != nil {
		return nil, err
	}
	spacing, hasSpacing, err := args.num("spacing")
	if err != nil {
		return nil, err
	}
	if hasSpacing && spacing < 0 {
		return nil, fmt.Errorf("spacing 不能是负数，收到 %v", spacing)
	}

	// The reference: the set's own box, its first or last block, or a
	// block outside the set to line up against.
	var ref mcpRect
	var refBlock *block
	switch strings.ToLower(strings.TrimSpace(to)) {
	case "", "bounds", "set":
		if len(blocks) < 2 {
			return nil, fmt.Errorf("按 bounds 对齐需要至少两个 block；对齐到一个 block 用 to")
		}
		ref = boundsOf(blocks)
	case "first":
		if len(blocks) < 2 {
			return nil, fmt.Errorf("按 first 对齐需要至少两个 block")
		}
		refBlock = blocks[0]
		ref = rectOf(refBlock)
	case "last":
		if len(blocks) < 2 {
			return nil, fmt.Errorf("按 last 对齐需要至少两个 block")
		}
		refBlock = blocks[len(blocks)-1]
		ref = rectOf(refBlock)
	default:
		refBlock = d.blockByID(to)
		if refBlock == nil {
			return nil, fmt.Errorf("to 要是 bounds、first、last 或一个 block ID，收到 %q", to)
		}
		ref = rectOf(refBlock)
	}

	horizontal := align == "left" || align == "right" || align == "centerX"
	target := make(map[*block]placement, len(blocks))
	for _, b := range blocks {
		target[b] = placement{block: b, x: b.X, y: b.Y}
	}
	for _, b := range blocks {
		if b == refBlock {
			continue
		}
		p := target[b]
		switch align {
		case "left":
			p.x = ref.X
		case "right":
			p.x = ref.right() - b.W
		case "centerX":
			p.x = ref.centerX() - b.W/2
		case "top":
			p.y = ref.Y
		case "bottom":
			p.y = ref.bottom() - b.H
		case "centerY":
			p.y = ref.centerY() - b.H/2
		}
		target[b] = p
	}

	if hasSpacing {
		// Stack along the other axis, in the order the blocks are in from
		// the reference's leading edge.
		ordered := append([]*block(nil), blocks...)
		sort.SliceStable(ordered, func(i, j int) bool {
			a, b := ordered[i], ordered[j]
			if horizontal {
				if a.Y != b.Y {
					return a.Y < b.Y
				}
				return a.X < b.X
			}
			if a.X != b.X {
				return a.X < b.X
			}
			return a.Y < b.Y
		})
		var cursor float32
		if horizontal {
			cursor = ref.Y
		} else {
			cursor = ref.X
		}
		if refBlock != nil {
			if horizontal {
				cursor = refBlock.Y
			} else {
				cursor = refBlock.X
			}
		}
		for _, b := range ordered {
			p := target[b]
			if horizontal {
				p.y = cursor
				cursor += b.H + float32(spacing)
			} else {
				p.x = cursor
				cursor += b.W + float32(spacing)
			}
			target[b] = p
		}
	}

	ps := make([]placement, 0, len(blocks))
	for _, b := range blocks {
		ps = append(ps, target[b])
	}
	moved := d.applyPlacements(ps)
	toLabel := "bounds"
	if refBlock != nil {
		toLabel = refBlock.ID
	}
	out := map[string]any{
		"align":  align,
		"to":     toLabel,
		"moved":  moved,
		"blocks": mcpGeometryList(blocks),
	}
	if hasSpacing {
		out["spacing"] = spacing
	}
	return out, nil
}

// mcpDistributeBlocks spreads blocks evenly along an axis, the outermost
// two staying where they are.
func (d *deck) mcpDistributeBlocks(args mcpArgs) (map[string]any, error) {
	blocks, err := d.mcpLayoutBlocks(args)
	if err != nil {
		return nil, err
	}
	axisText, _, err := args.str("axis")
	if err != nil {
		return nil, err
	}
	axis, err := normalizeAxis(axisText)
	if err != nil {
		return nil, err
	}
	modeText, _, err := args.str("mode")
	if err != nil {
		return nil, err
	}
	mode := "gaps"
	switch strings.ToLower(strings.TrimSpace(modeText)) {
	case "", "gaps", "gap", "space":
	case "centers", "center":
		mode = "centers"
	default:
		return nil, fmt.Errorf("mode 要是 gaps 或 centers，收到 %q", modeText)
	}
	allowOverlap, _, err := args.boolean("allowOverlap")
	if err != nil {
		return nil, err
	}
	if len(blocks) < 3 {
		return nil, fmt.Errorf("distribute_blocks 需要至少三个 block（首尾保持不动），收到 %d 个", len(blocks))
	}

	horizontal := axis == "x"
	// Centers mode orders by center, gaps mode by leading edge: the order
	// the outer blocks keep is the order the calls are measured in.
	key := func(b *block) float32 {
		switch {
		case horizontal && mode == "centers":
			return rectOf(b).centerX()
		case horizontal:
			return b.X
		case mode == "centers":
			return rectOf(b).centerY()
		}
		return b.Y
	}
	cross := func(b *block) float32 {
		if horizontal {
			return b.Y
		}
		return b.X
	}
	ordered := append([]*block(nil), blocks...)
	sort.SliceStable(ordered, func(i, j int) bool {
		if a, b := key(ordered[i]), key(ordered[j]); a != b {
			return a < b
		}
		return cross(ordered[i]) < cross(ordered[j])
	})
	first, last := ordered[0], ordered[len(ordered)-1]
	target := make(map[*block]placement, len(ordered))
	for _, b := range ordered {
		target[b] = placement{block: b, x: b.X, y: b.Y}
	}
	gap, step := float32(0), float32(0)
	if mode == "gaps" {
		var span, sizes float32
		if horizontal {
			span = (last.X + last.W) - first.X
			for _, b := range ordered {
				sizes += b.W
			}
		} else {
			span = (last.Y + last.H) - first.Y
			for _, b := range ordered {
				sizes += b.H
			}
		}
		gap = (span - sizes) / float32(len(ordered)-1)
		if gap < 0 && !allowOverlap {
			return nil, fmt.Errorf("按当前首尾排不下：block 共占 %.0f，首尾跨度 %.0f；允许重叠传 allowOverlap=true", sizes, span)
		}
		cursor := first.X
		if !horizontal {
			cursor = first.Y
		}
		for _, b := range ordered {
			p := target[b]
			if horizontal {
				p.x = cursor
				cursor += b.W + gap
			} else {
				p.y = cursor
				cursor += b.H + gap
			}
			target[b] = p
		}
	} else {
		var firstCenter, lastCenter float32
		if horizontal {
			firstCenter, lastCenter = rectOf(first).centerX(), rectOf(last).centerX()
		} else {
			firstCenter, lastCenter = rectOf(first).centerY(), rectOf(last).centerY()
		}
		step = (lastCenter - firstCenter) / float32(len(ordered)-1)
		for i, b := range ordered {
			c := firstCenter + step*float32(i)
			p := target[b]
			if horizontal {
				p.x = c - b.W/2
			} else {
				p.y = c - b.H/2
			}
			target[b] = p
		}
	}

	ps := make([]placement, 0, len(ordered))
	for _, b := range ordered {
		ps = append(ps, target[b])
	}
	moved := d.applyPlacements(ps)
	out := map[string]any{
		"axis":   axis,
		"mode":   mode,
		"moved":  moved,
		"blocks": mcpGeometryList(ordered),
	}
	if mode == "gaps" {
		out["gap"] = roundCoord(gap)
	} else {
		out["step"] = roundCoord(step)
	}
	return out, nil
}

// mcpArrangeBlocks packs blocks into a row, a column or a grid, top-left
// first, keeping each block's size.
func (d *deck) mcpArrangeBlocks(args mcpArgs) (map[string]any, error) {
	blocks, err := d.mcpLayoutBlocks(args)
	if err != nil {
		return nil, err
	}
	layoutText, _, err := args.str("layout")
	if err != nil {
		return nil, err
	}
	layout, err := normalizeLayout(layoutText)
	if err != nil {
		return nil, err
	}
	gapX, hasGapX, err := args.num("gapX")
	if err != nil {
		return nil, err
	}
	gapY, hasGapY, err := args.num("gapY")
	if err != nil {
		return nil, err
	}
	if !hasGapX {
		gapX = 24
	}
	if !hasGapY {
		gapY = 24
	}
	if gapX < 0 || gapY < 0 {
		return nil, fmt.Errorf("gapX 与 gapY 不能是负数")
	}
	columns, _, err := args.integer("columns")
	if err != nil {
		return nil, err
	}
	originX, hasOriginX, err := args.num("originX")
	if err != nil {
		return nil, err
	}
	originY, hasOriginY, err := args.num("originY")
	if err != nil {
		return nil, err
	}
	order, _, err := args.str("order")
	if err != nil {
		return nil, err
	}

	ordered := append([]*block(nil), blocks...)
	switch strings.ToLower(strings.TrimSpace(order)) {
	case "", "ids":
	case "position":
		// A row reads left to right; a column or a grid top to bottom.
		row := layout == "row"
		sort.SliceStable(ordered, func(i, j int) bool {
			a, b := ordered[i], ordered[j]
			lead, cross := a.Y, a.X
			otherLead, otherCross := b.Y, b.X
			if row {
				lead, cross = a.X, a.Y
				otherLead, otherCross = b.X, b.Y
			}
			if lead != otherLead {
				return lead < otherLead
			}
			return cross < otherCross
		})
	default:
		return nil, fmt.Errorf("order 要是 ids 或 position，收到 %q", order)
	}

	n := len(ordered)
	cols := n
	switch layout {
	case "row":
		cols = n
	case "column":
		cols = 1
	case "grid":
		if columns > 0 {
			cols = columns
		} else {
			cols = int(math.Ceil(math.Sqrt(float64(n))))
		}
	}
	cols = min(max(cols, 1), n)
	rows := (n + cols - 1) / cols

	// Cell sizes: a column is as wide as its widest block, a row as tall
	// as its tallest.
	colW := make([]float32, cols)
	rowH := make([]float32, rows)
	for i, b := range ordered {
		c, r := i%cols, i/cols
		colW[c] = max(colW[c], b.W)
		rowH[r] = max(rowH[r], b.H)
	}
	colX := make([]float32, cols)
	rowY := make([]float32, rows)
	for c := 1; c < cols; c++ {
		colX[c] = colX[c-1] + colW[c-1] + float32(gapX)
	}
	for r := 1; r < rows; r++ {
		rowY[r] = rowY[r-1] + rowH[r-1] + float32(gapY)
	}

	bounds := boundsOf(ordered)
	if !hasOriginX {
		originX = float64(bounds.X)
	}
	if !hasOriginY {
		originY = float64(bounds.Y)
	}
	ps := make([]placement, 0, n)
	for i, b := range ordered {
		c, r := i%cols, i/cols
		ps = append(ps, placement{
			block: b,
			x:     float32(originX) + colX[c],
			y:     float32(originY) + rowY[r],
		})
	}

	moved := d.applyPlacements(ps)
	return map[string]any{
		"layout":  layout,
		"columns": cols,
		"rows":    rows,
		"gapX":    gapX,
		"gapY":    gapY,
		"moved":   moved,
		"blocks":  mcpGeometryList(ordered),
	}, nil
}

// mcpGeometryList reports where a layout put each block.
func mcpGeometryList(blocks []*block) []map[string]any {
	out := make([]map[string]any, len(blocks))
	for i, b := range blocks {
		out[i] = map[string]any{
			"id": b.ID, "kind": string(b.Kind), "title": b.Title,
			"x": roundCoord(b.X), "y": roundCoord(b.Y),
			"w": roundCoord(b.W), "h": roundCoord(b.H),
		}
	}
	return out
}
