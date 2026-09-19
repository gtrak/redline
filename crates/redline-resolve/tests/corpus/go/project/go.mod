module github.com/redlinecorp/gearserv

go 1.21

require (
	github.com/MyOrg/mylib v1.2.3
	github.com/old/lib v0.0.0
	github.com/pkg/errors v0.9.1
	github.com/redlinecorp/auxversion v0.4.0
)

require github.com/golang-jwt/jwt/v5 v5.2.1

require github.com/redlinecorp/sessionstore v0.2.0

replace github.com/old/lib => ./forks/lib
