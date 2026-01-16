# `ministore` — A General-Purpose Search Index

A single-file search index with explicit schemas, boolean queries, and cursor pagination. No server.

---

## Glossary

| Term | Definition |
|------|------------|
| **index** | A named collection of items with a defined schema. One `.db` file per index. |
| **item** | A record in an index, identified by a unique path. |
| **path** | Unique identifier for an item within an index. Hierarchical string (e.g., `/core/how-to-allocate`). |
| **schema** | Definition of field names and types for an index. |
| **field** | A named attribute of an item (e.g., `tags`, `priority`). |
| **query** | Boolean expression to filter items (e.g., `tags:memory & priority:>5`). |
| **cursor** | Opaque token for paginating through results. |

---

## High-level approach

### Indexes

An **index** is a named collection with a defined schema:

```bash
ministore index create -i docs --schema schema.json
```

```json
{
  "fields": {
    "title":    { "type": "text", "weight": 3.0 },
    "tags":     { "type": "keyword", "multi": true },
    "category": { "type": "keyword" },
    "author":   { "type": "keyword" },
    "priority": { "type": "number" },
    "due":      { "type": "date" },
    "content":  { "type": "text", "weight": 1.0 }
  }
}
```

Different indexes can have different schemas. A `tools` index might have `capabilities`, `input_types`, `output_types`. A `memory` index might have `source`, `conversation_id`, `importance`.

### Items

An **item** lives in an index and has:

* **path** — unique identifier within the index (use descriptive paths)
* **fields** — values matching the schema

```json
{
  "path": "/core/how-to-allocate-memory",
  "title": "How to allocate memory",
  "tags": ["memory", "heap", "allocation"],
  "category": "concept",
  "priority": 5,
  "content": "Full body text here..."
}
```

### Field types

| Type | Storage | Query support | Example |
|------|---------|---------------|---------|
| `keyword` | exact values | exact, prefix, wildcard | `tags:memory`, `tags:mem*` |
| `text` | tokenized, FTS | words, phrases, prefix | `content:"error handling"` |
| `number` | numeric | comparisons, ranges | `priority:>5`, `score:0.5..0.9` |
| `date` | epoch | comparisons, ranges, relative | `due:<7d`, `created:>2024-01-01` |
| `bool` | 0/1 | true/false | `active:true`, `!deprecated` |

Add `"multi": true` to any type to allow multiple values (like tags).

### Paths

Paths are unique identifiers within an index. Use descriptive, human-readable paths:

```
/core/how-to-allocate-memory
/stdlib/collections/arraylist
/patterns/memory-cleanup-with-defer
/2024-01-15/standup-notes
```

Paths support **prefix queries**:

```
path:/stdlib/*           # everything under /stdlib/
path:/2024-01/*          # all items from January 2024
```

---

## Why this approach

### Why explicit schemas?

Schemaless sounds nice until you need:

* **Consistent querying** — is it `tag` or `tags`? `priority` as number or string?
* **Optimized storage** — numbers need different indexes than keywords
* **Validation** — catch typos at insert time, not query time
* **Discovery** — "what fields exist?" has a clear answer

Elasticsearch learned this. You *can* auto-detect, but explicit mappings are better for production.

### Why dictionary + postings for keywords?

The naive approach (one row per keyword occurrence) doesn't scale for wildcards.

With 1M items and multi-value tags, you might have 10-50M rows. A contains scan (`LIKE '%alloc%'`) over all of them is slow.

Dictionary + postings means:
* **Contains wildcard** scans unique values (1000s), not occurrences (millions)
* **Prefix wildcard** uses B-tree range scan on dictionary
* **Exact match** is single lookup + postings join

### Why multi-column FTS?

Field-prefixed tokens in a single column (`title:memory title:allocation`) don't work reliably with FTS5's tokenizer and query syntax.

Multi-column FTS (one column per text field) gives you:
* Native `title:memory` queries that work correctly
* Proper phrase matching per field
* `bm25()` ranking with field weights

### Why single-file storage?

* No server to run
* Easy to distribute (ship the `.db` file)
* Works offline
* Trivial backup (copy one file)

SQLite handles concurrency well enough for CLI/embedded use.

---

## Storage: SQLite with per-type tables

Single file per index. Schema stored in metadata. Fields routed to appropriate tables based on declared type.

**Schema:**

