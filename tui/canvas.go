package main

import (
	"math"
	"strings"

	"github.com/charmbracelet/lipgloss"
)

// The room, drawn.
//
// The old console asked you to type coordinates into a file and then read a
// number to find out whether they were any good. This draws the room instead
// and lets you move things in it, so "the geometry is poor" arrives as a colour
// that improves under your hands rather than as a term you have to go and look
// up. Nobody should need to know what dilution of precision means to place a
// sensor well.

type cell struct {
	r  rune
	fg lipgloss.Color
	bg lipgloss.Color
}

type canvas struct {
	w, h  int
	cells []cell
	// Metres per cell, tracked separately for each axis because a terminal
	// cell is about twice as tall as it is wide. Ignoring that draws every
	// room as though it were half as deep as it is, and then every distance
	// ring comes out an ellipse.
	mx, my float64
	room   Room
}

const (
	colFloor = lipgloss.Color("236")
	colWall  = lipgloss.Color("244")
	colNode  = lipgloss.Color("81")
	colDev   = lipgloss.Color("213")
	colDim   = lipgloss.Color("240")
	colText  = lipgloss.Color("252")
)

// Green through amber to red. Used for how well the nodes can pin a position
// down at each spot, and nowhere else — one meaning per colour ramp.
var heat = []lipgloss.Color{"29", "35", "70", "106", "142", "178", "172", "166", "160"}

func newCanvas(w, h int, room Room) *canvas {
	if room.Width <= 0 {
		room.Width = 10
	}
	if room.Height <= 0 {
		room.Height = 8
	}
	c := &canvas{w: w, h: h, room: room,
		mx: room.Width / float64(w), my: room.Height / float64(h)}
	c.cells = make([]cell, w*h)
	for i := range c.cells {
		c.cells[i] = cell{r: ' ', fg: colText, bg: colFloor}
	}
	return c
}

func (c *canvas) at(x, y int) *cell {
	if x < 0 || y < 0 || x >= c.w || y >= c.h {
		return nil
	}
	return &c.cells[y*c.w+x]
}

// cellOf converts a position in metres to a cell.
func (c *canvas) cellOf(p point) (int, int) {
	return int(p.X / c.mx), int(p.Y / c.my)
}

// metresOf is the inverse, taken at the centre of the cell.
func (c *canvas) metresOf(x, y int) point {
	return point{(float64(x) + 0.5) * c.mx, (float64(y) + 0.5) * c.my}
}

// shadeQuality paints how well the current nodes could locate something at each
// spot. This is the whole point of the placement view: you move a node and
// watch the room get greener.
func (c *canvas) shadeQuality(nodes []point) {
	for y := 0; y < c.h; y++ {
		for x := 0; x < c.w; x++ {
			g := gdopAt(nodes, c.metresOf(x, y))
			cl := c.at(x, y)
			if math.IsInf(g, 1) {
				cl.bg = lipgloss.Color("235")
				continue
			}
			// Compress a long tail into the ramp: below 2 is as good as it
			// gets and above 12 is uniformly bad, so spending resolution
			// there would only make good layouts harder to tell apart.
			f := (g - 1.5) / 10.5
			i := int(f * float64(len(heat)-1))
			if i < 0 {
				i = 0
			}
			if i >= len(heat) {
				i = len(heat) - 1
			}
			cl.bg = heat[i]
		}
	}
}

// ring draws the set of places a device could be, given one node heard it at
// this distance. With one node that is the honest picture: a circle, not a dot.
func (c *canvas) ring(centre point, metres float64, fg lipgloss.Color, glyph rune) {
	if metres <= 0 || metres > c.room.Width+c.room.Height {
		return
	}
	steps := 240
	for i := 0; i < steps; i++ {
		a := 2 * math.Pi * float64(i) / float64(steps)
		p := point{centre.X + metres*math.Cos(a), centre.Y + metres*math.Sin(a)}
		x, y := c.cellOf(p)
		if cl := c.at(x, y); cl != nil && cl.r == ' ' {
			cl.r, cl.fg = glyph, fg
		}
	}
}

// mark places a labelled glyph, keeping the label inside the room.
func (c *canvas) mark(p point, glyph rune, label string, fg lipgloss.Color) {
	x, y := c.cellOf(p)
	if cl := c.at(x, y); cl != nil {
		cl.r, cl.fg = glyph, fg
	}
	if label == "" {
		return
	}
	lx := x + 2
	if lx+len(label) >= c.w {
		lx = x - 1 - len(label)
	}
	for i, r := range label {
		if cl := c.at(lx+i, y); cl != nil {
			cl.r, cl.fg = r, fg
		}
	}
}

func (c *canvas) render() string {
	var b strings.Builder
	top := "┌" + strings.Repeat("─", c.w) + "┐"
	b.WriteString(lipgloss.NewStyle().Foreground(colWall).Render(top) + "\n")
	wall := lipgloss.NewStyle().Foreground(colWall).Render("│")
	for y := 0; y < c.h; y++ {
		b.WriteString(wall)
		// Runs of identical styling are merged: emitting an escape sequence
		// per cell makes a 64x22 grid flicker visibly on redraw.
		x := 0
		for x < c.w {
			cl := c.cells[y*c.w+x]
			j := x
			var run strings.Builder
			for j < c.w {
				n := c.cells[y*c.w+j]
				if n.fg != cl.fg || n.bg != cl.bg {
					break
				}
				run.WriteRune(n.r)
				j++
			}
			b.WriteString(lipgloss.NewStyle().Foreground(cl.fg).Background(cl.bg).Render(run.String()))
			x = j
		}
		b.WriteString(wall + "\n")
	}
	b.WriteString(lipgloss.NewStyle().Foreground(colWall).Render("└" + strings.Repeat("─", c.w) + "┘"))
	return b.String()
}

// ── plain language ───────────────────────────────────────────────────────────
//
// Numbers are available on demand, but the first thing on screen should be a
// sentence. "1.2 m, basis advertised" is a reading; "at your desk, estimated"
// is an answer.

func whereabouts(metres float64) string {
	switch {
	case metres < 1:
		return "right here"
	case metres < 2.5:
		return "at your desk"
	case metres < 6:
		return "in this room"
	case metres < 12:
		return "next room, or through a wall"
	default:
		return "somewhere in the building"
	}
}

func confidence(basis string) string {
	switch basis {
	case "measured":
		return "measured here"
	case "advertised":
		return "estimated"
	default:
		return "rough guess"
	}
}

// reliability turns the heard ratio into something actionable. The number is a
// fraction of sweeps; what you want to know is whether to move the node.
func reliability(ratio float64) (string, lipgloss.Color) {
	switch {
	case ratio >= 0.9:
		return "hears it consistently", lipgloss.Color("42")
	case ratio >= 0.6:
		return "hears it most of the time", lipgloss.Color("214")
	case ratio > 0:
		return "only catches it occasionally — try moving this one", lipgloss.Color("203")
	default:
		return "cannot hear it at all", lipgloss.Color("203")
	}
}

// bar is a five-cell meter. Easier to compare down a column than percentages.
func bar(f float64) string {
	const n = 5
	full := int(f*n + 0.5)
	return strings.Repeat("▰", full) + strings.Repeat("▱", n-full)
}
