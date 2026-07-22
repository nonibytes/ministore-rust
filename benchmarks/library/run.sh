#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_DIR="$(cd "$SCRIPT_DIR/../.." && pwd)"
CORPUS="${1:-$PROJECT_DIR/load/data/100k.jsonl}"
SCHEMA="${2:-$PROJECT_DIR/load/data/schema.json}"
ITERATIONS="${ITERATIONS:-100}"

if [[ ! -f "$CORPUS" || ! -f "$SCHEMA" ]]; then
    echo "Corpus or schema missing. Generate them with ./load/load_test.sh 100k." >&2
    exit 1
fi

RESULT_DIR="$(mktemp -d /tmp/ministore-library-bench.XXXXXX)"
trap 'rm -rf "$RESULT_DIR"' EXIT

(cd "$PROJECT_DIR" && cargo build --release -p ministore-ffi -p ministore-library-bench)
(cd "$SCRIPT_DIR/go" && go mod tidy)
(cd "$SCRIPT_DIR/go" && CGO_ENABLED=0 go build -o "$RESULT_DIR/go-pure" .)
(cd "$SCRIPT_DIR/go" && CGO_ENABLED=1 go build -tags "cgo_sqlite fts5" -o "$RESULT_DIR/go-cgo" .)

LIBRARY="$PROJECT_DIR/target/release/libministore_ffi.so"
if [[ "$(uname -s)" == "Darwin" ]]; then
    LIBRARY="$PROJECT_DIR/target/release/libministore_ffi.dylib"
fi

"$PROJECT_DIR/target/release/ministore-library-bench" "$SCHEMA" "$CORPUS" "$RESULT_DIR/rust-native.db" "$ITERATIONS" | tee "$RESULT_DIR/rust-native.json"
"$RESULT_DIR/go-pure" -implementation go -schema "$SCHEMA" -corpus "$CORPUS" -database "$RESULT_DIR/go-pure.db" -iterations "$ITERATIONS" | tee "$RESULT_DIR/go-pure.json"
"$RESULT_DIR/go-cgo" -implementation go -schema "$SCHEMA" -corpus "$CORPUS" -database "$RESULT_DIR/go-cgo.db" -iterations "$ITERATIONS" | tee "$RESULT_DIR/go-cgo.json"
"$RESULT_DIR/go-pure" -implementation rust-purego -library "$LIBRARY" -schema "$SCHEMA" -corpus "$CORPUS" -database "$RESULT_DIR/rust-purego.db" -iterations "$ITERATIONS" | tee "$RESULT_DIR/rust-purego.json"
