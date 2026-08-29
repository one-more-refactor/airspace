// airspace-console — the room, and the things in it.
//
// The design rule, and everything below follows from it: **you should never
// need to be told what a word means to use this.** The previous version made
// you type coordinates into a TOML file and then read a dilution figure to find
// out whether they were any good. Both of those are handbook moments. Here you
// move a marker with the arrow keys and the floor goes green under it.
//
// Three consequences worth naming:
//
//   - There is one screen. Modes are overlays on the room rather than tabs you
//     navigate between, so you never lose your bearings.
//   - The first line about any device is a sentence — "at your desk, estimated"
//     — and the numbers are underneath for when you want them.
//   - The footer only ever offers keys that do something right now.
package main

import (
	"fmt"
	"math"
	"os"
	"strings"
	"time"

	tea "github.com/charmbracelet/bubbletea"
	"github.com/charmbracelet/lipgloss"
)

type mode int

const (
	modeWatch mode = iota // looking at the room
	modeMove              // moving a node around in it
	modeCalibrate
)

type tickMsg time.Time
type stateMsg struct {
	snap *Snapshot
	err  error
}
type fitMsg struct {
	res *fitResult
	err error
}

type model struct {
	api  *client
	mode mode

	snap    *Snapshot
	lastErr error
	lastOK  time.Time

	sel int // selected node, in snapshot order

	// moving
	moved map[string]point // pending positions, not yet written

	// calibrating
	calDevice  string
	calStage   int
	calSamples [][2]float64
	calStatus  string
	calFitted  *fitResult

	// The run: started by one keypress and then hands-free, because the
	// person being measured is across the room holding the device.
	calRun        bool
	calDeadline   time.Time // when this station's reading is taken
	calFlashUntil time.Time // non-zero while showing a capture
	calFrame      int       // animation clock
	calWindow     []int     // readings gathered just before the buzzer
	calNote       string

	width, height int
}

// Spread over a decade so the fit has real leverage on the exponent. Readings
// at one and one-and-a-half metres describe nearly the same point twice.
var calDistances = []float64{1, 2, 4, 8}

func main() {
	base := os.Getenv("AIRSPACE_URL")
	if base == "" {
		base = "http://127.0.0.1:9970"
	}
	m := model{api: newClient(base), moved: map[string]point{}, width: 80, height: 30}
	if _, err := tea.NewProgram(m, tea.WithAltScreen()).Run(); err != nil {
		fmt.Fprintln(os.Stderr, err)
		os.Exit(1)
	}
}

func (m model) Init() tea.Cmd { return tea.Batch(fetch(m.api), tick()) }

func tick() tea.Cmd {
	return tea.Tick(time.Second, func(t time.Time) tea.Msg { return tickMsg(t) })
}

func fetch(c *client) tea.Cmd {
	return func() tea.Msg { s, err := c.state(); return stateMsg{s, err} }
}

// nodes returns positions with any unsaved moves applied, so the room you are
// looking at is the room you are editing.
func (m model) nodes() []Node {
	if m.snap == nil {
		return nil
	}
	out := make([]Node, len(m.snap.Nodes))
	copy(out, m.snap.Nodes)
	for i := range out {
		if p, ok := m.moved[out[i].Name]; ok {
			out[i].X, out[i].Y = p.X, p.Y
		}
	}
	return out
}

func (m model) nodePoints() []point {
	ns := m.nodes()
	ps := make([]point, len(ns))
	for i, n := range ns {
		ps[i] = point{n.X, n.Y}
	}
	return ps
}

