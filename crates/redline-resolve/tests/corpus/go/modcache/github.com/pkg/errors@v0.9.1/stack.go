package errors

// A source represents a source-code location.
type source interface {
	error
	Source() string
}

type sourceImpl struct{ msg string }

func (s sourceImpl) Error() string  { return s.msg }
func (s sourceImpl) Source() string { return s.msg }

// withStack supports retrieving the stack that caused the error to be
// raised.
type withStack struct {
	cause error
	stack
}

type stack []uintptr

func (w *withStack) Error() string { return w.cause.Error() }
func (w *withStack) Cause() error  { return w.cause }
