package main

import (
	"fmt"
	"math"
	"strings"

	"github.com/charmbracelet/lipgloss"
)

// The two screens that are not measuring.
//
// Same rules as the measuring run: take the whole screen, say the important
// thing in type you can read from across the room, and keep something moving so
// a live system never looks like a frozen one. A sidebar full of small grey
// numbers is a spreadsheet, and nobody walks around their flat holding a
// spreadsheet.

// watchScreen is the resting state: which of your things is where.
func (m model) watchScreen() string {
	w := m.width
	if w < 40 {
		w = 40
	}
	var b strings.Builder

	dev := m.focusedDevice()

	// The headline is the answer, not the data. "PHONE / at your desk" is what
	// you came to find out; -58 dBm is trivia you can have underneath.
	if dev != nil {
		name := strings.ToUpper(dev.Label)
		if bigWidth(name) > w-4 {
			name = firstWord(name)
		}
		colour := stDev
		if best, ok := bestHeard(dev); ok && best.Metres < 2.5 {
			colour = lipgloss.NewStyle().Foreground(cAccent)
		}
		for _, row := range centred(bigText(name), w) {
			b.WriteString(colour.Render(row) + "\n")
		}
		b.WriteString("\n")
		if best, ok := bestHeard(dev); ok {
			line := fmt.Sprintf("%s   ·   %.1f m   ·   %s",
				strings.ToUpper(whereabouts(best.Metres)), best.Metres, confidence(best.Basis))
			b.WriteString(centreLine(stInk.Render(line), w) + "\n")
			if dev.InEar != nil {
				worn := stFaint.Render("not being worn")
				if *dev.InEar {
					worn = stGood.Render("being worn")
				}
				b.WriteString(centreLine(worn, w) + "\n")
			}
		} else {
			b.WriteString(centreLine(stFaint.Render("not heard just now"), w) + "\n")
		}
	} else {
		for _, row := range centred(bigText("QUIET"), w) {
			b.WriteString(stFaint.Render(row) + "\n")
		}
		b.WriteString("\n" + centreLine(stMuted.Render(
			"Only devices you have named are reported."), w) + "\n")
		b.WriteString(centreLine(stFaint.Render(
			"Everything else is dropped before it is written down anywhere."), w) + "\n")
	}

	b.WriteString("\n" + centreLine(rule(min(w-8, 64)), w) + "\n\n")
	b.WriteString(m.roomBlock(w, false) + "\n")

	// One row per node, showing how well it hears the focused device. This is
	// the health check: a node that has stopped listening looks different from
	// a quiet room, which is the failure this project keeps meeting.
	if dev != nil {
		b.WriteString("\n")
		var rows []string
		for _, n := range m.nodes() {
			var h *Heard
			for i := range dev.Heard {
				if dev.Heard[i].Node == n.Name {
					h = &dev.Heard[i]
				}
			}
			label := stNode.Render(fmt.Sprintf("%-8s", strings.ToUpper(n.Name)))
			if h == nil {
				rows = append(rows, label+" "+meter(0, 24, cBad)+"  "+
					stBad.Render("cannot hear it"))
				continue
			}
			word, col := reliability(h.HeardRatio)
			rows = append(rows, label+" "+meter(h.HeardRatio, 24, col)+"  "+
				lipgloss.NewStyle().Foreground(col).Render(word))
		}
		b.WriteString(centreAsBlock(rows, w) + "\n")
	}

	// Everything else you track, still on screen. Showing only the focused
	// device made the others disappear entirely — a HUD that hides two thirds
	// of the state to look tidy is a worse HUD.
	if len(m.snap.Devices) > 1 {
		b.WriteString("\n" + m.roster(w) + "\n")
	}
	return b.String()
}

// roster is the strip of everything tracked, with the focused one lit.
func (m model) roster(w int) string {
	var parts []string
	focus := m.focusedDevice()
	for _, d := range m.snap.Devices {
		name := strings.ToUpper(d.Label)
		if focus != nil && d.ID == focus.ID {
			parts = append(parts, lipgloss.NewStyle().Bold(true).
				Foreground(cDev).Render("▸ "+name))
			continue
		}
		// Dim, but coloured by whether anything can currently hear it — the
		// roster doubles as an at-a-glance "is everything still there".
		col := cFaint
		if _, ok := bestHeard(&d); ok {
			col = cDevDim
		}
		parts = append(parts, lipgloss.NewStyle().Foreground(col).Render("  "+name))
	}
	return centreLine(strings.Join(parts, stFaint.Render("   ")), w)
}