func (m model) Update(msg tea.Msg) (tea.Model, tea.Cmd) {
	switch msg := msg.(type) {
	case tea.WindowSizeMsg:
		m.width, m.height = msg.Width, msg.Height

	case tickMsg:
		return m, tea.Batch(fetch(m.api), tick())

	case stateMsg:
		if msg.err != nil {
			m.lastErr = msg.err
		} else {
			m.snap, m.lastErr, m.lastOK = msg.snap, nil, time.Now()
			// Gather readings in the seconds before the buzzer so the sample
			// is a median rather than whichever advertisement landed last.
			if m.calRun && m.calFlashUntil.IsZero() &&
				time.Until(m.calDeadline) <= calSettleWindow {
				if ns := m.nodes(); len(ns) > 0 && m.sel < len(ns) {
					if r, ok := m.rssiFor(m.calDevice, ns[m.sel].Name); ok {
						m.calWindow = append(m.calWindow, r)
					}
				}
			}
		}

	case frameMsg:
		if m.mode != modeCalibrate {
			return m, nil
		}
		m.calFrame++
		if !m.calRun {
			return m, frame()
		}
		now := time.Time(msg)
		switch {
		case !m.calFlashUntil.IsZero():
			if now.After(m.calFlashUntil) {
				m.calFlashUntil = time.Time{}
				m.calStage++
				if m.calStage >= len(calDistances) {
					m.calRun = false
					samples, api := m.calSamples, m.api
					return m, tea.Batch(frame(), func() tea.Msg {
						r, err := api.fit(samples)
						return fitMsg{r, err}
					})
				}
				m.calDeadline, m.calWindow = now.Add(calStationFor), nil
			}
		case !now.Before(m.calDeadline):
			m = m.captureStation()
			m.calFlashUntil = now.Add(calFlashFor)
		}
		return m, frame()

	case fitMsg:
		if msg.err != nil {
			m.calStatus, m.calFitted = msg.err.Error(), nil
		} else {
			m.calFitted = msg.res
			node := ""
			if ns := m.nodes(); len(ns) > 0 && m.sel < len(ns) {
				node = ns[m.sel].Name
			}
			err := saveCalibration(calibration{
				Node: node, Device: m.calDevice,
				RSSIAt1m: msg.res.RSSIAtOneMetre, Exponent: msg.res.Exponent, Samples: msg.res.Samples,
			})
			if err != nil {
				m.calStatus = "measured, but could not save: " + err.Error()
			} else {
				m.calStatus = "saved — distances from this node are now measured rather than guessed"
			}
		}

	case tea.KeyMsg:
		return m.key(msg)
	}
	return m, nil
}

func (m model) key(msg tea.KeyMsg) (tea.Model, tea.Cmd) {
	ns := m.nodes()
	switch msg.String() {
	case "q", "ctrl+c":
		return m, tea.Quit
	case "esc":
		if m.mode == modeMove && m.sel < len(ns) {
			// Abandon the move rather than half-applying it.
			delete(m.moved, ns[m.sel].Name)
		}
		m.mode = modeWatch
		m.calStatus, m.calRun, m.calNote = "", false, ""
		return m, nil
	}

	switch m.mode {
	case modeWatch:
		switch msg.String() {
		case "tab", "n":
			if len(ns) > 0 {
				m.sel = (m.sel + 1) % len(ns)
			}
		case "m":
			if len(ns) > 0 {
				m.mode = modeMove
			}
		case "c":
			if m.snap != nil && len(m.snap.Devices) > 0 {
				m.mode = modeCalibrate
				m.calDevice, m.calStage, m.calSamples = m.snap.Devices[0].Label, 0, nil
				m.calFitted, m.calStatus = nil, ""
				m.calRun, m.calNote, m.calFrame = false, "", 0
				return m, frame()
			}
		}

	case modeMove:
		if len(ns) == 0 {
			m.mode = modeWatch
			return m, nil
		}
		n := ns[m.sel]
		p := point{n.X, n.Y}
		const step = 0.25
		switch msg.String() {
		case "left", "h":
			p.X -= step
		case "right", "l":
			p.X += step
		case "up", "k":
			p.Y -= step
		case "down", "j":
			p.Y += step
		case "enter", " ":
			if err := savePlacement(m.nodes()); err != nil {
				m.calStatus = "could not save: " + err.Error()
			} else {
				m.calStatus = "placed — restart the collector to apply"
				m.moved = map[string]point{}
			}
			m.mode = modeWatch
			return m, nil
		case "s":
			// Put it where the geometry would like it most.
			others := []point{}
			for i, q := range m.nodePoints() {
				if i != m.sel {
					others = append(others, q)
				}
			}
			if at, score := suggest(others, m.snap.Room.Width, m.snap.Room.Height); !math.IsInf(score, 1) {
				p = at
			}
		}
		p.X = clamp(p.X, 0, m.snap.Room.Width)
		p.Y = clamp(p.Y, 0, m.snap.Room.Height)
		m.moved[n.Name] = p

	case modeCalibrate:
		switch msg.String() {
		case "tab":
			// Changing device mid-run would mix two devices into one fit.
			if !m.calRun && m.snap != nil && len(m.snap.Devices) > 0 {
				for i, d := range m.snap.Devices {
					if d.Label == m.calDevice {
						m.calDevice = m.snap.Devices[(i+1)%len(m.snap.Devices)].Label
						m.calStage, m.calSamples, m.calFitted = 0, nil, nil
						break
					}
				}
			}
		case "enter", " ":
			// The only keypress of the whole run.
			if m.calFitted == nil && !m.calRun {
				m.calRun = true
				m.calStage, m.calSamples = 0, nil
				m.calWindow, m.calNote = nil, ""
				m.calFlashUntil = time.Time{}
				m.calDeadline = time.Now().Add(calStationFor)
				return m, frame()
			}
		}
	}
	return m, nil
}

