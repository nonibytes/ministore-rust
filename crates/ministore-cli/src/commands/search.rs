use clap::Args;
use ministore::{Result, Index, SearchOptions, RankMode, CursorMode, OutputFieldSelector, IndexOptions};
use crate::output::{OutputFormat, print_search_results};
use crate::resolve::resolve_index_path;
use std::time::Instant;

#[derive(Args)]
pub struct SearchArgs {
    #[arg(short, long)]
    pub index: String,

    #[arg(short='w', long="where")] // Renamed to query in help?
    pub where_q: String,

    #[arg(long, default_value_t = 20)]
    pub limit: usize,

    #[arg(long)]
    pub after: Option<String>,

    #[arg(long, default_value="full")] 
    pub cursor: String, // short|full - simplified to full default based on patch

    #[arg(long, default_value="default")]
    pub rank: String, // default|recency|none|field:<name>

    #[arg(long)]
    pub show: Option<String>, // "all" or "f1,f2"

    #[arg(long, default_value="pretty")]
    pub format: String, // pretty|paths|json

    #[arg(long)]
    pub explain: bool,
}

pub fn run(args: SearchArgs) -> Result<()> {
    let index_path = resolve_index_path(&args.index)?;
    let index = Index::open(&index_path, IndexOptions::default())?;
    
    // Parse rank
    let rank = match args.rank.as_str() {
        "default" => RankMode::Default,
        "recency" => RankMode::Recency,
        "none" => RankMode::None,
        s if s.starts_with("field:") => RankMode::Field(s[6..].to_string()),
        _ => RankMode::Default, // or error?
    };
    
    // Parse show
    let show = match args.show.as_deref() {
        Some("all") => OutputFieldSelector::All,
        Some(s) => OutputFieldSelector::Fields(s.split(',').map(|s| s.trim().to_string()).collect()),
        None => OutputFieldSelector::None,
    };
    
    // Parse format
    let format = args.format.parse::<OutputFormat>().unwrap_or(OutputFormat::Pretty);
    
    // Parse cursor mode
    let cursor_mode = match args.cursor.as_str() {
        "short" => CursorMode::Short,
        "full" => CursorMode::Full,
        _ => CursorMode::Full, // default
    };
    
    let opts = SearchOptions {
        limit: args.limit,
        after: args.after,
        cursor_mode,
        rank,
        show,
        explain: args.explain,
    };
    
    let start = Instant::now();
    let page = index.search(&args.where_q, opts)?;
    let duration = start.elapsed();
    
    print_search_results(format, &page, duration.as_millis());
    
    Ok(())
}
