# Open Knowledge Format user guide

MiniStore can validate an Open Knowledge Format (OKF) v0.2 bundle and synchronize
its concepts into a searchable MiniStore index. The source Markdown remains the
authority. The index is a deterministic projection that can be rebuilt whenever
the bundle changes.

This guide covers the Go and Rust implementations. Their OKF validation,
projections, hashes, SQLite indexes, JSON reports, and query behavior are
compatible. The Go CLI additionally supports PostgreSQL targets.

## What MiniStore does

MiniStore's OKF support:

- parses YAML frontmatter without flattening unknown extension fields;
- preserves each concept's exact UTF-8 source, including line endings and a BOM;
- validates base conformance and the optional v0.2 metadata families;
- validates root and nested `index.md` files and `log.md` files;
- resolves local Markdown links into forward links and backlinks;
- derives trust, lifecycle, provenance, and computation fields;
- creates a fixed, searchable MiniStore projection;
- synchronizes additions, edits, backlink-only changes, renames, and deletions in
  one target transaction; and
- produces compatible results in Go and Rust.

MiniStore does not fetch external resources, execute computations, run executors
or attesters, or treat trust metadata as authorization. Those values are validated
and indexed for discovery only.

## Bundle basics

An OKF bundle is a directory. Every non-reserved `.md` file is a concept. The two
reserved filenames are:

- `index.md`: a human-readable bundle or directory index;
- `log.md`: a dated change log.

Other regular files are ancillary resources. Symbolic links and special files are
not followed.

A minimal concept contains YAML frontmatter with a non-empty string `type`:

```markdown
---
type: Reference
title: Refund policy
tags: [support, policy]
---
# Refund policy

Refunds are available within 30 days.
```

Example layout:

```text
knowledge/
├── index.md
├── log.md
├── policies/
│   ├── index.md
│   └── refunds.md
└── runbooks/
    └── issue-refund.md
```

The file `policies/refunds.md` becomes the MiniStore item
`/policies/refunds`. Concept IDs are bundle-relative POSIX paths without the
`.md` suffix.

## Quick start

Validate before creating or opening an index:

```bash
ministore okf validate --bundle ./knowledge
```

Synchronize into SQLite:

```bash
ministore okf sync \
  --bundle ./knowledge \
  --index ./knowledge.db
```

Inspect what a later synchronization would change:

```bash
ministore okf sync \
  --bundle ./knowledge \
  --index ./knowledge.db \
  --dry-run \
  --format json
```

Search the resulting index through the normal MiniStore interface:

```bash
ministore search -i ./knowledge.db \
  -w 'type:"BigQuery Table" AND tags:finance' \
  --show title,status,trust_tier
```

## Validate a bundle

```text
ministore okf validate --bundle DIR [--strict] [--format pretty|json]
```

Validation does not open, create, or modify an index. It reads concepts one at a
time and emits findings in deterministic path and source order.

An error means the bundle is not base-conformant. A warning identifies an
optional-family, compatibility, graph, or style issue. Warnings do not fail the
command unless `--strict` is present.

```bash
# Normal validation: warnings are reported, exit status remains zero.
ministore okf validate --bundle ./knowledge

# CI validation: warnings also produce exit status 1.
ministore okf validate --bundle ./knowledge --strict --format json
```

Exit status is `0` when the selected policy passes and `1` for validation,
usage, filesystem, staging, schema, or database failure.

### JSON validation report

`--format json` emits one object containing the findings and summary:

```json
{
  "findings": [
    {
      "severity": "warning",
      "code": "OKF400",
      "path": "runbooks/issue-refund.md",
      "line": 8,
      "column": 5,
      "spec_section": "4.1",
      "message": "local link target does not exist: ../policies/missing.md"
    }
  ],
  "ok": true,
  "validation": {
    "target_version": "0.2",
    "declared_version": "0.2",
    "bundle": "/absolute/path/to/knowledge",
    "concepts": 14,
    "errors": 0,
    "warnings": 1
  }
}
```

The `ok` value includes strict-mode policy. The summary's error and warning counts
always describe the bundle itself.

### Finding codes

Finding codes are stable. Messages and parser-specific source locations may become
more precise without changing a code's meaning.

