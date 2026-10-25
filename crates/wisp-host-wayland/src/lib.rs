//! Linux desktop presentation. The GPU program remains in wisp-runtime.
pub use wisp_host::{Assignment, Config, Options};
#[cfg(target_os = "linux")]
mod host;
#[cfg(target_os = "linux")]
pub use host::run;
