# Minisore Ops & Scaling TODOs

This document captures the actionable TODOs for making **ministore** (and the `ministore` CLI + potential service wrapper) viable for high-QPS, mostly-read workloads, with rare offline rebuilds and hot-swaps.

## Current decisions (assumed for this plan)

- Data updates are **rare** (e.g., weekly) and can be handled via **offline rebuild + hot swap**.
- Query traffic is **high**, mostly **read-only**.
- **Short cursor** tokens can be **mapped locally** (agent runtime / CLI wrapper), while services can prefer **full cursors** for portability.
- We want to preserve *all ministore features*: FTS-ish text search, keyword search with wildcards/guardrails, numeric/date filters, ranking modes, stable pagination, discover/stats.

---

## 1) Cursor strategy

### 1.1 Cursor policy per deployment mode
- [ ] Define official cursor policy:
  - **Local CLI / agent runtime**: allow `CursorMode::Short` via local mapping.
  - **Network service**: default to `CursorMode::Full` (portable; stateless).

### 1.2 Local short-cursor store (agent/CLI-side)
- [ ] Add a local short-cursor store abstraction (outside the DB file):
  - [ ] Map `handle -> CursorPosition` (including `hash`).
  - [ ] TTL eviction (default 1h; configurable).
  - [ ] Max-size cap + eviction policy (LRU or FIFO).
  - [ ] Concurrency safety (RwLock/Mutex).
- [ ] Cursor resolve validation:
  - [ ] If handle missing/expired: return “cursor expired; rerun query”.
  - [ ] If cursor hash mismatch: return “cursor does not match query/schema/rank”.

### 1.3 Dataset version invalidation
- [ ] Define a dataset/version identifier:
  - [ ] Option: DB file mtime + size.
  - [ ] Option: explicit version stored in `meta`.
  - [ ] Option: build id of the dataset release.
- [ ] Include dataset version in `hash_query()` (or in cursor payload hashing) so hot swaps reliably invalidate old cursors.

---

## 2) Offline rebuild + hot swap operations

### 2.1 Rebuild workflow
- [ ] Document a “weekly rebuild” workflow:
  - [ ] Build new DB offline (new path).
  - [ ] Validate (open + schema verification + sanity checks).
  - [ ] Atomically swap the active file pointer (symlink swap or config update).

### 2.2 Safe hot swap in a service process
- [ ] Service reload strategy:
  - [ ] File watcher or periodic polling for new “active DB” pointer.
  - [ ] Keep old DB handle around until in-flight queries complete (grace period).
  - [ ] Ensure new requests use the new DB version.
- [ ] Expose dataset version in diagnostics and logs.

---

## 3) SQLite performance & concurrency posture

### 3.1 Read-only service mode
- [ ] Open DB in **read-only** mode (SQLite URI `mode=ro` when available).
- [ ] Enforce “no writes” in service mode (cursor store moved elsewhere or local-only).

### 3.2 Connection management
- [ ] Decide strategy:
  - [ ] Per-request connection (simpler; may be slower).
  - [ ] Connection pool (likely needed at high concurrency).
- [ ] Decide pool size heuristic (e.g., cores * N; benchmark-driven).

### 3.3 Pragmas & caching plan (service mode)
- [ ] Confirm pragmas for read-heavy workloads:
  - [ ] WAL mode (still commonly fine even if writers are rare/offline).
  - [ ] `PRAGMA mmap_size` (memory-map the DB file).
  - [ ] `PRAGMA cache_size` (page cache sizing).
  - [ ] `PRAGMA temp_store` (avoid disk temp where possible).
- [ ] Document defaults and how to tune based on RAM and dataset size.

### 3.4 Benchmark + load testing
- [ ] Add a benchmark harness:
  - [ ] Query types: FTS text, keyword exact/prefix/contains/glob, numeric/date filters, recency rank, field rank.
  - [ ] Pagination workload: multiple pages per query.
  - [ ] Dataset sizes: small/medium/large (define realistic corpus sizes).
- [ ] Run concurrency tests:
  - [ ] Measure p50/p95/p99 latency.
  - [ ] Measure sustained QPS vs tail latency under load.
  - [ ] Record CPU, RSS, page faults, IO, and lock contention.

---

## 4) Network service: “serve sqlite over the network”

### 4.1 Decide network model
- [ ] Choose a serving approach:
  - **Option A**: build a thin HTTP/gRPC wrapper around ministore (recommended for control).
  - **Option B**: adopt an existing “SQLite server” layer (evaluate constraints/limitations).