// moveScreen is placement: the floor is the feedback.
func (m model) moveScreen() string {
	w := m.width
	if w < 40 {
		w = 40
	}
	var b strings.Builder
	ns := m.nodes()
	name := "NODE"
	if m.sel < len(ns) {
		name = strings.ToUpper(ns[m.sel].Name)
	}

	// The selected node's name pulses, so it is obvious which marker the arrow
	// keys are attached to without having to hunt for a highlight.
	t := breathe(float64(m.frame), 14)
	col := lipgloss.NewStyle().Foreground(fade(cNodeDim, cNode, t))
	for _, row := range centred(bigText(name), w) {
		b.WriteString(col.Render(row) + "\n")
	}
	b.WriteString("\n")

	med, _, usable := coverage(m.nodePoints(), m.snap.Room.Width, m.snap.Room.Height)
	verdict, vcol := placementWord(len(ns), med)
	b.WriteString(centreLine(lipgloss.NewStyle().Bold(true).Foreground(vcol).Render(verdict), w) + "\n")
	if !math.IsInf(med, 1) {
		b.WriteString(centreLine(stFaint.Render(
			fmt.Sprintf("covers %.0f%% of the room", usable*100)), w) + "\n")
	}
	b.WriteString("\n" + m.roomBlock(w, true) + "\n")
	b.WriteString("\n" + centreLine(stMuted.Render(
		"greener floor is better — walk the marker around and watch it change"), w) + "\n")
	return b.String()
}

// roomBlock draws the room, centred, with everything alive in it.
func (m model) roomBlock(w int, heat bool) string {
	// A terminal cell is roughly twice as tall as it is wide. Sizing the
	// canvas without accounting for that stretched every distance ring into a
	// wide ellipse — which matters here, because the ring IS the claim being
	// made about where something is.
	maxW := min(w-8, 96)
	maxH := m.height - 19
	if maxH < 8 {
		maxH = 8
	}
	if maxH > 22 {
		maxH = 22
	}
	const cellAspect = 2.0
	cw := maxW
	ch := int(float64(cw) * m.snap.Room.Height / (cellAspect * m.snap.Room.Width))
	if ch > maxH {
		ch = maxH
		cw = int(float64(ch) * cellAspect * m.snap.Room.Width / m.snap.Room.Height)
	}
	if cw < 24 {
		cw = 24
	}
	if ch < 6 {
		ch = 6
	}

	c := newCanvas(cw, ch, m.snap.Room)
	if heat {
		c.shadeQuality(m.nodePoints())
	}

	// Rings breathe rather than blink. A device that is genuinely there should
	// look like it is there.
	t := breathe(float64(m.frame), 22)
	ringCol := fade(cDevDim, cDev, t)
	glyph := '·'
	if t > 0.6 {
		glyph = '∙'
	}
	focus := m.focusedDevice()
	for _, d := range m.snap.Devices {
		col, g := cDevDim, '·'
		if focus != nil && d.ID == focus.ID {
			col, g = ringCol, glyph
		}
		for _, h := range d.Heard {
			for _, n := range m.nodes() {
				if n.Name == h.Node {
					c.ring(point{n.X, n.Y}, h.Metres, col, g)
				}
			}
		}
	}
	for i, n := range m.nodes() {
		g, col := '◆', cNode
		if i == m.sel && m.mode == modeMove {
			g = '◈'
			col = fade(cNodeDim, lipgloss.Color("226"), breathe(float64(m.frame), 8))
		}
		c.mark(point{n.X, n.Y}, g, strings.ToUpper(n.Name), col)
	}

	var out strings.Builder
	for _, line := range strings.Split(c.render(), "\n") {
		out.WriteString(centreLine(line, w) + "\n")
	}
	return strings.TrimRight(out.String(), "\n")
}

// placementWord says what the geometry is worth, in words and in a colour.
// Nobody should need to know what dilution of precision means to place a
// sensor well.
func placementWord(nodes int, med float64) (string, lipgloss.Color) {
	switch {
	case nodes < 2:
		return "DISTANCE ONLY — A SECOND EAR GIVES DIRECTION", cWarn
	case nodes == 2:
		return "TWO POSSIBLE PLACES — A THIRD NARROWS IT TO ONE", cWarn
	case math.IsInf(med, 1):
		return "IN A LINE — MOVE ONE OFF THE AXIS", cBad
	case med < 3:
		return "EXCELLENT", cGood
	case med < 8:
		return "WORKABLE", cWarn
	default:
		return "POOR — LET IT SEE THE ROOM FROM ELSEWHERE", cBad
	}
}

func (m model) focusedDevice() *Device {
	if m.snap == nil || len(m.snap.Devices) == 0 {
		return nil
	}
	i := m.devSel % len(m.snap.Devices)
	return &m.snap.Devices[i]
}

func bestHeard(d *Device) (Heard, bool) {
	if len(d.Heard) == 0 {
		return Heard{}, false
	}
	best := d.Heard[0]
	for _, h := range d.Heard {
		if h.Metres < best.Metres {
			best = h
		}
	}
	return best, true
}

func firstWord(s string) string {
	if i := strings.IndexByte(s, ' '); i > 0 {
		return s[:i]
	}
	return s
}

func min(a, b int) int {
	if a < b {
		return a
	}
	return b
}
