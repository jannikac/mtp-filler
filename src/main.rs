use anyhow::Result;

mod generic;
mod shared;

fn main() -> Result<()> {
    generic::run()?;
    Ok(())
}
