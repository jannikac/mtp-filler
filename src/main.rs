use anyhow::Result;

#[cfg(windows)]
mod windows;

#[cfg(unix)]
mod unix;

mod shared;

fn main() -> Result<()> {
    #[cfg(windows)]
    windows::run()?;
    #[cfg(unix)]
    unix::run()?;
    Ok(())
}
