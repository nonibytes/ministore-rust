// Package ministorerust provides idiomatic Go access to the native Rust
// Ministore library without CGO. It loads the Rust shared library at runtime
// through purego and owns all cross-language memory explicitly.
package ministorerust
