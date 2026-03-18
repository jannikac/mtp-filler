mod app;
mod backend;
mod cli;
mod shared;

use anyhow::Result;
use clap::{Parser, Subcommand};
use eframe::NativeOptions;

#[derive(Parser)]
#[command(name = "mtp-filler")]
struct Args {
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
    Cli,
}

fn main() -> Result<()> {
    let args = Args::parse();
    match args.command {
        Some(Command::Cli) => cli::run_cli()?,
        None => {
            let options = NativeOptions::default();
            eframe::run_native(
                "MTP Filler",
                options,
                Box::new(|_cc| Ok(Box::<app::MtpFillerApp>::default())),
            )
            .map_err(|e| anyhow::anyhow!(e.to_string()))?;
        }
    }

    Ok(())
}
