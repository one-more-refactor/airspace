package main

import (
	"math"
	"strings"

	"github.com/charmbracelet/lipgloss"
)

// The look, in one place.
//
// The measuring screen proved the point: chunky type, motion and saturated
// colour make a tool people will actually walk around the flat with. That was
// not a one-off for one screen — it is how the whole console should feel. So
// the font, the palette and the motion helpers live here rather than being
// reinvented per view, and every screen draws from the same box of parts.
//
// One rule holds the palette together: **a colour means one thing everywhere.**
// Cyan is always a node, magenta is always a device, the green-amber-red ramp
// is always quality. Reusing a hue for a second meaning is how a colourful
// interface turns into a confusing one.

const (
	cNode    = lipgloss.Color("51") // cyan — a listening ear
	cNodeDim = lipgloss.Color("30")
	cDev     = lipgloss.Color("213") // magenta — a tracked device
	cDevDim  = lipgloss.Color("96")
	cGood    = lipgloss.Color("47")
	cWarn    = lipgloss.Color("214")
	cBad     = lipgloss.Color("203")
	cInk     = lipgloss.Color("255")
	cMuted   = lipgloss.Color("245")
	cFaint   = lipgloss.Color("240")
	cFloor   = lipgloss.Color("234")
	cWallC   = lipgloss.Color("60")
	cAccent  = lipgloss.Color("219")
)

// quality is the one ramp in the program: green through amber to red. It is
// used for how well the geometry can place something and for how reliably a
// node hears a device, which are the same question asked twice.
var quality = []lipgloss.Color{
	"48", "42", "77", "112", "148", "184", "214", "208", "203", "196",
}

func qualityAt(f float64) lipgloss.Color {
	i := int(f * float64(len(quality)-1))
	if i < 0 {
		i = 0
	}
	if i >= len(quality) {
		i = len(quality) - 1
	}
	return quality[i]
}

var (
	stInk   = lipgloss.NewStyle().Foreground(cInk)
	stMuted = lipgloss.NewStyle().Foreground(cMuted)
	stFaint = lipgloss.NewStyle().Foreground(cFaint)
	stNode  = lipgloss.NewStyle().Foreground(cNode)
	stDev   = lipgloss.NewStyle().Foreground(cDev)
	stGood  = lipgloss.NewStyle().Foreground(cGood)
	stWarn  = lipgloss.NewStyle().Foreground(cWarn)
	stBad   = lipgloss.NewStyle().Foreground(cBad)
	stBold  = lipgloss.NewStyle().Bold(true).Foreground(cInk)
)

// ── big type ─────────────────────────────────────────────────────────────────
//
// Five rows tall, three columns per glyph. Readable from across a room, which
// is the entire reason it exists: a label you have to walk back to the desk to
// read is a label that has failed.

var glyphs = map[rune][5]string{
	'A':  {"███", "█ █", "███", "█ █", "█ █"},
	'B':  {"██ ", "█ █", "██ ", "█ █", "██ "},
	'C':  {"███", "█  ", "█  ", "█  ", "███"},
	'D':  {"██ ", "█ █", "█ █", "█ █", "██ "},
	'E':  {"███", "█  ", "██ ", "█  ", "███"},
	'F':  {"███", "█  ", "██ ", "█  ", "█  "},
	'G':  {"███", "█  ", "█ █", "█ █", "███"},
	'H':  {"█ █", "█ █", "███", "█ █", "█ █"},
	'I':  {"███", " █ ", " █ ", " █ ", "███"},
	'J':  {"███", "  █", "  █", "█ █", "███"},
	'K':  {"█ █", "█ █", "██ ", "█ █", "█ █"},
	'L':  {"█  ", "█  ", "█  ", "█  ", "███"},
	'M':  {"█ █", "███", "███", "█ █", "█ █"},
	'N':  {"██ ", "█ █", "█ █", "█ █", "█ █"},
	'O':  {"███", "█ █", "█ █", "█ █", "███"},
	'P':  {"███", "█ █", "███", "█  ", "█  "},
	'Q':  {"███", "█ █", "█ █", "███", "  █"},
	'R':  {"███", "█ █", "██ ", "█ █", "█ █"},
	'S':  {"███", "█  ", "███", "  █", "███"},
	'T':  {"███", " █ ", " █ ", " █ ", " █ "},
	'U':  {"█ █", "█ █", "█ █", "█ █", "███"},
	'V':  {"█ █", "█ █", "█ █", "█ █", " █ "},
	'W':  {"█ █", "█ █", "███", "███", "█ █"},
	'X':  {"█ █", "█ █", " █ ", "█ █", "█ █"},
	'Y':  {"█ █", "█ █", " █ ", " █ ", " █ "},
	'Z':  {"███", "  █", " █ ", "█  ", "███"},
	'0':  {"███", "█ █", "█ █", "█ █", "███"},
	'1':  {"  █", "  █", "  █", "  █", "  █"},
	'2':  {"███", "  █", "███", "█  ", "███"},
	'3':  {"███", "  █", "███", "  █", "███"},
	'4':  {"█ █", "█ █", "███", "  █", "  █"},
	'5':  {"███", "█  ", "███", "  █", "███"},
	'6':  {"███", "█  ", "███", "█ █", "███"},
	'7':  {"███", "  █", "  █", "  █", "  █"},
	'8':  {"███", "█ █", "███", "█ █", "███"},
	'9':  {"███", "█ █", "███", "  █", "███"},
	' ':  {"   ", "   ", "   ", "   ", "   "},
	'.':  {"   ", "   ", "   ", "   ", " █ "},
	'-':  {"   ", "   ", "███", "   ", "   "},
	'\'': {" █ ", " █ ", "   ", "   ", "   "},
	'!':  {" █ ", " █ ", " █ ", "   ", " █ "},
	'?':  {"███", "  █", " ██", "   ", " █ "},
	':':  {"   ", " █ ", "   ", " █ ", "   "},
}