### 4.2 API design (if building your own)
- [ ] Define endpoints:
  - [ ] `GET /item?path=...` (get/peek)
  - [ ] `POST /search` (query + options)
  - [ ] `POST /discover/values`
  - [ ] `POST /discover/fields`
  - [ ] `POST /stats`
- [ ] Cursor contract:
  - [ ] Default `CursorMode::Full` for stateless paging.
  - [ ] (Optional) short cursors only if backed by a shared cursor store.

### 4.3 Safety & ops
- [ ] Add:
  - [ ] request timeouts + cancellation
  - [ ] rate limiting / quotas
  - [ ] structured logs (query, rank, limit, latency)
  - [ ] tracing spans per request
  - [ ] load shedding for expensive queries
  - [ ] health endpoints (liveness/readiness) and version endpoint

---

## 5) Correctness hardening for pagination

### 5.1 Confirm stable ordering per rank mode
- [ ] Verify cursor payload matches ORDER BY exactly:
  - Default + FTS: `ORDER BY score DESC, item_id ASC`
  - Default (no FTS): `ORDER BY updated_at DESC, path ASC`
  - Recency: `ORDER BY updated_at DESC, path ASC`
  - Field: `ORDER BY rank_value DESC, updated_at DESC, path ASC`
  - None: `ORDER BY item_id ASC`

### 5.2 Tie-breaking and duplicate prevention
- [ ] Ensure every ordering includes deterministic tie-breaks:
  - [ ] Prefer `item_id` or `path` consistently.
- [ ] Add tests:
  - [ ] many items with same score / same updated_at
  - [ ] ensure no duplicates or missing across pages

---

## 6) Guardrails & query cost control (service readiness)

- [ ] Re-evaluate positive anchor rules for production traffic:
  - [ ] ensure no “scan everything” queries
- [ ] Confirm wildcard constraints are sufficient:
  - [ ] `MIN_PREFIX_LEN`, `MIN_CONTAINS_LEN`, `MAX_PREFIX_EXPANSION`
- [ ] Add metrics:
  - [ ] rejected queries count
  - [ ] query plan complexity stats (CTE count, joins, etc.)
  - [ ] slow query logs sampling

---

## 7) Alternative backends roadmap (future, feature parity required)

### 7.1 Write the backend contract (requirements)
- [ ] Document backend capabilities needed for full parity:
  - [ ] Text search with ranking (BM25-ish or acceptable alternative)
  - [ ] Keyword inverted index with wildcard support and guardrails
  - [ ] Numeric/date indexing + comparisons + range queries
  - [ ] “discover” (top values) and “stats”
  - [ ] Stable pagination consistent with ranking order
  - [ ] Efficient query planning/execution under concurrency

### 7.2 Candidate backend tiers
- [ ] Tier 1 (closest parity):
  - [ ] Postgres: `tsvector` / `GIN` for text, plus relational indexes for keyword/number/date
  - [ ] Dedicated search engine (if acceptable): OpenSearch/Elasticsearch (would need mapping for discover/stats and exact semantics)
- [ ] Tier 2 (harder for parity):
  - [ ] Redis: requires custom index structures, persistence model, and careful memory planning

### 7.3 Architecture for pluggable backends
- [ ] Extract a `Backend` trait boundary:
  - [ ] `put/delete` (optional for service mode)
  - [ ] `search/get/peek/discover/stats`
- [ ] Separate:
  - [ ] parsing/normalization (backend-agnostic)
  - [ ] planning/execution (backend-specific)
- [ ] Define an intermediate query plan representation (IR) to target multiple engines.

---

## 8) Documentation deliverables

- [ ] `docs/deployment.md`:
  - [ ] CLI local
  - [ ] Local agent sidecar
  - [ ] Network service
- [ ] `docs/cursors.md`:
  - [ ] Full cursor portability
  - [ ] Short cursor local mapping
  - [ ] Cursor invalidation on dataset version change
- [ ] `docs/ops/hotswap.md`:
  - [ ] offline rebuild
  - [ ] atomic swap
  - [ ] safe reload / grace period
- [ ] `docs/perf/benchmarks.md`:
  - [ ] record benchmark methodology and results
  - [ ] recommended tuning knobs

---

## Open questions to resolve (capture as issues)

- [ ] Do we *ever* need server-side short cursors? If yes, where do we store them (Redis vs in-process vs DB)?
- [ ] What is the target dataset size and RAM budget for the service?
- [ ] What latency target do we need at 10k QPS (p95/p99)?
- [ ] Which ranking semantics must be exactly preserved if we move to Postgres or another backend?