// captureStation records the reading for the station just finished. A station
// the node simply could not hear is skipped and said so rather than aborting
// the run — being unable to hear it from eight metres is itself a result, and
// the fit only needs two points.
func (m model) captureStation() model {
	if m.calStage >= len(calDistances) {
		return m
	}
	ns := m.nodes()
	if len(ns) == 0 || m.sel >= len(ns) {
		m.calNote = "no node selected"
		return m
	}
	rssi, ok := 0, false
	if len(m.calWindow) > 0 {
		rssi, ok = medianInt(m.calWindow), true
	} else if r, o := m.rssiFor(m.calDevice, ns[m.sel].Name); o {
		rssi, ok = r, true
	}
	if !ok {
		m.calNote = fmt.Sprintf("%s could not hear it at %.0f m — skipping that one",
			ns[m.sel].Name, calDistances[m.calStage])
		return m
	}
	m.calSamples = append(m.calSamples, [2]float64{calDistances[m.calStage], float64(rssi)})
	m.calNote = ""
	return m
}

func (m model) rssiFor(device, node string) (int, bool) {
	if m.snap == nil {
		return 0, false
	}
	for _, d := range m.snap.Devices {
		if d.Label != device {
			continue
		}
		for _, h := range d.Heard {
			if h.Node == node {
				return h.RSSI, true
			}
		}
	}
	return 0, false
}

func clamp(v, lo, hi float64) float64 { return math.Max(lo, math.Min(hi, v)) }

// ── view ─────────────────────────────────────────────────────────────────────

var (
	dim    = lipgloss.NewStyle().Foreground(colDim)
	bold   = lipgloss.NewStyle().Bold(true).Foreground(colText)
	accent = lipgloss.NewStyle().Foreground(colNode)
	alarm  = lipgloss.NewStyle().Foreground(lipgloss.Color("203"))
	okc    = lipgloss.NewStyle().Foreground(lipgloss.Color("42"))
)

func (m model) View() string {
	if m.snap == nil {
		if m.lastErr != nil {
			return "\n  " + alarm.Render("Cannot reach the collector.") + "\n  " +
				dim.Render(m.lastErr.Error()) + "\n\n  " +
				dim.Render("Is it running?  systemctl --user status airspace") + "\n"
		}
		return "\n  " + dim.Render("Listening…") + "\n"
	}

	// Measuring takes the whole screen: for those forty seconds it is the only
	// thing happening, and a countdown tucked in a sidebar is one you cannot
	// read from across the room.
	if m.mode == modeCalibrate {
		return m.calibrateScreen()
	}

	cw := m.width - 34
	if cw < 30 {
		cw = 30
	}
	if cw > 78 {
		cw = 78
	}
	ch := m.height - 10
	if ch < 10 {
		ch = 10
	}
	if ch > 24 {
		ch = 24
	}

	c := newCanvas(cw, ch, m.snap.Room)
	if m.mode == modeMove {
		c.shadeQuality(m.nodePoints())
	}
	m.drawDevices(c)
	m.drawNodes(c)

	room := c.render()
	side := m.sidebar()
	body := lipgloss.JoinHorizontal(lipgloss.Top, room, "  ", side)

	return "\n" + m.title() + "\n" + body + "\n" + m.footer() + "\n"
}

