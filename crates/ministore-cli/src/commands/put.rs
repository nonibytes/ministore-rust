use clap::Args;
use ministore::{Result, Index, IndexOptions, Batch};
use crate::resolve::resolve_index_path;
use serde_json::Value;
use std::io::Read;
use std::fs::File;

#[derive(Args)]
#[command(about = "Single doc: --path + --set. Batch: --import file.jsonl or --json (stdin)")]
pub struct PutArgs {
    /// Path to index file
    #[arg(short, long)]
    pub index: String,

    /// Document path (for single doc mode)
    #[arg(short, long)]
    pub path: Option<String>,

    /// Set field: key=value (repeatable)
    #[arg(long="set")]
    pub sets: Vec<String>,

    /// Read JSONL from stdin
    #[arg(long)]
    pub json: bool,

    /// Import JSONL file (one JSON object per line)
    #[arg(long)]
    pub import: Option<std::path::PathBuf>,
}

pub fn run(args: PutArgs) -> Result<()> {
    let index_path = resolve_index_path(&args.index)?;
    let index = Index::open(&index_path, IndexOptions::default())?;

    if let Some(path) = &args.path {
        // Path + sets mode
        let mut obj = serde_json::Map::new();
        for set in &args.sets {
            if let Some((k, v)) = set.split_once('=') {
                // Try to guess type? For now treat as string or bool/number if obvious?
                // V1 usually keeps simple. Strings are safe.
                // Schema coercion handles types.
                obj.insert(k.to_string(), Value::String(v.to_string()));
            } else {
                // key only? maybe bool true?
                obj.insert(set.to_string(), Value::Bool(true));
            }
        }
        index.put_fields(path, Value::Object(obj))?;
        println!("Put {}", path);
    } else {
        // Import mode (stdin or file)
        let reader: Box<dyn Read> = if let Some(path) = &args.import {
            Box::new(File::open(path)?)
        } else if args.json {
            Box::new(std::io::stdin())
        } else {
            return Err(ministore::MinistoreError::Io(std::io::Error::new(std::io::ErrorKind::InvalidInput, "Must provide --path or --json/--import")));
        };
        
        // Parse JSONL (line-by-line JSON)
        use std::io::BufRead;
        let mut batch = Batch::new();
        let buf_reader = std::io::BufReader::new(reader);
        for line in buf_reader.lines() {
            let line = line?;
            if line.trim().is_empty() {
                continue;
            }
            let val: Value = serde_json::from_str(&line)
                .map_err(|e| ministore::MinistoreError::Schema(format!("JSON parse error: {}", e)))?;
            batch.put_json(val)?;
        }
        let count = index.batch(batch)?;
        println!("Imported {} items", count);
    }

    Ok(())
}
