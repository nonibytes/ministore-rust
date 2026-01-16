Below is a repo-grade design spec for **ministore v1** in Rust (library + CLI), using **rusqlite** and SQLite **FTS5**, with every mechanic pinned down to concrete structs, modules, SQL shapes, and function signatures—**but intentionally stopping short of implementation** (bodies are described, not written).

The layout is a Cargo workspace with two crates:

* `crates/ministore` → reusable library (future bindings target)
* `crates/ministore-cli` → CLI that depends on the library

At the end there’s a **Python expansion script** that can turn this document into a skeleton repo by extracting the per-file code blocks.

---

## Global conventions

### Index file resolution

* CLI `--index/-i` accepts either:

  1. A bare name like `docs` → resolved to `./docs.db`
  2. A path containing `/` or ending in `.db` → used as-is
* Index “list” scans current directory for `*.db` and validates `meta.ministore_magic`.

### SQLite requirements

* Must have FTS5 enabled.
* On open/create, verify:

  * `SELECT sqlite_compileoption_used('ENABLE_FTS5')` == 1
  * If not, error `MinistoreError::SqliteFeatureMissing("FTS5")`

### Time

* Epoch milliseconds stored as `i64` everywhere.
* `now_ms()` is always UTC-based system time.

### JSON

* Schema and item documents use `serde_json::Value`.
* `items.data_json` stores the **full original document**, including `"path"`.

### Field naming constraints

* Field names must match regex: `^[A-Za-z_][A-Za-z0-9_]*$`
* This prevents SQL identifier injection when generating FTS table columns and `ALTER TABLE search ADD COLUMN ...`.

### Guardrails (configurable constants)

* Keyword contains wildcard minimum inner length: `MIN_CONTAINS_LEN = 3`
* Keyword prefix minimum literal prefix length: `MIN_PREFIX_LEN = 2`
* Dictionary expansion cap for prefix patterns: `MAX_PREFIX_EXPANSION = 20_000`
* Cursor TTL default: `CURSOR_TTL_MS = 60 * 60 * 1000` (1 hour)

---

## Data model: schema and implicit fields

### Schema JSON format

Stored in `meta` as JSON:

```json
{
  "fields": {
    "title":    { "type": "text", "weight": 3.0 },
    "tags":     { "type": "keyword", "multi": true },
    "priority": { "type": "number" },
    "due":      { "type": "date" },
    "active":   { "type": "bool" },
    "content":  { "type": "text", "weight": 1.0 }
  }
}
```

### Implicit fields (always queryable)

Not in schema JSON, but treated as available fields:

* `path` (special)
* `created` (maps to `items.created_at`)
* `updated` (maps to `items.updated_at`)

---

## SQLite schema (per index DB)

Exactly as described in the prompt, with two additions:

* `meta.ministore_magic = "ministore"`
* `meta.ministore_version = "1"`

FTS:

* Virtual table `search` created dynamically with one column per schema text field.
* Uses external content: `content=''`
* We explicitly insert/delete rows in `search` with `rowid = items.id`.

---

## Query language: pinned grammar and semantics

### Lexer tokens

* Identifiers: `[A-Za-z_][A-Za-z0-9_]*`
* Operators: `& | ! ( ) :`
* Comparators: `> >= < <=`
* Range operator: `..`
* Strings: `"..."` with `\"` escape
* Bare terms: either identifier-like or quoted phrase
* Wildcards: `*` and `?` (keyword/path only)

### Precedence

`!` > `&` > `|`

### Fielded predicates by type

* keyword: exact or wildcard patterns (`*`, `?`)
* text: FTS query terms/phrases/prefix (`alloc*` means FTS prefix)
* number: comparisons and inclusive range `a..b`
* date:

  * absolute ISO date `YYYY-MM-DD` (treated as UTC midnight)
  * absolute ISO datetime `YYYY-MM-DDTHH:MM:SSZ` (optional support; library parses if present)
  * relative duration `7d`, `24h`, `2w`, `3m`, `1y`
* bool: `true/false`, plus shorthand `!field` → `field:false`
* has: `has:field`

### Relative date semantics (important and explicit)

We must satisfy both:

* `created:<7d` meaning “within last 7 days”
* `due:<7d` meaning “due before now + 7 days”

Rule:

* For implicit fields `created` and `updated`: relative durations are interpreted as **age**

  * `created:<7d` ⇒ `created_at >= now - 7d`
  * `updated:>30d` ⇒ `updated_at <= now - 30d`
* For all other date fields (schema-defined): relative durations are interpreted as **offset from now**

  * `due:<7d` ⇒ `due_ms < now + 7d`
  * `due:>7d` ⇒ `due_ms > now + 7d`

This rule is enforced in the planner based on the field name.

### Pure negative query rejection

A query is rejected if it has no “positive anchor”:

* Positive anchor types:

  * FTS term (bare text or text field predicate)
  * keyword exact match (no wildcards) on any field
  * path prefix pattern `path:/literal/prefix*` (must start with literal)
* If query is only negations (e.g. `!tags:deprecated`), error.

### Path wildcard rule

* If pattern contains `*` or `?` not at the end, require a literal prefix before the first wildcard.
* Compilation:

  1. Candidate set via indexed `LIKE 'prefix%'`
  2. Post-filter via `GLOB 'pattern'`

---

## Query planning: CTE set algebra (pinned)

Each predicate compiles to a named CTE returning `item_id`:

* AND → `INTERSECT`
* OR → `UNION`
* NOT → `EXCEPT`

Planner also emits:

* `params: Vec<SqlParam>` in positional order
* `explain_steps: Vec<ExplainStep>` (strings + optional counts)
* `requires_fts: bool` and `fts_match_param: Option<String>`

Special OR optimization:

* If an OR subtree is `field:value1 | field:value2 | ...` for **same keyword field** and all are **exact** (no wildcards), compile to one CTE using `IN (...)`.

---

## Ranking and pagination (mechanics)

### Rank modes

* `default`

  * If query contains any FTS constraint: use weighted bm25 score
  * Else: fallback to `updated_at DESC, path ASC` (documented behavior)
* `recency`: `updated_at DESC, path ASC`
* `field:<name>`:

  * field must be number/date
  * ranking value per item is:

    * single field: its value
    * multi field: `MAX(value)` for descending ranking
  * order: `rank_value DESC, updated_at DESC, path ASC`
* `none`: `items.id ASC` (insertion order), tie stable inherently

### Weighted bm25

FTS5’s `bm25()` is “smaller is better”. To keep “higher is better”:

* define `score = -bm25(search, w1, w2, ...)`
* order by `score DESC, items.id ASC`

Weights:

* For each text field, default weight is:

  * schema weight if present
  * else `1.0`
* Provide weight vector in FTS bm25 call in the **same order** as FTS columns.

### Cursor payload

Cursor must encode:

* query hash (so cursor can’t be used with different query)
* rank mode
* last sort position (rank keys + tiebreaker)
* expires_at (for full cursor too, so stateless clients can know)

Rust struct:

* `CursorPayload { query_hash, rank, position, expires_at_ms }`

Position variants:

