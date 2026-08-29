// airspace-console — the operations view.
//
// Not a setup wizard. A wizard runs once and lies forever afterwards:
// calibration drifts when furniture moves, a door closes, a node gets nudged.
// This is a thing you leave open, which keeps measuring and tells you when the
// picture stopped being true. Initial and continuous calibration are the same
// code path; the only difference is whether you are being prompted.
//
// It also exists because every failure in this project so far presented as
// SILENCE — a beacon blind after suspend, a collector alive with a dead D-Bus
// connection, firmware logging to pins with no cable on them — and silence is
// indistinguishable from a quiet room. A live rate makes that visible without
// anyone having to suspect it first.
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

type view int

const (
	viewLive view = iota
	viewCalibrate
	viewPlace
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
	view view

	snap    *Snapshot
	lastErr error
	// When the collector last answered. The console must not present stale
	// numbers as live ones — that is the failure mode it exists to catch.
	lastOK time.Time

	cursor int

	// calibration wizard
	calDevice  string
	calNode    string
	calStage   int
	calSamples [][2]float64
	calStatus  string
	calFitted  *fitResult

	width, height int
}

// The distances to sample at. Spread over a decade so the fit has real leverage
// on the exponent: readings at 1 and 1.5 metres describe a line through almost
// the same point twice.
var calDistances = []float64{1, 2, 4, 8}

func main() {
	base := os.Getenv("AIRSPACE_URL")
	if base == "" {
		base = "http://127.0.0.1:9970"
	}
	m := model{api: newClient(base), view: viewLive}
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
	return func() tea.Msg {
		s, err := c.state()
		return stateMsg{s, err}
	}
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
		}

	case fitMsg:
		if msg.err != nil {
			m.calStatus = msg.err.Error()
			m.calFitted = nil
		} else {
			m.calFitted = msg.res
			err := saveCalibration(calibration{
				Node: m.calNode, Device: m.calDevice,
				RSSIAt1m: msg.res.RSSIAtOneMetre, Exponent: msg.res.Exponent,
				Samples: msg.res.Samples,
			})
			if err != nil {
				m.calStatus = "fitted, but could not write the file: " + err.Error()
			} else {
				m.calStatus = "saved to " + calibrationPath() +
					" — the collector picks it up on its next restart"
			}
		}

	case tea.KeyMsg:
		switch msg.String() {
		case "q", "ctrl+c":
			return m, tea.Quit
		case "tab":
			m.view = (m.view + 1) % 3
			m.cursor = 0
		case "up", "k":
			if m.cursor > 0 {
				m.cursor--
			}
		case "down", "j":
			m.cursor++
		case "enter", " ":
			return m.enter()
		case "r":
			if m.view == viewCalibrate {
				m = m.resetCalibration()
			}
		}
	}
	return m, nil
}

func (m model) enter() (tea.Model, tea.Cmd) {
	if m.view != viewCalibrate || m.snap == nil {
		return m, nil
	}
	switch {
	case m.calDevice == "":
		if m.cursor < len(m.snap.Devices) {
			m.calDevice = m.snap.Devices[m.cursor].Label
			m.cursor = 0
		}
	case m.calNode == "":
		if m.cursor < len(m.snap.Nodes) {
			m.calNode = m.snap.Nodes[m.cursor].Name
		}
	case m.calStage < len(calDistances):
		// Take the reading that node currently has for that device. Using the
		// live value rather than an average is deliberate: the point of the
		// prompt is that you are standing still at a known distance right now.
		rssi, ok := m.currentRSSI()
		if !ok {
			m.calStatus = "that node cannot hear the device from here — " +
				"which is itself a finding, and a reason to move it"
			return m, nil
		}
		m.calSamples = append(m.calSamples, [2]float64{calDistances[m.calStage], float64(rssi)})
		m.calStage++
		m.calStatus = ""
		if m.calStage == len(calDistances) {
			samples := m.calSamples
			api := m.api
			return m, func() tea.Msg {
				r, err := api.fit(samples)
				return fitMsg{r, err}
			}
		}
	}
	return m, nil
}

func (m model) currentRSSI() (int, bool) {
	if m.snap == nil {
		return 0, false
	}
	for _, d := range m.snap.Devices {
		if d.Label != m.calDevice {
			continue
		}
		for _, h := range d.Heard {
			if h.Node == m.calNode {
				return h.RSSI, true
			}
		}
	}
	return 0, false
}

func (m model) resetCalibration() model {
	m.calDevice, m.calNode, m.calStage = "", "", 0
	m.calSamples, m.calFitted, m.calStatus, m.cursor = nil, nil, "", 0
	return m
}

// ── styling ──────────────────────────────────────────────────────────────────

