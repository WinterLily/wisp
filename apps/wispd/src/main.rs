use anyhow::Result;
use clap::Parser;
use std::path::PathBuf;

#[derive(Parser)]
#[command(
    version,
    about = "Wisp wallpaper daemon for Windows, Niri and Hyprland"
)]
struct Args {
    /// Capture system output audio (or the selected device)
    #[arg(long, conflicts_with = "config")]
    audio: bool,
    /// CPAL device ID; use --list-audio-devices to discover devices
    #[arg(long, requires = "audio")]
    audio_device: Option<String>,
    #[arg(long, default_value_t = 1.0, requires = "audio")]
    audio_gain: f32,
    /// List audio device IDs without opening a capture stream or display
    #[arg(long, conflicts_with_all = ["config", "package", "assign", "check", "frames", "list_outputs", "audio"])]
    list_audio_devices: bool,
    /// JSON monitor assignments; edited configs are reloaded automatically
    #[arg(long, conflicts_with_all = ["package", "assign"])]
    config: Option<PathBuf>,
    /// Default wallpaper for all monitors, including newly connected ones
    #[arg(long)]
    package: Option<PathBuf>,
    /// Assign a wallpaper to an output: --assign DP-1=wallpaper.wisp
    #[arg(long)]
    assign: Vec<String>,
    #[arg(long, default_value_t = 60, value_parser = clap::value_parser!(u32).range(1..=240))]
    fps: u32,
    /// Print output names and dimensions, then exit
    #[arg(long, conflicts_with_all = ["config", "package", "assign", "frames", "check"])]
    list_outputs: bool,
    /// Emit structured monitor information
    #[arg(long, requires = "list_outputs")]
    json: bool,
    /// Hold an exclusive lock for a managed daemon instance
    #[arg(long)]
    lock_file: Option<PathBuf>,
    /// Publish initialized readiness, accepted config generations and output state
    #[arg(long, conflicts_with_all = ["check", "list_outputs", "list_audio_devices"])]
    status_file: Option<PathBuf>,
    /// Exit gracefully when this file appears (for an owning test/session harness)
    #[arg(long, conflicts_with_all = ["check", "list_outputs", "list_audio_devices"])]
    stop_file: Option<PathBuf>,
    /// Validate all configured wallpapers without a display server or GPU
    #[arg(long, conflicts_with = "frames")]
    check: bool,
    /// Exit after every assigned output has presented this many frames
    #[arg(long, value_parser = clap::value_parser!(u64).range(1..))]
    frames: Option<u64>,
}

#[cfg(any(target_os = "linux", target_os = "windows"))]
fn main() -> Result<()> {
    use anyhow::{Context, ensure};
    use wisp_host::{Assignment, Config, Options};
    #[cfg(target_os = "linux")]
    use wisp_host_wayland::run;
    #[cfg(target_os = "windows")]
    use wisp_host_windows::run;
    let args = Args::parse();
    if args.list_audio_devices {
        for (id, name) in wisp_audio::devices()? {
            println!("{id}\t{name}");
        }
        return Ok(());
    }
    let config = if let Some(path) = &args.config {
        Config::load(path)?
    } else {
        let mut config = Config {
            audio: wisp_audio::Config {
                enabled: args.audio,
                device: args.audio_device,
                gain: args.audio_gain,
            },
            default: args.package.map(|p| Assignment::new(p, args.fps)),
            ..Default::default()
        };
        for value in args.assign {
            let (name, path) = value
                .split_once('=')
                .context("assignment must be OUTPUT=PACKAGE")?;
            ensure!(
                !name.is_empty() && !path.is_empty(),
                "assignment requires output and package"
            );
            ensure!(
                config
                    .outputs
                    .insert(name.into(), Assignment::new(path.into(), args.fps))
                    .is_none(),
                "duplicate output assignment: {name}"
            );
        }
        config
    };
    ensure!(
        args.list_outputs
            || args.config.is_some()
            || config.default.is_some()
            || !config.outputs.is_empty(),
        "provide --package, --assign or --config (use --list-outputs to discover monitors)"
    );
    config.validate()?;
    if args.check {
        println!("Valid wallpaper assignments");
        return Ok(());
    }
    let _instance_lock = if let Some(path) = args.lock_file {
        use fs2::FileExt;
        let file = std::fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(path)?;
        file.try_lock_exclusive()
            .context("a managed wallpaper daemon is already running")?;
        // Refresh ownership before status publication so a previous process's snapshot is stale.
        use std::io::Write;
        file.set_len(0)?;
        writeln!(&file, "{}", std::process::id())?;
        Some(file)
    } else {
        None
    };
    let result = run(Options {
        config,
        config_path: args.config,
        list_outputs: args.list_outputs,
        json: args.json,
        frames: args.frames,
        status_path: args.status_file,
        stop_path: args.stop_file.clone(),
    });
    if let Some(path) = args.stop_file {
        match std::fs::remove_file(path) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
    }
    result
}

#[cfg(not(any(target_os = "linux", target_os = "windows")))]
fn main() -> Result<()> {
    let _ = Args::parse();
    anyhow::bail!("wispd supports Windows and Linux Niri/Hyprland")
}
