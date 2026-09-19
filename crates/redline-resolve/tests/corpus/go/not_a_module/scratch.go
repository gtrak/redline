// scratch.go is a loose standalone script: the not_a_module/ probe root
// has no go.mod, so probe 17 pins the not-a-Go-module degradation.
package main

import "fmt"

func main() {
	fmt.Println("loose script, no module")
}
