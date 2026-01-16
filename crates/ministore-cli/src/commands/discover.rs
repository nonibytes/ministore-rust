use clap::{Args, Subcommand};
use ministore::{Result, Index, IndexOptions};
use crate::resolve::resolve_index_path;
use crate::output::print_json;

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

pub fn run(cmd: DiscoverCmd) -> Result<()> {
    match cmd {
        DiscoverCmd::Fields(args) => {
            let index_path = resolve_index_path(&args.index)?;
            let index = Index::open(&index_path, IndexOptions::default())?;
            let fields = index.discover_fields()?;
            print_json(&serde_json::Value::Array(fields));
        }
        DiscoverCmd::Values(args) => {
            let index_path = resolve_index_path(&args.index)?;
            let index = Index::open(&index_path, IndexOptions::default())?;
            let values = index.discover_values(&args.field, args.where_q.as_deref(), args.top)?;
            
            // Format output as list or json?
            // Let's output JSON array of [value, count] tuples (as arrays)
            let arr: Vec<serde_json::Value> = values.into_iter()
                .map(|(val, count)| {
                    serde_json::json!({"value": val, "count": count})
                })
                .collect();
            print_json(&serde_json::Value::Array(arr));
        }
    }
    Ok(())
}
