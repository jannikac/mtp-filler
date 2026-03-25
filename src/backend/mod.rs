#[cfg(unix)]
mod unix;
#[cfg(windows)]
mod windows;

#[cfg(unix)]
pub use unix::AppState;
#[cfg(windows)]
pub use windows::AppState;

#[cfg(not(any(unix, windows)))]
compile_error!("mtp-filler supports Unix and Windows only");
