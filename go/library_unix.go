//go:build darwin || freebsd || linux || netbsd

package ministorerust

import (
	"encoding/json"
	"errors"
	"fmt"
	"sync"
	"unsafe"

	"github.com/ebitengine/purego"
)

const abiVersion = 1

type nativeFunctions struct {
	abiVersion   func() uint32
	create       func(string, string) *byte
	open         func(string) *byte
	close        func(uint64) *byte
	putJSON      func(uint64, string) *byte
	batchPutJSON func(uint64, string) *byte
	get          func(uint64, string) *byte
	delete       func(uint64, string) *byte
	search       func(uint64, string, string) *byte
	optimize     func(uint64) *byte
	free         func(*byte)
}

type Library struct {
	mu     sync.Mutex
	handle uintptr
	active int
	closed bool
	funcs  nativeFunctions
}

func Load(libraryPath string) (*Library, error) {
	handle, err := purego.Dlopen(libraryPath, purego.RTLD_NOW|purego.RTLD_LOCAL)
	if err != nil {
		return nil, fmt.Errorf("load ministore Rust library: %w", err)
	}
	library := &Library{handle: handle}
	register := func(target any, name string) (err error) {
		defer func() {
			if recovered := recover(); recovered != nil {
				err = fmt.Errorf("resolve %s: %v", name, recovered)
			}
		}()
		purego.RegisterLibFunc(target, handle, name)
		return nil
	}
	bindings := []struct {
		target any
		name   string
	}{
		{&library.funcs.abiVersion, "ministore_abi_version"},
		{&library.funcs.create, "ministore_create"},
		{&library.funcs.open, "ministore_open"},
		{&library.funcs.close, "ministore_close"},
		{&library.funcs.putJSON, "ministore_put_json"},
		{&library.funcs.batchPutJSON, "ministore_batch_put_json"},
		{&library.funcs.get, "ministore_get"},
		{&library.funcs.delete, "ministore_delete"},
		{&library.funcs.search, "ministore_search"},
		{&library.funcs.optimize, "ministore_optimize"},
		{&library.funcs.free, "ministore_string_free"},
	}
	for _, binding := range bindings {
		if err := register(binding.target, binding.name); err != nil {
			_ = purego.Dlclose(handle)
			return nil, err
		}
	}
	if version := library.funcs.abiVersion(); version != abiVersion {
		_ = purego.Dlclose(handle)
		return nil, fmt.Errorf("unsupported ministore ABI version %d, want %d", version, abiVersion)
	}
	return library, nil
}

func (l *Library) Create(path string, schema Schema) (*Index, error) {
	if err := l.reserveIndex(); err != nil {
		return nil, err
	}
	schemaJSON, err := json.Marshal(schema)
	if err != nil {
		l.releaseIndex()
		return nil, fmt.Errorf("marshal schema: %w", err)
	}
	var handle uint64
	if err := l.consume("create", l.funcs.create(path, string(schemaJSON)), &handle); err != nil {
		l.releaseIndex()
		return nil, err
	}
	return &Index{library: l, handle: handle}, nil
}

func (l *Library) Open(path string) (*Index, error) {
	if err := l.reserveIndex(); err != nil {
		return nil, err
	}
	var handle uint64
	if err := l.consume("open", l.funcs.open(path), &handle); err != nil {
		l.releaseIndex()
		return nil, err
	}
	return &Index{library: l, handle: handle}, nil
}

func (l *Library) Close() error {
	l.mu.Lock()
	defer l.mu.Unlock()
	if l.closed {
		return nil
	}
	if l.active != 0 {
		return fmt.Errorf("close ministore Rust library: %d indexes still open", l.active)
	}
	if err := purego.Dlclose(l.handle); err != nil {
		return fmt.Errorf("close ministore Rust library: %w", err)
	}
	l.closed = true
	return nil
}

func (l *Library) reserveIndex() error {
	l.mu.Lock()
	defer l.mu.Unlock()
	if l.closed {
		return errors.New("ministore Rust library is closed")
	}
	l.active++
	return nil
}

func (l *Library) releaseIndex() {
	l.mu.Lock()
	l.active--
	l.mu.Unlock()
}

func (l *Library) consume(operation string, pointer *byte, destination any) error {
	if pointer == nil {
		return &Error{Operation: operation, Message: "native library returned a null response"}
	}
	defer l.funcs.free(pointer)
	responseJSON, err := copyCString(pointer)
	if err != nil {
		return &Error{Operation: operation, Message: err.Error()}
	}
	var response struct {
		OK    json.RawMessage `json:"ok"`
		Error string          `json:"error"`
	}
	if err := json.Unmarshal(responseJSON, &response); err != nil {
		return &Error{Operation: operation, Message: "invalid native response: " + err.Error()}
	}
	if response.Error != "" {
		return &Error{Operation: operation, Message: response.Error}
	}
	if len(response.OK) == 0 {
		return &Error{Operation: operation, Message: "native response has neither ok nor error"}
	}
	if destination == nil {
		return nil
	}
	if err := json.Unmarshal(response.OK, destination); err != nil {
		return &Error{Operation: operation, Message: "decode native result: " + err.Error()}
	}
	return nil
}

func copyCString(pointer *byte) ([]byte, error) {
	const maximumResponseSize = 256 << 20
	for length := 0; length < maximumResponseSize; length++ {
		if *(*byte)(unsafe.Add(unsafe.Pointer(pointer), length)) == 0 {
			return []byte(unsafe.String(pointer, length)), nil
		}
	}
	return nil, errors.New("native response exceeds 256 MiB or is not NUL-terminated")
}
