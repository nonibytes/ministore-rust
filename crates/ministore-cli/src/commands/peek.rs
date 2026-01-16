use clap::Args;
use ministore::{Result, Index, IndexOptions};
use crate::resolve::resolve_index_path;
use crate::output::print_json;

#[derive(Args)]
pub struct PeekArgs {
    #[arg(short, long)]
    pub index: String,
    #[arg(short, long)]
    pub path: String,
}

pub fn run(args: PeekArgs) -> Result<()> {
    let index_path = resolve_index_path(&args.index)?;
    let index = Index::open(&index_path, IndexOptions::default())?;
    
    let doc = index.peek(&args.path)?;
    print_json(&doc);
    
    Ok(())
}
