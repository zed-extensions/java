#[cfg(unix)]
mod unix;
#[cfg(windows)]
mod windows;

#[cfg(unix)]
pub use unix::spawn_parent_monitor;
#[cfg(windows)]
pub use windows::spawn_parent_monitor;
