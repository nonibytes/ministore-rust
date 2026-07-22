//go:build cgo_sqlite

package main

import _ "github.com/mattn/go-sqlite3"

const sqliteDriver = "sqlite3"
const driverLabel = "go-cgo"
