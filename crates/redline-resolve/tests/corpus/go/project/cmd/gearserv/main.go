// Command gearserv loads a gear manifest, validates it, and prints a
// per-gear summary.
package main

import (
	"context"
	"flag"
	"fmt"

	"github.com/myorg/mylib"
	"github.com/old/lib"

	"github.com/redlinecorp/gearserv"
	"github.com/redlinecorp/gearserv/internal/cache"
	"github.com/redlinecorp/gearserv/internal/util"
)

func main() {
	manifest := flag.String("manifest", "gears.json", "path to the gear manifest")
	sessions := flag.Bool("sessions", false, "open the session cache")
	flag.Parse()

	raw, err := util.ReadManifest(*manifest)
	if err != nil {
		fmt.Printf("fatal: reading manifest: %v\n", err)
		return
	}

	app, err := gearserv.NewApp(raw)
	if err != nil {
		fmt.Printf("fatal: %v\n", err)
		return
	}

	if !util.Validate(app.Gears) {
		fmt.Println("manifest failed validation")
		return
	}

	if *sessions {
		if _, err := cache.New(context.Background(), "localhost:6379"); err != nil {
			fmt.Printf("sessions unavailable: %v\n", err)
		}
	}

	// Mint a deterministic session token per gear and encode it through
	// the external codec.
	codec := mylib.NewCodec()
	for _, g := range app.Gears {
		token := lib.MintToken(g.ID)
		_ = codec.Encode(token)
	}

	app.Run()
}
