use std::io::{BufRead, BufReader, Seek, SeekFrom, Write};
use std::path::PathBuf;

use clap::{Args, Subcommand, ValueEnum};
use ministore::{Index, IndexOptions, MinistoreError, Result};
use ministore_okf::{projection_schema, sync, validate_bundle, SyncOptions, ValidateOptions};

use crate::resolve::resolve_index_path;

#[derive(Subcommand)]
pub enum OkfCmd {
    Validate(ValidateArgs),
    Sync(SyncArgs),
}

#[derive(Clone, Copy, ValueEnum)]
pub enum Format {
    Pretty,
    Json,
}

#[derive(Args)]
pub struct ValidateArgs {
    #[arg(long)]
    pub bundle: PathBuf,
    #[arg(long)]
    pub strict: bool,
    #[arg(long, value_enum, default_value = "pretty")]
    pub format: Format,
}

#[derive(Args)]
pub struct SyncArgs {
    #[arg(long)]
    pub bundle: PathBuf,
    #[arg(short, long)]
    pub index: String,
    #[arg(long)]
    pub strict: bool,
    #[arg(long)]
    pub dry_run: bool,
    #[arg(long, value_enum, default_value = "pretty")]
    pub format: Format,
}

pub fn run(command: OkfCmd) -> Result<()> {
    match command {
        OkfCmd::Validate(args) => validate(args),
        OkfCmd::Sync(args) => synchronize(args),
    }
}

fn validate(args: ValidateArgs) -> Result<()> {
    let json_output = matches!(args.format, Format::Json);
    let mut spool = tempfile::tempfile()?;
    let summary = validate_bundle(&args.bundle, &ValidateOptions::default(), |f| {
        if json_output {
            serde_json::to_writer(&mut spool, &f)?;
            spool.write_all(b"\n")?;
        } else {
            println!("{}", serde_json::to_string(&f)?);
        }
        Ok(())
    })
    .map_err(okf_error)?;
    let ok = summary.errors == 0 && (!args.strict || summary.warnings == 0);
    if json_output {
        spool.seek(SeekFrom::Start(0))?;
        print!("{{\"findings\":[");
        let mut reader = BufReader::new(spool);
        let mut line = String::new();
        let mut first = true;
        loop {
            line.clear();
            if reader.read_line(&mut line)? == 0 {
                break;
            }
            if !first {
                print!(",")
            }
            first = false;
            print!("{}", line.trim_end_matches('\n'));
        }
        println!(
            "],\"ok\":{},\"validation\":{}}}",
            ok,
            serde_json::to_string(&summary)?
        );
    } else {
        print(
            args.format,
            &serde_json::json!({"ok":ok,"validation":summary}),
        );
    }
    if ok {
        Ok(())
    } else {
        Err(MinistoreError::InvalidArgument(
            "OKF validation failed".into(),
        ))
    }
}

fn synchronize(args: SyncArgs) -> Result<()> {
    let path = resolve_index_path(&args.index)?;
    let mut temporary = None;
    let index = if path.exists() {
        Index::open(&path, IndexOptions::default())?
    } else if args.dry_run {
        let directory = tempfile::tempdir()?;
        let index = Index::create(
            directory.path().join("index.db"),
            projection_schema(),
            IndexOptions::default(),
        )?;
        temporary = Some(directory);
        index
    } else {
        let summary = validate_bundle(&args.bundle, &ValidateOptions::default(), |_| Ok(()))
            .map_err(okf_error)?;
        if summary.errors > 0 || (args.strict && summary.warnings > 0) {
            return Err(MinistoreError::InvalidArgument(
                "OKF validation failed".into(),
            ));
        }
        Index::create(&path, projection_schema(), IndexOptions::default())?
    };
    let report = sync(
        &args.bundle,
        &index,
        &SyncOptions {
            target_version: None,
            strict: args.strict,
            dry_run: args.dry_run,
        },
    )
    .map_err(okf_error)?;
    print(args.format, &report);
    drop(temporary);
    if report.ok {
        Ok(())
    } else {
        Err(MinistoreError::InvalidArgument(
            "OKF synchronization blocked by validation".into(),
        ))
    }
}

fn print(format: Format, value: &impl serde::Serialize) {
    match format {
        Format::Json => println!("{}", serde_json::to_string(value).unwrap()),
        Format::Pretty => println!("{}", serde_json::to_string_pretty(value).unwrap()),
    }
}
fn okf_error(error: ministore_okf::OkfError) -> MinistoreError {
    MinistoreError::Internal(error.to_string())
}