var (
	dim    = lipgloss.NewStyle().Foreground(lipgloss.Color("245"))
	bold   = lipgloss.NewStyle().Bold(true)
	good   = lipgloss.NewStyle().Foreground(lipgloss.Color("42"))
	warn   = lipgloss.NewStyle().Foreground(lipgloss.Color("214"))
	bad    = lipgloss.NewStyle().Foreground(lipgloss.Color("203"))
	header = lipgloss.NewStyle().Bold(true).Foreground(lipgloss.Color("39"))
)

func (m model) View() string {
	var b strings.Builder
	tabs := []string{"live", "calibrate", "place"}
	for i, t := range tabs {
		if view(i) == m.view {
			b.WriteString(header.Render("[" + t + "]"))
		} else {
			b.WriteString(dim.Render(" " + t + " "))
		}
		b.WriteString(" ")
	}
	b.WriteString(dim.Render("  tab switches · q quits"))
	b.WriteString("\n\n")

	// Staleness is reported before anything else. A console showing numbers
	// from a collector that stopped answering four minutes ago is exactly the
	// silent failure this whole thing exists to make loud.
	if m.lastErr != nil {
		b.WriteString(bad.Render("collector unreachable: "+m.lastErr.Error()) + "\n")
		if !m.lastOK.IsZero() {
			b.WriteString(dim.Render(fmt.Sprintf("last answered %s ago — everything below is stale\n",
				time.Since(m.lastOK).Round(time.Second))))
		}
		b.WriteString("\n")
	}
	if m.snap == nil {
		b.WriteString(dim.Render("waiting for the collector…"))
		return b.String()
	}

	switch m.view {
	case viewLive:
		b.WriteString(m.liveView())
	case viewCalibrate:
		b.WriteString(m.calibrateView())
	case viewPlace:
		b.WriteString(m.placeView())
	}
	return b.String()
}

func (m model) liveView() string {
	var b strings.Builder
	s := m.snap

	b.WriteString(bold.Render(fmt.Sprintf("%d tracked device(s), %d node(s)", len(s.Devices), len(s.Nodes))))
	if !s.CanLocate {
		b.WriteString(dim.Render("  — distance only; direction needs three nodes"))
	}
	b.WriteString("\n\n")

	if len(s.Devices) == 0 {
		b.WriteString(dim.Render("Nothing being tracked.\n\n" +
			"That is not the same as nothing being there: with track_only_known set,\n" +
			"only devices with a configured identity are reported. Everything else is\n" +
			"dropped before it is written down anywhere."))
		return b.String()
	}

	for _, d := range s.Devices {
		name := bold.Render(d.Label)
		if !d.Known {
			name += dim.Render(" (unrecognised)")
		}
		b.WriteString(name + "\n")
		b.WriteString(dim.Render("  "+d.ID) + "\n")

		for _, h := range d.Heard {
			// The basis is shown every time. A measured distance and a guessed
			// one look identical if you only print the number.
			basis := dim.Render("assumed")
			switch h.Basis {
			case "measured":
				basis = good.Render("measured")
			case "advertised":
				basis = warn.Render("advertised")
			}
			ratio := good.Render(fmt.Sprintf("%3.0f%%", h.HeardRatio*100))
			switch {
			case h.HeardRatio < 0.3:
				ratio = bad.Render(fmt.Sprintf("%3.0f%%", h.HeardRatio*100))
			case h.HeardRatio < 0.7:
				ratio = warn.Render(fmt.Sprintf("%3.0f%%", h.HeardRatio*100))
			}
			b.WriteString(fmt.Sprintf("  %-10s %6.1f m  %4d dBm  %-22s heard %s  %s\n",
				h.Node, h.Metres, h.RSSI, basis, ratio, dim.Render(fmt.Sprintf("%ds ago", h.Age))))
		}
		if d.InEar != nil {
			state := "out of ear"
			if *d.InEar {
				state = "in ear"
			}
			b.WriteString(dim.Render("  "+state) + "\n")
		}
		if d.Unreliable != nil {
			b.WriteString(warn.Render("  ! "+*d.Unreliable) + "\n")
		}
		b.WriteString("\n")
	}
	return b.String()
}

