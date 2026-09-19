// Package gearserv wires the gear-inventory service together.
package gearserv

import (
	"encoding/json"
	"fmt"

	"github.com/pkg/errors"
)

// Gear is one row of the manifest.
type Gear struct {
	ID    uint64 `json:"id"`
	Name  string `json:"name"`
	Teeth int    `json:"teeth"`
}

// App holds the loaded gear inventory.
type App struct {
	Gears []Gear
}

// NewApp loads and validates a gear manifest from JSON.
func NewApp(raw []byte) (*App, error) {
	var gears []Gear
	if err := json.Unmarshal(raw, &gears); err != nil {
		return nil, errors.Wrap(err, "unmarshaling gear manifest")
	}
	if len(gears) == 0 {
		return nil, errors.New("gear manifest is empty")
	}
	return &App{Gears: gears}, nil
}

// Run prints a summary line per gear.
func (a *App) Run() {
	for _, g := range a.Gears {
		fmt.Printf("gear %d: %s (%d teeth)\n", g.ID, g.Name, g.Teeth)
	}
}