| Codes | Meaning |
|---|---|
| `OKF100`–`OKF108` | UTF-8, frontmatter delimiters, YAML, required `type`, duplicate keys, and BOM handling |
| `OKF200`–`OKF207` | Reserved indexes/logs, path collisions, ignored special files, and version declarations |
| `OKF300`–`OKF304` | Sources, source IDs, usage windows, and credibility fields |
| `OKF310`–`OKF315` | Generation, verification, actor syntax, and verification chronology |
| `OKF320`–`OKF341` | Lifecycle, freshness, tags, and v0.1 compatibility metadata |
| `OKF350`–`OKF360` | Attested Computation contracts and source footnotes |
| `OKF400`–`OKF402` | Missing links, root escapes, and unsafe percent encoding |

The public Go `FindingCode` constants and Rust `FindingCode` enum contain the full
catalog.

## Synchronize a bundle

SQLite in Go or Rust:

```text
ministore okf sync --bundle DIR --index INDEX
                   [--strict] [--dry-run] [--format pretty|json]
```

PostgreSQL with the Go CLI:

```bash
ministore okf sync \
  --bundle ./knowledge \
  --index 'postgres://user:password@localhost/knowledge?sslmode=require' \
  --backend postgres \
  --schema-name okf_catalog
```

Synchronization performs these operations:

```mermaid
flowchart LR
    B["OKF bundle"] --> V["Parse and validate one file at a time"]
    V --> S["Private temporary SQLite stage"]
    S --> G["Resolve graph and classify changes"]
    G --> T["One atomic MiniStore transaction"]
```

1. Enumerate and validate the bundle without following symlinks.
2. Store bundle-wide state in a private temporary SQLite database.
3. Resolve links and backlinks after every path is known.
4. Compare projection hashes with the target.
5. Classify each path as added, updated, unchanged, or deleted.
6. Apply all changes in one target transaction.

An unchanged item is not rewritten, so its MiniStore creation and update timestamps
are preserved. A rename is one addition and one deletion. A changed link can update
another concept because backlinks are part of its projection hash.

### Dedicated-index rule

The selected index must be dedicated to one OKF bundle. Synchronization makes its
paths exactly match that bundle and deletes target paths that no longer have a
concept.

If the target is missing, the CLI creates it with the canonical OKF schema. If it
exists with any other schema, synchronization refuses it. MiniStore does not embed
a bundle identity in the index, so selecting a compatible index for the wrong
bundle replaces its contents. Always use a distinct index path or PostgreSQL schema
for each bundle.

### Dry run

`--dry-run` validates, resolves the graph, compares an existing target when
present, and returns the predicted counts. It does not create a missing target or
write to an existing target.

Against a missing target, every valid concept is reported as an addition.

### Strict synchronization

Without `--strict`, validation errors block writes but warnings do not. With
`--strict`, any warning also blocks writes. A blocked synchronization returns a
report with `ok: false` and leaves the target unchanged.

### JSON synchronization report

```json
{
  "ok": true,
  "bundle": "/absolute/path/to/knowledge",
  "projection_version": 1,
  "concepts": 14,
  "added": 2,
  "updated": 3,
  "unchanged": 8,
  "deleted": 1,
  "duration_ms": 94,
  "validation": {
    "target_version": "0.2",
    "declared_version": "0.2",
    "bundle": "/absolute/path/to/knowledge",
    "concepts": 14,
    "errors": 0,
    "warnings": 0
  }
}
```

## Searchable projection

The original document is available as `raw_document`. Recognized values are also
projected into these fixed fields:

| Field | Type | Meaning |
|---|---|---|
| `type` | keyword | Required OKF concept type |
| `title` | text | `title`, or the filename stem when absent |
| `description` | text | Short description |
| `body` | text | Exact Markdown body after frontmatter |
| `tags` | keyword[] | Normalized tags |
| `status` | keyword | Lifecycle status; defaults to `stable` |
| `resource` | keyword | Primary external or local resource |
| `generated_by` | keyword | Generator actor |
| `generated_at` | date | Generation time or legacy `timestamp` fallback |
| `verified_by` | keyword[] | Actors from valid verification events |
| `latest_verified_at` | date | Latest valid verification time |
| `trust_tier` | keyword | Derived trust tier |
| `stale_after` | date | Caller-selected freshness boundary |
| `source_ids` | keyword[] | Valid `sources[].id` values |
| `source_resources` | keyword[] | Valid `sources[].resource` values |
| `source_authors` | keyword[] | Valid `sources[].author` actors |
| `source_last_modified` | date[] | Valid source modification dates |
| `source_usage_counts` | number[] | Valid non-negative source usage counts |
| `runtime` | keyword | Attested Computation runtime |
| `link_targets` | keyword[] | Resolved forward concept paths |
| `backlinks` | keyword[] | Concepts linking to this concept |
| `okf_version` | keyword | Effective OKF version |
| `okf_source_path` | keyword | Bundle-relative source filename |
| `okf_projection_hash` | keyword | Deterministic SHA-256 projection hash |
| `okf_projection_version` | number | Projection contract version, currently `1` |

