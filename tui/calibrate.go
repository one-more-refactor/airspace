package main

// The measuring screen.
//
// Calibration means standing a known distance away holding the device, which
// is exactly the moment you cannot reach the keyboard. So the keyboard is used
// once, at the start, and then not again: you press enter, and the screen
// counts you to each station in turn while you walk. Four stations, one run,
// under a minute.
//
// It takes the whole screen rather than a sidebar because during those forty
// seconds it is the only thing happening, and because a countdown you have to
// find in a corner is a countdown you will miss from across the room.

import (
	"fmt"
	"math"
	"strings"
	"time"

	tea "github.com/charmbracelet/bubbletea"
	"github.com/charmbracelet/lipgloss"
)

const (
	// Long enough to cross a room without hurrying; short enough that four of
	// them stays under a minute.
	calStationFor = 10 * time.Second
	// How long the capture stays on screen before the next station begins.
	calFlashFor = 1400 * time.Millisecond
	// ~12fps. Enough that the rings read as motion, not enough to cost anything.
	calFramePeriod = 80 * time.Millisecond
	// The reading is the median of everything heard during the whole station,
	// not of the last few seconds.
	//
	// This is the difference between a rough calibration and a usable one, and
	// it is a physics problem rather than a software one. Standing still
	// samples ONE multipath realisation: at 2.4 GHz the wavelength is 12 cm, so
	// constructive and destructive interference swing the reading by ±10 dB
	// over the width of your hand. Averaging for longer in the same spot
	// removes receiver noise and leaves that error completely untouched.
	//
	// Moving does remove it. Walking a slow arc at the target radius sweeps
	// through many realisations, and their median is the actual path loss at
	// that distance. So the instruction is "walk a slow arc", the whole ten
	// seconds is the sampling window, and the spread is shown on screen — a
	// tight spread means you stood still and the number is worth less than it
	// looks.
	calSettleWindow = 3 * time.Second
)

type frameMsg time.Time

func frame() tea.Cmd {
	return tea.Tick(calFramePeriod, func(t time.Time) tea.Msg { return frameMsg(t) })
}

// spreadMeans explains the residual, because a number in dB is only useful to
// someone who already knows what a good one looks like.
func spreadMeans(rms float64) string {
	switch {
	case rms < 3:
		return "the readings sat close to the\ncurve — about as good as a\nradio in a room gets."
	case rms < 6:
		return "some scatter, which is normal.\nDistances will be right to\nroughly a metre or two."
	case rms < 9:
		return "noisy. Walk a wider, slower\narc next time — standing still\nsamples one spot's echoes."
	default:
		return "these readings are not\ndescribing distance. Something\nwas in the way, or a station\nwas measured from the wrong\nplace."
	}
}

// ── chunky digits ────────────────────────────────────────────────────────────

var bigDigits = [10][5]string{
	{"███", "█ █", "█ █", "█ █", "███"},
	{"  █", "  █", "  █", "  █", "  █"},
	{"███", "  █", "███", "█  ", "███"},
	{"███", "  █", "███", "  █", "███"},
	{"█ █", "█ █", "███", "  █", "  █"},
	{"███", "█  ", "███", "  █", "███"},
	{"███", "█  ", "███", "█ █", "███"},
	{"███", "  █", "  █", "  █", "  █"},
	{"███", "█ █", "███", "█ █", "███"},
	{"███", "█ █", "███", "  █", "███"},
}

// bigNumber renders n as five-row digits with every column doubled, because a
// terminal cell is about twice as tall as it is wide and undoubled glyphs come
// out as thin slivers.
func bigNumber(n int) []string {
	ds := fmt.Sprintf("%d", n)
	rows := make([]string, 5)
	for r := 0; r < 5; r++ {
		var b strings.Builder
		for i, ch := range ds {
			if ch < '0' || ch > '9' {
				continue
			}
			if i > 0 {
				b.WriteString("  ")
			}
			for _, p := range bigDigits[ch-'0'][r] {
				b.WriteRune(p)
				b.WriteRune(p)
			}
		}
		rows[r] = b.String()
	}
	return rows
}

