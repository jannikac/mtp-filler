// prevent terminal opening on windows when double click
#![cfg_attr(windows, windows_subsystem = "windows")]

use anyhow::Result;
use clap::{Parser, Subcommand};

mod cli;
mod gui;
mod shared;

pub use mtp_filler::{BackendEvent, BackendWrite};

#[derive(Parser)]
#[command(version, about)]
struct Args {
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
    #[command(about = "Run the terminal-based CLI instead of the GUI")]
    Cli,
}

fn main() -> Result<()> {
    let args = Args::parse();

    match args.command {
        Some(Command::Cli) => cli::run_cli(),
        None => gui::run_gui(),
    }
}