* `Recency { updated_at_ms, path }`
* `Field { field, value_num_or_date, updated_at_ms, path }`
* `Fts { score, item_id }`
* `None { item_id }`

### Short cursor storage

* Token format: `c:<base62>`
* Stored in `cursor_store(handle, payload_json, created_at, expires_at)`
* Cleanup:

  * On each search, delete expired rows: `DELETE FROM cursor_store WHERE expires_at < now`

### Pagination filter SQL

Applied before ORDER BY + LIMIT.

Examples:

* Recency:

  * ordering: `(updated_at DESC, path ASC)`
  * after position `(u0, p0)` means next page is rows where:

    * `updated_at < u0 OR (updated_at = u0 AND path > p0)`
* Field:

  * ordering: `(rank_value DESC, updated_at DESC, path ASC)`
  * after `(v0, u0, p0)`:

    * `rank_value < v0 OR (rank_value = v0 AND (updated_at < u0 OR (updated_at = u0 AND path > p0)))`
* FTS:

  * ordering: `(score DESC, item_id ASC)`
  * after `(s0, id0)`:

    * `score < s0 OR (score = s0 AND item_id > id0)`
* None:

  * ordering: `(item_id ASC)`
  * after `id0`: `item_id > id0`

---

## Library API surface (binding-friendly)

Design goal: public API uses:

* `String`, `Vec`, `HashMap`
* `serde_json::Value`
* simple enums and structs
  No lifetimes in public return types; iterators are avoided in public API to help FFI later.

---

# Files

Each file below includes:

* Purpose
* Key structs/enums
* Function signatures
* Any SQL constants it owns
* “Algorithm notes” describing exactly what the implementation does

Where code blocks appear, they’re intended as **skeletons**.

---

[File] Name: Cargo.toml

```toml
[workspace]
resolver = "2"
members = [
  "crates/ministore",
  "crates/ministore-cli",
]
```

---

[File] Name: crates/ministore/Cargo.toml

```toml
[package]
name = "ministore"
version = "0.1.0"
edition = "2021"

[lib]
name = "ministore"
path = "src/lib.rs"

[dependencies]
rusqlite = { version = "0.31", features = ["bundled", "serde_json"] }
serde = { version = "1", features = ["derive"] }
serde_json = "1"
thiserror = "1"
base64 = "0.22"
sha2 = "0.10"
chrono = { version = "0.4", default-features = false, features = ["clock", "std"] }
rand = "0.8"
```

Notes:

* `bundled` ensures SQLite with FTS5. (If you want system SQLite, remove and keep runtime check.)

---

[File] Name: crates/ministore/src/lib.rs

```rust
//! Ministore library: single-file search index on SQLite.

pub mod error;
pub mod schema;
pub mod index;
pub mod item;
pub mod query;
pub mod cursor;
pub mod discover;
pub mod stats;
pub mod batch;
pub mod db;

pub use crate::error::{MinistoreError, Result};
pub use crate::schema::{Schema, FieldSpec, FieldType};
pub use crate::index::{Index, IndexOptions, SearchOptions, SearchResultPage, RankMode, CursorMode, OutputFieldSelector};
pub use crate::item::{ItemDoc, ItemMeta, ItemView};
```

---

[File] Name: crates/ministore/src/error.rs

```rust
use thiserror::Error;

pub type Result<T> = std::result::Result<T, MinistoreError>;

#[derive(Debug, Error)]
pub enum MinistoreError {
  #[error("io error: {0}")]
  Io(#[from] std::io::Error),

  #[error("sqlite error: {0}")]
  Sqlite(#[from] rusqlite::Error),

  #[error("schema error: {0}")]
  Schema(String),

  #[error("query parse error: {0}")]
  QueryParse(String),

  #[error("query rejected: {0}")]
  QueryRejected(String),

  #[error("unknown field: {0}")]
  UnknownField(String),

  #[error("type mismatch for field '{field}': {message}")]
  TypeMismatch { field: String, message: String },

  #[error("cursor error: {0}")]
  Cursor(String),

  #[error("sqlite missing required feature: {0}")]
  SqliteFeatureMissing(String),
}
```

Error message strings are stable because the CLI prints them directly.

---

[File] Name: crates/ministore/src/schema.rs

```rust
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Schema {
  pub fields: BTreeMap<String, FieldSpec>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FieldSpec {
  #[serde(rename = "type")]
  pub field_type: FieldType,
  #[serde(default)]
  pub multi: bool,
  #[serde(default)]
  pub weight: Option<f64>, // only used for text
}

#[derive(Debug, Copy, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum FieldType {
  Keyword,
  Text,
  Number,
  Date,
  Bool,
}

impl Schema {
  pub fn validate(&self) -> crate::Result<()> { /* described in doc */ unimplemented!() }

  pub fn text_fields_in_order(&self) -> Vec<(String, f64)> { unimplemented!() }
  // Returns (field_name, weight), sorted by field name for deterministic FTS column order.

  pub fn get(&self, field: &str) -> Option<&FieldSpec> { self.fields.get(field) }
  pub fn has_field(&self, field: &str) -> bool { self.fields.contains_key(field) }
}
```

Algorithm notes for `validate()`:

* Ensure at least 1 text field if you want bare text queries to work (not required, but warn in explain).
* Validate field names against regex.
* Validate `weight` is only present for `Text` and is positive if present.
* Enforce: if `field_type != Text`, ignore `weight` (or error; choose strict → error).

FTS column order rule:

* Deterministic ordering by field name ensures cursor/query hash stable across processes.

---

[File] Name: crates/ministore/src/item.rs

```rust
use serde_json::Value;

#[derive(Debug, Clone)]
pub struct ItemDoc {
  pub path: String,
  pub fields: Value, // object containing user fields (may include "path" too, but we normalize)
}

#[derive(Debug, Clone)]
pub struct ItemMeta {
  pub created_at_ms: i64,
  pub updated_at_ms: i64,
}

#[derive(Debug, Clone)]
pub struct ItemView {
  pub path: String,
  pub doc: Value,     // full document (includes "path")
  pub meta: ItemMeta, // timestamps
}

impl ItemDoc {
  pub fn from_json(v: Value) -> crate::Result<Self> { unimplemented!() }
  // Enforces "path" exists and is string.

  pub fn to_storage_json(&self) -> Value { unimplemented!() }
  // Ensures "path" included in stored JSON.
}
```

---

[File] Name: crates/ministore/src/index.rs