```sql
-- Schema metadata
CREATE TABLE meta (
  key TEXT PRIMARY KEY,
  value TEXT
);
-- stores: {"fields": {"title": {"type": "text"}, ...}}

-- Core item storage
CREATE TABLE items (
  id INTEGER PRIMARY KEY,
  path TEXT UNIQUE NOT NULL,
  data_json TEXT,             -- original document for retrieval
  created_at INTEGER NOT NULL, -- epoch milliseconds
  updated_at INTEGER NOT NULL  -- epoch milliseconds
);
CREATE INDEX idx_items_path ON items(path);
CREATE INDEX idx_items_updated ON items(updated_at);
CREATE INDEX idx_items_created ON items(created_at);

-- Field presence (for has: queries on any field type)
CREATE TABLE field_present (
  item_id INTEGER NOT NULL REFERENCES items(id),
  field TEXT NOT NULL,
  PRIMARY KEY (item_id, field)
);
CREATE INDEX idx_present_field ON field_present(field, item_id);

-- Keyword dictionary (unique values per field)
CREATE TABLE kw_dict (
  id INTEGER PRIMARY KEY,
  field TEXT NOT NULL,
  value TEXT NOT NULL,
  doc_freq INTEGER DEFAULT 0,  -- number of items with this value
  UNIQUE (field, value)
);
CREATE INDEX idx_kw_dict_lookup ON kw_dict(field, value);

-- Keyword postings (which items have which values)
CREATE TABLE kw_postings (
  field TEXT NOT NULL,
  value_id INTEGER NOT NULL REFERENCES kw_dict(id),
  item_id INTEGER NOT NULL REFERENCES items(id),
  PRIMARY KEY (value_id, item_id)
);
CREATE INDEX idx_kw_postings_item ON kw_postings(item_id);
CREATE INDEX idx_kw_postings_field ON kw_postings(field, value_id);

-- Number fields (comparisons, ranges)
CREATE TABLE field_number (
  item_id INTEGER NOT NULL REFERENCES items(id),
  field TEXT NOT NULL,
  value REAL NOT NULL,
  PRIMARY KEY (item_id, field, value)  -- allows multi-value
);
CREATE INDEX idx_num_lookup ON field_number(field, value);

-- Date fields (comparisons, ranges) - epoch milliseconds
CREATE TABLE field_date (
  item_id INTEGER NOT NULL REFERENCES items(id),
  field TEXT NOT NULL,
  value INTEGER NOT NULL,
  PRIMARY KEY (item_id, field, value)  -- allows multi-value
);
CREATE INDEX idx_date_lookup ON field_date(field, value);

-- Bool fields
CREATE TABLE field_bool (
  item_id INTEGER NOT NULL REFERENCES items(id),
  field TEXT NOT NULL,
  value INTEGER NOT NULL,     -- 0 or 1
  PRIMARY KEY (item_id, field)
);
CREATE INDEX idx_bool_lookup ON field_bool(field, value);

-- Full-text search (columns created dynamically from schema)
-- Example for schema with title + content text fields:
CREATE VIRTUAL TABLE search USING fts5(
  title,
  content,
  content='',           -- external content (we manage sync)
  tokenize='unicode61'
);

-- Cursor storage (for short cursor handles)
CREATE TABLE cursor_store (
  handle TEXT PRIMARY KEY,
  payload TEXT NOT NULL,      -- JSON: sort position + query hash
  created_at INTEGER NOT NULL,
  expires_at INTEGER NOT NULL
);
CREATE INDEX idx_cursor_expires ON cursor_store(expires_at);
```

### Why dictionary + postings for keywords?

The naive approach (`field_keyword(item_id, field, value)`) doesn't scale for wildcards.

With 1M items and multi-value tags, you might have 10-50M rows. A `LIKE '%alloc%'` scan over all of them is slow.

**Dictionary + postings** fixes this:

* **Exact match**: lookup `value_id` in `kw_dict` (log N), join postings
* **Prefix match**: range scan on `kw_dict.value LIKE 'mem%'` (indexed), get small set of `value_id`s, join postings
* **Contains match**: scan **unique values in dictionary** (often 100-1000× smaller than postings), then join

```sql
-- tags:*alloc* compiles to:
SELECT p.item_id 
FROM kw_dict d
JOIN kw_postings p ON p.value_id = d.id
WHERE d.field = 'tags' AND d.value LIKE '%alloc%'
```

This scans unique tag values (maybe 1000) instead of every tag occurrence (maybe 5M).

### Multi-column FTS

Text fields get their own FTS columns, created dynamically from schema:

```sql
-- Schema defines: title (text), summary (text), content (text)
-- Creates:
CREATE VIRTUAL TABLE search USING fts5(title, summary, content, ...);
```

This makes fielded text queries work natively:

```sql
-- title:memory & content:"error handling"
SELECT rowid FROM search 
WHERE search MATCH 'title:memory AND content:"error handling"'
```

FTS5 handles field-scoped queries, phrase matching, and ranking (via `bm25()`) correctly.

### Presence table

The `field_present` table tracks which items have which fields, regardless of type. This makes `has:` queries fast:

```sql
-- has:notes
SELECT item_id FROM field_present WHERE field = 'notes'
```

Without this, checking existence on text fields would require FTS tricks or full scans.

### How fields get stored

On insert, each field is routed based on schema:

| Schema type | Tables | Index supports |
|-------------|--------|----------------|
| `keyword` | `kw_dict` + `kw_postings` + `field_present` | exact, prefix, contains, existence |
| `keyword` + `multi` | same (multiple postings per item) | same |
| `text` | FTS column + `field_present` | words, phrases, prefix |
| `number` | `field_number` + `field_present` | `=`, `>`, `<`, `>=`, `<=`, ranges |
| `date` | `field_date` + `field_present` | same, plus relative (`<7d`) |
| `bool` | `field_bool` + `field_present` | `true`, `false` |

Dates are stored as **epoch milliseconds** consistently.

---

## Query language

### Bare text shorthand

A query without `field:` is treated as full-text search across all text fields:

