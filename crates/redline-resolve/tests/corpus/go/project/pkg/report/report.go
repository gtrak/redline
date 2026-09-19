// Package report renders the gear-inventory report header.
package report

import (
	. "github.com/redlinecorp/auxversion"
)

// Stamp builds the report header. BuildVersion is the dot-imported
// auxversion helper, used bare. Version has two candidates at file
// scope — the dot-imported auxversion.Version and the package-level
// report.Version declared in stamp.go — so the import-context handoff
// treats it as an ambiguous dot-import and emits no hint for it (probe
// 14 pins that degradation; probe 05 pins the disambiguated-hint twin).
func Stamp() (header, version string) {
	header = "gearserv report"
	version = BuildVersion() + " [" + Version + "]"
	return header, version
}