```rust
use crate::{Result, Schema};
use serde_json::Value;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub struct IndexOptions {
  pub cursor_ttl_ms: i64,
}

impl Default for IndexOptions {
  fn default() -> Self {
    Self { cursor_ttl_ms: 60 * 60 * 1000 }
  }
}

#[derive(Debug, Clone)]
pub enum CursorMode { Short, Full }

#[derive(Debug, Clone)]
pub enum RankMode {
  Default,
  Recency,
  Field(String), // number/date fields only
  None,
}

#[derive(Debug, Clone)]
pub enum OutputFieldSelector {
  None,         // paths only
  All,          // entire doc
  Fields(Vec<String>), // subset
}

#[derive(Debug, Clone)]
pub struct SearchOptions {
  pub limit: usize,
  pub after: Option<String>,
  pub cursor_mode: CursorMode,
  pub rank: RankMode,
  pub show: OutputFieldSelector,
  pub explain: bool,
}

#[derive(Debug, Clone)]
pub struct SearchResultPage {
  pub items: Vec<Value>,          // output objects (already shaped for CLI / bindings)
  pub next_cursor: Option<String>,
  pub explain_sql: Option<String>,  // present if explain==true
  pub explain_steps: Option<Vec<String>>,
}

pub struct Index {
  db_path: PathBuf,
  schema: Schema,
  opts: IndexOptions,
  // Internals: connection pool is NOT used in v1; a single rusqlite::Connection per Index.
  // To keep binding-friendly, we hide rusqlite types behind db module.
}

impl Index {
  pub fn create<P: AsRef<Path>>(path: P, schema: Schema, opts: IndexOptions) -> Result<Self> { unimplemented!() }
  pub fn open<P: AsRef<Path>>(path: P, opts: IndexOptions) -> Result<Self> { unimplemented!() }

  pub fn schema(&self) -> &Schema { &self.schema }
  pub fn path(&self) -> &Path { &self.db_path }

  // Put / Get / Delete
  pub fn put_json(&self, doc: Value) -> Result<()> { unimplemented!() }
  pub fn put_fields(&self, path: &str, fields: Value) -> Result<()> { unimplemented!() }
  pub fn get(&self, path: &str) -> Result<crate::item::ItemView> { unimplemented!() }
  pub fn peek(&self, path: &str) -> Result<Value> { unimplemented!() }
  pub fn delete(&self, path: &str) -> Result<bool> { unimplemented!() }
  pub fn delete_where(&self, query: &str) -> Result<u64> { unimplemented!() }

  // Search / Discover / Stats
  pub fn search(&self, query: &str, options: SearchOptions) -> Result<SearchResultPage> { unimplemented!() }
  pub fn discover_values(&self, field: &str, query: Option<&str>, top: usize) -> Result<Vec<(String, u64)>> { unimplemented!() }
  pub fn discover_fields(&self) -> Result<Vec<Value>> { unimplemented!() } // returns structured rows for CLI
  pub fn stats(&self, field: &str, query: Option<&str>) -> Result<Value> { unimplemented!() }

  // Index management
  pub fn optimize(&self) -> Result<()> { unimplemented!() }
  pub fn migrate_rebuild<P: AsRef<Path>>(&self, new_path: P, new_schema: Schema) -> Result<()> { unimplemented!() }
  pub fn apply_schema(&self, new_schema: Schema) -> Result<()> { unimplemented!() }
}
```

### Index method mechanics (exact)

#### `create()`

1. `schema.validate()`
2. open connection
3. enforce FTS5 available
4. run initial DDL (all tables except FTS)
5. create FTS virtual table `search` with columns from `schema.text_fields_in_order()`
6. insert meta:

   * `ministore_magic = "ministore"`
   * `ministore_version = "1"`
   * `schema_json = <string>`
7. set pragmas:

   * `PRAGMA journal_mode=WAL`
   * `PRAGMA synchronous=NORMAL`
   * `PRAGMA foreign_keys=ON`

#### `open()`

1. open connection
2. verify `meta.ministore_magic`
3. load schema from `meta.schema_json`
4. validate schema (for safety)
5. verify FTS columns match schema (see db module)
6. set pragmas

#### `put_json() / put_fields()`

Implemented in db layer via a single transaction:

* Upsert items row by path:

  * if exists, keep `created_at`, set `updated_at=now`
  * else set both to `now`
* Delete old per-field entries for this item (in correct order):

  * read old keyword postings value_ids to adjust doc_freq
  * delete from `kw_postings` for item_id
  * delete from number/date/bool tables for item_id
  * delete from field_present for item_id
  * delete from search for rowid=item_id
* Insert new field_present rows for each provided field with any value
* Insert keyword postings:

  * For each keyword value:

    * `INSERT OR IGNORE kw_dict(field,value,doc_freq=0)`
    * fetch dict id
    * `INSERT OR IGNORE kw_postings(field,value_id,item_id)`
    * if posting was newly inserted, increment kw_dict.doc_freq by 1
* Insert numbers/dates:

  * if multi: insert each value
  * else: insert exactly one value
* Insert bool:

  * one row per field
* Insert FTS row:

  * `INSERT INTO search(rowid, col1, col2, ...) VALUES (?, ?, ...)`
* Commit

All conversions/validation happen before touching DB:

* unknown fields error
* type mismatch error
* multi enforcement:

  * if schema says multi=false but provided array → error
  * if schema says multi=true but provided scalar → accept by wrapping into list (v1 choice: accept)

#### `delete(path)`

Transaction:

1. find item_id by path; if none return false
2. decrement doc_freq for all dict entries referenced by postings for this item:

   * query: `SELECT value_id FROM kw_postings WHERE item_id=?`
   * for each value_id: `UPDATE kw_dict SET doc_freq=doc_freq-1 WHERE id=?`
3. delete from all per-field tables by item_id
4. delete from items
5. commit

#### `delete_where(query)`

Transaction:

1. compile query to a `result` CTE
2. compute affected keyword value_id counts:

   * `SELECT value_id, COUNT(*) FROM kw_postings WHERE item_id IN result GROUP BY value_id`
3. apply deletes:

   * delete kw_postings where item_id IN result
   * decrement kw_dict.doc_freq by the computed counts
   * delete number/date/bool/present/search/items where id in result
4. return number of deleted items: `SELECT COUNT(*) FROM result`

---

[File] Name: crates/ministore/src/db/mod.rs

```rust
pub mod sql;
pub mod ddl;
pub mod meta;
pub mod verify;
pub mod put;
pub mod delete;
pub mod search;
pub mod migrate;

use rusqlite::Connection;

pub struct Db {
  pub conn: Connection,
}

impl Db {
  pub fn open(path: &std::path::Path) -> crate::Result<Self> { unimplemented!() }
}
```

`Db` is internal; `Index` owns it (either stored directly or recreated per call; v1 stores one).

---

[File] Name: crates/ministore/src/db/sql.rs

```rust
// Central place for SQL string constants (non-DDL).

pub const SQL_GET_META: &str = "SELECT value FROM meta WHERE key = ?1";
pub const SQL_SET_META: &str = "INSERT INTO meta(key,value) VALUES(?1,?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value";

pub const SQL_FIND_ITEM_ID_BY_PATH: &str = "SELECT id FROM items WHERE path = ?1";
pub const SQL_GET_ITEM_BY_PATH: &str = "SELECT data_json, created_at, updated_at FROM items WHERE path = ?1";

pub const SQL_DELETE_SEARCH_ROW: &str = "DELETE FROM search WHERE rowid = ?1";
pub const SQL_DELETE_PRESENT_BY_ITEM: &str = "DELETE FROM field_present WHERE item_id = ?1";
pub const SQL_DELETE_POSTINGS_BY_ITEM: &str = "DELETE FROM kw_postings WHERE item_id = ?1";
pub const SQL_DELETE_NUMBER_BY_ITEM: &str = "DELETE FROM field_number WHERE item_id = ?1";
pub const SQL_DELETE_DATE_BY_ITEM: &str = "DELETE FROM field_date WHERE item_id = ?1";
pub const SQL_DELETE_BOOL_BY_ITEM: &str = "DELETE FROM field_bool WHERE item_id = ?1";

pub const SQL_CLEANUP_EXPIRED_CURSORS: &str = "DELETE FROM cursor_store WHERE expires_at < ?1";
pub const SQL_GET_CURSOR: &str = "SELECT payload FROM cursor_store WHERE handle = ?1";
pub const SQL_PUT_CURSOR: &str = "INSERT INTO cursor_store(handle, payload, created_at, expires_at) VALUES(?1,?2,?3,?4)";
```