```
memory                    # same as: text:memory
"error handling"          # same as: text:"error handling"
memory & tags:beginner    # mixed: full-text + fielded
```

### Operators

```
&               # AND
|               # OR
!               # NOT
( )             # grouping
```

### Wildcards

```
*               # match any characters
?               # match single character
```

Wildcards work on **keyword** and **path** fields only:

```
tags:mem*           # tags starting with "mem"
tags:*alloc*        # tags containing "alloc"
author:j?n          # "jan", "jen", "jon", etc.
path:/docs/*/intro  # intro files in any subdirectory
```

Wildcards do **not** apply to `number`, `date`, or `bool` fields (use comparisons instead).

### Predicates by field type

**Keyword fields** (exact match, wildcards)

```
tags:memory             # exact match
tags:mem*               # prefix
tags:*alloc*            # contains (scans dictionary, not postings)
category:beginner       # single value
author:"jane doe"       # quoted for spaces
```

**Text fields** (full-text search via FTS5)

```
title:memory            # word in title
content:"error handling"  # phrase in content
content:alloc*          # prefix match on word
```

Note: text fields use FTS operators (words, phrases, prefix), not glob patterns.

**Number fields** (comparisons, ranges)

```
priority:5              # equals
priority:>5             # greater than
priority:>=5            # greater than or equal
priority:<10            # less than
priority:<=10           # less than or equal
priority:5..10          # range (inclusive)
score:>=0.5
```

**Date fields** (comparisons, relative)

```
due:2024-06-01          # exact date
due:>2024-01-01         # after date
due:<2024-12-31         # before date
due:2024-01-01..2024-06-30  # date range
created:<7d             # within last 7 days
updated:>30d            # more than 30 days ago
```

Relative units: `h` (hours), `d` (days), `w` (weeks), `m` (months), `y` (years)

**Bool fields**

```
active:true
deprecated:false
!archived               # shorthand for archived:false
```

**Path (always available)**

```
path:/docs/memory/*     # prefix match
path:"/exact/path"      # exact match
path:/docs/*/intro      # glob pattern
```

Path globs with internal wildcards (`/docs/*/intro`) compile to: prefix candidate set (`/docs/`) + post-filter. Require a prefix before the first wildcard for index use.

**Existence**

```
has:notes               # field has any value (any type)
!has:deprecated         # field has no value
```

### Precedence

`!` > `&` > `|`

```
a | b & c          # means: a | (b & c)
(a | b) & c        # use parens to change
!a & b             # means: (!a) & b
```

### Full examples

```bash
# Items tagged with anything starting with "mem", beginner category
tags:mem* & category:beginner

# High priority items due soon
priority:>=8 & due:<7d & !archived

# Text search with filters
content:"error handling" & (tags:rust | tags:zig)

# Wildcard on author, numeric range
author:j* & score:0.7..1.0

# Path glob with field filter
path:/docs/*/guide & category:tutorial

# Recently updated, any tag containing "alloc"
updated:<24h & tags:*alloc*

# Existence check
has:notes & !has:deprecated
```

---

## Query planning

SQLite handles physical execution, but **you control semantic planning**. The compiler decides which constraints to apply first and how to combine results.

### Compilation by predicate type

| Predicate | Compiles to |
|-----------|-------------|
| `tags:memory` | `kw_dict` lookup → `kw_postings` join |
| `tags:mem*` | `kw_dict` range scan (`LIKE 'mem%'`) → postings join |
| `tags:*alloc*` | `kw_dict` scan (`LIKE '%alloc%'`) → postings join |
| `priority:>5` | `field_number` range query |
| `due:<7d` | `field_date` with computed epoch |
| `title:memory` | FTS `MATCH 'title:memory'` |
| `content:"error"` | FTS `MATCH 'content:"error"'` |
| `path:/docs/*` | `items.path LIKE '/docs/%'` |
| `has:notes` | `field_present` lookup |

### CTE set algebra

Compile boolean expressions to **set operations**, not nested subqueries.

Each predicate becomes a CTE returning `item_id`s:
- `AND` → `INTERSECT`
- `OR` → `UNION`
- `NOT` → `EXCEPT`

This yields predictable performance and clean `--explain` output.

### Example compilations

**Query:** `tags:mem* & category:beginner & priority:>5`

```sql
WITH
kw_tags AS (
  SELECT p.item_id
  FROM kw_dict d
  JOIN kw_postings p ON p.value_id = d.id
  WHERE d.field = 'tags' AND d.value LIKE 'mem%'
),
kw_cat AS (
  SELECT p.item_id
  FROM kw_dict d
  JOIN kw_postings p ON p.value_id = d.id
  WHERE d.field = 'category' AND d.value = 'beginner'
),
num_priority AS (
  SELECT item_id
  FROM field_number
  WHERE field = 'priority' AND value > 5
),
result AS (
  SELECT item_id FROM kw_tags
  INTERSECT
  SELECT item_id FROM kw_cat
  INTERSECT
  SELECT item_id FROM num_priority
)
SELECT i.path, json_extract(i.data_json, '$.title') AS title
FROM items i
JOIN result r ON r.item_id = i.id;
```

**Query:** `content:"error handling" & tags:rust & !tags:deprecated`

