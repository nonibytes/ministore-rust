use clap::{Parser, Subcommand};

mod commands;
mod output;
mod resolve;

#[derive(Parser)]
#[command(name = "ministore")]
#[command(
    about = "Single-file search index",
    after_help = "Use `ministore <COMMAND> --help` for command details."
)]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Manage indexes: create, optimize, apply-schema
    Index {
        #[command(subcommand)]
        cmd: commands::index::IndexCmd,
    },
    /// Insert/update docs (--path or --import JSONL)
    Put(commands::put::PutArgs),
    /// Get document by path (full JSON)
    Get(commands::get::GetArgs),
    /// Get document metadata only
    Peek(commands::peek::PeekArgs),
    /// Delete by path or query
    Delete(commands::delete::DeleteArgs),
    /// Query documents (returns matches)
    Search(commands::search::SearchArgs),
    /// Explore field values
    Discover {
        #[command(subcommand)]
        cmd: commands::discover::DiscoverCmd,
    },
    /// Compute min/max/avg for fields
    Stats(commands::stats::StatsArgs),
    /// Validate and synchronize OKF bundles
    Okf {
        #[command(subcommand)]
        cmd: commands::okf::OkfCmd,
    },
}

fn main() {
    let cli = Cli::parse();

    let result = match cli.cmd {
        Cmd::Index { cmd } => commands::index::run(cmd),
        Cmd::Put(args) => commands::put::run(args),
        Cmd::Get(args) => commands::get::run(args),
        Cmd::Peek(args) => commands::peek::run(args),
        Cmd::Delete(args) => commands::delete::run(args),
        Cmd::Search(args) => commands::search::run(args),
        Cmd::Discover { cmd } => commands::discover::run(cmd),
        Cmd::Stats(args) => commands::stats::run(args),
        Cmd::Okf { cmd } => commands::okf::run(cmd),
    };

    if let Err(e) = result {
        eprintln!("Error: {}", e);
        std::process::exit(1);
    }
}