---

[File] Name: crates/ministore/src/db/ddl.rs

```rust
// DDL creation for all base tables; FTS DDL is generated in code because it depends on schema text fields.

pub const DDL_BASE: &str = r#"
CREATE TABLE IF NOT EXISTS meta (
  key TEXT PRIMARY KEY,
  value TEXT
);

CREATE TABLE IF NOT EXISTS items (
  id INTEGER PRIMARY KEY,
  path TEXT UNIQUE NOT NULL,
  data_json TEXT NOT NULL,
  created_at INTEGER NOT NULL,
  updated_at INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_items_path ON items(path);
CREATE INDEX IF NOT EXISTS idx_items_updated ON items(updated_at);
CREATE INDEX IF NOT EXISTS idx_items_created ON items(created_at);

CREATE TABLE IF NOT EXISTS field_present (
  item_id INTEGER NOT NULL REFERENCES items(id),
  field TEXT NOT NULL,
  PRIMARY KEY (item_id, field)
);
CREATE INDEX IF NOT EXISTS idx_present_field ON field_present(field, item_id);

CREATE TABLE IF NOT EXISTS kw_dict (
  id INTEGER PRIMARY KEY,
  field TEXT NOT NULL,
  value TEXT NOT NULL,
  doc_freq INTEGER DEFAULT 0,
  UNIQUE (field, value)
);
CREATE INDEX IF NOT EXISTS idx_kw_dict_lookup ON kw_dict(field, value);

CREATE TABLE IF NOT EXISTS kw_postings (
  field TEXT NOT NULL,
  value_id INTEGER NOT NULL REFERENCES kw_dict(id),
  item_id INTEGER NOT NULL REFERENCES items(id),
  PRIMARY KEY (value_id, item_id)
);
CREATE INDEX IF NOT EXISTS idx_kw_postings_item ON kw_postings(item_id);
CREATE INDEX IF NOT EXISTS idx_kw_postings_field ON kw_postings(field, value_id);

CREATE TABLE IF NOT EXISTS field_number (
  item_id INTEGER NOT NULL REFERENCES items(id),
  field TEXT NOT NULL,
  value REAL NOT NULL,
  PRIMARY KEY (item_id, field, value)
);
CREATE INDEX IF NOT EXISTS idx_num_lookup ON field_number(field, value);

CREATE TABLE IF NOT EXISTS field_date (
  item_id INTEGER NOT NULL REFERENCES items(id),
  field TEXT NOT NULL,
  value INTEGER NOT NULL,
  PRIMARY KEY (item_id, field, value)
);
CREATE INDEX IF NOT EXISTS idx_date_lookup ON field_date(field, value);

CREATE TABLE IF NOT EXISTS field_bool (
  item_id INTEGER NOT NULL REFERENCES items(id),
  field TEXT NOT NULL,
  value INTEGER NOT NULL,
  PRIMARY KEY (item_id, field)
);
CREATE INDEX IF NOT EXISTS idx_bool_lookup ON field_bool(field, value);

CREATE TABLE IF NOT EXISTS cursor_store (
  handle TEXT PRIMARY KEY,
  payload TEXT NOT NULL,
  created_at INTEGER NOT NULL,
  expires_at INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_cursor_expires ON cursor_store(expires_at);
"#;
```

FTS DDL is generated in `db::verify` / `db::search` module using schema text fields.

---

[File] Name: crates/ministore/src/db/meta.rs

```rust
use crate::Result;
use rusqlite::Connection;

pub const META_MAGIC_KEY: &str = "ministore_magic";
pub const META_MAGIC_VAL: &str = "ministore";
pub const META_VERSION_KEY: &str = "ministore_version";
pub const META_SCHEMA_KEY: &str = "schema_json";

pub fn read_meta(conn: &Connection, key: &str) -> Result<Option<String>> { unimplemented!() }
pub fn write_meta(conn: &Connection, key: &str, val: &str) -> Result<()> { unimplemented!() }
```

---

[File] Name: crates/ministore/src/db/verify.rs

```rust
use crate::{Result, Schema};
use rusqlite::Connection;

pub fn require_fts5(conn: &Connection) -> Result<()> { unimplemented!() }

// Ensures the existing FTS table columns match schema text fields.
// On mismatch: Schema error suggesting migrate.
pub fn verify_fts_columns(conn: &Connection, schema: &Schema) -> Result<()> { unimplemented!() }

// Build CREATE VIRTUAL TABLE search USING fts5(...) statement from schema.
pub fn build_fts_ddl(schema: &Schema) -> Result<String> { unimplemented!() }

// Apply additive schema changes:
// - If new text fields were added, ALTER TABLE search ADD COLUMN <field>.
pub fn apply_schema_additive(conn: &Connection, old: &Schema, new: &Schema) -> Result<()> { unimplemented!() }
```

Mechanics for `verify_fts_columns`:

* Query: `PRAGMA table_info(search)` works on virtual tables; for FTS5 it returns columns.
* Compare exactly to schema text fields order rule.
* If mismatch, error: `"fts columns mismatch; run ministore index migrate"`.

---

[File] Name: crates/ministore/src/db/put.rs

```rust
use crate::{Result, Schema};
use rusqlite::{Connection, Transaction};
use serde_json::Value;

pub struct PutPrepared {
  pub path: String,
  pub data_json: String,
  pub text_cols: Vec<Option<String>>, // ordered to match FTS columns
  pub keyword_fields: Vec<(String, Vec<String>)>,
  pub number_fields: Vec<(String, Vec<f64>)>,
  pub date_fields: Vec<(String, Vec<i64>)>,
  pub bool_fields: Vec<(String, bool)>,
  pub present_fields: Vec<String>,
}

// Pre-validate and coerce doc according to schema.
// Produces normalized values ready to write to DB.
pub fn prepare_put(schema: &Schema, doc: Value) -> Result<PutPrepared> { unimplemented!() }

pub fn execute_put(tx: &Transaction, schema: &Schema, prep: PutPrepared, now_ms: i64) -> Result<()> { unimplemented!() }

// Internal helper: insert/update items table and return (item_id, created_at_ms)
pub fn upsert_item_row(tx: &Transaction, path: &str, data_json: &str, now_ms: i64) -> Result<(i64, i64)> { unimplemented!() }
```

Algorithm notes for `prepare_put`:

