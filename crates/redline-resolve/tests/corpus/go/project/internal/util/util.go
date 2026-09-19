// Package util provides manifest I/O and validation helpers.
package util

import (
	"fmt"
	"os"

	"github.com/redlinecorp/gearserv"
)

// ReadManifest reads the manifest file's bytes.
func ReadManifest(path string) ([]byte, error) {
	raw, err := os.ReadFile(path)
	if err != nil {
		return nil, fmt.Errorf("util: reading %s: %w", path, err)
	}
	return raw, nil
}

// Validate checks that every gear id is unique.
func Validate(gears []gearserv.Gear) bool {
	seen := make(map[uint64]bool, len(gears))
	for _, g := range gears {
		if seen[g.ID] {
			return false
		}
		seen[g.ID] = true
	}
	return true
}
