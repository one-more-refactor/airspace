package main

import (
	"os"
	"path/filepath"
	"strings"
	"testing"
)

// A collector — or anything pretending to be one — must not be able to drive
// the terminal through a device label or a node name.
func TestHostileNamesNeverReachTheTerminal(t *testing.T) {
	osc52 := "\x1b]52;c;Y3VybCB4LnNofHNoCg==\x07"
	snap := sample()
	snap.Nodes[0].Name = "carl\x1b[2J"
	snap.Devices[0].Label = osc52 + "phone"
	snap.Devices[0].Heard[0].Node = "carl\x1b[2J"
	evil := "\x1b]2;PWNED\x07acme"
	snap.Devices[0].Vendor = &evil
	snap.Devices[0].Doing = []string{"x\x1b[H"}
	snap.Devices[1].Label = "Home\u202etxt.exe\u200b"

	m := model{api: fakeCollector(t, snap), moved: map[string]point{}, width: 120, height: 40}
	got, err := m.api.state()
	if err != nil {
		t.Fatal(err)
	}
	m.snap = got
	if got.Devices[1].Label != "Hometxt.exe" {
		t.Errorf("bidi/zero-width not stripped: %q", got.Devices[1].Label)
	}
	m.calDevice = got.Devices[0].Label

	for _, mode := range []mode{modeWatch, modeMove, modeCalibrate} {
		m.mode = mode
		out := m.View()
		// Rendering adds SGR colour (ESC [ … m) of its own; what must not
		// appear is anything that came from the collector.
		for _, bad := range []string{"\x1b]", "\x07", "\x1b[2J", "\x1b[H", "\u202e", "\u200b"} {
			if strings.Contains(out, bad) {
				t.Errorf("mode %v: output contains %q", mode, bad)
			}
		}
	}
}

func TestPrintableClampsAndKeepsOrdinaryText(t *testing.T) {
	if got := printable("Philip’s AirPods Pro"); got != "Philip’s AirPods Pro" {
		t.Errorf("ordinary text changed: %q", got)
	}
	if n := len([]rune(printable(strings.Repeat("é", 300)))); n != maxLabel {
		t.Errorf("clamped to %d runes, want %d", n, maxLabel)
	}
}

// A node name with a control character used to be written with Go's %q, which
// is not TOML, and the collector then refused to start.
func TestCalibrationFileIsValidTOMLAndRoundTrips(t *testing.T) {
	dir := t.TempDir()
	t.Setenv("XDG_CONFIG_HOME", dir)
	names := []string{"carl\x1b[2J", `say "hi"`, `back\slash`, "plain"}
	for _, n := range names {
		if err := saveCalibration(calibration{Node: n, Device: "phone", RSSIAt1m: -59, Exponent: 2, Samples: 5}); err != nil {
			t.Fatal(err)
		}
	}
	data, err := os.ReadFile(filepath.Join(dir, "airspace", "calibration.toml"))
	if err != nil {
		t.Fatal(err)
	}
	if strings.Contains(string(data), `\x`) || strings.Contains(string(data), "\x1b") {
		t.Errorf("not TOML-safe:\n%s", data)
	}
	got := readCalibrations(calibrationPath())
	if len(got) != len(names) {
		t.Fatalf("read %d entries, want %d", len(got), len(names))
	}
	for i, n := range names {
		if got[i].Node != n {
			t.Errorf("entry %d: node %q, want %q", i, got[i].Node, n)
		}
	}
}

func TestTOMLStringEscapes(t *testing.T) {
	for in, want := range map[string]string{
		"a":         `"a"`,
		`"`:         `"\""`,
		`\`:         `"\\"`,
		"\x1b":      `"\u001B"`,
		"\a\x7f":    `"\u0007\u007F"`,
		"tab\there": `"tab\u0009here"`,
	} {
		if got := tomlString(in); got != want {
			t.Errorf("tomlString(%q) = %s, want %s", in, got, want)
		}
	}
}