* Input doc must be object; must have `"path"` string.
* For each schema field:

  * If missing, ignore (no presence)
  * If present:

    * keyword:

      * accept string or array of strings
      * if multi=false and array length != 1 → error
      * normalize to Vec<String>
    * text:

      * accept string; if multi=true and array of strings, join with `\n` (v1 choice) or error (choose strict? pinned here: **strict error** unless you explicitly allow; for v1 keep **strict**)
    * number:

      * accept number; multi accepts array of numbers
    * date:

      * accept string; parse ISO date/datetime to epoch ms
      * multi accepts array of strings
    * bool:

      * accept bool
* `present_fields` includes fields that had any value (even empty string counts as present).

Algorithm notes for `execute_put`:

* Must do diff-based doc_freq adjustments:

  * Read old value_ids by `SELECT value_id FROM kw_postings WHERE item_id=?`
  * Delete old postings
  * Insert new postings, increment doc_freq only when a new posting is inserted
  * Decrement doc_freq for old value_ids that are no longer present
* Simpler but correct approach:

  * Read old set (HashSet<i64>)
  * Build new set during insertion
  * After insertion:

    * `old - new` ⇒ decrement doc_freq
    * `new - old` ⇒ increment doc_freq (only if posting inserted; but if dict reused, still counts as new for this item)

---

[File] Name: crates/ministore/src/db/delete.rs

```rust
use crate::Result;
use rusqlite::Transaction;

pub fn delete_by_item_id(tx: &Transaction, item_id: i64) -> Result<()> { unimplemented!() }
```

Pinned deletion order:

1. collect `value_id`s from kw_postings
2. decrement doc_freq for each
3. delete from kw_postings
4. delete field_number/date/bool
5. delete field_present
6. delete from search (rowid=item_id)
7. delete from items

---

[File] Name: crates/ministore/src/db/search.rs

```rust
use crate::{Result, Schema};
use crate::index::{RankMode, SearchOptions};
use crate::cursor::{CursorPayload, CursorMode};
use rusqlite::Connection;

#[derive(Debug, Clone)]
pub struct PlannedQuery {
  pub sql: String,
  pub params: Vec<rusqlite::types::Value>,
  pub explain_steps: Vec<String>,
  pub query_hash: String,
  pub rank: RankMode,
  pub cursor_mode: CursorMode,
}

// Compile a query string into an executable SQL statement using CTE set algebra.
pub fn plan_search(conn: &Connection, schema: &Schema, query: &str, opts: &SearchOptions) -> Result<PlannedQuery> { unimplemented!() }

// Execute the planned query and return (rows, next_cursor_payload?).
pub fn run_search(conn: &Connection, schema: &Schema, plan: PlannedQuery, limit: usize, after: Option<CursorPayload>) -> Result<(Vec<SearchRow>, Option<CursorPayload>)> { unimplemented!() }

#[derive(Debug, Clone)]
pub struct SearchRow {
  pub item_id: i64,
  pub path: String,
  pub data_json: String,
  pub created_at_ms: i64,
  pub updated_at_ms: i64,

  // ranking keys (only one variant used per plan)
  pub score: Option<f64>,
  pub rank_value_num: Option<f64>,
  pub rank_value_date: Option<i64>,
}
```

Pinned SQL structure emitted by `plan_search`:

```
WITH
  p1 AS (...),
  p2 AS (...),
  result AS (
    SELECT item_id FROM p1
    INTERSECT
    SELECT item_id FROM p2
    ...
  ),
  ranked AS (
    SELECT i.id AS item_id, i.path, i.data_json, i.created_at, i.updated_at,
           <score_expr or rank_value_expr> AS __rank
    FROM items i
    JOIN result r ON r.item_id = i.id
    <LEFT JOIN search s ON s.rowid = i.id if FTS is used for scoring>
    <LEFT JOIN (SELECT item_id, MAX(value) ...) if field ranking>
    WHERE <after_filter_if_any>
    ORDER BY <rank_order>
    LIMIT ?limit_plus_one
  )
SELECT * FROM ranked;
```

Notes:

* We always select `data_json` and parse in Rust for `--show`.
* We always fetch `limit + 1` rows to determine if there’s another page.

---

[File] Name: crates/ministore/src/db/migrate.rs

```rust
use crate::{Result, Schema};
use rusqlite::Connection;
use std::path::Path;

pub fn rebuild_into_new_db(old_path: &Path, new_path: &Path, new_schema: Schema) -> Result<()> { unimplemented!() }
```

Pinned rebuild algorithm:

1. open old db read-only
2. create new db with new_schema (same DDL + FTS)
3. stream all items:

   * `SELECT path, data_json, created_at, updated_at FROM items ORDER BY id`
4. for each row:

   * parse data_json to Value
   * call internal `execute_put_preserve_timestamps(tx, ...)` so created/updated remain identical
5. atomic replace strategy:

   * write to temp file `index.db.tmp`
   * fsync
   * rename original to backup, rename tmp to final (platform dependent)
   * delete backup on success

---

[File] Name: crates/ministore/src/query/mod.rs

```rust
pub mod ast;
pub mod lexer;
pub mod parser;
pub mod normalize;
pub mod planner;

pub use ast::*;
pub use parser::parse_query;
```

---

[File] Name: crates/ministore/src/query/ast.rs

```rust
#[derive(Debug, Clone)]
pub enum Expr {
  And(Box<Expr>, Box<Expr>),
  Or(Box<Expr>, Box<Expr>),
  Not(Box<Expr>),
  Pred(Predicate),
}

#[derive(Debug, Clone)]
pub enum Predicate {
  Has { field: String },

  // path special
  PathGlob { pattern: String },

  // keyword
  Keyword { field: String, pattern: String, kind: KeywordPatternKind },

  // text / FTS
  Text { field: Option<String>, fts: String }, // field None means "all text fields"

  // number
  NumberCmp { field: String, op: CmpOp, value: f64 },
  NumberRange { field: String, lo: f64, hi: f64 },

  // date
  DateCmpAbs { field: String, op: CmpOp, epoch_ms: i64 },
  DateRangeAbs { field: String, lo_ms: i64, hi_ms: i64 },
  DateCmpRel { field: String, op: CmpOp, amount: i64, unit: RelUnit }, // interpreted later

  // bool
  Bool { field: String, value: bool },
}

#[derive(Debug, Copy, Clone)]
pub enum CmpOp { Eq, Gt, Gte, Lt, Lte }

#[derive(Debug, Copy, Clone)]
pub enum RelUnit { H, D, W, M, Y }

#[derive(Debug, Copy, Clone)]
pub enum KeywordPatternKind { Exact, Prefix, Contains, Glob } // Glob includes '?' mixed with '*'
```

---

[File] Name: crates/ministore/src/query/lexer.rs

```rust
#[derive(Debug, Clone, PartialEq)]
pub enum Tok {
  Ident(String),
  String(String),
  Number(f64),

  Colon,
  And,
  Or,
  Not,
  LParen,
  RParen,

  Gt,
  Gte,
  Lt,
  Lte,
  DotDot,
}

pub fn lex(input: &str) -> crate::Result<Vec<Tok>> { unimplemented!() }
```

Pinned lexer rules:

* `& | ! ( ) :` are single tokens.
* `>= <=` recognized.
* `..` recognized.
* Quoted strings allow `\"` and `\\`.
* Bare words that include `*` or `?` are still `Ident(...)` (pattern parsing happens in parser).