```sql
WITH
fts_match AS (
  SELECT rowid AS item_id
  FROM search
  WHERE search MATCH 'content:"error handling"'
),
kw_rust AS (
  SELECT p.item_id
  FROM kw_dict d
  JOIN kw_postings p ON p.value_id = d.id
  WHERE d.field = 'tags' AND d.value = 'rust'
),
kw_deprecated AS (
  SELECT p.item_id
  FROM kw_dict d
  JOIN kw_postings p ON p.value_id = d.id
  WHERE d.field = 'tags' AND d.value = 'deprecated'
),
result AS (
  SELECT item_id FROM fts_match
  INTERSECT
  SELECT item_id FROM kw_rust
  EXCEPT
  SELECT item_id FROM kw_deprecated
)
SELECT i.path, json_extract(i.data_json, '$.title') AS title, bm25(search) AS score
FROM items i
JOIN result r ON r.item_id = i.id
JOIN search s ON s.rowid = i.id
ORDER BY score;
```

**Query:** `(tags:a | tags:b) & (category:x | category:y)`

ORs within the same field optimize to `IN`:

```sql
WITH
kw_tags AS (
  SELECT p.item_id
  FROM kw_dict d
  JOIN kw_postings p ON p.value_id = d.id
  WHERE d.field = 'tags' AND d.value IN ('a', 'b')
),
kw_cat AS (
  SELECT p.item_id
  FROM kw_dict d
  JOIN kw_postings p ON p.value_id = d.id
  WHERE d.field = 'category' AND d.value IN ('x', 'y')
),
result AS (
  SELECT item_id FROM kw_tags
  INTERSECT
  SELECT item_id FROM kw_cat
)
SELECT i.path FROM items i JOIN result r ON r.item_id = i.id;
```

ORs across different fields use `UNION`:

```sql
-- (tags:memory | priority:>8)
WITH
kw_tags AS (...),
num_priority AS (...),
result AS (
  SELECT item_id FROM kw_tags
  UNION
  SELECT item_id FROM num_priority
)
...
```

### Negation rules

Use `EXCEPT` instead of `NOT IN` (cleaner NULL semantics):

```sql
-- !tags:deprecated
result AS (
  SELECT item_id FROM positive_predicates
  EXCEPT
  SELECT item_id FROM kw_deprecated
)
```

**Pure negative queries** (no positive anchor) are rejected. Require at least one of:
- Path prefix (`path:/docs/*`)
- Exact keyword match
- FTS term

### Path wildcards with internal `*`

`path:/docs/*/intro` compiles to:
1. Prefix candidate set: `path LIKE '/docs/%'`
2. Post-filter: `path GLOB '/docs/*/intro'`

```sql
WITH
path_candidates AS (
  SELECT id AS item_id FROM items WHERE path LIKE '/docs/%'
),
path_filtered AS (
  SELECT item_id FROM path_candidates c
  JOIN items i ON i.id = c.item_id
  WHERE i.path GLOB '/docs/*/intro'
)
...
```

**Guardrail**: require a literal prefix before the first wildcard for index use.

### Why CTE set algebra?

Nested `IN (subquery)` spam can confuse SQLite's query planner with complex boolean trees. CTEs:
- Make each predicate's cost explicit
- Map cleanly to `--explain` output
- Give SQLite clear set operations to optimize
- Handle `NOT` cleanly with `EXCEPT` (no NULL weirdness)

---

## Discover

Explore what's in your index without knowing exact values.

### List field values

```bash
# All unique values for a keyword field
ministore discover values -i docs --field tags
```

Output:
```
memory (142)
allocation (89)
heap (67)
arena (34)
stack (28)
...
```

### Filter values (uses query language)

```bash
# Values starting with "mem"
ministore discover values -i docs --field tags -w 'tags:mem*'

# Values containing "alloc"
ministore discover values -i docs --field tags -w 'tags:*alloc*'

# Values in a subset of items
ministore discover values -i docs --field tags -w 'category:beginner'

# Both: subset + value pattern
ministore discover values -i docs --field tags -w 'category:beginner & tags:mem*'
```

### Field stats

```bash
# Overview of all fields
ministore discover fields -i docs
```

Output:
```
Field       Type      Count   Unique   Example values
─────────────────────────────────────────────────────
title       text      1000    998      "How to allocate..."
tags        keyword   1000    127      memory, heap, stack
category    keyword   1000    12       beginner, advanced
priority    number    850     10       1, 2, 3, 5, 8
due         date      420     89       2024-01-15, 2024-02-01
```

### Number/date stats

```bash
# Numeric stats (query-scoped)
ministore stats -i docs --field priority -w 'tags:memory'
```

Output:
```
priority: min=1, max=10, avg=4.2, median=4
```

```bash
# Date stats
ministore stats -i docs --field created -w 'tags:memory'
```

Output:
```
created: 2023-06-15 to 2024-01-20 (234 items)
```

This makes the index explorable without prior knowledge of its contents.

---

## Ranking

Results are ordered by relevance. Ranking is optimized for **top-K** (`--limit`), not sorting millions of matches.

**Ranking modes:**

