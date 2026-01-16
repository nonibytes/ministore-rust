use clap::Args;
use ministore::{Result, Index, IndexOptions};
use crate::resolve::resolve_index_path;

#[derive(Args)]
pub struct DeleteArgs {
    #[arg(short, long)]
    pub index: String,

    #[arg(short, long)]
    pub path: Option<String>,

    #[arg(short='w', long="where")]
    pub where_q: Option<String>,
}

pub fn run(args: DeleteArgs) -> Result<()> {
    let index_path = resolve_index_path(&args.index)?;
    let index = Index::open(&index_path, IndexOptions::default())?;
    
    if let Some(path) = &args.path {
        if index.delete(path)? {
            println!("Deleted {}", path);
        } else {
            println!("Not found: {}", path);
        }
    } else if let Some(query) = &args.where_q {
        let count = index.delete_where(query)?;
        println!("Deleted {} items", count);
    } else {
        eprintln!("Must provide --path or --where");
    }
    
    Ok(())
}