---

[File] Name: crates/ministore/src/query/parser.rs

```rust
use crate::query::ast::Expr;

pub fn parse_query(input: &str) -> crate::Result<Expr> { unimplemented!() }
```

Pinned parser mechanics:

* Pratt parser or recursive descent with precedence.
* Supports:

  * `field:value`
  * `field:>5`
  * `field:1..10`
  * `field:<7d` (relative; parser keeps `DateCmpRel`)
  * bare terms:

    * word → `Predicate::Text { field: None, fts: word }`
    * `"phrase"` → `Predicate::Text { field: None, fts: "\"phrase\"" }` (escaped)
* bool shorthand:

  * `!archived` when token is `Not` followed by `Ident("archived")` with no `:` becomes `Bool(field="archived", value=false)`

---

[File] Name: crates/ministore/src/query/normalize.rs

```rust
use crate::query::ast::Expr;

// Normalization passes:
// - flatten nested And/Or to vectors (internal)
// - detect OR of exact keyword matches on same field → mark for IN optimization in planner
// - compute "positive anchor" presence
pub fn normalize(expr: Expr) -> crate::Result<Expr> { unimplemented!() }

pub fn has_positive_anchor(expr: &Expr) -> bool { unimplemented!() }
```

Anchor rules enforced here; planner assumes normalized.

---

[File] Name: crates/ministore/src/query/planner.rs

```rust
use crate::{Result, Schema};
use crate::query::ast::{Expr, Predicate, KeywordPatternKind, CmpOp, RelUnit};
use crate::index::RankMode;

#[derive(Debug, Clone)]
pub struct CompileOutput {
  pub ctes: Vec<Cte>,
  pub result_cte_name: String,
  pub fts_match: Option<String>, // full MATCH string if any
  pub params: Vec<rusqlite::types::Value>,
  pub explain_steps: Vec<String>,
  pub requires_fts_join: bool,
}

#[derive(Debug, Clone)]
pub struct Cte {
  pub name: String,
  pub sql: String, // "name AS ( ... )"
}

pub fn compile_to_ctes(schema: &Schema, expr: Expr, now_ms: i64) -> Result<CompileOutput> { unimplemented!() }

// Build final SELECT given compiled CTEs and rank mode, including cursor after-filter.
pub fn build_search_sql(schema: &Schema, compiled: CompileOutput, rank: &RankMode, limit_plus_one: usize, after_filter: Option<String>) -> Result<(String, Vec<rusqlite::types::Value>)> { unimplemented!() }
```

Pinned predicate → CTE compilation:

* `Has { field }`:

  ```sql
  SELECT item_id FROM field_present WHERE field = ?
  ```

* `Keyword Exact`:

  ```sql
  SELECT p.item_id
  FROM kw_dict d
  JOIN kw_postings p ON p.value_id = d.id
  WHERE d.field = ? AND d.value = ?
  ```

* `Keyword Prefix`:

  ```sql
  ... WHERE d.field = ? AND d.value LIKE ?  -- "mem%"
  ```

* `Keyword Contains`:

  ```sql
  ... WHERE d.field = ? AND d.value LIKE ?  -- "%alloc%"
  ```

* `Keyword Glob` (contains `?` or internal `*`):

  * Use dictionary candidate with LIKE if possible:

    * if pattern starts with literal prefix before first wildcard → use `LIKE 'prefix%'`
    * then post-filter in Rust OR via SQL `GLOB` on `d.value`:

      ```sql
      WHERE d.field = ? AND d.value LIKE 'prefix%' AND d.value GLOB ?
      ```

* `PathGlob`:

  * If endswith `*` and no other wildcard: `items.path LIKE 'prefix%'`
  * Else candidate prefix required:

    ```sql
    WITH cand AS (SELECT id AS item_id FROM items WHERE path LIKE 'prefix%')
    SELECT c.item_id FROM cand c JOIN items i ON i.id=c.item_id WHERE i.path GLOB ?
    ```

* `Text`:

  ```sql
  SELECT rowid AS item_id FROM search WHERE search MATCH ?
  ```

  Where MATCH is built as:

  * If field-scoped: `title:term`
  * If bare/all: `(title:term OR content:term OR ...)`
  * AND/OR between bare terms are handled by set algebra outside, so each text predicate yields its own MATCH CTE (simpler and predictable).

* `NumberCmp`:

  ```sql
  SELECT item_id FROM field_number WHERE field=? AND value <op> ?
  ```

* `NumberRange`:

  ```sql
  SELECT item_id FROM field_number WHERE field=? AND value BETWEEN ? AND ?
  ```

* `DateCmpAbs` / `DateRangeAbs`: same shape with `field_date.value` (INTEGER)

* `DateCmpRel`:

  * Resolve in planner using pinned rule:

    * If field is `created` or `updated`: compute `threshold = now - duration`

      * `<Nd` ⇒ `value >= threshold`
      * `>Nd` ⇒ `value <= threshold`
    * Else: compute `target = now + duration`

      * `<Nd` ⇒ `value < target`
      * `>Nd` ⇒ `value > target`
  * Then compile to `field_date` or `items.created_at/items.updated_at` depending on implicit field:

    * implicit created/updated compile against items table directly:

      ```sql
      SELECT id AS item_id FROM items WHERE created_at >= ?
      ```

---

[File] Name: crates/ministore/src/cursor.rs

```rust
use serde::{Deserialize, Serialize};
use crate::index::{RankMode, CursorMode};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CursorPayload {
  pub query_hash: String,
  pub rank: RankModeSer,
  pub position: CursorPosition,
  pub expires_at_ms: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag="mode", rename_all="lowercase")]
pub enum RankModeSer {
  Default,
  Recency,
  Field { field: String },
  None,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag="kind", rename_all="lowercase")]
pub enum CursorPosition {
  Recency { updated_at_ms: i64, path: String },
  FieldNum { field: String, value: f64, updated_at_ms: i64, path: String },
  FieldDate { field: String, value_ms: i64, updated_at_ms: i64, path: String },
  Fts { score: f64, item_id: i64 },
  None { item_id: i64 },
}

pub fn hash_query(schema_json: &str, query: &str, rank: &RankModeSer) -> String { unimplemented!() }
// sha256(schema_json + "\n" + query + "\n" + rank_json) hex

pub fn encode_full(payload: &CursorPayload) -> crate::Result<String> { unimplemented!() }
// base64url(no padding) of payload json

pub fn decode_full(token: &str) -> crate::Result<CursorPayload> { unimplemented!() }

pub fn make_short_handle() -> String { unimplemented!() } // base62(6-8 bytes)

pub fn is_short_cursor(s: &str) -> bool { s.starts_with("c:") }
pub fn strip_short_prefix(s: &str) -> Option<&str> { s.strip_prefix("c:") }
```

Short cursor resolution is in db layer:

* if `after` begins with `c:`:

  * look up handle in cursor_store, parse payload JSON
  * check expiry
  * check query_hash matches current query_hash (else error “cursor does not match query”)

---

[File] Name: crates/ministore/src/discover.rs

