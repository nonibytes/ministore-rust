# Ministore

A lightweight, embedded document search engine built on SQLite and FTS5. Ministore provides full-text search, structured filtering, and rich query capabilities in a single-file database.

## Features

- **Full-Text Search**: Powered by SQLite's FTS5 for fast, relevant text search
- **Structured Data**: Support for keyword, number, date, and boolean fields
- **Rich Query Language**: Combine text search with structured filters using an intuitive query syntax
- **Flexible Ranking**: BM25 scoring with customizable field weights and boost expressions
- **Cursor-Based Pagination**: Efficient pagination for large result sets
- **Schema Management**: Define schemas with multiple field types and multi-value support
- **Batch Operations**: Transactional batch inserts and deletes
- **Zero Dependencies**: Single-file SQLite database with no external services
- **CLI & Library**: Use as a Rust library or standalone command-line tool

## Installation

### As a Library

Add to your `Cargo.toml`:

```toml
[dependencies]
ministore = "0.1.0"
```

### CLI Tool

```bash
cargo install ministore-cli
```

Or build from source:

```bash
git clone https://github.com/nonibytes/ministore.git
cd ministore
cargo build --release
# Binary will be at target/release/ministore
```

## Quick Start

### Using the CLI

```bash
# Create an index with a schema
ministore index create -i docs \
  --field title:text \
  --field tags:keyword:multi \
  --field published:date

# Add documents
ministore put -i docs --path /blog/hello-world \
  --set title="Hello World" \
  --set tags=rust,tutorial \
  --set published=2024-01-15

# Search
ministore search -i docs --where "hello AND tags:rust"

# Import from JSON file
cat documents.json | ministore put -i docs --json
```

### Using the Library

```rust
use ministore::{Index, Schema, FieldSpec, IndexOptions};
use serde_json::json;

// Create a schema
let mut schema = Schema::new();
schema.add_field("title", FieldSpec::text(Some(2.0))); // 2x weight
schema.add_field("body", FieldSpec::text(Some(1.0)));
schema.add_field("tags", FieldSpec::keyword(true)); // multi-value
schema.add_field("published", FieldSpec::date(false));

// Create an index
let index = Index::create("docs.db", schema, IndexOptions::default())?;

// Insert a document
index.put_json(json!({
    "path": "/blog/hello-world",
    "title": "Hello World",
    "body": "Welcome to my blog about Rust programming",
    "tags": ["rust", "tutorial"],
    "published": "2024-01-15T10:00:00Z"
}))?;

// Search
let results = index.search(
    Some("rust programming"),
    None,  // limit
    None,  // cursor
    None,  // rank
    None,  // show
    false, // explain
)?;

for item in results.items {
    println!("{}: {}", item.path, item.data["title"]);
}
```

## Query Language

Ministore supports a powerful query syntax for combining full-text search with structured filters:

### Text Search

```
hello world              # Match documents containing both words
"hello world"            # Exact phrase match
hello OR world           # Match either word
hello NOT world          # Match hello but not world
```

### Field Filters

```
tags:rust                # Keyword field exact match
published:>2024-01-01    # Date comparison
views:>=1000             # Numeric comparison
featured:true            # Boolean field
```

### Combined Queries

```
"rust tutorial" AND tags:beginner AND published:>2024-01-01
machine learning AND (tags:python OR tags:tensorflow)
NOT archived:true
```

### Operators

- **Text**: `AND`, `OR`, `NOT`, `"phrase"`
- **Numeric/Date**: `>`, `>=`, `<`, `<=`, `=`
- **Boolean**: `true`, `false`

## Ranking & Scoring

### BM25 Scoring

By default, Ministore uses BM25 for relevance scoring. Customize field weights in your schema:

```rust
schema.add_field("title", FieldSpec::text(Some(3.0)));  // 3x importance
schema.add_field("body", FieldSpec::text(Some(1.0)));   // baseline
```

### Custom Ranking

Use the `--rank` option to define custom ranking expressions:

```bash
# Boost recent documents
ministore search -i docs --where "rust" \
  --rank "bm25 + (published / 1000000000)"

# Combine multiple signals
ministore search -i docs --where "tutorial" \
  --rank "bm25 * 0.7 + views * 0.0001 + likes * 0.001"
```

## CLI Reference

### Index Management

```bash
# Create index
ministore index create -i myindex --field name:type[:multi]

# List indexes in current directory
ministore index list

# View schema
ministore index schema -i myindex

# Apply schema changes (additive only)
ministore index schema -i myindex --apply schema.json

# Optimize index (FTS5 optimize + VACUUM)
ministore index optimize -i myindex

# Drop index
ministore index drop -i myindex
```