`path` and `raw_document` are stored but not declared as separately indexed
fields. Empty optional fields are omitted. Unknown YAML keys and unsupported
extension structures remain intact in `raw_document`.

### Trust derivation

| Valid verification events | `trust_tier` |
|---|---|
| None | `unverified` |
| At least one, no `human:*` actor | `machine-confirmed` |
| At least one `human:*` actor | `human-reviewed` |

Trust is descriptive metadata, not access control.

### Query examples

```bash
# Find stable finance concepts.
ministore search -i knowledge.db \
  -w 'status:stable AND tags:finance' \
  --show title,type,trust_tier

# Find human-reviewed concepts due for review by a chosen date.
ministore search -i knowledge.db \
  -w 'trust_tier:human-reviewed AND stale_after:<=2026-07-31' \
  --show title,stale_after

# Find concepts derived from a particular source.
ministore search -i knowledge.db \
  -w 'source_resources:"https://example.com/policy"' \
  --show title,source_authors

# Find everything that links to a concept.
ministore search -i knowledge.db \
  -w 'backlinks:"/metrics/revenue"' \
  --show title,type

# Retrieve the authoritative source document.
ministore get -i knowledge.db --path /policies/refunds
```

`stale_after` is stored rather than an `is_stale` boolean, so the caller chooses
the comparison date and results do not become silently outdated.

## Links and backlinks

MiniStore parses CommonMark links rather than scanning Markdown with regular
expressions. Inline and reference-style links participate. Images and destinations
inside code do not.

For a concept at `runbooks/issue-refund.md`:

```markdown
See the [refund policy](../policies/refunds.md#exceptions).
```

the indexed target is `/policies/refunds`. Queries and fragments do not affect
the concept edge. Absolute bundle paths start with `/`; relative paths start from
the source concept's directory.

External URIs and fragment-only anchors do not create graph edges. Existing
ancillary files satisfy local path resolution but are not concepts. Missing local
paths, root escapes, malformed percent escapes, and encoded separators produce
warnings. Broken links never create edges.

## Metadata behavior

### Lifecycle and compatibility

Missing `status` becomes `stable`. The standard values are `draft`, `stable`, and
`deprecated`; unknown values are retained and warned. A valid `stale_after` date is
indexed for caller-relative freshness queries.

The v0.2 compatibility fallbacks recognize legacy `timestamp` and `citations`
metadata and report warnings. A bundle declaring a newer unknown version receives
a warning and a best-effort projection of recognized fields.

### Provenance

Each structurally valid `sources` entry can contribute an ID, resource, author,
last-modified date, usage count, and usage window. A source without `resource`
warns but its other valid values can still be projected. Footnote definitions are
checked against source IDs.

Parallel projected arrays are intended for search; they do not preserve the
association among values in one source entry. Read `raw_document` when that
association matters.

### Attested Computations

For `type: Attested Computation`, validation covers `runtime`, `parameters`, the
choice between a local computation file and an inline `# Computation` fenced code
block, executor receipts, attester shape, and local resource existence.

MiniStore indexes `runtime` and preserves the rest in `raw_document`. It never
executes or fetches computation content.

## Library usage

The CLI creates a missing index. The library synchronization functions require an
already open index whose schema exactly equals the canonical OKF projection schema.

### Go

```go
package main

import (
    "context"
    "fmt"

    "github.com/ministore/ministore/ministore"
    "github.com/ministore/ministore/ministore/storage/sqlite"
    "github.com/ministore/ministore/okf"
)

func main() {
    ctx := context.Background()

    // Validate and stream findings without collecting the whole report in RAM.
    summary, err := okf.ValidateBundle(
        ctx,
        "./knowledge",
        okf.ValidateOptions{},
        func(finding okf.Finding) error {
            fmt.Printf("%s %s: %s\n", finding.Severity, finding.Path, finding.Message)
            return nil
        },
    )
    if err != nil {
        panic(err)
    }
    if !summary.OK() {
        panic("bundle is not conformant")
    }

    ix, err := ministore.Create(
        ctx,
        sqlite.New("knowledge.db"),
        okf.ProjectionSchema(),
        ministore.DefaultIndexOptions(),
    )
    if err != nil {
        panic(err)
    }
    defer ix.Close()

    report, err := okf.Sync(ctx, "./knowledge", ix, okf.SyncOptions{})
    if err != nil {
        panic(err)
    }
    fmt.Printf("added=%d updated=%d deleted=%d\n",
        report.Added, report.Updated, report.Deleted)
}
```

