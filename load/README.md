# Ministore Load Tests

Needle-in-a-haystack benchmarks for measuring search performance at scale.

## Usage

```bash
# Build release binary first
cargo build --release

# Run 100k document test
./load_test.sh 100k

# Run 1M document test
./load_test.sh 1m

# Run both
./load_test.sh both

# Clean up generated data
./load_test.sh clean
```

## What It Tests

1. **FTS needle search** - Find a unique term in FTS index
2. **Keyword exact match** - Find document by unique category
3. **Number range search** - Find document by priority > 900
4. **Complex query** - FTS term + keyword filter
5. **Broad search** - Return 100 results from ~10% of corpus

## Generated Data

- `data/100k.jsonl` - 100,000 document corpus (~15MB)
- `data/1m.jsonl` - 1,000,000 document corpus (~150MB)
- `data/*.db` - SQLite indexes

The "needle" document has:
- `title: "NEEDLE_UNIQUE_XYZ_12345"`
- `body: "...MAGIC_HAYSTACK_FINDER..."`
- `category: "needle"`
- `priority: 999`
