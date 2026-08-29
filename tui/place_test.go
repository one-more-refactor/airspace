package main

import (
	"math"
	"testing"
)

func TestCollinearNodesCannotSolve(t *testing.T) {
	// Three nodes in a straight line, and a point on that same line. There is
	// no second dimension to solve in, and reporting a number here would be
	// the exact class of confident lie this project keeps tripping over.
	line := []point{{0, 4}, {5, 4}, {10, 4}}
	if g := gdopAt(line, point{3, 4}); !math.IsInf(g, 1) {
		t.Fatalf("collinear geometry should be unsolvable, got %v", g)
	}
}

func TestSpreadBeatsClustered(t *testing.T) {
	spread := []point{{1, 1}, {9, 1}, {5, 7}}
	clustered := []point{{4.5, 4}, {5, 4}, {5.5, 4.2}}
	ms, _, _ := coverage(spread, 10, 8)
	mc, _, _ := coverage(clustered, 10, 8)
	if !(ms < mc) {
		t.Fatalf("spread nodes (%v) should beat clustered (%v)", ms, mc)
	}
	if ms > 5 {
		t.Fatalf("a sensible triangle should score well, got %v", ms)
	}
}

func TestOneNodeIsHonestAboutIt(t *testing.T) {
	if g := gdopAt([]point{{5, 4}}, point{1, 1}); !math.IsInf(g, 1) {
		t.Fatal("a single receiver has no geometry at all")
	}
	if v := verdict(1, math.Inf(1)); v == "" {
		t.Fatal("verdict must say something for the single-node case")
	}
}

func TestSuggestImprovesOnWhatExists(t *testing.T) {
	// Two nodes close together: the suggestion should be far from both.
	existing := []point{{2, 2}, {3, 2}}
	at, score := suggest(existing, 10, 8)
	if math.IsInf(score, 1) {
		t.Fatal("should find somewhere")
	}
	before, _, _ := coverage(existing, 10, 8)
	if !(score < before) {
		t.Fatalf("suggested layout (%v) should beat the current one (%v)", score, before)
	}
	if math.Hypot(at.X-2.5, at.Y-2) < 2 {
		t.Fatalf("suggestion %v is too close to the existing cluster", at)
	}
}

func TestCalibrationRoundTrips(t *testing.T) {
	dir := t.TempDir()
	t.Setenv("XDG_CONFIG_HOME", dir)
	in := calibration{Node: "carl", Device: "philip's phone", RSSIAt1m: -57.5, Exponent: 2.85, Samples: 4}
	if err := saveCalibration(in); err != nil {
		t.Fatal(err)
	}
	out := readCalibrations(calibrationPath())
	if len(out) != 1 || out[0].Node != in.Node || out[0].Device != in.Device {
		t.Fatalf("round trip lost the entry: %+v", out)
	}
	if math.Abs(out[0].Exponent-in.Exponent) > 0.001 {
		t.Fatalf("exponent drifted: %v", out[0].Exponent)
	}
	// Recalibrating replaces rather than duplicating.
	in.Exponent = 3.2
	if err := saveCalibration(in); err != nil {
		t.Fatal(err)
	}
	out = readCalibrations(calibrationPath())
	if len(out) != 1 {
		t.Fatalf("recalibration should replace, got %d entries", len(out))
	}
	if math.Abs(out[0].Exponent-3.2) > 0.001 {
		t.Fatalf("replacement did not take: %v", out[0].Exponent)
	}
}
