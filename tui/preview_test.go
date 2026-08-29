package main

import (
	"fmt"
	"testing"
	"time"
)

// Not an assertion — a way to look at the thing. `go test -run Preview -v`
// prints each mode so the design can be judged rather than assumed.
func TestPreview(t *testing.T) {
	m := model{api: fakeCollector(t, sample()), moved: map[string]point{}, width: 104, height: 46}
	s, err := m.api.state()
	if err != nil {
		t.Skip(err)
	}
	m.snap = s
	for _, c := range []struct {
		name string
		mo   mode
	}{{"WATCHING", modeWatch}, {"MOVING A NODE", modeMove}} {
		m.mode = c.mo
		fmt.Printf("\n───── %s ─────\n%s\n", c.name, m.View())
	}
}

// TestPreviewMeasuring prints the states of a calibration run so the screen can
// be judged rather than assumed. `go test -run PreviewMeasuring -v`.
func TestPreviewMeasuring(t *testing.T) {
	m := model{api: fakeCollector(t, sample()), moved: map[string]point{}, width: 96, height: 30}
	s, err := m.api.state()
	if err != nil {
		t.Skip(err)
	}
	m.snap, m.mode, m.calDevice = s, modeCalibrate, "philip's phone"

	show := func(name string, mm model) {
		fmt.Printf("\n═════ %s ═════\n%s\n", name, mm.View())
	}

	show("READY", m)

	run := m
	run.calRun, run.calStage, run.calFrame = true, 1, 9
	run.calSamples = [][2]float64{{1, -57}}
	run.calDeadline = time.Now().Add(7 * time.Second)
	show("COUNTING (station 2 of 4, 7s left)", run)

	last := run
	last.calStage, last.calFrame = 3, 21
	last.calSamples = [][2]float64{{1, -57}, {2, -66}, {4, -74}}
	last.calDeadline = time.Now().Add(2 * time.Second)
	show("COUNTING (last station, 2s left)", last)

	flash := run
	flash.calFlashUntil = time.Now().Add(time.Second)
	flash.calSamples = [][2]float64{{1, -57}, {2, -66}}
	show("CAPTURED", flash)

	done := m
	done.calFitted = &fitResult{RSSIAtOneMetre: -57.5, Exponent: 2.85, Samples: 4}
	show("MEASURED", done)
}
