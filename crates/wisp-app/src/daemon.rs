//! The GUI and daemon communicate through atomic assignment files and bounded discovery.
use crate::library::{Library, lock_file};
use anyhow::{Context, Result, ensure};
use fs2::FileExt;
use std::{
    fs::OpenOptions,
    io::Read,
    path::PathBuf,
    process::{Command, Stdio},
    time::{Duration, Instant},
};
use wisp_host::MonitorInfo;

fn executable() -> Result<PathBuf> {
    let path =
        std::env::current_exe()?.with_file_name(if cfg!(windows) { "wispd.exe" } else { "wispd" });
    ensure!(
        path.is_file(),
        "wispd is missing beside Wisp; build or install both applications"
    );
    Ok(path)
}
fn command() -> Result<Command> {
    let mut command = Command::new(executable()?);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000); // CREATE_NO_WINDOW
    }
    command.stdin(Stdio::null());
    Ok(command)
}
pub fn monitors() -> Result<Vec<MonitorInfo>> {
    // Temp files avoid blocking on pipe capacity, even if discovery fails noisily.
    let out = tempfile::tempfile()?;
    let err = tempfile::tempfile()?;
    let mut child = command()?
        .args(["--list-outputs", "--json"])
        .stdout(out.try_clone()?)
        .stderr(err.try_clone()?)
        .spawn()?;
    let deadline = Instant::now() + Duration::from_secs(8);
    let status = loop {
        if let Some(status) = child.try_wait()? {
            break status;
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            anyhow::bail!("monitor discovery timed out");
        }
        std::thread::sleep(Duration::from_millis(25));
    };
    fn contents(mut file: std::fs::File) -> Result<String> {
        use std::io::{Seek, SeekFrom};
        ensure!(
            file.metadata()?.len() <= 65536,
            "monitor response is too large"
        );
        file.seek(SeekFrom::Start(0))?;
        let mut text = String::new();
        file.take(65537).read_to_string(&mut text)?;
        Ok(text)
    }
    ensure!(
        status.success(),
        "could not discover monitors: {}",
        contents(err)?
    );
    let monitors: Vec<MonitorInfo> =
        serde_json::from_str(&contents(out)?).context("invalid monitor response")?;
    ensure!(monitors.len() <= 64, "too many monitors");
    Ok(monitors)
}
pub fn running(library: &Library) -> Result<bool> {
    let file = lock_file(&library.root.join("daemon.lock"))?;
    match file.try_lock_exclusive() {
        Ok(()) => Ok(false),
        Err(error) if error.raw_os_error() == fs2::lock_contended_error().raw_os_error() => {
            Ok(true)
        }
        Err(error) => Err(error.into()),
    }
}
pub fn start(library: &Library) -> Result<()> {
    if running(library)? {
        return Ok(());
    }
    library.config()?.validate()?;
    if !library.config_path().exists() {
        library.save_config(&Default::default())?;
    }
    let log = OpenOptions::new()
        .create(true)
        .truncate(true)
        .write(true)
        .open(library.root.join("daemon.log"))?;
    let mut child = command()?
        .arg("--config")
        .arg(library.config_path())
        .arg("--lock-file")
        .arg(library.root.join("daemon.lock"))
        .stdout(Stdio::null())
        .stderr(log)
        .spawn()
        .context("could not start wallpaper daemon")?;
    // Keep a reaper in this process without tying the daemon's lifetime to the UI.
    std::thread::spawn(move || {
        let _ = child.wait();
    });
    let deadline = Instant::now() + Duration::from_secs(3);
    while Instant::now() < deadline {
        if running(library)? {
            return Ok(());
        }
        std::thread::sleep(Duration::from_millis(30));
    }
    anyhow::bail!(
        "wallpaper daemon did not start; see {}",
        library.root.join("daemon.log").display()
    )
}
