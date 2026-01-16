use clap::Args;
use ministore::{Result, Index, IndexOptions};
use crate::resolve::resolve_index_path;
use crate::output::print_json;

#[derive(Args)]
pub struct StatsArgs {
    #[arg(short, long)]
    pub index: String,

    #[arg(long)]
    pub field: String,

    #[arg(short='w', long="where")]
    pub where_q: Option<String>,
}

pub fn run(args: StatsArgs) -> Result<()> {
    let index_path = resolve_index_path(&args.index)?;
    let index = Index::open(&index_path, IndexOptions::default())?;
    
    let stats = index.stats(&args.field, args.where_q.as_deref())?;
    print_json(&stats);
    
    Ok(())
}
