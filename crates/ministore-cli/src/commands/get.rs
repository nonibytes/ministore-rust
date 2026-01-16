use clap::Args;
use ministore::{Result, Index, IndexOptions};
use crate::resolve::resolve_index_path;
use crate::output::print_json; // Assuming we export it

#[derive(Args)]
pub struct GetArgs {
    #[arg(short, long)]
    pub index: String,
    #[arg(short, long)]
    pub path: String,
    #[arg(long)]
    pub format: Option<String>, // pretty|json
}

pub fn run(args: GetArgs) -> Result<()> {
    let index_path = resolve_index_path(&args.index)?;
    let index = Index::open(&index_path, IndexOptions::default())?;
    
    let view = index.get(&args.path)?;
    
    // Default format logic (json usually for get)
    // Or pretty print fields?
    // Let's use json by default for direct get to match peek but with metadata?
    // Implementation choice: print full doc as json.
    
    let doc = view.doc;
    // Maybe verify format arg if needed
    
    print_json(&doc);
    
    Ok(())
}