func (m model) calibrateView() string {
	var b strings.Builder
	b.WriteString(bold.Render("Calibrate a device against a node") + "\n")
	b.WriteString(dim.Render(
		"Distance is currently textbook constants: −59 dBm at one metre, exponent 2.5.\n"+
			"Both are wrong for your walls. Measuring replaces them with a fit.\n") + "\n")

	switch {
	case m.calDevice == "":
		b.WriteString("Which device?\n\n")
		for i, d := range m.snap.Devices {
			p := "  "
			if i == m.cursor {
				p = header.Render("> ")
			}
			b.WriteString(p + d.Label + "\n")
		}
	case m.calNode == "":
		b.WriteString("Heard by which node?\n\n")
		for i, n := range m.snap.Nodes {
			p := "  "
			if i == m.cursor {
				p = header.Render("> ")
			}
			b.WriteString(fmt.Sprintf("%s%s  (%.1f, %.1f)\n", p, n.Name, n.X, n.Y))
		}
	case m.calStage < len(calDistances):
		d := calDistances[m.calStage]
		b.WriteString(fmt.Sprintf("Stand %.0f metre(s) from %s, holding %s.\n",
			d, bold.Render(m.calNode), bold.Render(m.calDevice)))
		b.WriteString(dim.Render(
			"Line of sight if you can — a body between the two is worth about 10 dB,\n"+
				"which at these exponents is a factor of two and a half in apparent distance.\n"+
				"If it is an earbud, wear it: a bud in a pocket is a different measurement.\n") + "\n")
		if r, ok := m.currentRSSI(); ok {
			b.WriteString(fmt.Sprintf("  reading now: %s\n", bold.Render(fmt.Sprintf("%d dBm", r))))
		} else {
			b.WriteString(bad.Render("  that node cannot hear it from here\n"))
		}
		b.WriteString("\n" + header.Render("  press enter to take the sample") + "\n")
		if len(m.calSamples) > 0 {
			b.WriteString("\n" + dim.Render("taken so far:") + "\n")
			for _, s := range m.calSamples {
				b.WriteString(dim.Render(fmt.Sprintf("  %.0f m → %.0f dBm\n", s[0], s[1])))
			}
		}
	default:
		if m.calFitted != nil {
			b.WriteString(good.Render("Fitted.") + "\n\n")
			b.WriteString(fmt.Sprintf("  at one metre:  %.1f dBm\n", m.calFitted.RSSIAtOneMetre))
			b.WriteString(fmt.Sprintf("  exponent:      %.2f\n", m.calFitted.Exponent))
			b.WriteString(dim.Render(fmt.Sprintf(
				"\n  Free space is 2.0. A flat is usually 2.5 to 4. Above 4 means\n"+
					"  something substantial is in the way of at least one reading.\n")))
		} else {
			b.WriteString("fitting…\n")
		}
	}

	if m.calStatus != "" {
		b.WriteString("\n" + warn.Render(m.calStatus) + "\n")
	}
	b.WriteString("\n" + dim.Render("r starts over"))
	return b.String()
}

func (m model) placeView() string {
	var b strings.Builder
	s := m.snap
	nodes := make([]point, 0, len(s.Nodes))
	for _, n := range s.Nodes {
		nodes = append(nodes, point{n.X, n.Y})
	}

	b.WriteString(bold.Render("Node placement") + "\n")
	b.WriteString(dim.Render(
		"Two ears in a line tell you almost nothing; two spread across the room give\n"+
			"rings that cross at a useful angle. This scores the geometry you have.\n") + "\n")

	med, worst, usable := coverage(nodes, s.Room.Width, s.Room.Height)
	b.WriteString(fmt.Sprintf("  room       %.1f × %.1f m\n", s.Room.Width, s.Room.Height))
	b.WriteString(fmt.Sprintf("  nodes      %d\n", len(nodes)))
	for _, n := range s.Nodes {
		b.WriteString(dim.Render(fmt.Sprintf("             %-10s (%.1f, %.1f)\n", n.Name, n.X, n.Y)))
	}
	if math.IsInf(med, 1) {
		b.WriteString("  geometry   " + bad.Render("not solvable") + "\n")
	} else {
		style := good
		if med >= 5 {
			style = warn
		}
		if med >= 10 {
			style = bad
		}
		b.WriteString("  dilution   " + style.Render(fmt.Sprintf("%.1f median, %.1f worst", med, worst)) + "\n")
		b.WriteString(fmt.Sprintf("  usable     %.0f%% of the room\n", usable*100))
	}
	b.WriteString("\n  " + verdict(len(nodes), med) + "\n")

	if len(nodes) >= 1 {
		at, score := suggest(nodes, s.Room.Width, s.Room.Height)
		if !math.IsInf(score, 1) {
			b.WriteString("\n" + bold.Render("Best place for the next node") + "\n")
			b.WriteString(fmt.Sprintf("  (%.1f, %.1f) — would give a median dilution of %.1f\n", at.X, at.Y, score))
			b.WriteString(dim.Render(
				"  Geometry only. It does not know where there is a plug socket,\n" +
					"  a shelf, or a wall.\n"))
		}
	}
	return b.String()
}