// ── the ripple field ─────────────────────────────────────────────────────────

// Nearest ring brightest, fading outward, so the motion reads as leaving the
// node rather than arriving at it.
var rippleStyles = []lipgloss.Style{
	lipgloss.NewStyle().Foreground(lipgloss.Color("81")),
	lipgloss.NewStyle().Foreground(lipgloss.Color("38")),
	lipgloss.NewStyle().Foreground(lipgloss.Color("31")),
	lipgloss.NewStyle().Foreground(lipgloss.Color("238")),
}

type rcell struct {
	r  rune
	st int // >=0 index into rippleStyles, -1 blank, -2 overlay
}

// rippleField draws rings expanding from the centre with a clear rectangle in
// the middle for the overlay to sit in, so the countdown is never fighting the
// animation for the same cells.
func rippleField(w, h int, phase float64, overlay []string, ovStyle lipgloss.Style) string {
	if w < 8 || h < 5 {
		return strings.Join(overlay, "\n")
	}
	cx, cy := float64(w-1)/2, float64(h-1)/2
	// Radius is bounded by the VERTICAL extent, not the diagonal. A terminal
	// row is about twice a column, so a ring of radius r is r columns wide and
	// r/2 rows tall; sizing to the diagonal of a wide box sends the rings off
	// the top and bottom almost immediately and what is left on screen reads as
	// two vertical bars rather than a wave.
	maxR := cy*2 + 5

	holeW, holeH := 0, 0
	for _, l := range overlay {
		if n := len([]rune(l)); n > holeW {
			holeW = n
		}
	}
	if holeW > 0 {
		holeW += 4
		holeH = len(overlay) + 2
	}

	grid := make([][]rcell, h)
	for y := range grid {
		grid[y] = make([]rcell, w)
		for x := range grid[y] {
			grid[y][x] = rcell{' ', -1}
		}
	}

	for y := 0; y < h; y++ {
		for x := 0; x < w; x++ {
			if math.Abs(float64(x)-cx) <= float64(holeW)/2 &&
				math.Abs(float64(y)-cy) <= float64(holeH)/2 {
				continue
			}
			d := math.Hypot(float64(x)-cx, (float64(y)-cy)*2)
			for k := 0; k < 4; k++ {
				r := math.Mod(phase+float64(k)*maxR/4, maxR)
				if math.Abs(d-r) < 0.7 {
					si := int(r / maxR * float64(len(rippleStyles)))
					if si >= len(rippleStyles) {
						si = len(rippleStyles) - 1
					}
					grid[y][x] = rcell{'·', si}
					break
				}
			}
		}
	}

	if len(overlay) > 0 {
		top := int(cy) - len(overlay)/2
		for i, line := range overlay {
			y := top + i
			if y < 0 || y >= h {
				continue
			}
			runes := []rune(line)
			left := int(cx) - len(runes)/2
			for j, r := range runes {
				x := left + j
				if x < 0 || x >= w || r == ' ' {
					continue
				}
				grid[y][x] = rcell{r, -2}
			}
		}
	}

	// Render in runs of equal style: a handful of spans per row rather than one
	// per cell, which matters at twelve frames a second.
	var b strings.Builder
	for y := 0; y < h; y++ {
		for x := 0; x < w; {
			st := grid[y][x].st
			var run strings.Builder
			j := x
			for j < w && grid[y][j].st == st {
				run.WriteRune(grid[y][j].r)
				j++
			}
			switch st {
			case -1:
				b.WriteString(run.String())
			case -2:
				b.WriteString(ovStyle.Render(run.String()))
			default:
				b.WriteString(rippleStyles[st].Render(run.String()))
			}
			x = j
		}
		if y < h-1 {
			b.WriteString("\n")
		}
	}
	return b.String()
}

