mod classifier;
mod db;
mod reporting;

use anyhow::Result;
use clap::Parser;
use std::path::PathBuf;

#[derive(Parser, Debug)]
#[command(author, version, about)]
struct Args {
    #[arg(long, default_value = "/Users/kooshapari/forge/.forge.db")]
    db_path: PathBuf,
    #[arg(long)]
    apply: bool,
    #[arg(long, default_value_t = 1)]
    tier: u8,
    #[arg(long)]
    yes: bool,
}

fn main() -> Result<()> {
    let args = Args::parse();
    reporting::run(&args.db_path, args.apply, args.tier, args.yes)
}

