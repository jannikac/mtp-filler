use anyhow::Result;
use mtp_filler::{BackendEvent, BackendWrite};

use crate::gui::run_gui;

mod gui;
mod shared;

fn main() -> Result<()> {
    run_gui()?;
    Ok(())
}
