# In-process library benchmark

This benchmark compares equivalent library calls. It does not launch a CLI in
the measured sections.

The four implementations are:

- native Rust using the `ministore` crate directly;
- Go calling Rust through the CGO-free `purego` binding;
- Go using `modernc.org/sqlite` (pure Go SQLite);
- Go using `mattn/go-sqlite3` through CGO.

## Run

The Go benchmark module expects the Go Ministore repository to be checked out
next to this repository as `../ministore`.

Generate the corpus once, then run:

```bash
./load/load_test.sh 100k
ITERATIONS=1000 ./benchmarks/library/run.sh
```

The harness reads the corpus before timing. Import time includes JSON decoding,
batch construction, and one transactional library import; index creation and
optimization are outside the timed section. Search time is the average of 1,000
sequential calls against an open, warmed index. Each implementation creates its
own database from the same schema and corpus and reuses its open database
connection or connection pool throughout the search phase.

## Results: 100k documents

| Implementation | Import | vs native Rust |
|---|---:|---:|
| Native Rust | 9.46s | 1.00x |
| Go → Rust (`purego`, CGO disabled) | 9.57s | 1.01x |
| Go + SQLite via CGO | 20.62s | 2.18x |
| Go + pure-Go SQLite (`modernc`) | 26.17s | 2.77x |

Average hot search latency:

| Query | Native Rust | Go → Rust (`purego`) | Go CGO | Go pure |
|---|---:|---:|---:|---:|
| FTS needle | 0.084ms | 0.111ms | 0.200ms | 0.354ms |
| Keyword exact | 0.054ms | 0.079ms | 0.062ms | 0.121ms |
| Number range | 0.046ms | 0.058ms | 0.054ms | 0.106ms |
| Complex | 0.081ms | 0.110ms | 0.140ms | 0.317ms |
| Broad, 100 results | 12.272ms | 13.301ms | 12.440ms | 19.411ms |

These results remove the connection-lifecycle mismatch from the previous run.
The Rust `Index` now retains its SQLite connection instead of opening one for
every operation, while Go retains its `database/sql` pool. Native Rust is the
fastest implementation on all five searches in this run, although the native
Rust and CGO results are effectively close on the keyword, number, and broad
queries.

The previous per-operation connection behavior cost much more than the query
itself for narrow searches: the four simple native Rust measurements fell from
roughly 0.48–0.60ms to 0.046–0.084ms. The Go → Rust path remains close to native
Rust. Its additional work includes the purego call and JSON serialization across
the ABI; that cost is most visible when returning 100 broad-query results.

The Rust index uses one mutex-protected persistent connection, so operations on
the same `Index` are thread-safe but serialized. This benchmark is deliberately
sequential and does not measure concurrent read scaling; a Rust connection pool
would be a separate optimization for that workload.
