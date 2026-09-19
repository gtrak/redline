// Package audit logs validation failures with wrapped context.
package audit

import (
	pe "github.com/pkg/errors"
)

// Log wraps err with audit context. The ALIASED import (`pe`) is the
// corpus use site for probe 08: the app's import-context hint maps the
// alias back to the real package path (errors.New), but the go provider
// takes the alias itself as the package name and bails — probe 08 pins
// that behavior byte-for-byte (the corpus observes, it does not fix).
func Log(err error) error {
	return pe.Wrap(err, "audit: validation failed")
}