func (m model) title() string {
	s := bold.Render(" airspace")
	if m.lastErr != nil && !m.lastOK.IsZero() {
		s += alarm.Render(fmt.Sprintf("   stale — no answer for %s",
			time.Since(m.lastOK).Round(time.Second)))
	}
	switch m.mode {
	case modeMove:
		ns := m.nodes()
		if len(ns) > 0 {
			s += accent.Render(fmt.Sprintf("   moving %s — the floor shows how well it could place things",
				ns[m.sel].Name))
		}
	case modeCalibrate:
		s += accent.Render("   measuring " + m.calDevice)
	}
	return s
}

func (m model) drawDevices(c *canvas) {
	ns := m.nodes()
	for _, d := range m.snap.Devices {
		for _, h := range d.Heard {
			for _, n := range ns {
				if n.Name == h.Node {
					// A ring, not a dot: with one node this is genuinely all
					// that is known, and drawing a point would be an invention.
					c.ring(point{n.X, n.Y}, h.Metres, colDev, '·')
				}
			}
		}
	}
}

func (m model) drawNodes(c *canvas) {
	for i, n := range m.nodes() {
		glyph := '◆'
		col := colNode
		if i == m.sel {
			glyph = '◈'
			if m.mode == modeMove {
				col = lipgloss.Color("226")
			}
		}
		c.mark(point{n.X, n.Y}, glyph, n.Name, col)
	}
}

func (m model) sidebar() string {
	var b strings.Builder
	switch m.mode {
	case modeCalibrate:
		return m.calibrateSidebar()
	case modeMove:
		med, _, usable := coverage(m.nodePoints(), m.snap.Room.Width, m.snap.Room.Height)
		b.WriteString(bold.Render("Placement") + "\n\n")
		b.WriteString(verdictPlain(len(m.nodes()), med) + "\n\n")
		if !math.IsInf(med, 1) {
			b.WriteString(dim.Render(fmt.Sprintf("covers %.0f%% of the room", usable*100)) + "\n")
		}
		b.WriteString("\n" + multiline(dim, "greener floor is better.\nthe corners matter most —\nthat is where two rings\nmeet at a shallow angle."))
		return b.String()
	}

	if len(m.snap.Devices) == 0 {
		return bold.Render("Nothing tracked") + "\n\n" +
			multiline(dim, "Only devices you have named\nare reported. Everything else\nis dropped before it is\nwritten down anywhere.")
	}

	for _, d := range m.snap.Devices {
		b.WriteString(bold.Render(d.Label) + "\n")
		if len(d.Heard) == 0 {
			b.WriteString(dim.Render("  not heard just now") + "\n\n")
			continue
		}
		best := d.Heard[0]
		for _, h := range d.Heard {
			if h.Metres < best.Metres {
				best = h
			}
		}
		b.WriteString("  " + whereabouts(best.Metres) + "\n")
		b.WriteString(dim.Render(fmt.Sprintf("  %.1f m · %s", best.Metres, confidence(best.Basis))) + "\n")
		if d.InEar != nil {
			if *d.InEar {
				b.WriteString(okc.Render("  being worn") + "\n")
			} else {
				b.WriteString(dim.Render("  not being worn") + "\n")
			}
		}
		for _, h := range d.Heard {
			word, col := reliability(h.HeardRatio)
			b.WriteString(lipgloss.NewStyle().Foreground(col).Render(
				fmt.Sprintf("  %s %s", bar(h.HeardRatio), h.Node)) + "\n")
			if h.HeardRatio < 0.6 {
				b.WriteString(dim.Render("    "+word) + "\n")
			}
		}
		if d.Unreliable != nil {
			b.WriteString(multiline(dim, "  distance not comparable\n  while it is not worn") + "\n")
		}
		b.WriteString("\n")
	}
	return b.String()
}

