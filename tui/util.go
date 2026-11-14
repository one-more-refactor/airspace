package main

import (
	"fmt"
	"strconv"
	"strings"
	"unicode"

	tea "github.com/charmbracelet/bubbletea"
)

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

// unquote reads back a basic string as tomlString writes it. Anything it
// cannot parse is taken bare, which is what this did before escapes existed.
func unquote(s string) string {
	if u, err := strconv.Unquote(s); err == nil && strings.HasPrefix(s, `"`) {
		return u
	}
	return strings.Trim(s, `"`)
}

// tomlString writes s as a TOML basic string.
//
// Go's %q is not TOML: it writes \x1b and \a, which TOML does not have, so a
// node name with a control character in it made a file the collector refused
// to start with.
func tomlString(s string) string {
	var b strings.Builder
	b.WriteByte('"')
	for _, r := range s {
		switch {
		case r == '"':
			b.WriteString(`\"`)
		case r == '\\':
			b.WriteString(`\\`)
		case r < 0x20 || r == 0x7f:
			fmt.Fprintf(&b, `\u%04X`, r)
		default:
			b.WriteRune(r)
		}
	}
	b.WriteByte('"')
	return b.String()
}

// maxLabel matches the collector's clamp: longer than any advertised name or
// SSID can be.
const maxLabel = 64

// strip drops control characters (ESC, BEL and the rest of C0/C1) and the
// invisible format characters — bidi overrides, zero-width marks — that make
// a label read as something it is not.
func strip(s string) string {
	return strings.Map(func(r rune) rune {
		if unicode.IsControl(r) || unicode.Is(unicode.Cf, r) ||
			r == '\u2028' || r == '\u2029' {
			return -1
		}
		return r
	}, s)
}

// printable is strip plus a length clamp, for labels.
func printable(s string) string {
	s = strip(s)
	if r := []rune(s); len(r) > maxLabel {
		s = string(r[:maxLabel])
	}
	return s
}

// keyOf builds a key message for tests, so the update path can be driven
// without a terminal.
func keyOf(s string) tea.KeyMsg {
	switch s {
	case "left":
		return tea.KeyMsg{Type: tea.KeyLeft}
	case "right":
		return tea.KeyMsg{Type: tea.KeyRight}
	case "up":
		return tea.KeyMsg{Type: tea.KeyUp}
	case "down":
		return tea.KeyMsg{Type: tea.KeyDown}
	case "esc":
		return tea.KeyMsg{Type: tea.KeyEsc}
	case "enter":
		return tea.KeyMsg{Type: tea.KeyEnter}
	default:
		return tea.KeyMsg{Type: tea.KeyRunes, Runes: []rune(s)}
	}
}
