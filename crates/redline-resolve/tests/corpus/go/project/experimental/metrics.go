// Package experimental is deliberately orphaned from the module build:
// it imports github.com/gin-gonic/gin, which is NOT in go.mod's
// require block. Probe 12 pins the module-not-required degradation.
package experimental

import (
	"github.com/gin-gonic/gin"
)

// Metrics wires the (never-built) experimental metrics routes.
func Metrics() *gin.Engine {
	r := gin.New()
	r.GET("/metrics", func(c *gin.Context) { c.String(200, "ok") })
	return r
}