| Mode | Behavior | Output shows |
|------|----------|--------------|
| `--rank=default` | Weighted `bm25()` using schema field weights | (nothing extra) |
| `--rank=recency` | Sort by `updated_at DESC` | `updated` timestamp |
| `--rank=field:priority` | Sort by numeric/date field | field value |
| `--rank=none` | No ranking, insertion order | (nothing extra) |

When ranking by a field (not `default` or `none`), the rank value appears in search output automatically.

**Tie-breakers:**

All orderings include a unique tie-breaker (`path`) for stable pagination:

| Mode | ORDER BY |
|------|----------|
| `recency` | `updated_at DESC, path ASC` |
| `field:priority` | `priority DESC, updated_at DESC, path ASC` |
| `default` (FTS) | `bm25 DESC, rowid ASC` |

**Schema-level weights:**

```json
"title":   { "type": "text", "weight": 3.0 },
"content": { "type": "text", "weight": 1.0 }
```

Higher weight = matches in that field score higher. Default weights: `title` > other text fields.

**Performance:**

- Ranking does **not** make queries slow by itself
- Broad queries are slow because they match too much
- Field ranking is optimized for top-K (`--limit`); deep sorting of huge result sets is naturally expensive

---

## Pagination

Cursor-based, not offset-based. Deep `OFFSET` gets slower as page number grows.

**Basic usage:**

```bash
# First page
ministore search -i docs -w 'tags:memory' --limit 20 --rank=field:priority
```
```
[1] /core/how-to-allocate-memory           priority: 8
...
[20] /stdlib/mem/pool-patterns             priority: 3

next: c:7K2p9
```

```bash
# Next page
ministore search -i docs -w 'tags:memory' --limit 20 --rank=field:priority --after c:7K2p9
```

**Cursor modes:**

| Mode | Cursor format | Use case |
|------|---------------|----------|
| `--cursor=short` (default) | `c:7K2p9` | CLI, agents (small tokens) |
| `--cursor=full` | `eyJ1cGRhdGVkX2F0Ijo...` | Stateless clients |

Short cursors are stored locally in the index DB. Full cursors are self-contained but longer.

**Cursor expiry:**

- Default TTL: 1 hour
- Expired cursor returns error: `cursor expired; rerun query`
- Expired cursors cleaned up opportunistically on search

**Cursor stability:**

Cursors encode the sort position (e.g., `{priority, updated_at, path}`). Same query + same ranking + same cursor = deterministic next page.

---

## CLI surface

**Design principle:** Positional tokens are CLI keywords only. User data goes behind flags.

```
ministore <verb> [<noun>] --index <n> [--where <query>] [flags...]
```

| Concern | Mechanism |
|---------|-----------|
| Filtering | `--where` / `-w` with query language (same everywhere) |
| Output | `--format pretty\|paths\|json`, `--show` |
| Paging | `--limit`, `--after`, `--cursor` |
| Ranking | `--rank` |
| Debugging | `--explain` |

**Short flags:** `-i` for `--index`, `-w` for `--where`, `-p` for `--path`

### 1) Index management

```bash
# Create index
ministore index create -i docs --schema schema.json

# Create with inline fields
ministore index create -i docs \
  --field title:text \
  --field tags:keyword:multi \
  --field category:keyword \
  --field priority:number \
  --field content:text

# List indexes
ministore index list

# Show schema
ministore index schema -i docs

# Apply schema changes (additive)
ministore index schema -i docs --apply schema.json

# Migrate (rebuild for type changes)
ministore index migrate -i docs

# Stats
ministore index stats -i docs

# Optimize
ministore index optimize -i docs

# Drop
ministore index drop -i docs
```

Schema file format:
```json
{
  "fields": {
    "title":    { "type": "text", "weight": 3.0 },
    "tags":     { "type": "keyword", "multi": true },
    "category": { "type": "keyword" },
    "priority": { "type": "number" },
    "due":      { "type": "date" },
    "active":   { "type": "bool" },
    "content":  { "type": "text", "weight": 1.0 }
  }
}
```

### 2) Put / Get / Delete

```bash
# Put (insert or update)
ministore put -i docs -p /core/how-to-allocate-memory \
  --set title="How to allocate memory" \
  --set tags=memory,heap,allocation \
  --set category=concept \
  --set priority=5

# Put from JSON
echo '{"path": "/core/how-to-allocate-memory", ...}' | ministore put -i docs --json

# Bulk import
ministore put -i docs --import items.jsonl

# Get single item
ministore get -i docs -p /core/how-to-allocate-memory

# Peek (preview)
ministore peek -i docs -p /core/how-to-allocate-memory

# Delete single item
ministore delete -i docs -p /core/how-to-allocate-memory

# Delete by query
ministore delete -i docs -w 'path:/drafts/* & !tags:keep'
```

### 3) Search

```bash
ministore search -i docs -w 'tags:mem* & category:beginner'
```

**Default output (path only):**
```
[1] /core/how-to-allocate-memory
[2] /stdlib/mem/choosing-an-allocator
[3] /patterns/memory-cleanup-with-defer
```

**With `--rank=field:priority`:**
```bash
ministore search -i docs -w 'tags:memory' --rank=field:priority
```
```
[1] /core/how-to-allocate-memory           priority: 8
[2] /patterns/memory-cleanup-with-defer    priority: 6
[3] /stdlib/mem/choosing-an-allocator      priority: 3
```

