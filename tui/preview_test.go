package main

import (
	"fmt"
	"testing"
)

// Not an assertion — a way to look at the thing. `go test -run Preview -v`
// prints each mode so the design can be judged rather than assumed.
func TestPreview(t *testing.T) {
	m := model{api: fakeCollector(t, sample()), moved: map[string]point{}, width: 104, height: 28}
	s, err := m.api.state()
	if err != nil {
		t.Skip(err)
	}
	m.snap = s
	for _, c := range []struct {
		name string
		mo   mode
	}{{"WATCHING", modeWatch}, {"MOVING A NODE", modeMove}, {"MEASURING", modeCalibrate}} {
		m.mode = c.mo
		if c.mo == modeCalibrate {
			m.calDevice = "philip's phone"
		}
		fmt.Printf("\n───── %s ─────\n%s\n", c.name, m.View())
	}
}
