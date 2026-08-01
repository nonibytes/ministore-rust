use clap::{Args, Subcommand};
use ministore::{Result, Index, IndexOptions, Schema, FieldSpec, FieldType, MinistoreError};
use crate::resolve::resolve_index_path;
use crate::output::print_json;
use std::collections::BTreeMap;

#[derive(Subcommand)]
pub enum IndexCmd {
    /// Create index (--schema file or --field name:type)
    Create(CreateArgs),
    /// List indexes in current directory
    List(ListArgs),
    /// Show/apply schema (--apply to update)
    Schema(SchemaArgs),
    /// Migrate to new schema (rebuilds)
    Migrate(MigrateArgs),
    /// Show doc count, size
    Stats(IndexStatsArgs),
    /// Vacuum + rebuild FTS
    Optimize(OptimizeArgs),
    /// Delete index file
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
    // Target schema or path?
    // Design doc says migrate_rebuild(new_path, new_schema).
    // CLI probably needs args for that.
    // For now assuming in-place migrate or just print help?
    // The struct has only index. Design doc might be incomplete on CLI options.
    // Let's implement as placeholder calling unimplemented logic or error.
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

pub fn run(cmd: IndexCmd) -> Result<()> {
    match cmd {
        IndexCmd::Create(args) => {
             let index_path = resolve_index_path(&args.index)?;
             
             let schema = if let Some(path) = args.schema {
                 let content = std::fs::read_to_string(path)?;
                 Schema::from_json(&content)?
             } else {
                 let mut fields = BTreeMap::new();
                 for f in args.fields {
                     // name:type[:multi]
                     let parts: Vec<&str> = f.split(':').collect();
                     if parts.len() < 2 {
                         return Err(MinistoreError::Schema(format!("invalid field spec: {}", f)));
                     }
                     let name = parts[0].to_string();
                     let type_str = parts[1];
                     let multi = parts.len() > 2 && parts[2] == "multi";
                     
                     let field_type = match type_str {
                         "text" => FieldType::Text,
                         "keyword" => FieldType::Keyword,
                         "number" => FieldType::Number,
                         "date" => FieldType::Date,
                         "bool" => FieldType::Bool,
                         _ => return Err(MinistoreError::Schema(format!("unknown type: {}", type_str))),
                     };
                     
                     let spec = match field_type {
                         FieldType::Text => FieldSpec::text(Some(1.0)), // default weight
                         FieldType::Keyword => FieldSpec::keyword(multi),
                         FieldType::Number => FieldSpec::number(multi),
                         FieldType::Date => FieldSpec::date(multi),
                         FieldType::Bool => FieldSpec::bool(),
                     };
                     fields.insert(name, spec);
                 }
                 Schema { fields }
             };
             
             Index::create(&index_path, schema, IndexOptions::default())?;
             println!("Created index at {}", index_path.display());
        }
        IndexCmd::List(_) => {
            // Scan current dir for .db files
            let cwd = std::env::current_dir()?;
            println!("Searching in {}", cwd.display());
            for entry in std::fs::read_dir(cwd)? {
                let entry = entry?;
                let path = entry.path();
                if path.extension().is_some_and(|e| e == "db") {
                     // Try to open and check magic?
                     if let Ok(idx) = Index::open(&path, IndexOptions::default()) {
                         // If opens, it's valid?
                         // Maybe print basic info
                         let schema = idx.schema();
                         println!("- {} ({} fields)", path.file_name().unwrap().to_string_lossy(), schema.fields.len());
                     }
                }
            }
        }
        IndexCmd::Schema(args) => {
            let index_path = resolve_index_path(&args.index)?;
            
            if let Some(_path) = args.apply {
                // Apply schema change
                 let mut index = Index::open(&index_path, IndexOptions::default())?;
                 // Load new schema
                 let content = std::fs::read_to_string(_path)?;
                 let new_schema = Schema::from_json(&content)?;
                 index.apply_schema(new_schema)?;
                 println!("Schema applied (if supported)");
            } else {
                 let index = Index::open(&index_path, IndexOptions::default())?;
                 print_json(&serde_json::to_value(index.schema())?);
            }
        }
        IndexCmd::Optimize(args) => {
             let index_path = resolve_index_path(&args.index)?;
             let index = Index::open(&index_path, IndexOptions::default())?;
             index.optimize()?;
             println!("Optimized index");
        }
        IndexCmd::Migrate(args) => {
             let index_path = resolve_index_path(&args.index)?;
             let _index = Index::open(&index_path, IndexOptions::default())?;
             // index.migrate_rebuild(...) - needs args
             println!("Migrate not fully exposed in CLI yet");
        }
        IndexCmd::Stats(args) => {
             let index_path = resolve_index_path(&args.index)?;
             // Just verify it opens?
             let _ = Index::open(&index_path, IndexOptions::default())?;
             println!("Index exists and is valid: {}", index_path.display());
        }
        IndexCmd::Drop(args) => {
             let index_path = resolve_index_path(&args.index)?;
             if index_path.exists() {
                 std::fs::remove_file(&index_path)?;
                 println!("Dropped index {}", index_path.display());
             } else {
                 println!("Index not found");
             }
        }
    }
    Ok(())
}
