# Ministore for Go, without CGO

This module is an idiomatic Go wrapper over the Rust Ministore library. It uses
`github.com/ebitengine/purego` to load a stable C ABI dynamically, so the Go
toolchain does not compile or link any C code.

## Build

From the repository root:

```bash
cargo build --release -p ministore-ffi
cd go
CGO_ENABLED=0 go test ./...
```

The native artifact is:

- Linux, FreeBSD, NetBSD: `target/release/libministore_ffi.so`
- macOS: `target/release/libministore_ffi.dylib`

Pass its deployed path to `ministorerust.Load`. The library validates its ABI
version before returning.

## API

`Library` owns the dynamically loaded Rust library. `Index` owns an opaque Rust
index handle. Close every index before closing its library:

```go
library, err := ministorerust.Load(nativeLibraryPath)
if err != nil {
    return err
}
defer library.Close()

index, err := library.Open("docs.db")
if err != nil {
    return err
}
defer index.Close()
```

The wrapper provides:

- `Library.Create` and `Library.Open`
- `Index.PutJSON` and transactional `Index.BatchPutJSON`
- `Index.Get`, `Index.Delete`, and `Index.Search`
- `Index.Optimize`

Use `BatchPutJSON` for imports. It crosses the Go/Rust boundary once and runs
the complete batch in one Rust/SQLite transaction:

```go
count, err := index.BatchPutJSON([][]byte{
    []byte(`{"path":"/one","title":"First"}`),
    []byte(`{"path":"/two","title":"Second"}`),
})
```

Search results can be rendered with the same text conventions as the Rust CLI:

```go
result, err := index.Search("memory", ministorerust.SearchOptions{
    Limit: 10,
    Show: "fields",
    Fields: []string{"title", "category"},
})
if err != nil {
    return err
}
output, err := ministorerust.FormatSearchResults(result, ministorerust.SearchOutputOptions{
    Format: ministorerust.SearchOutputPretty,
})
if err != nil {
    return err
}
fmt.Print(output)
```

`SearchOutputPretty` produces compact human-readable records,
`SearchOutputPaths` emits one path per line, and `SearchOutputJSON` emits the
complete structured page.

All returned strings are copied into Go-owned memory and immediately released
through the Rust allocator. Rust panics are caught at the ABI boundary and
returned as Go errors. Calls on one `Index` are thread-safe, and `Close` waits
for active calls through the Go handle lock. The Rust index retains one SQLite
connection and serializes operations on that connection; use separate indexes
when application-level concurrency requires independent database connections.

## Deployment

The Go binary and Rust shared library are separate artifacts. Package them
together and resolve the library path from application configuration or the
executable directory. This is not a static pure-Go binary, but it is fully
CGO-free from the Go build system's perspective:

```bash
CGO_ENABLED=0 go build ./cmd/myapp
```

The wrapper supports Linux, macOS, FreeBSD, and NetBSD. It does not support
Windows because its dynamic-library loader uses the Unix `dlopen` interface
exposed by purego.