**With `--rank=recency`:**
```bash
ministore search -i docs -w 'tags:memory' --rank=recency
```
```
[1] /core/how-to-allocate-memory           updated: 2024-01-15
[2] /patterns/memory-cleanup-with-defer    updated: 2024-01-10
[3] /stdlib/mem/choosing-an-allocator      updated: 2023-12-20
```

**With `--show=title,priority`:**
```bash
ministore search -i docs -w 'tags:memory' --show=title,priority
```
```
[1] /core/how-to-allocate-memory
    title: How to allocate memory
    priority: 8

[2] /stdlib/mem/choosing-an-allocator
    title: Choosing an allocator
    priority: 3
```

**With `--format=json`:**
```bash
ministore search -i docs -w 'tags:memory' --format=json
```
```json
{
  "results": [
    {
      "path": "/core/how-to-allocate-memory",
      "title": "How to allocate memory",
      "tags": ["memory", "heap", "allocation"],
      "category": "concept",
      "priority": 8
    }
  ]
}
```

**Options:**

| Option | Effect |
|--------|--------|
| `--limit N` | Cap results (required for pagination) |
| `--after <cursor>` | Continue from cursor position |
| `--cursor short\|full` | Cursor format (default: short) |
| `--rank default\|recency\|field:<name>\|none` | Ranking mode |
| `--show field1,field2` | Include extra fields |
| `--show all` | Include all fields |
| `--format pretty\|paths\|json` | Output format (default: pretty) |
| `--explain` | Print compiled query |

**Bare text shorthand:**

A query without `field:` is treated as full-text search across all text fields:

```bash
ministore search -i docs -w 'memory'              # same as: text:memory
ministore search -i docs -w 'memory & tags:beginner'  # mixed
```

**Explain mode:**

Preview what a query will do before executing:

```bash
ministore search -i docs -w 'tags:mem* & priority:>5' --explain
```

Output:
```
Query plan:
  1. kw_dict scan: tags LIKE 'mem%' → ~12 values
  2. kw_postings join: ~340 items
  3. field_number filter: priority > 5 → ~89 items
  4. ORDER BY: bm25 DESC, path ASC
Estimated results: ~89
```

### 4) Discover

```bash
# List all fields
ministore discover fields -i docs
```

Output:
```
Field       Type      Count   Unique   Example values
─────────────────────────────────────────────────────
title       text      1000    998      "How to allocate..."
tags        keyword   1000    127      memory, heap, stack
category    keyword   1000    12       beginner, advanced
priority    number    850     10       1, 2, 3, 5, 8
```

```bash
# List values for a field
ministore discover values -i docs --field tags
```

Output:
```
memory (142)
allocation (89)
heap (67)
...
```

```bash
# Filter values by pattern
ministore discover values -i docs --field tags -w 'tags:mem*'

# Scope to subset of items
ministore discover values -i docs --field tags -w 'category:beginner'

# Both: subset + value pattern
ministore discover values -i docs --field tags -w 'category:beginner & tags:mem*' --top=50
```

### 5) Stats

```bash
# Numeric stats (query-scoped)
ministore stats -i docs --field priority -w 'tags:memory'
```

Output:
```
priority: min=1, max=10, avg=4.2, median=4
```

```bash
# Date stats
ministore stats -i docs --field created -w 'tags:memory'
```

Output:
```
created: 2023-06-15 to 2024-01-20 (234 items)
```

---

## Flag reference

**Global flags** (work on all commands):

| Flag | Short | Description |
|------|-------|-------------|
| `--index <name>` | `-i` | Target index |
| `--format <fmt>` | | Output format: `pretty` (default), `paths`, `json` |
| `--explain` | | Show query plan without executing |

**Filtering** (search, discover, stats, delete):

| Flag | Short | Description |
|------|-------|-------------|
| `--where <query>` | `-w` | Filter using query language |

**Item operations** (put, get, peek, delete):

| Flag | Short | Description |
|------|-------|-------------|
| `--path <path>` | `-p` | Item path |
| `--set <k>=<v>` | | Set field value (put only) |
| `--json` | | Read item from stdin as JSON (put only) |
| `--import <file>` | | Bulk import from JSONL file (put only) |

**Search**:

| Flag | Short | Description |
|------|-------|-------------|
| `--limit <n>` | | Max results (required for pagination) |
| `--after <cursor>` | | Continue from cursor |
| `--cursor <mode>` | | Cursor format: `short` (default), `full` |
| `--rank <mode>` | | Ranking: `default`, `recency`, `field:<name>`, `none` |
| `--show <fields>` | | Include fields in output: `field1,field2` or `all` |

**Discover**:

| Flag | Short | Description |
|------|-------|-------------|
| `--field <name>` | | Field to discover values for |
| `--top <n>` | | Limit number of values returned |

**Stats**:

| Flag | Short | Description |
|------|-------|-------------|
| `--field <name>` | | Field to compute stats for |

---

## Compact help

For agent context windows. Full command reference in one screen.

