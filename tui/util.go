package main

import (
	"strings"

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

func unquote(s string) string { return strings.Trim(s, `"`) }

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
