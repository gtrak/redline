// Package lib is the local fork of github.com/old/lib, wired in via
// go.mod's `replace github.com/old/lib => ./forks/lib`.
package lib

// MintToken derives a deterministic 64-bit session token from a gear id
// (FNV-1a 64-bit over the byte representation of the id).
func MintToken(id uint64) uint64 {
	h := uint64(1469598103934665603)
	for i := 0; i < 8; i++ {
		h ^= uint64((id>>(8*uint(i))) & 0xff)
		h *= 1099511628211
	}
	return h
}
