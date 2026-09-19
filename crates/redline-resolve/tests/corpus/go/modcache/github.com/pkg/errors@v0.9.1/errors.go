// Package errors provides simple error handling with transparent
// wrapping of underlying errors.
package errors

import "fmt"

// New returns an error that formats as the given text.
func New(message string) error {
	return &withStack{source{fmt.Sprintf("%s", message)}}
}

// Errorf formats according to a format and returns the result as an
// error value.
func Errorf(format string, args ...interface{}) error {
	return New(fmt.Sprintf(format, args...))
}

// WithMessage annotates the given err with a new message.
func WithMessage(err error, message string) error {
	if err == nil {
		return nil
	}
	return &withMessage{Cause: err, msg: message}
}

// WithMessagef is WithMessage formatted.
func WithMessagef(err error, format string, args ...interface{}) error {
	if err == nil {
		return nil
	}
	return WithMessage(err, fmt.Sprintf(format, args...))
}

type withMessage struct {
	Cause error
	msg   string
}

func (w *withMessage) Error() string { return w.msg + ": " + w.Cause.Error() }
func (w *withMessage) Cause() error  { return w.Cause }