// bigText renders a string as five rows of blocks. Anything without a glyph
// becomes a space rather than an error — a headline is not worth crashing over.
func bigText(s string) []string {
	rows := make([]string, 5)
	for _, r := range strings.ToUpper(s) {
		g, ok := glyphs[r]
		if !ok {
			g = glyphs[' ']
		}
		for i := 0; i < 5; i++ {
			rows[i] += g[i] + " "
		}
	}
	for i := range rows {
		rows[i] = strings.TrimRight(rows[i], " ")
	}
	return rows
}

func bigWidth(s string) int {
	if s == "" {
		return 0
	}
	return len([]rune(s))*4 - 1
}

// ── motion ───────────────────────────────────────────────────────────────────

// breathe returns a 0..1 value that eases in and out, for anything that should
// look alive rather than blink. Linear pulsing reads as a fault indicator;
// a sine reads as a heartbeat.
func breathe(phase float64, period float64) float64 {
	return 0.5 + 0.5*math.Sin(2*math.Pi*phase/period)
}

// fade picks a colour along a two-stop ramp, for pulsing a marker between a
// dim and a bright version of its own hue rather than flashing it on and off.
func fade(dimC, brightC lipgloss.Color, t float64) lipgloss.Color {
	if t > 0.5 {
		return brightC
	}
	return dimC
}

// meter is a chunky bar. Wider than the old five cells because a five-cell bar
// cannot show the difference between "usually" and "always", which is exactly
// the difference that tells you whether to move a node.
func meter(f float64, width int, col lipgloss.Color) string {
	if f < 0 {
		f = 0
	}
	if f > 1 {
		f = 1
	}
	full := int(f*float64(width) + 0.5)
	on := lipgloss.NewStyle().Foreground(col).Render(strings.Repeat("━", full))
	off := stFaint.Render(strings.Repeat("━", width-full))
	return on + off
}

// rule draws a horizontal divider that fades at the ends, so the screen has
// structure without being boxed into panels.
func rule(w int) string {
	if w < 6 {
		return strings.Repeat("─", max(0, w))
	}
	mid := strings.Repeat("━", w-4)
	return stFaint.Render("╺━") + stFaint.Render(mid) + stFaint.Render("━╸")
}

func max(a, b int) int {
	if a > b {
		return a
	}
	return b
}

// centreAsBlock centres a group of lines as one unit, aligned on their common
// left edge. Centring each line on its own makes a column of rows with
// different trailing text stagger, which reads as a bug even when the numbers
// are right.
func centreAsBlock(lines []string, w int) string {
	widest := 0
	for _, l := range lines {
		if n := lipgloss.Width(l); n > widest {
			widest = n
		}
	}
	pad := (w - widest) / 2
	if pad < 0 {
		pad = 0
	}
	var b strings.Builder
	for i, l := range lines {
		if i > 0 {
			b.WriteString("\n")
		}
		b.WriteString(strings.Repeat(" ", pad) + l)
	}
	return b.String()
}

// centred pads a block of lines so it sits in the middle of a width.
func centred(lines []string, w int) []string {
	out := make([]string, len(lines))
	for i, l := range lines {
		pad := (w - lipgloss.Width(l)) / 2
		if pad < 0 {
			pad = 0
		}
		out[i] = strings.Repeat(" ", pad) + l
	}
	return out
}
