use clap::Args;
use ministore::{Result, Index, SearchOptions, RankMode, CursorMode, OutputFieldSelector, IndexOptions};
use crate::output::{OutputFormat, print_search_results};
use crate::resolve::resolve_index_path;
use std::time::Instant;

#[derive(Args)]
#[command(about = "Query syntax: field:value, \"text\", field>N, field:a..b")]
pub struct SearchArgs {
    /// Path to index file
    #[arg(short, long)]
    pub index: String,

    /// Query (e.g. "category:rust priority>5")
    #[arg(short='w', long="where")]
    pub where_q: String,

    /// Max results per page
    #[arg(long, default_value_t = 20)]
    pub limit: usize,

    /// Cursor for pagination
    #[arg(long)]
    pub after: Option<String>,

    /// Cursor mode: short|full
    #[arg(long, default_value="short")]
    pub cursor: String,

    /// Ranking: default|recency|none|field:<name>
    #[arg(long, default_value="default")]
    pub rank: String,

    /// Fields: "all" or "f1,f2"
    #[arg(long)]
    pub show: Option<String>,

    /// Output: pretty|paths|json
    #[arg(long, default_value="pretty")]
    pub format: String,

    /// Show query plan
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
