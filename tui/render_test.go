package main

import (
	"encoding/json"
	"net/http"
	"net/http/httptest"
	"strings"
	"testing"
)

func fakeCollector(t *testing.T, snap Snapshot) *client {
	t.Helper()
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		_ = json.NewEncoder(w).Encode(snap)
	}))
	t.Cleanup(srv.Close)
	return newClient(srv.URL)
}

func sample() Snapshot {
	return Snapshot{
		Room:  Room{Width: 10, Height: 8},
		Nodes: []Node{{Name: "carl", X: 5, Y: 4}, {Name: "ear1", X: 1, Y: 7}},
		Devices: []Device{
			{ID: "EC:A9:07:A0:3A:60", Label: "philip's phone", Known: true,
				Heard: []Heard{{Node: "carl", RSSI: -58, Metres: 3.2, Basis: "advertised", HeardRatio: 1}}},
			{ID: "1C:B3:C9:C5:44:14", Label: "airpods", Known: true,
				Heard: []Heard{{Node: "carl", RSSI: -70, Metres: 9, Basis: "assumed", HeardRatio: 0.2}}},
		},
	}
}

// Every mode has to render without a terminal. This is where a nil pointer
// hides: a device with no readings, a room with no nodes, a fit that has not
// come back yet.
func TestEveryModeRenders(t *testing.T) {
	m := model{api: fakeCollector(t, sample()), moved: map[string]point{}, width: 100, height: 30}
	got, err := m.api.state()
	if err != nil {
		t.Fatal(err)
	}
	m.snap = got
	for _, mo := range []mode{modeWatch, modeMove, modeCalibrate} {
		m.mode = mo
		if out := m.View(); out == "" {
			t.Fatalf("mode %d rendered nothing", mo)
		}
	}
}

func TestDegenerateStatesDoNotPanic(t *testing.T) {
	empty := Snapshot{Room: Room{Width: 10, Height: 8}}
	m := model{api: fakeCollector(t, empty), moved: map[string]point{}, width: 100, height: 30}
	s, _ := m.api.state()
	m.snap = s
	for _, mo := range []mode{modeWatch, modeMove, modeCalibrate} {
		m.mode = mo
		_ = m.View()
	}
	// A device that is not currently heard by anything.
	m.snap.Devices = []Device{{ID: "x", Label: "silent"}}
	m.mode = modeWatch
	if !strings.Contains(m.View(), "silent") {
		t.Fatal("a device with no readings should still be listed")
	}
}

// The console must never present numbers from a collector that stopped
// answering as though they were live — that is the exact silent failure the
// whole project keeps running into.
func TestSaysSoWhenItCannotReachTheCollector(t *testing.T) {
	m := model{api: newClient("http://127.0.0.1:1"), moved: map[string]point{}, width: 100, height: 30}
	out := m.View()
	if !strings.Contains(out, "Listening") && !strings.Contains(out, "Cannot reach") {
		t.Fatalf("must say what is happening, got: %q", out)
	}
}

// Moving a node must change the picture and stay inside the room.
func TestMovingANodeIsBoundedByTheRoom(t *testing.T) {
	m := model{api: fakeCollector(t, sample()), moved: map[string]point{}, width: 100, height: 30}
	s, _ := m.api.state()
	m.snap, m.mode, m.sel = s, modeMove, 0

	for i := 0; i < 200; i++ {
		next, _ := m.key(keyOf("left"))
		m = next.(model)
	}
	p := m.moved["carl"]
	if p.X < 0 {
		t.Fatalf("node escaped the room: %v", p)
	}
	for i := 0; i < 200; i++ {
		next, _ := m.key(keyOf("down"))
		m = next.(model)
	}
	if m.moved["carl"].Y > m.snap.Room.Height {
		t.Fatalf("node escaped the room: %v", m.moved["carl"])
	}
}

// Cancelling a move must put the node back, not half-apply it.
func TestEscapeAbandonsAMove(t *testing.T) {
	m := model{api: fakeCollector(t, sample()), moved: map[string]point{}, width: 100, height: 30}
	s, _ := m.api.state()
	m.snap, m.mode, m.sel = s, modeMove, 0
	next, _ := m.key(keyOf("left"))
	m = next.(model)
	if _, ok := m.moved["carl"]; !ok {
		t.Fatal("move should have been staged")
	}
	next, _ = m.key(keyOf("esc"))
	m = next.(model)
	if _, ok := m.moved["carl"]; ok {
		t.Fatal("escape should have abandoned the staged move")
	}
	if m.mode != modeWatch {
		t.Fatal("escape should return to watching")
	}
}

// The plain-language layer is the entire point of the redesign: nobody should
// need to know what an exponent is to read the screen.
func TestPlainLanguageCoversTheRange(t *testing.T) {
	for _, d := range []float64{0.5, 2, 5, 10, 40} {
		if whereabouts(d) == "" {
			t.Fatalf("no words for %v m", d)
		}
	}
	for _, b := range []string{"measured", "advertised", "assumed", "something else"} {
		if confidence(b) == "" {
			t.Fatalf("no words for basis %q", b)
		}
	}
	for _, n := range []float64{1.8, 2.5, 3.5, 5} {
		if falloffMeans(n) == "" {
			t.Fatalf("no words for exponent %v", n)
		}
	}
	// A single node must be told about honestly rather than scored.
	if !strings.Contains(verdictPlain(1, 0), "direction") {
		t.Fatal("one node should be explained, not graded")
	}
}
