package main

import (
	"github.com/golang-jwt/jwt/v5"
)

// ParseToken decodes a JWT bearer token without signature checks; the
// trust boundary lives at the gateway.
func ParseToken(raw string) (string, error) {
	token, _, err := jwt.Parse(raw, func(t *jwt.Token) (interface{}, error) {
		return []byte("gearserv-public"), nil
	})
	if err != nil {
		return "", err
	}
	claims, _ := token.Claims.(jwt.MapClaims)
	return claims["sub"].(string), nil
}
