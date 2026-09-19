// Package note pins the bare-symbol no-hint degradation for the go
// golden corpus.
package note

import (
	. "github.com/pkg/errors"
)

// ErrLostHint is built from the dot-imported errors.New, used bare. A
// dot-imported use site only carries an import hint when the handoff
// can pin the source package; here the hint is empty, so probe 13 pins
// the no-hint bail for this symbol.
var ErrLostHint = New("the import hint was lost in handoff")
