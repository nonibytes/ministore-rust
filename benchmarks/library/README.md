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
ITERATIONS=100 ./benchmarks/library/run.sh
```

The harness reads the corpus before timing. Import time includes JSON decoding,
batch construction, and one transactional library import; index creation and
optimization are outside the timed section. Search time is the average of 100
calls against an open, warmed index. Each implementation creates its own
database from the same schema and corpus.

## Results: 100k documents

| Implementation | Import | vs native Rust |
|---|---:|---:|
| Native Rust | 11.07s | 1.00x |
| Go → Rust (`purego`, CGO disabled) | 12.74s | 1.15x |
| Go + SQLite via CGO | 22.88s | 2.07x |
| Go + pure-Go SQLite (`modernc`) | 29.28s | 2.64x |

Average hot search latency:

| Query | Native Rust | Go → Rust (`purego`) | Go CGO | Go pure |
|---|---:|---:|---:|---:|
| FTS needle | 0.599ms | 0.738ms | 0.296ms | 0.484ms |
| Keyword exact | 0.487ms | 0.516ms | 0.083ms | 0.189ms |
| Number range | 0.482ms | 0.513ms | 0.079ms | 0.146ms |
| Complex | 0.602ms | 0.639ms | 0.228ms | 0.373ms |
| Broad, 100 results | 14.884ms | 14.897ms | 13.674ms | 20.178ms |

The Rust implementation opens a SQLite connection for each search, while the
Go implementation retains a `database/sql` pool. Consequently, these search
figures compare the current public library semantics, not only SQLite query
execution. The important binding result is the close match between native Rust
and Go → Rust: purego adds roughly 0.03–0.14ms on the simple calls and is lost
in the noise on the broad query.
