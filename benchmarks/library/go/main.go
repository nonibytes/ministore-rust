package main

import (
	"bufio"
	"context"
	"encoding/json"
	"flag"
	"fmt"
	"os"
	"time"

	gostore "github.com/ministore/ministore/ministore"
	"github.com/ministore/ministore/ministore/storage/sqlite"
	ruststore "github.com/nonibytes/ministore-rust/go"
)

type results struct {
	Implementation string             `json:"implementation"`
	Documents      int                `json:"documents"`
	ImportMS       float64            `json:"import_ms"`
	SearchesMS     map[string]float64 `json:"searches_ms"`
}

func main() {
	implementation := flag.String("implementation", "go", "go or rust-purego")
	libraryPath := flag.String("library", "", "Rust shared library path")
	schemaPath := flag.String("schema", "", "schema JSON path")
	corpusPath := flag.String("corpus", "", "JSONL corpus path")
	databasePath := flag.String("database", "", "output database path")
	iterations := flag.Int("iterations", 100, "search iterations")
	flag.Parse()
	if *schemaPath == "" || *corpusPath == "" || *databasePath == "" || *iterations < 1 {
		flag.Usage()
		os.Exit(2)
	}

	schemaJSON := must(os.ReadFile(*schemaPath))
	documents := readDocuments(*corpusPath)
	_ = os.Remove(*databasePath)
	var measured results
	switch *implementation {
	case "go":
		measured = benchmarkGo(schemaJSON, documents, *databasePath, *iterations)
		measured.Implementation = driverLabel
	case "rust-purego":
		if *libraryPath == "" {
			fatal("-library is required for rust-purego")
		}
		measured = benchmarkRustPureGo(*libraryPath, schemaJSON, documents, *databasePath, *iterations)
		measured.Implementation = "rust-purego"
	default:
		fatal("unknown implementation " + *implementation)
	}
	output, err := json.MarshalIndent(measured, "", "  ")
	if err != nil {
		panic(err)
	}
	fmt.Println(string(output))
}

func benchmarkGo(schemaJSON []byte, documents [][]byte, database string, iterations int) results {
	ctx := context.Background()
	var schema gostore.Schema
	if err := json.Unmarshal(schemaJSON, &schema); err != nil {
		panic(err)
	}
	index := must(gostore.Create(ctx, sqlite.NewWithDriver(database, sqliteDriver), schema, gostore.DefaultIndexOptions()))
	defer index.Close()
	started := time.Now()
	batch := gostore.NewBatch()
	for _, document := range documents {
		if err := batch.PutJSON(document); err != nil {
			panic(err)
		}
	}
	count := must(batch.Execute(ctx, index))
	importMS := float64(time.Since(started)) / float64(time.Millisecond)
	if err := index.Optimize(ctx); err != nil {
		panic(err)
	}
	return results{Documents: count, ImportMS: importMS, SearchesMS: benchmarkSearches(iterations, func(query string, limit int) int {
		page := must(index.Search(ctx, query, gostore.SearchOptions{Limit: limit, CursorMode: gostore.CursorFull}))
		return len(page.Items)
	})}
}

func benchmarkRustPureGo(libraryPath string, schemaJSON []byte, documents [][]byte, database string, iterations int) results {
	library := must(ruststore.Load(libraryPath))
	defer library.Close()
	var schema ruststore.Schema
	if err := json.Unmarshal(schemaJSON, &schema); err != nil {
		panic(err)
	}
	index := must(library.Create(database, schema))
	defer index.Close()
	started := time.Now()
	count := must(index.BatchPutJSON(documents))
	importMS := float64(time.Since(started)) / float64(time.Millisecond)
	if err := index.Optimize(); err != nil {
		panic(err)
	}
	return results{Documents: count, ImportMS: importMS, SearchesMS: benchmarkSearches(iterations, func(query string, limit int) int {
		page := must(index.Search(query, ruststore.SearchOptions{Limit: limit, CursorMode: "full"}))
		return len(page.Items)
	})}
}

func benchmarkSearches(iterations int, search func(string, int) int) map[string]float64 {
	queries := []struct {
		name, query string
		limit       int
	}{
		{"fts_needle", "NEEDLE_UNIQUE_XYZ_12345", 5},
		{"keyword", "category:needle", 5},
		{"number", "priority>900", 5},
		{"complex", "MAGIC_HAYSTACK_FINDER category:needle", 5},
		{"broad", "category:cat5", 100},
	}
	results := make(map[string]float64, len(queries))
	for _, query := range queries {
		for range 5 {
			_ = search(query.query, query.limit)
		}
		started := time.Now()
		count := 0
		for range iterations {
			count += search(query.query, query.limit)
		}
		if count == 0 {
			panic("search unexpectedly returned no results")
		}
		results[query.name] = float64(time.Since(started)) / float64(time.Millisecond) / float64(iterations)
	}
	return results
}

func readDocuments(path string) [][]byte {
	file := must(os.Open(path))
	defer file.Close()
	var documents [][]byte
	scanner := bufio.NewScanner(file)
	for scanner.Scan() {
		if len(scanner.Bytes()) != 0 {
			documents = append(documents, append([]byte(nil), scanner.Bytes()...))
		}
	}
	if err := scanner.Err(); err != nil {
		panic(err)
	}
	return documents
}

func must[T any](value T, err error) T {
	if err != nil {
		panic(err)
	}
	return value
}

func fatal(message string) {
	fmt.Fprintln(os.Stderr, message)
	os.Exit(1)
}
