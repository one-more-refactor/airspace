package main

import (
	"testing"
	"time"
)

// The contract that matters: one keypress starts the run and the keyboard is
// never needed again, because the person being measured is across the room
// holding the device. This drives the whole run with nothing but frame ticks.
func TestRunNeedsExactlyOneKeypress(t *testing.T) {
	m := model{api: fakeCollector(t, sample()), moved: map[string]point{}, width: 96, height: 30}
	s, err := m.api.state()
	if err != nil {
		t.Skip(err)
	}
	m.snap = s
	if len(m.snap.Devices) == 0 {
		t.Skip("fixture has no devices")
	}
	m.mode, m.calDevice = modeCalibrate, m.snap.Devices[0].Label

	// The one keypress.
	mi, _ := m.key(keyOf("enter"))
	cur := mi.(model)
	if !cur.calRun {
		t.Fatal("enter should start the run")
	}
	if got := time.Until(cur.calDeadline); got > calStationFor+time.Second || got < calStationFor-time.Second {
		t.Fatalf("first station should be %s out, got %s", calStationFor, got)
	}

	for station := 0; station < len(calDistances); station++ {
		if cur.calStage != station {
			t.Fatalf("expected to be at station %d, got %d", station, cur.calStage)
		}
		// A tick before the buzzer must not capture anything.
		before := len(cur.calSamples)
		mi, _ = cur.Update(frameMsg(time.Now()))
		cur = mi.(model)
		if len(cur.calSamples) != before {
			t.Fatal("captured a reading before the countdown finished")
		}

		// The buzzer.
		cur.calDeadline = time.Now().Add(-time.Millisecond)
		mi, _ = cur.Update(frameMsg(time.Now()))
		cur = mi.(model)
		if len(cur.calSamples) != before+1 {
			t.Fatalf("station %d: expected a reading to be captured", station)
		}
		if cur.calFlashUntil.IsZero() {
			t.Fatal("a capture should be shown before moving on")
		}

		// The flash expiring is what advances the run — not a keypress.
		mi, _ = cur.Update(frameMsg(cur.calFlashUntil.Add(time.Millisecond)))
		cur = mi.(model)
	}

	if cur.calRun {
		t.Fatal("run should end after the last station")
	}
	if len(cur.calSamples) != len(calDistances) {
		t.Fatalf("expected %d readings, got %d", len(calDistances), len(cur.calSamples))
	}
	for i, s := range cur.calSamples {
		if s[0] != calDistances[i] {
			t.Fatalf("reading %d recorded against %.0f m, want %.0f m", i, s[0], calDistances[i])
		}
	}
}

// A station the node cannot hear is skipped with a note rather than aborting:
// not hearing it from eight metres is itself a result, and the fit only needs
// two points.
func TestUnheardStationIsSkippedNotFatal(t *testing.T) {
	m := model{api: fakeCollector(t, sample()), moved: map[string]point{}, width: 96, height: 30}
	s, _ := m.api.state()
	m.snap, m.mode = s, modeCalibrate
	m.calDevice = "a device nothing can hear"
	m.calRun, m.calStage = true, 0
	m.calDeadline = time.Now().Add(-time.Millisecond)

	mi, _ := m.Update(frameMsg(time.Now()))
	cur := mi.(model)
	if len(cur.calSamples) != 0 {
		t.Fatal("should not invent a reading for a device it cannot hear")
	}
	if cur.calNote == "" {
		t.Fatal("should say why the station was skipped")
	}
	if !cur.calRun {
		t.Fatal("an unheard station should not end the run")
	}
}

func TestMedianIgnoresAStraySample(t *testing.T) {
	// One advertisement several dB off should not become the reading.
	if got := medianInt([]int{-61, -60, -20, -62, -60}); got != -60 {
		t.Fatalf("median = %d, want -60", got)
	}
}
