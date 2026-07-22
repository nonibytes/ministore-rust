//go:build darwin || freebsd || linux || netbsd

package ministorerust

import (
	"bytes"
	"encoding/json"
	"errors"
	"fmt"
	"sync"
)

type Index struct {
	mu      sync.RWMutex
	library *Library
	handle  uint64
	closed  bool
}

func (i *Index) PutJSON(document []byte) error {
	if !json.Valid(document) {
		return errors.New("ministore put: invalid JSON document")
	}
	return i.call("put", func(handle uint64) *byte {
		return i.library.funcs.putJSON(handle, string(document))
	}, nil)
}

func (i *Index) BatchPutJSON(documents [][]byte) (int, error) {
	var payload bytes.Buffer
	payload.WriteByte('[')
	for index, document := range documents {
		if !json.Valid(document) {
			return 0, fmt.Errorf("ministore batch put: document %d is invalid JSON", index)
		}
		if index != 0 {
			payload.WriteByte(',')
		}
		payload.Write(document)
	}
	payload.WriteByte(']')
	var count int
	err := i.call("batch put", func(handle uint64) *byte {
		return i.library.funcs.batchPutJSON(handle, payload.String())
	}, &count)
	return count, err
}

func (i *Index) Get(path string) (Item, error) {
	var item Item
	err := i.call("get", func(handle uint64) *byte { return i.library.funcs.get(handle, path) }, &item)
	return item, err
}

func (i *Index) Delete(path string) (bool, error) {
	var deleted bool
	err := i.call("delete", func(handle uint64) *byte { return i.library.funcs.delete(handle, path) }, &deleted)
	return deleted, err
}

func (i *Index) Search(query string, options SearchOptions) (SearchResult, error) {
	optionsJSON, err := json.Marshal(options)
	if err != nil {
		return SearchResult{}, fmt.Errorf("marshal search options: %w", err)
	}
	var result SearchResult
	err = i.call("search", func(handle uint64) *byte {
		return i.library.funcs.search(handle, query, string(optionsJSON))
	}, &result)
	return result, err
}

func (i *Index) Optimize() error {
	return i.call("optimize", i.library.funcs.optimize, nil)
}

func (i *Index) Close() error {
	i.mu.Lock()
	defer i.mu.Unlock()
	if i.closed {
		return nil
	}
	if err := i.library.consume("close", i.library.funcs.close(i.handle), nil); err != nil {
		return err
	}
	i.closed = true
	i.library.releaseIndex()
	return nil
}

func (i *Index) call(operation string, invoke func(uint64) *byte, destination any) error {
	i.mu.RLock()
	defer i.mu.RUnlock()
	if i.closed {
		return &Error{Operation: operation, Message: "index is closed"}
	}
	i.library.mu.Lock()
	libraryClosed := i.library.closed
	i.library.mu.Unlock()
	if libraryClosed {
		return &Error{Operation: operation, Message: "native library is closed"}
	}
	return i.library.consume(operation, invoke(i.handle), destination)
}