func (m model) calibrateSidebar() string {
	var b strings.Builder
	b.WriteString(bold.Render("Measuring") + "\n")
	b.WriteString(accent.Render(m.calDevice) + "\n\n")

	if m.calFitted != nil {
		b.WriteString(okc.Render("Done.") + "\n\n")
		b.WriteString(dim.Render(fmt.Sprintf("at 1 m   %.0f dBm", m.calFitted.RSSIAtOneMetre)) + "\n")
		b.WriteString(dim.Render(fmt.Sprintf("falloff  %.2f", m.calFitted.Exponent)) + "\n")
		b.WriteString("\n" + multiline(dim, falloffMeans(m.calFitted.Exponent)) + "\n")
	} else if m.calStage < len(calDistances) {
		d := calDistances[m.calStage]
		b.WriteString(bold.Render(fmt.Sprintf("Stand %.0f m away", d)) + "\n")
		b.WriteString(dim.Render("from the highlighted node,") + "\n")
		b.WriteString(dim.Render("holding the device.") + "\n\n")
		ns := m.nodes()
		if len(ns) > 0 {
			if r, ok := m.rssiFor(m.calDevice, ns[m.sel].Name); ok {
				b.WriteString(fmt.Sprintf("reading  %s\n", bold.Render(fmt.Sprintf("%d dBm", r))))
			} else {
				b.WriteString(alarm.Render("cannot hear it\n"))
			}
		}
		b.WriteString("\n" + dim.Render(fmt.Sprintf("%d of %d taken", m.calStage, len(calDistances))))
	}
	if m.calStatus != "" {
		b.WriteString("\n\n" + dim.Render(wrap(m.calStatus, 26)))
	}
	return b.String()
}

// falloffMeans explains the exponent without naming it, because the number is
// only useful if you know what a normal one looks like.
func falloffMeans(n float64) string {
	switch {
	case n < 2.2:
		return "almost nothing in the way —\nan unusually open room."
	case n < 3.2:
		return "normal for a room with\nfurniture in it."
	case n < 4:
		return "something solid is in the\nway of at least one reading."
	default:
		return "very lossy. A wall, or a\nreading taken through you."
	}
}

func verdictPlain(nodes int, med float64) string {
	switch {
	case nodes < 2:
		return "One ear hears a distance,\nnever a direction.\n\nA second gives you two\npossible places; a third\nnarrows it to one."
	case nodes == 2:
		return "Two rings cross in two\nplaces. You will know how\nfar, and have two candidates\nfor where.\n\nA third resolves it."
	case math.IsInf(med, 1):
		return multiline(alarm, "These nodes are in a line.\nThere is no second direction\nto solve in — move one off\nthe axis.")
	case med < 3:
		return multiline(okc, "Good geometry.")
	case med < 8:
		return "Workable, but a small error\nin signal becomes a large\none in position."
	default:
		return multiline(alarm, "Poor. Move a node so it\nsees the room from a\ndifferent direction.")
	}
}

// footer offers only what is live right now. A key that does nothing is a key
// you have to learn to ignore.
func (m model) footer() string {
	var keys []string
	switch m.mode {
	case modeWatch:
		if len(m.nodes()) > 1 {
			keys = append(keys, "tab next node")
		}
		if len(m.nodes()) > 0 {
			keys = append(keys, "m move it")
		}
		if len(m.snap.Devices) > 0 {
			keys = append(keys, "c measure a device")
		}
		keys = append(keys, "q quit")
	case modeMove:
		keys = []string{"←↑↓→ move", "s best spot", "enter place it", "esc cancel"}
	case modeCalibrate:
		switch {
		case m.calFitted != nil:
			keys = []string{"esc done"}
		case m.calRun:
			keys = []string{"esc stop"}
		default:
			keys = []string{"enter start", "tab other device", "esc cancel"}
		}
	}
	return " " + dim.Render(strings.Join(keys, "   ·   "))
}

// multiline styles each line separately. Styling a whole block pads every line
// to the widest one and eats the trailing newline, which quietly shifts
// whatever is written next.
func multiline(st lipgloss.Style, s string) string {
	lines := strings.Split(s, "\n")
	for i, l := range lines {
		lines[i] = st.Render(l)
	}
	return strings.Join(lines, "\n")
}

func wrap(s string, w int) string {
	var out, line strings.Builder
	for _, word := range strings.Fields(s) {
		if line.Len()+len(word)+1 > w {
			out.WriteString(line.String() + "\n")
			line.Reset()
		}
		if line.Len() > 0 {
			line.WriteString(" ")
		}
		line.WriteString(word)
	}
	out.WriteString(line.String())
	return out.String()
}