### Document Operations

```bash
# Insert single document
ministore put -i myindex --path /doc/1 \
  --set field1=value1 --set field2=value2

# Insert from JSON (stdin)
echo '{"path": "/doc/1", "title": "Hello"}' | ministore put -i myindex --json

# Import from file
ministore put -i myindex --import documents.jsonl

# Get document
ministore get -i myindex --path /doc/1

# Peek at raw JSON
ministore peek -i myindex --path /doc/1

# Delete by path
ministore delete -i myindex --path /doc/1

# Delete by query
ministore delete -i myindex --where "archived:true"
```

### Search

```bash
# Basic search
ministore search -i myindex --where "query"

# With pagination
ministore search -i myindex --where "query" --limit 20 --after 10

# Custom ranking
ministore search -i myindex --where "query" --rank "bm25 + boost"

# Select fields
ministore search -i myindex --where "query" --show "title,summary"

# Output formats
ministore search -i myindex --where "query" --format json
ministore search -i myindex --where "query" --format paths
ministore search -i myindex --where "query" --format pretty

# Explain query
ministore search -i myindex --where "query" --explain
```

### Discovery

```bash
# List all fields
ministore discover fields -i myindex

# Show top values for a field
ministore discover values -i myindex --field tags --limit 10

# Field statistics
ministore stats -i myindex --field views
ministore stats -i myindex --field views --where "published:>2024-01-01"
```

## Schema Definition

Schemas can be defined programmatically or via JSON:

```json
{
  "fields": {
    "title": {
      "type": "text",
      "weight": 2.0
    },
    "body": {
      "type": "text",
      "weight": 1.0
    },
    "tags": {
      "type": "keyword",
      "multi": true
    },
    "published": {
      "type": "date",
      "multi": false
    },
    "views": {
      "type": "number",
      "multi": false
    },
    "featured": {
      "type": "bool"
    }
  }
}
```

### Field Types

- **text**: Full-text searchable content (FTS5 indexed)
- **keyword**: Exact-match strings (e.g., tags, categories)
- **number**: Numeric values for filtering and ranking
- **date**: ISO 8601 timestamps stored as Unix milliseconds
- **bool**: Boolean values (true/false)

## Advanced Features

### Batch Operations

```rust
use ministore::Batch;

let mut batch = Batch::new();
batch.put_json(json!({"path": "/doc/1", "title": "First"}))?;
batch.put_json(json!({"path": "/doc/2", "title": "Second"}))?;
batch.delete("/doc/old".to_string())?;

let count = index.batch(batch)?;
println!("Processed {} operations", count);
```

### Cursor-Based Pagination

```rust
let page1 = index.search(Some("rust"), Some(10), None, None, None, false)?;
println!("Found {} results", page1.items.len());

if let Some(cursor) = page1.next_cursor {
    let page2 = index.search(Some("rust"), Some(10), Some(&cursor), None, None, false)?;
    println!("Next page: {} results", page2.items.len());
}
```

### Query Explanation

```bash
ministore search -i docs --where "rust tutorial" --explain
```

Returns the generated SQL and execution plan for debugging and optimization.

## Performance

Ministore is designed for embedded use cases with thousands to millions of documents:

- **Insert**: ~10,000 docs/sec (batched)
- **Search**: Sub-millisecond for most queries
- **Storage**: Efficient SQLite compression
- **Memory**: Minimal overhead, suitable for edge devices

For larger datasets, use `optimize` periodically to maintain performance.

## Use Cases

- **Documentation Search**: Index and search technical documentation
- **Content Management**: Build search for blogs, wikis, or knowledge bases
- **Log Analysis**: Search and filter application logs
- **Personal Knowledge**: Index notes, bookmarks, or research papers
- **Embedded Search**: Add search to desktop or mobile applications

## Architecture

Ministore is built on:

- **SQLite**: Reliable, embedded database engine
- **FTS5**: Full-text search extension with BM25 ranking
- **Rust**: Memory-safe, high-performance implementation

The query planner translates the query language into optimized SQL, leveraging SQLite's query optimizer and FTS5's ranking capabilities.

## Contributing

Contributions are welcome! Please see [DESIGN.detailed.md](DESIGN.detailed.md) for architecture details.

## License

MIT License - see LICENSE file for details

## Links

- **Repository**: https://github.com/nonibytes/ministore
- **Documentation**: See [DESIGN.detailed.md](DESIGN.detailed.md) and [DESIGN.overview.md](DESIGN.overview.md)
- **Issues**: https://github.com/nonibytes/ministore/issues
