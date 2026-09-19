// Package auxversion stamps gearserv builds with an auxiliary version
// string.
package auxversion

// Version is this module's own semver.
const Version = "0.4.0"

// BuildVersion renders the full build-stamp string.
func BuildVersion() string {
	return "gearserv-aux " + Version
}
