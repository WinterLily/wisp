use clap::Parser;
use std::path::PathBuf;

#[derive(Parser)]
#[command(version, about = "Wisp wallpaper library and live preview")]
struct Args {
    /// Override the platform-specific Wisp data directory
    #[arg(long)]
    data_dir: Option<PathBuf>,
    /// Import a .wisp package or compiled project on startup
    #[arg(long)]
    import: Option<PathBuf>,
    /// Create from an image or reopen an Easy Designer project
    #[arg(long, conflicts_with = "import")]
    create: Option<PathBuf>,
    /// Open a visual graph project
    #[arg(long, conflicts_with_all = ["import", "create"])]
    graph: Option<PathBuf>,
    /// Validate the library and saved assignments without opening a window
    #[arg(long)]
    check: bool,
    /// Exit after this many live preview frames (for smoke tests)
    #[arg(long, conflicts_with = "check", value_parser = clap::value_parser!(u64).range(1..))]
    frames: Option<u64>,
    /// Save the application window to a new PNG and exit (also works without a wallpaper)
    #[arg(long, conflicts_with = "check")]
    screenshot: Option<PathBuf>,
    /// Minimum startup settling time for standalone screenshots
    #[arg(long, requires = "screenshot", default_value_t = 1000, value_parser = clap::value_parser!(u64).range(0..=30_000))]
    screenshot_delay_ms: u64,
    /// Run an opt-in JSON audit command sequence through the UI update handlers
    #[arg(long, conflicts_with = "check")]
    audit_commands: Option<PathBuf>,
}
fn main() -> anyhow::Result<()> {
    let args = Args::parse();
    wisp_app::run(wisp_app::LaunchOptions {
        data_dir: args.data_dir,
        import: args.import,
        create: args.create,
        graph: args.graph,
        check: args.check,
        frames: args.frames,
        screenshot: args.screenshot,
        screenshot_delay: std::time::Duration::from_millis(args.screenshot_delay_ms),
        audit_commands: args.audit_commands,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn screenshots_work_independently_and_frame_limits_remain_strict() {
        let args = Args::try_parse_from(["wisp", "--screenshot", "empty.png"]).unwrap();
        assert_eq!(args.frames, None);
        assert_eq!(args.screenshot_delay_ms, 1000);
        assert!(Args::try_parse_from(["wisp", "--frames", "0"]).is_err());
        assert!(Args::try_parse_from(["wisp", "--check", "--screenshot", "a.png"]).is_err());
        assert!(Args::try_parse_from(["wisp", "--screenshot-delay-ms", "10"]).is_err());
        assert!(Args::try_parse_from(["wisp", "--frames", "2", "--screenshot", "a.png"]).is_ok());
    }
}
