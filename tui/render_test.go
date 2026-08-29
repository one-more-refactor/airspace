package main

import (
	"encoding/json"
	"net/http"
	"net/http/httptest"
	"strings"
	"testing"
)

// The views are where a nil pointer hides: a device with no heard entries, a
// collector that never answered, an in-ear field that is absent rather than
// false. Rendering them headless is cheap and catches all three.
func TestViewsRenderWithoutATTY(t *testing.T) {
	snap := Snapshot{
		Room:  Room{Width: 10, Height: 8},
		Nodes: []Node{{Name: "carl", X: 5, Y: 4}},
		Devices: []Device{
			{ID: "EC:A9:07:A0:3A:60", Label: "philip's phone", Known: true,
				Heard: []Heard{{Node: "carl", RSSI: -58, Metres: 3.2, Basis: "advertised", HeardRatio: 1}}},
			{ID: "1C:B3:C9:C5:44:14", Label: "airpods", Known: true,
				Heard: []Heard{{Node: "carl", RSSI: -70, Metres: 9, Basis: "assumed", HeardRatio: 0.2}}},
		},
	}
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		json.NewEncoder(w).Encode(snap)
	}))
	defer srv.Close()

	c := newClient(srv.URL)
	got, err := c.state()
	if err != nil {
		t.Fatal(err)
	}
	m := model{api: c, snap: got}
	for _, v := range []view{viewLive, viewCalibrate, viewPlace} {
		m.view = v
		out := m.View()
		if out == "" {
			t.Fatalf("view %d rendered nothing", v)
		}
	}
	// A device with no readings at all must not panic the live view.
	m.snap.Devices = append(m.snap.Devices, Device{ID: "x", Label: "silent"})
	m.view = viewLive
	if !strings.Contains(m.View(), "silent") {
		t.Fatal("a device with no readings should still be listed")
	}
	// And the console must say so when it has never reached the collector.
	empty := model{api: newClient("http://127.0.0.1:1")}
	if !strings.Contains(empty.View(), "waiting") {
		t.Fatal("must say it is waiting rather than showing an empty room")
	}
}
