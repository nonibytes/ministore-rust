//go:build darwin || freebsd || linux || netbsd

package ministorerust

import (
	"encoding/json"
	"os"
	"path/filepath"
	"runtime"
	"testing"
)

func testLibraryPath(t *testing.T) string {
	t.Helper()
	if path := os.Getenv("MINISTORE_RUST_LIBRARY"); path != "" {
		return path
	}
	name := "libministore_ffi.so"
	if runtime.GOOS == "darwin" {
		name = "libministore_ffi.dylib"
	}
	path := filepath.Join("..", "target", "debug", name)
	if _, err := os.Stat(path); err != nil {
		t.Skipf("Rust FFI library not built: %v", err)
	}
	return path
}

func TestPureGoBindingCRUDAndSearch(t *testing.T) {
	library, err := Load(testLibraryPath(t))
	if err != nil {
		t.Fatal(err)
	}
	defer library.Close()
	weight := 2.0
	index, err := library.Create(filepath.Join(t.TempDir(), "test.db"), Schema{Fields: map[string]FieldSpec{
		"title": {Type: FieldText, Weight: &weight},
		"tags":  {Type: FieldKeyword, Multi: true},
	}})
	if err != nil {
		t.Fatal(err)
	}
	defer index.Close()

	documents := [][]byte{
		[]byte(`{"path":"/one","title":"hello rust","tags":["a","b"]}`),
		[]byte(`{"path":"/two","title":"hello go","tags":["b"]}`),
	}
	if count, err := index.BatchPutJSON(documents); err != nil || count != 2 {
		t.Fatalf("BatchPutJSON() = %d, %v", count, err)
	}
	result, err := index.Search("hello", SearchOptions{Limit: 10, Show: "all", CursorMode: "full"})
	if err != nil {
		t.Fatal(err)
	}
	if len(result.Items) != 2 {
		t.Fatalf("Search returned %d items", len(result.Items))
	}
	item, err := index.Get("/one")
	if err != nil {
		t.Fatal(err)
	}
	var document map[string]any
	if err := json.Unmarshal(item.Document, &document); err != nil {
		t.Fatal(err)
	}
	if document["title"] != "hello rust" {
		t.Fatalf("unexpected document: %v", document)
	}
	if deleted, err := index.Delete("/one"); err != nil || !deleted {
		t.Fatalf("Delete() = %v, %v", deleted, err)
	}
}

func TestPureGoBindingErrorsAfterClose(t *testing.T) {
	library, err := Load(testLibraryPath(t))
	if err != nil {
		t.Fatal(err)
	}
	index, err := library.Create(filepath.Join(t.TempDir(), "test.db"), Schema{Fields: map[string]FieldSpec{"title": {Type: FieldText}}})
	if err != nil {
		t.Fatal(err)
	}
	if err := index.Close(); err != nil {
		t.Fatal(err)
	}
	if err := index.PutJSON([]byte(`{"path":"/closed","title":"no"}`)); err == nil {
		t.Fatal("PutJSON succeeded after Close")
	}
	if err := library.Close(); err != nil {
		t.Fatal(err)
	}
}
