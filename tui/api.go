package main

import (
	"bytes"
	"encoding/json"
	"fmt"
	"net/http"
	"os"
	"path/filepath"
	"time"
)

// The collector's wire format. Only the fields the console actually uses are
// declared — the collector is free to add more without breaking this.

type Heard struct {
	Node        string  `json:"node"`
	RSSI        int     `json:"rssi"`
	Metres      float64 `json:"metres"`
	Basis       string  `json:"basis"`
	Age         int     `json:"age"`
	HeardRatio  float64 `json:"heard_ratio"`
}

type Device struct {
	ID         string   `json:"id"`
	Label      string   `json:"label"`
	AddrType   string   `json:"at"`
	Vendor     *string  `json:"vendor"`
	Doing      []string `json:"doing"`
	Heard      []Heard  `json:"heard"`
	Known      bool     `json:"known"`
	InEar      *bool    `json:"in_ear"`
	Worn       bool     `json:"worn"`
	Unreliable *string  `json:"unreliable"`
}

type Node struct {
	Name string  `json:"name"`
	X    float64 `json:"x"`
	Y    float64 `json:"y"`
}

type Room struct {
	Width  float64 `json:"width"`
	Height float64 `json:"height"`
}

type Snapshot struct {
	Room      Room     `json:"room"`
	Nodes     []Node   `json:"nodes"`
	Devices   []Device `json:"devices"`
	Now       int64    `json:"now"`
	CanLocate bool     `json:"can_locate"`
}

type client struct {
	base string
	http *http.Client
}

func newClient(base string) *client {
	return &client{base: base, http: &http.Client{Timeout: 4 * time.Second}}
}

func (c *client) state() (*Snapshot, error) {
	resp, err := c.http.Get(c.base + "/api/state")
	if err != nil {
		return nil, err
	}
	defer resp.Body.Close()
	if resp.StatusCode != 200 {
		return nil, fmt.Errorf("collector returned %d", resp.StatusCode)
	}
	var s Snapshot
	if err := json.NewDecoder(resp.Body).Decode(&s); err != nil {
		return nil, err
	}
	return &s, nil
}

type fitResult struct {
	RSSIAtOneMetre float64 `json:"rssi_at_1m"`
	Exponent       float64 `json:"exponent"`
	Samples        int     `json:"samples"`
}

// fit hands the samples to the collector rather than doing the arithmetic here.
//
// Two implementations of a least-squares fit would eventually disagree, and the
// one in the collector is the one with tests against a model it has to recover.
// A 422 is a real answer, not a failure: it means the readings do not describe
// a radio getting weaker with distance.
func (c *client) fit(samples [][2]float64) (*fitResult, error) {
	body, _ := json.Marshal(map[string]any{"samples": samples})
	resp, err := c.http.Post(c.base+"/api/fit", "application/json", bytes.NewReader(body))
	if err != nil {
		return nil, err
	}
	defer resp.Body.Close()
	if resp.StatusCode == 422 {
		return nil, fmt.Errorf("those readings do not fit a path-loss model — " +
			"usually one was taken with the device in a pocket, or two were taken at the same spot")
	}
	if resp.StatusCode != 200 {
		return nil, fmt.Errorf("collector returned %d", resp.StatusCode)
	}
	var f fitResult
	if err := json.NewDecoder(resp.Body).Decode(&f); err != nil {
		return nil, err
	}
	return &f, nil
}

// Calibrations are written by the console and only read by the daemon, so the
// daemon never needs write access to a home directory its unit mounts
// read-only. It is a separate file from the config for the same reason a
// generated file is always separate from one a human maintains.

func calibrationPath() string {
	dir := os.Getenv("XDG_CONFIG_HOME")
	if dir == "" {
		dir = filepath.Join(os.Getenv("HOME"), ".config")
	}
	return filepath.Join(dir, "airspace", "calibration.toml")
}

type calibration struct {
	Node     string
	Device   string
	RSSIAt1m float64
	Exponent float64
	Samples  int
}

// saveCalibration rewrites the file with this entry replacing any previous one
// for the same pair. Recalibrating is therefore just doing it again.
func saveCalibration(c calibration) error {
	path := calibrationPath()
	existing := readCalibrations(path)
	out := []calibration{}
	for _, e := range existing {
		if e.Node == c.Node && e.Device == c.Device {
			continue
		}
		out = append(out, e)
	}
	out = append(out, c)

	var b bytes.Buffer
	b.WriteString("# Written by `airspace-console`. Measured path-loss models,\n")
	b.WriteString("# one per (node, device). Safe to delete: without it the collector\n")
	b.WriteString("# falls back to advertised transmit power, and then to textbook\n")
	b.WriteString("# constants, and says on screen which it used.\n")
	for _, e := range out {
		fmt.Fprintf(&b, "\n[[calibration]]\nnode = %q\ndevice = %q\nrssi_at_1m = %.2f\nexponent = %.3f\nsamples = %d\n",
			e.Node, e.Device, e.RSSIAt1m, e.Exponent, e.Samples)
	}
	if err := os.MkdirAll(filepath.Dir(path), 0o700); err != nil {
		return err
	}
	return os.WriteFile(path, b.Bytes(), 0o600)
}

// readCalibrations parses only what this file writes. A full TOML parser would
// be a dependency for four keys in a file with one shape.
func readCalibrations(path string) []calibration {
	data, err := os.ReadFile(path)
	if err != nil {
		return nil
	}
	var out []calibration
	var cur *calibration
	for _, line := range splitLines(string(data)) {
		line = trimSpace(line)
		switch {
		case line == "[[calibration]]":
			if cur != nil {
				out = append(out, *cur)
			}
			cur = &calibration{}
		case cur == nil || line == "" || line[0] == '#':
			continue
		default:
			k, v, ok := cutKV(line)
			if !ok {
				continue
			}
			switch k {
			case "node":
				cur.Node = unquote(v)
			case "device":
				cur.Device = unquote(v)
			case "rssi_at_1m":
				fmt.Sscanf(v, "%f", &cur.RSSIAt1m)
			case "exponent":
				fmt.Sscanf(v, "%f", &cur.Exponent)
			case "samples":
				fmt.Sscanf(v, "%d", &cur.Samples)
			}
		}
	}
	if cur != nil {
		out = append(out, *cur)
	}
	return out
}