```rust
use crate::{Result, Schema};
use rusqlite::Connection;

// Values for a keyword field, optionally scoped by query (compiled to result CTE).
pub fn discover_keyword_values(conn: &Connection, schema: &Schema, field: &str, scoped_query: Option<&str>, top: usize) -> Result<Vec<(String, u64)>> { unimplemented!() }

// Overview of fields: count present, unique values (where applicable), example values.
pub fn discover_fields_overview(conn: &Connection, schema: &Schema) -> Result<Vec<serde_json::Value>> { unimplemented!() }
```

Pinned SQL for keyword values (scoped):

* If no scope query:

  ```sql
  SELECT d.value, d.doc_freq
  FROM kw_dict d
  WHERE d.field = ?
  ORDER BY d.doc_freq DESC, d.value ASC
  LIMIT ?
  ```
* If scope query present:

  ```sql
  WITH ... result AS (...)
  SELECT d.value, COUNT(DISTINCT p.item_id) AS cnt
  FROM kw_dict d
  JOIN kw_postings p ON p.value_id = d.id
  JOIN result r ON r.item_id = p.item_id
  WHERE d.field = ?
  GROUP BY d.id
  ORDER BY cnt DESC, d.value ASC
  LIMIT ?
  ```

Field overview row structure (JSON object per field):

```json
{
  "field": "tags",
  "type": "keyword",
  "count": 1000,
  "unique": 127,
  "examples": ["memory", "heap", "stack"]
}
```

---

[File] Name: crates/ministore/src/stats.rs

```rust
use crate::{Result, Schema};
use rusqlite::Connection;

pub fn stats_for_field(conn: &Connection, schema: &Schema, field: &str, scoped_query: Option<&str>) -> Result<serde_json::Value> { unimplemented!() }
```

Pinned behaviors:

* number field:

  * compute min/max/avg
  * compute median using ordered values and count:

    * `lo = value at offset (n-1)/2`
    * `hi = value at offset n/2`
    * median = (lo+hi)/2
* date field:

  * compute earliest/latest (min/max) and count
* implicit fields `created` / `updated` supported as date stats by reading `items.created_at/updated_at`.

---

[File] Name: crates/ministore/src/batch.rs

```rust
use crate::{Result, Schema};
use serde_json::Value;

pub struct Batch<'a> {
  // holds a transaction internally; hidden from public API for bindings friendliness
  _private: std::marker::PhantomData<&'a ()>,
}

impl<'a> Batch<'a> {
  pub fn put_json(&mut self, doc: Value) -> Result<()> { unimplemented!() }
  pub fn delete(&mut self, path: &str) -> Result<bool> { unimplemented!() }
  pub fn commit(self) -> Result<()> { unimplemented!() }
  pub fn rollback(self) -> Result<()> { unimplemented!() }
}
```

Pinned rule:

* `Index::batch()` (method not shown earlier; add it if desired) returns `Batch` with a single transaction.
* `commit()` required; if dropped without commit, transaction rolls back.

(If you want “commit on drop”, flip the rule; but it’s riskier for bindings.)

---

# CLI crate design

---

[File] Name: crates/ministore-cli/Cargo.toml

```toml
[package]
name = "ministore-cli"
version = "0.1.0"
edition = "2021"

[[bin]]
name = "ministore"
path = "src/main.rs"

[dependencies]
ministore = { path = "../ministore" }
clap = { version = "4.5", features = ["derive"] }
serde_json = "1"
```

---

[File] Name: crates/ministore-cli/src/main.rs

```rust
use clap::{Parser, Subcommand};

mod output;
mod resolve;
mod commands;

#[derive(Parser)]
#[command(name="ministore")]
#[command(about="single-file search index", long_about=None)]
struct Cli {
  #[command(subcommand)]
  cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
  Index { #[command(subcommand)] cmd: commands::index::IndexCmd },
  Put(commands::put::PutArgs),
  Get(commands::get::GetArgs),
  Peek(commands::peek::PeekArgs),
  Delete(commands::delete::DeleteArgs),
  Search(commands::search::SearchArgs),
  Discover { #[command(subcommand)] cmd: commands::discover::DiscoverCmd },
  Stats(commands::stats::StatsArgs),
}

fn main() {
  // Parse args, dispatch, print errors as single line to stderr, exit 1 on error.
  unimplemented!()
}
```

---

[File] Name: crates/ministore-cli/src/resolve.rs

```rust
use std::path::PathBuf;

pub fn resolve_index_path(index: &str) -> PathBuf { unimplemented!() }
// rules described in "Index file resolution"
```

---

[File] Name: crates/ministore-cli/src/output.rs

```rust
use serde_json::Value;

#[derive(Debug, Clone)]
pub enum OutputFormat { Pretty, Paths, Json }

pub fn print_search_results(fmt: OutputFormat, items: Vec<Value>, next: Option<String>, show_rank: bool) { unimplemented!() }
pub fn print_json(v: &Value) { unimplemented!() }
pub fn print_error(e: &ministore::MinistoreError) { unimplemented!() }
```

Pinned output rules:

* `pretty`:

  * `[n] <path>` always
  * if rank mode is recency or field:<f>, print `f: value` on same line
  * if `--show` provided, print indented lines below
  * print `next: <cursor>` at end if present
* `paths`:

  * one path per line
  * cursor printed as a final line: `next: ...` (consistent)
* `json`:

  ```json
  { "results": [...], "next_cursor": "..." }
  ```

---

[File] Name: crates/ministore-cli/src/commands/mod.rs

```rust
pub mod index;
pub mod put;
pub mod get;
pub mod peek;
pub mod delete;
pub mod search;
pub mod discover;
pub mod stats;
```

---

[File] Name: crates/ministore-cli/src/commands/index.rs

```rust
use clap::{Args, Subcommand};

#[derive(Subcommand)]
pub enum IndexCmd {
  Create(CreateArgs),
  List(ListArgs),
  Schema(SchemaArgs),
  Migrate(MigrateArgs),
  Stats(IndexStatsArgs),
  Optimize(OptimizeArgs),
  Drop(DropArgs),
}

#[derive(Args)]
pub struct CreateArgs {
  #[arg(short, long)]
  pub index: String,

  #[arg(long)]
  pub schema: Option<std::path::PathBuf>,

  #[arg(long="field")]
  pub fields: Vec<String>, // "name:type" or "name:type:multi"
}

#[derive(Args)]
pub struct ListArgs {}

#[derive(Args)]
pub struct SchemaArgs {
  #[arg(short, long)]
  pub index: String,
  #[arg(long)]
  pub apply: Option<std::path::PathBuf>,
}

#[derive(Args)]
pub struct MigrateArgs {
  #[arg(short, long)]
  pub index: String,
}

#[derive(Args)]
pub struct IndexStatsArgs {
  #[arg(short, long)]
  pub index: String,
}

#[derive(Args)]
pub struct OptimizeArgs {
  #[arg(short, long)]
  pub index: String,
}

#[derive(Args)]
pub struct DropArgs {
  #[arg(short, long)]
  pub index: String,
}
```

Pinned behavior:

* `index create`:

  * if `--schema` given: read JSON → Schema
  * else if `--field` entries given: build Schema:

    * `title:text` or `tags:keyword:multi`