Use `ministore.Open` instead of `ministore.Create` for an existing target. Go's
`context.Context` cancels filesystem, staging, and database work.

`okf.WalkProjections` streams canonical projections in source-path order when an
application needs the projection without synchronization.

### Rust

Add the OKF crate alongside `ministore` in the workspace or application:

```toml
[dependencies]
ministore = { path = "../ministore-rust/crates/ministore" }
ministore-okf = { path = "../ministore-rust/crates/ministore-okf" }
```

```rust
use std::path::Path;

use ministore::{Index, IndexOptions};
use ministore_okf::{
    projection_schema, sync, validate_bundle, SyncOptions, ValidateOptions,
};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let bundle = Path::new("./knowledge");

    let summary = validate_bundle(bundle, &ValidateOptions::default(), |finding| {
        println!("{:?} {}: {}", finding.severity, finding.path, finding.message);
        Ok(())
    })?;
    if !summary.ok() {
        return Err("bundle is not conformant".into());
    }

    let index = Index::create(
        "knowledge.db",
        projection_schema(),
        IndexOptions::default(),
    )?;

    let report = sync(bundle, &index, &SyncOptions::default())?;
    println!(
        "added={} updated={} deleted={}",
        report.added, report.updated, report.deleted
    );
    Ok(())
}
```

Use `Index::open` for an existing target. `walk_projections` streams projections
without returning a bundle-sized collection.

## Storage, memory, and failure behavior

Validation and synchronization create a private SQLite staging database in the
standard operating-system temporary directory. The stage contains source text,
normalized metadata, graph edges, findings, existing paths, and planned actions.
On supported platforms its directory and database are owner-only.

Set the standard temporary-directory environment variable before starting the
process when another disk should hold staging data:

```bash
TMPDIR=/mnt/secure-fast-temp \
  ministore okf sync --bundle ./knowledge --index ./knowledge.db
```

The stage is removed after success and handled failure. An abruptly killed process
can leave a private temporary directory; normal operating-system temporary-file
cleanup policy should remove such orphans. Treat the temporary filesystem as
sensitive because `raw_document` may contain secrets.

There are no built-in file-size, bundle-size, document-count, YAML-depth,
finding-count, or link-count acceptance limits. Capacity is governed by available
temporary disk, target storage, and the ability to materialize the largest single
concept and its adjacency lists in memory. The complete bundle, graph, finding
set, projection set, and operation set are not retained in application RAM.

Filesystem, staging, disk-full, validation, or projection failure occurs before
the target write transaction. During apply, any write failure rolls the whole
target transaction back. A successful run exposes the complete new state; it does
not deliberately expose a partially synchronized state.

Concurrent synchronizations against one target are unsupported. Serialize sync
jobs per index. A bundle can change while being read; the next synchronization
converges the index to the later filesystem state.

For PostgreSQL, synchronization can hold a long transaction and generate
substantial WAL for a large change set. Provision WAL and temporary storage,
monitor replicas, and schedule large synchronizations accordingly. Search can
continue while staging; database transaction semantics govern readers during the
final apply.

## Troubleshooting

### The target schema is incompatible

An OKF target must use exactly the schema returned by `ProjectionSchema()` or
`projection_schema()`. Choose a new dedicated index, or delete and rebuild the old
dedicated OKF index. Do not point synchronization at a generic MiniStore index.

### Synchronization wants to delete unexpected paths

The target is probably associated with another bundle. Stop, select the correct
dedicated index, and rerun with `--dry-run` before writing.

### Strict mode blocks an otherwise usable bundle

Run validation without `--strict` and inspect warning codes. Correct the warnings,
or use normal mode when advisory findings are acceptable. Validation errors always
block synchronization.

### Temporary disk fills up

Remove abandoned `ministore-okf-*` directories only after confirming no sync is
running, then select a temporary filesystem with enough space through `TMPDIR` or
the platform equivalent. The existing target remains unchanged when staging
fails.

### A broken link does not appear in `link_targets`

Only resolved links to concept `.md` files create graph edges. Check the finding
for its original destination. Links to images, code, external URIs, fragments, and
ancillary files intentionally do not become concept edges.

## Version compatibility

The current target and projection contract is OKF v0.2, projection version `1`.
An omitted root declaration is interpreted using the v0.2 target. A syntactically
invalid or newer declaration warns and uses best-effort recognized-field behavior.

Changing indexed field names, derivation rules, or canonical hash input requires a
new projection version and a rebuild.
