// Package mylib is the MyOrg gear-encoding library.
package mylib

import (
	"encoding/binary"
	"fmt"
)

// Codec round-trips gear ids to a stable on-disk encoding.
type Codec struct {
	// BigEndian selects the endianness of Encode/Decode.
	BigEndian bool
}

// NewCodec returns a little-endian Codec.
func NewCodec() *Codec {
	return &Codec{}
}

// Encode marshals a gear id to its canonical byte form.
func (c *Codec) Encode(id uint64) []byte {
	b := make([]byte, 8)
	if c.BigEndian {
		binary.BigEndian.PutUint64(b, id)
	} else {
		binary.LittleEndian.PutUint64(b, id)
	}
	return b
}

// Decode is the inverse of Encode.
func (c *Codec) Decode(b []byte) (uint64, error) {
	if len(b) != 8 {
		return 0, fmt.Errorf("mylib: expected 8 bytes, got %d", len(b))
	}
	if c.BigEndian {
		return binary.BigEndian.Uint64(b), nil
	}
	return binary.LittleEndian.Uint64(b), nil
}
