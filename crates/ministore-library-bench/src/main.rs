use ministore::{
    Batch, CursorMode, Index, IndexOptions, OutputFieldSelector, Schema, SearchOptions,
};
use serde::Serialize;
use serde_json::Value;
use std::collections::BTreeMap;
use std::fs;
use std::hint::black_box;
use std::path::PathBuf;
use std::time::Instant;

#[derive(Serialize)]
struct Results {
    implementation: &'static str,
    documents: usize,
    import_ms: f64,
    searches_ms: BTreeMap<&'static str, f64>,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let schema_path = PathBuf::from(args.next().ok_or("missing schema path")?);
    let corpus_path = PathBuf::from(args.next().ok_or("missing corpus path")?);
    let database_path = PathBuf::from(args.next().ok_or("missing database path")?);
    let iterations: usize = args.next().unwrap_or_else(|| "100".to_string()).parse()?;

    let schema = Schema::from_json(&fs::read_to_string(schema_path)?)?;
    let lines: Vec<String> = fs::read_to_string(corpus_path)?
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(str::to_owned)
        .collect();
    let _ = fs::remove_file(&database_path);
    let index = Index::create(&database_path, schema, IndexOptions::default())?;

    let import_started = Instant::now();
    let mut batch = Batch::new();
    for line in &lines {
        batch.put_json(serde_json::from_str::<Value>(line)?)?;
    }
    let imported = index.batch(batch)?;
    let import_ms = import_started.elapsed().as_secs_f64() * 1000.0;
    index.optimize()?;

    let queries = [
        ("fts_needle", "NEEDLE_UNIQUE_XYZ_12345", 5),
        ("keyword", "category:needle", 5),
        ("number", "priority>900", 5),
        ("complex", "MAGIC_HAYSTACK_FINDER category:needle", 5),
        ("broad", "category:cat5", 100),
    ];
    let mut searches_ms = BTreeMap::new();
    for (name, query, limit) in queries {
        let options = SearchOptions {
            limit,
            cursor_mode: CursorMode::Full,
            show: OutputFieldSelector::None,
            ..SearchOptions::default()
        };
        for _ in 0..5 {
            black_box(index.search(query, options.clone())?);
        }
        let started = Instant::now();
        for _ in 0..iterations {
            black_box(index.search(query, options.clone())?);
        }
        searches_ms.insert(
            name,
            started.elapsed().as_secs_f64() * 1000.0 / iterations as f64,
        );
    }

    println!(
        "{}",
        serde_json::to_string_pretty(&Results {
            implementation: "rust-native",
            documents: imported,
            import_ms,
            searches_ms,
        })?
    );
    Ok(())
}
