package main

import "math"

// Geometry, and how much of it you are wasting.
//
// Two receivers in a line tell you almost nothing; two spread across the room
// give you rings that cross at a useful angle. That difference has a name and a
// number — geometric dilution of precision, borrowed from GPS — and it can be
// computed for a layout before anyone tapes anything to a shelf.
//
// The intuition: each node contributes a direction along which it can pin the
// device down. Nodes that see a point from similar directions contribute
// almost the same information twice, so a small error in signal strength turns
// into a large error in position. GDOP is the multiplier from one to the other.

type point struct{ X, Y float64 }

// gdopAt returns the dilution factor at one position, or +Inf when the geometry
// is degenerate — fewer than two nodes, or all of them in a straight line
// through the point, which no amount of accurate ranging can rescue.
func gdopAt(nodes []point, at point) float64 {
	if len(nodes) < 2 {
		return math.Inf(1)
	}
	// H is the geometry matrix: one row per node, holding the unit vector from
	// that node to the point. We only need H'H, which for two dimensions is a
	// 2x2 we can invert by hand rather than pulling in a linear algebra library
	// for four numbers.
	var a, b, c float64 // [[a b],[b c]]
	for _, n := range nodes {
		dx, dy := at.X-n.X, at.Y-n.Y
		d := math.Hypot(dx, dy)
		if d < 0.3 {
			// Standing on top of a node: the direction is undefined and the
			// ranging is excellent. Neither is interesting for placement.
			return math.Inf(1)
		}
		ux, uy := dx/d, dy/d
		a += ux * ux
		b += ux * uy
		c += uy * uy
	}
	det := a*c - b*b
	if math.Abs(det) < 1e-9 {
		return math.Inf(1) // collinear: no second dimension to solve in
	}
	// trace of the inverse of a symmetric 2x2
	trace := (c + a) / det
	if trace <= 0 {
		return math.Inf(1)
	}
	return math.Sqrt(trace)
}

// coverage scores a whole layout by sampling the room.
//
// Returns the median and the worst dilution over a grid, plus the fraction of
// the room where the geometry is usable at all. Median rather than mean because
// one unreachable corner should not condemn an otherwise good layout, and the
// worst case is reported separately rather than averaged away.
func coverage(nodes []point, w, h float64) (median, worst, usable float64) {
	if len(nodes) < 2 {
		return math.Inf(1), math.Inf(1), 0
	}
	const step = 0.5
	var vals []float64
	total, ok := 0, 0
	for y := step / 2; y < h; y += step {
		for x := step / 2; x < w; x += step {
			g := gdopAt(nodes, point{x, y})
			total++
			if math.IsInf(g, 1) {
				continue
			}
			ok++
			vals = append(vals, g)
		}
	}
	if len(vals) == 0 {
		return math.Inf(1), math.Inf(1), 0
	}
	// insertion sort: the grid is small and this avoids an import
	for i := 1; i < len(vals); i++ {
		for j := i; j > 0 && vals[j] < vals[j-1]; j-- {
			vals[j], vals[j-1] = vals[j-1], vals[j]
		}
	}
	median = vals[len(vals)/2]
	worst = vals[len(vals)-1]
	usable = float64(ok) / float64(total)
	return
}

// verdict turns a dilution figure into something a person can act on.
//
// The thresholds are the conventional GPS ones. They are a guide rather than a
// law, and the honest thing to say about a two-node layout is that no threshold
// rescues it: two circles cross in two places, so the geometry is ambiguous
// before it is imprecise.
func verdict(nodes int, median float64) string {
	if nodes < 2 {
		return "one ear hears a distance, never a direction — add a second"
	}
	if nodes == 2 {
		return "two ears give two possible places, not one — a third resolves the ambiguity"
	}
	switch {
	case math.IsInf(median, 1):
		return "degenerate — the nodes are in a line, so there is no second dimension to solve in"
	case median < 2:
		return "excellent geometry"
	case median < 5:
		return "good geometry"
	case median < 10:
		return "usable, but a small signal error becomes a large position error"
	default:
		return "poor — move a node so it sees the room from a different direction"
	}
}

// suggest looks for somewhere better to put the next node.
//
// Brute force over candidate positions, scoring each by the median dilution it
// would produce. Crude, and honest about it: it optimises geometry alone and
// knows nothing about where there is a plug socket, a shelf, or a wall.
func suggest(existing []point, w, h float64) (point, float64) {
	best := point{}
	bestScore := math.Inf(1)
	const step = 0.5
	for y := step; y < h; y += step {
		for x := step; x < w; x += step {
			cand := append(append([]point{}, existing...), point{x, y})
			med, _, usable := coverage(cand, w, h)
			if usable < 0.9 {
				continue
			}
			if med < bestScore {
				bestScore, best = med, point{x, y}
			}
		}
	}
	return best, bestScore
}