* `index schema --apply`:

  * additive only; otherwise error “type change requires migrate”
* `index migrate`:

  * rebuild file in-place using temp file and replace
* `index list`:

  * scan `*.db`, open meta, print valid indexes only

---

[File] Name: crates/ministore-cli/src/commands/put.rs

```rust
use clap::Args;

#[derive(Args)]
pub struct PutArgs {
  #[arg(short, long)]
  pub index: String,

  #[arg(short, long)]
  pub path: Option<String>,

  #[arg(long="set")]
  pub sets: Vec<String>, // "k=v"

  #[arg(long)]
  pub json: bool,

  #[arg(long)]
  pub import: Option<std::path::PathBuf>,
}
```

Pinned behavior:

* `--json`: read stdin fully, parse JSON object, pass to `Index::put_json`
* `--import`: read file as JSONL, each line is JSON object, run in batch transaction
* Otherwise:

  * `-p` required
  * build JSON object with `"path"` and each `--set`:

    * if value contains commas and field is keyword multi, CLI still passes string; library decides type.
      (Optionally CLI can split on commas only for keyword multi; v1 simplest: do split for any `k=v1,v2,v3` into JSON array of strings if there’s a comma.)
  * call `put_json`

---

[File] Name: crates/ministore-cli/src/commands/get.rs

```rust
use clap::Args;

#[derive(Args)]
pub struct GetArgs {
  #[arg(short, long)]
  pub index: String,
  #[arg(short, long)]
  pub path: String,
  #[arg(long)]
  pub format: Option<String>, // pretty|json
}
```

---

[File] Name: crates/ministore-cli/src/commands/peek.rs

```rust
use clap::Args;

#[derive(Args)]
pub struct PeekArgs {
  #[arg(short, long)]
  pub index: String,
  #[arg(short, long)]
  pub path: String,
}
```

---

[File] Name: crates/ministore-cli/src/commands/delete.rs

```rust
use clap::Args;

#[derive(Args)]
pub struct DeleteArgs {
  #[arg(short, long)]
  pub index: String,

  #[arg(short, long)]
  pub path: Option<String>,

  #[arg(short='w', long="where")]
  pub where_q: Option<String>,
}
```

Pinned: exactly one of `--path` or `--where` must be provided.

---

[File] Name: crates/ministore-cli/src/commands/search.rs

```rust
use clap::Args;

#[derive(Args)]
pub struct SearchArgs {
  #[arg(short, long)]
  pub index: String,

  #[arg(short='w', long="where")]
  pub where_q: String,

  #[arg(long)]
  pub limit: usize,

  #[arg(long)]
  pub after: Option<String>,

  #[arg(long, default_value="short")]
  pub cursor: String, // short|full

  #[arg(long, default_value="default")]
  pub rank: String, // default|recency|none|field:<name>

  #[arg(long)]
  pub show: Option<String>, // "all" or "f1,f2"

  #[arg(long, default_value="pretty")]
  pub format: String, // pretty|paths|json

  #[arg(long)]
  pub explain: bool,
}
```

---

[File] Name: crates/ministore-cli/src/commands/discover.rs

```rust
use clap::{Args, Subcommand};

#[derive(Subcommand)]
pub enum DiscoverCmd {
  Fields(FieldsArgs),
  Values(ValuesArgs),
}

#[derive(Args)]
pub struct FieldsArgs {
  #[arg(short, long)]
  pub index: String,
}

#[derive(Args)]
pub struct ValuesArgs {
  #[arg(short, long)]
  pub index: String,

  #[arg(long)]
  pub field: String,

  #[arg(short='w', long="where")]
  pub where_q: Option<String>,

  #[arg(long, default_value_t=100)]
  pub top: usize,
}
```

---

[File] Name: crates/ministore-cli/src/commands/stats.rs

```rust
use clap::Args;

#[derive(Args)]
pub struct StatsArgs {
  #[arg(short, long)]
  pub index: String,

  #[arg(long)]
  pub field: String,

  #[arg(short='w', long="where")]
  pub where_q: Option<String>,
}
```

---

# Hashing, explain, and determinism

### Query hash

Computed as sha256 hex over:

* schema_json string as stored
* raw query string (exact bytes)
* rank mode serialized

Purpose:

* cursor safety across schema changes and different queries

### Explain mode

If `SearchOptions.explain == true`:

* return:

  * `explain_sql`: full SQL (WITH CTEs + final SELECT) with placeholders
  * `explain_steps`: list of steps; planner populates:

    * predicate summaries
    * dictionary expansion counts where cheap:

      * prefix expansion count: `SELECT COUNT(*) FROM kw_dict WHERE field=? AND value LIKE ?`
      * if > `MAX_PREFIX_EXPANSION`, planner rejects query with message

---

# Binding-readiness decisions (future Python/Go)

* Public API stays serde_json-based.
* No rusqlite types leak to public.
* No lifetimes in public return structs.
* Search returns fully materialized JSON values.
* Cursor payload is JSON serializable and stable.

If/when you add FFI:

* Add a third crate `ministore-ffi` that wraps `Index` in opaque pointers and exposes C ABI.
* Python/Go bindings can sit on top.

---

# Expansion script (turn this doc into a skeleton repo)

This script scans for lines of the form:

* `[File] Name: path/to/file`
  and then writes the **first fenced code block** following that header into the file.

If a file section has no code block, it writes a placeholder comment.

[File] Name: tools/expand_design_doc.py

````python
#!/usr/bin/env python3
import os, re, sys
from pathlib import Path

HEADER_RE = re.compile(r'^\[File\]\s+Name:\s+(.+?)\s*$')
FENCE_RE = re.compile(r'^```([a-zA-Z0-9_-]+)?\s*$')

def main():
    if len(sys.argv) != 3:
        print("usage: expand_design_doc.py <design_doc.md> <out_dir>", file=sys.stderr)
        sys.exit(2)

    design_path = Path(sys.argv[1])
    out_dir = Path(sys.argv[2])
    text = design_path.read_text(encoding="utf-8").splitlines()

    i = 0
    files_written = 0
    while i < len(text):
        m = HEADER_RE.match(text[i])
        if not m:
            i += 1
            continue

        rel_path = m.group(1).strip()
        i += 1

        # find first code fence after header
        while i < len(text) and not FENCE_RE.match(text[i]):
            # stop early if next file header begins (no contents)
            if HEADER_RE.match(text[i]):
                break
            i += 1

        content = None
        if i < len(text) and FENCE_RE.match(text[i]):
            i += 1
            buf = []
            while i < len(text) and not text[i].startswith("```"):
                buf.append(text[i])
                i += 1
            # consume closing fence if present
            if i < len(text) and text[i].startswith("```"):
                i += 1
            content = "\n".join(buf) + ("\n" if buf else "")

        out_path = out_dir / rel_path
        out_path.parent.mkdir(parents=True, exist_ok=True)
        if content is None:
            # placeholder
            out_path.write_text("// TODO: skeleton generated; add contents in design doc\n", encoding="utf-8")
        else:
            out_path.write_text(content, encoding="utf-8")

        files_written += 1

    print(f"wrote {files_written} files into {out_dir}")

if __name__ == "__main__":
    main()
````

---