package main

import "strings"

// Tiny helpers for the calibration-file parser. It reads only what this program
// writes, so a full TOML library would be a dependency for four keys in a file
// with one shape.

func splitLines(s string) []string { return strings.Split(s, "\n") }
func trimSpace(s string) string    { return strings.TrimSpace(s) }

func cutKV(line string) (key, val string, ok bool) {
	k, v, found := strings.Cut(line, "=")
	if !found {
		return "", "", false
	}
	return strings.TrimSpace(k), strings.TrimSpace(v), true
}

func unquote(s string) string { return strings.Trim(s, `"`) }
