#!/usr/bin/env bash
set -euo pipefail

PROJECT_DIR="$(cd "$(dirname "$0")/.." && pwd)"

(cd "$PROJECT_DIR" && cargo build -p ministore-ffi)
(cd "$PROJECT_DIR/go" && CGO_ENABLED=0 go test ./...)
