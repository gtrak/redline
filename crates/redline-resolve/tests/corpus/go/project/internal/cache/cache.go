// Package cache keeps the small session-cache client.
package cache

import (
	"context"

	"github.com/redlinecorp/sessionstore"
)

// Client wraps the session-cache connection.
type Client struct {
	*sessionstore.Client
}

// New opens the session cache.
func New(ctx context.Context, addr string) (*Client, error) {
	c := sessionstore.Dial(addr)
	if err := c.Ping(ctx); err != nil {
		return nil, err
	}
	return &Client{Client: c}, nil
}
