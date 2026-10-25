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
    /// Save the application window to a new PNG before the frame-limited exit
    #[arg(long, requires = "frames")]
    screenshot: Option<PathBuf>,
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
    })
}
