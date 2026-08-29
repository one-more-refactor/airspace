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
	// Assert against the ladder this room actually uses, not a fixed one. The
	// stations adapt to the room now: a reading taken through a wall is not
	// describing the same propagation as the others and drags the exponent up.
	want := cur.stations()
	if len(cur.calSamples) != len(want) {
		t.Fatalf("expected %d readings, got %d", len(want), len(cur.calSamples))
	}
	for i, s := range cur.calSamples {
		if s[0] != want[i] {
			t.Fatalf("reading %d recorded against %.1f m, want %.1f m", i, s[0], want[i])
		}
	}
}

// The stations have to suit the room. Eight metres in a small flat is a
// reading taken through a wall, which is not the same propagation as the
// others and quietly widens every distance the model later reports.
func TestStationsFitTheRoom(t *testing.T) {
	for _, r := range []Room{{Width: 10, Height: 8}, {Width: 4, Height: 3}, {Width: 30, Height: 20}} {
		st := stationsFor(r)
		if len(st) != 4 {
			t.Fatalf("%v: want four stations, got %d", r, len(st))
		}
		longest := r.Width
		if r.Height > longest {
			longest = r.Height
		}
		if st[0] != 1 {
			t.Fatalf("%v: the first station should be one metre, got %v", r, st[0])
		}
		for i := 1; i < len(st); i++ {
			if st[i] <= st[i-1] {
				t.Fatalf("%v: stations must increase, got %v", r, st)
			}
		}
		// Never further than the room, and never so close together that the
		// fit has no leverage on the exponent.
		if st[3] > longest {
			t.Fatalf("%v: furthest station %v is outside the room", r, st[3])
		}
		if st[3] < 2.5 {
			t.Fatalf("%v: stations %v are too bunched to fit a falloff", r, st)
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
