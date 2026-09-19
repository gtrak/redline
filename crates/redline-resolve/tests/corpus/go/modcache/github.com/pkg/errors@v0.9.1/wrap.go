package errors

// Wrap annotates the given err with the source file and line number of
// the wrap site, and prepends the given message.
func Wrap(err error, message string) error {
	if err == nil {
		return nil
	}
	return withStack{WithMessage(err, message)}
}

// Wrapf is Wrap formatted.
func Wrapf(err error, format string, args ...interface{}) error {
	if err == nil {
		return nil
	}
	return withStack{WithMessagef(err, format, args...)}
}