// ── small parts ──────────────────────────────────────────────────────────────

// signalBar is the live reading, and it is the reason this is worth watching
// while you walk: the bar visibly falls as you back away, so you can see the
// thing being measured actually respond to you.
func signalBar(rssi int, ok bool) string {
	const n = 20
	if !ok {
		return alarm.Render(strings.Repeat("▱", n)) + "  " + dim.Render("nothing heard")
	}
	f := math.Max(0, math.Min(1, (float64(rssi)+100)/60))
	filled := int(f*float64(n) + 0.5)
	st := okc
	switch {
	case f < 0.2:
		st = alarm
	case f < 0.45:
		st = lipgloss.NewStyle().Foreground(lipgloss.Color("214"))
	}
	return st.Render(strings.Repeat("▰", filled)) +
		dim.Render(strings.Repeat("▱", n-filled)) +
		"  " + bold.Render(fmt.Sprintf("%d dBm", rssi))
}

// stationTrack is the progress through the run, drawn like checkpoints because
// that is what they are.
func stationTrack(stage int, done []bool, dists []float64) (string, string) {
	// One mark plus a six-character segment: a seven-column pitch that the
	// labels have to match exactly, or they walk away from their own marks.
	const pitch = 7
	var marks, labels strings.Builder
	for i, d := range dists {
		if i > 0 {
			seg := strings.Repeat("━", pitch-1)
			if i <= stage {
				marks.WriteString(accent.Render(seg))
			} else {
				marks.WriteString(dim.Render(seg))
			}
		}
		switch {
		case i < len(done) && done[i]:
			marks.WriteString(okc.Render("●"))
		case i == stage:
			marks.WriteString(accent.Render("◉"))
		default:
			marks.WriteString(dim.Render("○"))
		}

		lab := fmt.Sprintf("%.0fm", d)
		if i == stage {
			labels.WriteString(accent.Render(lab))
		} else {
			labels.WriteString(dim.Render(lab))
		}
		if i < len(dists)-1 {
			if pad := pitch - len([]rune(lab)); pad > 0 {
				labels.WriteString(strings.Repeat(" ", pad))
			}
		}
	}
	return marks.String(), labels.String()
}

func calFieldW(w int) int {
	fw := 52
	if w-4 < fw {
		fw = w - 4
	}
	if fw < 24 {
		fw = 24
	}
	return fw
}

func centreLine(s string, w int) string {
	pad := (w - lipgloss.Width(s)) / 2
	if pad < 0 {
		pad = 0
	}
	return strings.Repeat(" ", pad) + s
}

func centreBlock(s string, w int) string {
	out := make([]string, 0, 8)
	for _, l := range strings.Split(s, "\n") {
		out = append(out, centreLine(l, w))
	}
	return strings.Join(out, "\n")
}

func medianInt(v []int) int {
	if len(v) == 0 {
		return 0
	}
	s := append([]int(nil), v...)
	for i := 1; i < len(s); i++ {
		for j := i; j > 0 && s[j] < s[j-1]; j-- {
			s[j], s[j-1] = s[j-1], s[j]
		}
	}
	return s[len(s)/2]
}

// ── the screen ───────────────────────────────────────────────────────────────

