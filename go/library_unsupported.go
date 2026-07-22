//go:build !darwin && !freebsd && !linux && !netbsd

package ministorerust

import "errors"

type Library struct{}

func Load(string) (*Library, error) {
	return nil, errors.New("ministore purego bindings are supported on Linux, macOS, FreeBSD, and NetBSD")
}
