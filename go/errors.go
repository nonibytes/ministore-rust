package ministorerust

import "fmt"

type Error struct {
	Operation string
	Message   string
}

func (e *Error) Error() string {
	if e.Operation == "" {
		return e.Message
	}
	return fmt.Sprintf("ministore %s: %s", e.Operation, e.Message)
}