func (m model) calibrateScreen() string {
	w := m.width
	if w < 40 {
		w = 40
	}
	phase := float64(m.calFrame) * 0.32

	node := "the node"
	if ns := m.nodes(); len(ns) > 0 && m.sel < len(ns) {
		node = ns[m.sel].Name
	}

	head := centreLine(dim.Render("measuring  ")+bold.Render(m.calDevice), w)
	if m.calDevice == "" {
		return "\n\n" + centreLine(bold.Render("Nothing to measure"), w) + "\n\n" +
			centreBlock(multiline(dim, "Only devices you have named are\nreported, and none are being heard."), w) +
			"\n\n" + m.footer() + "\n"
	}

	var body string
	switch {
	// ── finished ─────────────────────────────────────────────────────────
	case m.calFitted != nil:
		f := m.calFitted
		word, qcol := fitQuality(m.calFitted.RMSdB)
		body = centreBlock(rippleField(calFieldW(w), 11, phase*0.5, []string{word},
			lipgloss.NewStyle().Bold(true).Foreground(qcol)), w) + "\n\n" +
			centreLine(dim.Render("at one metre  ")+bold.Render(fmt.Sprintf("%.0f dBm", f.RSSIAtOneMetre)), w) + "\n" +
			centreLine(dim.Render("falloff       ")+bold.Render(fmt.Sprintf("%.2f", f.Exponent)), w) + "\n\n" +
			centreLine(dim.Render("spread        ")+bold.Render(fmt.Sprintf("%.1f dB", f.RMSdB)), w) + "\n\n" +
			centreBlock(multiline(dim, falloffMeans(f.Exponent)), w) + "\n" +
			centreBlock(multiline(stFaint, spreadMeans(f.RMSdB)), w) + "\n\n" +
			centreLine(okc.Render(fmt.Sprintf("distances from %s are measured now, not guessed", node)), w)

	// ── running ──────────────────────────────────────────────────────────
	case m.calRun:
		dists := m.stations()
		done := make([]bool, len(dists))
		for i := range done {
			done[i] = i < len(m.calSamples)
		}
		stage := m.calStage
		if stage >= len(dists) {
			stage = len(dists) - 1
		}
		rssi, heard := 0, false
		if ns := m.nodes(); len(ns) > 0 && m.sel < len(ns) {
			rssi, heard = m.rssiFor(m.calDevice, ns[m.sel].Name)
		}

		var field, caption string
		if !m.calFlashUntil.IsZero() {
			// A capture: the rings burst outward instead of drifting.
			field = centreBlock(rippleField(calFieldW(w), 13, phase*3.2, []string{"GOT IT"}, okc.Bold(true)), w)
			caption = centreLine(dim.Render("got it — next station coming up"), w)
		} else {
			left := int(math.Ceil(time.Until(m.calDeadline).Seconds()))
			if left < 0 {
				left = 0
			}
			st := accent.Bold(true)
			if left <= 3 {
				st = lipgloss.NewStyle().Bold(true).Foreground(lipgloss.Color("214"))
			}
			field = centreBlock(rippleField(calFieldW(w), 13, phase, bigNumber(left), st), w)
			caption = centreLine(bold.Render(fmt.Sprintf("CIRCLE SLOWLY AT %.1f METRES", dists[stage]))+
				dim.Render(fmt.Sprintf("  from %s", node)), w)
		}

		marks, labels := stationTrack(stage, done, dists)
		body = field + "\n\n" + caption + "\n\n" +
			centreLine(signalBar(rssi, heard), w) + "\n\n" +
			centreLine(marks, w) + "\n" + centreLine(labels, w)
		if m.calNote != "" {
			body += "\n\n" + centreLine(alarm.Render(m.calNote), w)
		}

	// ── ready ────────────────────────────────────────────────────────────
	default:
		body = centreBlock(rippleField(calFieldW(w), 11, phase*0.6, []string{"READY"}, accent.Bold(true)), w) + "\n\n" +
			centreLine(bold.Render(fmt.Sprintf("Four readings — 1, 2, 4 and 8 metres from %s", node)), w) + "\n\n" +
			centreBlock(multiline(dim,
				"Press enter once and then put the keyboard down.\n"+
					"It counts you to each station while you walk, and takes\n"+
					"the reading when the number hits zero."), w) + "\n\n" +
			centreLine(accent.Render("[ enter ]")+dim.Render("  start the run"), w)
		if m.calNote != "" {
			body += "\n\n" + centreLine(alarm.Render(m.calNote), w)
		}
	}

	return "\n" + head + "\n\n" + body + "\n\n" + centreLine(m.footer(), w) + "\n"
}
