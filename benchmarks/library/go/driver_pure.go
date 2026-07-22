//go:build !cgo_sqlite

package main

import _ "modernc.org/sqlite"

const sqliteDriver = "sqlite"
const driverLabel = "go-modernc"
