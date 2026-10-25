//! Native Windows desktop presentation using the platform-independent Wisp runtime.
pub use wisp_host::{Assignment, Config, Options};

#[cfg(target_os = "windows")]
mod host;
#[cfg(any(target_os = "windows", test))]
mod monitor;
#[cfg(target_os = "windows")]
mod native;
#[cfg(target_os = "windows")]
pub use host::run;