```
ministore — single-file search index

GRAMMAR
  ministore <verb> [<noun>] -i <index> [-w <query>] [flags]

INDEX MANAGEMENT
  index create -i <n> --schema <file>     Create index with schema
  index create -i <n> --field <f>:<type>  Create with inline fields
  index list                              List all indexes
  index schema -i <n>                     Show schema
  index schema -i <n> --apply <file>      Apply schema changes
  index migrate -i <n>                    Rebuild for type changes
  index stats -i <n>                      Show index statistics
  index optimize -i <n>                   Vacuum and rebuild
  index drop -i <n>                       Delete index

DATA OPERATIONS
  put -i <n> -p <path> --set k=v          Insert or update item
  put -i <n> --json                       Insert from stdin JSON
  put -i <n> --import <file>              Bulk import JSONL
  get -i <n> -p <path>                    Get item
  peek -i <n> -p <path>                   Preview item
  delete -i <n> -p <path>                 Delete item
  delete -i <n> -w <query>                Delete by query

SEARCH
  search -i <n> -w <query>                Search items
    --limit <n>                           Max results
    --after <cursor>                      Pagination cursor
    --rank default|recency|field:<f>|none Ranking mode
    --show <fields>                       Fields to display
    --format pretty|paths|json            Output format

DISCOVER
  discover fields -i <n>                  List all fields
  discover values -i <n> --field <f>      List field values
    -w <query>                            Filter values/scope items
    --top <n>                             Limit values

STATS
  stats -i <n> --field <f>                Numeric/date statistics
    -w <query>                            Scope to query results

QUERY LANGUAGE
  tags:memory                             Exact match
  tags:mem*                               Prefix wildcard
  tags:*alloc*                            Contains wildcard
  priority:>5                             Numeric comparison
  priority:1..10                          Numeric range
  due:<7d                                 Relative date
  content:"error handling"                Phrase search
  has:notes                               Field exists
  a & b                                   AND
  a | b                                   OR
  !a                                      NOT
  (a | b) & c                             Grouping
  memory                                  Bare text = full-text search

SHORT FLAGS
  -i  --index
  -w  --where
  -p  --path
```

---

## Library API

```python
from ministore import Index, Schema, Field

# Define schema
schema = Schema(
    title=Field.text(weight=3.0),
    tags=Field.keyword(multi=True),
    category=Field.keyword(),
    priority=Field.number(),
    due=Field.date(),
    content=Field.text(weight=1.0),
)

# Create index
db = Index.create("docs.db", schema)

# Or open existing
db = Index.open("docs.db")

# Insert
db.put("/core/how-to-allocate-memory",
       title="How to allocate memory",
       tags=["memory", "heap", "allocation"],
       category="concept",
       priority=5,
       content="Full text here...")

# Search (bare text = full-text search across all text fields)
results = db.search("tags:mem* & priority:>3", limit=10)
for item in results:
    print(item.path)  # paths are descriptive, title optional via --show

# Bare text is shorthand for text:...
results = db.search("memory", limit=10, rank="recency")  # same as "text:memory"
results = db.search("memory", limit=10, rank="field:priority")

# Pagination
results = db.search("tags:memory", limit=20)
print(results.items)        # first 20 results
print(results.next_cursor)  # "c:7K2p9" or None if no more

# Next page
results = db.search("tags:memory", limit=20, after=results.next_cursor)

# Get single item
item = db.get("/core/how-to-allocate-memory")

# Discover (field values, all filtering via query)
tag_values = db.discover("tags")                                    # all values
tag_values = db.discover("tags", query="tags:mem*")                 # filter values
tag_values = db.discover("tags", query="category:beginner")         # scope to items
tag_values = db.discover("tags", query="category:beginner & tags:mem*")  # both

# Field stats (query-scoped)
stats = db.stats("priority", query="tags:memory")
# → {"min": 1, "max": 10, "avg": 4.2, "median": 4}

# Field overview
fields = db.fields()

# Delete
db.delete("/core/how-to-allocate-memory")
db.delete_where("path:/drafts/* & !tags:keep")

# Batch operations
with db.batch() as batch:
    batch.put("/a", title="A", tags=["x"])
    batch.put("/b", title="B", tags=["y"])
    batch.delete("/c")
```

---

## What you get

* **Explicit schemas** — define once, validate on insert, query with confidence
* **Wildcards on keywords** — `tags:mem*` and `tags:*alloc*` scale via dictionary+postings
* **Proper FTS** — multi-column full-text with native field scoping and `bm25()` ranking
* **Discover** — explore your index without knowing what's in it
* **Typed queries** — numeric ranges, date comparisons, boolean logic
* **Fast at scale** — dictionary+postings for keywords, B-tree for numbers/dates, FTS for text
* **Cursor pagination** — stable, agent-friendly, no server state
* **Minimal output** — paths only by default, add fields with `--show`
* **Single-file storage** — easy to distribute, no server
* **Predictable performance** — CTE set algebra, top-K ranking optimization
* **Deterministic results** — no embeddings, no ML

## What you avoid

* Schema drift and field name typos
* "What values can I filter on?" guessing games
* Slow wildcard searches on wrong index types
* Server dependencies
* Unpredictable query performance
* OFFSET pagination slowdown on deep pages

---

## Scaling notes

**Expected performance:**

