use clap::{Parser, Subcommand};

mod output;
mod resolve;
mod commands;

#[derive(Parser)]
#[command(name="ministore")]
#[command(about="single-file search index", long_about=None)]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    Index { #[command(subcommand)] cmd: commands::index::IndexCmd },
    Put(commands::put::PutArgs),
    Get(commands::get::GetArgs),
    Peek(commands::peek::PeekArgs),
    Delete(commands::delete::DeleteArgs),
    Search(commands::search::SearchArgs),
    Discover { #[command(subcommand)] cmd: commands::discover::DiscoverCmd },
    Stats(commands::stats::StatsArgs),
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
    };
    
    if let Err(e) = result {
        eprintln!("Error: {}", e);
        std::process::exit(1);
    }
}