* 1M items: <100ms for selective queries
* 10M items: <500ms for selective queries (broad queries naturally cost more)
* Beyond: consider dedicated search (Tantivy, Meilisearch)

**The dictionary+postings win:**

With 1M items and multi-value tags, naive storage might have 10-50M keyword rows. The dictionary approach means:
- Contains wildcard (`*alloc*`) scans ~1000 unique values, not 5M occurrences
- Prefix wildcard (`mem*`) uses B-tree range scan on dictionary
- Exact match is single dictionary lookup + postings join

**Watch for:**

* **Leading wildcards on huge vocabularies** — `*a*` on a field with 100K unique values still scans 100K rows
* **Pure negative queries** — require a positive anchor (path prefix, keyword, or FTS term)
* **ORs matching most of the corpus** — `tags:common1 | tags:common2` can return huge result sets
* **Large text content (>100KB)** — chunk or truncate for FTS

**Practical guardrails:**

* Require at least one positive anchor in every query
* Enforce minimum prefix length (3+ chars) for wildcards
* Cap dictionary expansion when prefix matches too many values

**Error messages for rejected queries:**

```bash
ministore search -i docs -w '!tags:deprecated'
# Error: query requires at least one positive anchor (path prefix, keyword, or FTS term)

ministore search -i docs -w 'tags:*a*'
# Error: contains pattern too short (minimum 3 characters)

ministore search -i docs -w 'tags:a*'
# Error: prefix 'a' matches too many values (15,234); add more specific filters
```

**Ranking and pagination:**

* Ranking is optimized for top-K (`--limit`); sorting millions of matches is naturally expensive
* Use `--rank=field:<name>` with `--limit` for fast "top N by priority" queries
* Cursor pagination stays fast regardless of "page number" (no OFFSET penalty)
* Short cursors (`c:7K2p9`) keep agent context windows small

**Bulk ingestion tips:**

* Wrap imports in a single transaction
* Use WAL mode: `PRAGMA journal_mode=WAL`
* Tune for import: `PRAGMA synchronous=NORMAL`

---

## Decisions

### One index per file

Each `.db` file is one index. Simplest operational model: copy one file = copy one index.

**Future**: Add "workspace" concept—a directory of `.db` files + manifest. Cross-index search merges results client-side.

### Schema evolution

| Change | Allowed? | Notes |
|--------|----------|-------|
| Add field | Yes | Old items implicitly missing (no presence entry) |
| Remove field | Yes (soft) | Field stays until `ministore optimize`; queries error on unknown fields |
| Change type | Requires `ministore migrate` | Triggers full reindex |
| Single → multi | Yes | No migration needed |
| Multi → single | Requires `ministore migrate` | May need to drop extra values |

Schema version stored in `meta`. Additive changes via `ministore schema apply`. Type changes via `ministore migrate`.

### Contains wildcards

No trigram index by default. Dictionary scans are fast enough for most vocabularies.

**Guardrails:**
- Minimum pattern length for contains (e.g., `*ab*` rejected, `*alloc*` allowed)
- Cap dictionary scan cost—if vocab > threshold and pattern too short, error with suggestion

**Future**: Optional per-field trigram indexing:
```json
"tags": { "type": "keyword", "multi": true, "contains_index": "trigram" }
```

### Cross-index queries

Indexes are isolated. No federated SQL.

**Workspace multi-search** (future):
```bash
ministore search --all 'tags:memory'   # searches all indexes in workspace
```

Schema mismatch handling: unknown field in query → no match + warning in `--explain`.

### Aggregations

Query-scoped only. No global aggregations on huge corpora.

```bash
# Faceted counts
ministore discover values -i docs --field tags -w 'category:beginner' --top=50

# Numeric stats
ministore stats -i docs --field priority -w 'tags:memory'
# → min: 1, max: 10, avg: 4.2, median: 4

# Date stats  
ministore stats -i docs --field created -w 'tags:memory'
# → earliest: 2023-06-15, latest: 2024-01-20
```

Implementation: aggregations join against the `result` CTE from query compilation.

### FTS ranking

Schema-level field weights, not raw `bm25()` exposure.

```json
{
  "fields": {
    "title":   { "type": "text", "weight": 3.0 },
    "content": { "type": "text", "weight": 1.0 }
  }
}
```

Default: `title` heavier than `content`.

CLI rank modes:
```bash
ministore search -i docs -w 'memory' --rank=default         # weighted bm25
ministore search -i docs -w 'memory' --rank=recency         # sort by updated_at, shows timestamp
ministore search -i docs -w 'memory' --rank=field:priority  # sort by field, shows value
ministore search -i docs -w 'memory' --rank=none            # no ranking, insertion order
```

All orderings include tie-breakers (`path`) for stable cursor pagination.

**Future**: Query-time boost overrides (`--boost title=5`).

---

## Future extensions

These are explicitly **not in v1** but the architecture supports them:

1. **Workspace**: Directory of indexes + manifest for cross-index search
2. **Trigram contains index**: Per-field opt-in for huge vocabularies
3. **Query-time ranking boosts**: `--boost title=5 content=1`
4. **Richer aggregations**: Histograms, percentiles, group-by
5. **Subscriptions**: Watch for new items matching a query
6. **Synonyms**: `memory` matches `heap` and `ram` automatically