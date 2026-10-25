//! The GUI and daemon communicate through atomic assignment files and bounded discovery.
use crate::library::{Library, lock_file};
use anyhow::{Context, Result, ensure};
use fs2::FileExt;
use std::{
    fs::OpenOptions,
    io::Read,
    path::PathBuf,
    process::{Child, Command, ExitStatus, Stdio},
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
    Ok(command_at(&executable()?))
}
fn command_at(path: &std::path::Path) -> Command {
    let mut command = Command::new(path);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000); // CREATE_NO_WINDOW
    }
    command.stdin(Stdio::null());
    command
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
pub fn status(library: &Library) -> Result<Option<wisp_host::status::DaemonStatus>> {
    if !running(library)? {
        return Ok(None);
    }
    let path = library.root.join("daemon.status.json");
    let metadata = match std::fs::metadata(&path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    let ownership = std::fs::metadata(library.root.join("daemon.lock"))?;
    // An acquired lock belongs to the new process before it can replace a stale ready snapshot.
    if metadata.modified()? < ownership.modified()? {
        return Ok(None);
    }
    match wisp_host::status::DaemonStatus::read(&path) {
        Ok(status) => Ok(Some(status)),
        Err(error)
            if error
                .downcast_ref::<std::io::Error>()
                .is_some_and(|error| error.kind() == std::io::ErrorKind::NotFound) =>
        {
            Ok(None)
        }
        Err(error) => Err(error),
    }
}

pub fn ready(library: &Library) -> Result<bool> {
    Ok(status(library)?.is_some_and(|status| status.running && status.ready))
}

fn spawn(
    library: &Library,
    stop_path: Option<&std::path::Path>,
    binary: &std::path::Path,
) -> Result<Child> {
    library.config()?.validate()?;
    if !library.config_path().exists() {
        library.save_config(&Default::default())?;
    }
    let log = OpenOptions::new()
        .create(true)
        .truncate(true)
        .write(true)
        .open(library.root.join("daemon.log"))?;
    let mut command = command_at(binary);
    command
        .arg("--config")
        .arg(library.config_path())
        .arg("--lock-file")
        .arg(library.root.join("daemon.lock"))
        .arg("--status-file")
        .arg(library.root.join("daemon.status.json"))
        .stdout(Stdio::null())
        .stderr(log);
    if let Some(path) = stop_path {
        command.arg("--stop-file").arg(path);
    }
    command.spawn().context("could not start wallpaper daemon")
}

fn wait_ready(library: &Library, mut child: Option<&mut Child>) -> Result<()> {
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        if let Some(child) = child.as_mut() {
            ensure!(
                child.try_wait()?.is_none(),
                "wallpaper daemon exited before initialization; see {}",
                library.root.join("daemon.log").display()
            );
        }
        if status(library)?.is_some_and(|status| {
            status.running
                && status.ready
                && child.as_ref().is_none_or(|child| child.id() == status.pid)
        }) {
            return Ok(());
        }
        ensure!(
            Instant::now() < deadline,
            "wallpaper daemon did not initialize; see {}",
            library.root.join("daemon.log").display()
        );
        std::thread::sleep(Duration::from_millis(30));
    }
}

pub fn start(library: &Library) -> Result<()> {
    if running(library)? {
        return wait_ready(library, None);
    }
    let mut child = spawn(library, None, &executable()?)?;
    let result = wait_ready(library, Some(&mut child));
    if result.is_err() {
        let _ = child.kill();
    }
    // A reaper prevents zombies while preserving the daemon when the GUI closes.
    std::thread::spawn(move || {
        let _ = child.wait();
    });
    result
}

/// Harnesses own their daemon's lifetime; normal GUI launches remain independent.
pub struct OwnedDaemon {
    child: Child,
    stop_path: PathBuf,
}
impl OwnedDaemon {
    pub fn start(library: &Library) -> Result<Self> {
        Self::start_with_executable(library, &executable()?)
    }
    pub fn start_with_executable(library: &Library, binary: &std::path::Path) -> Result<Self> {
        ensure!(!running(library)?, "cannot own an already running daemon");
        // Separate markers ensure a losing startup race cannot stop another owner's daemon.
        let marker = tempfile::Builder::new()
            .prefix("daemon-")
            .suffix(".stop")
            .tempfile_in(&library.root)?;
        let stop_path = marker.path().to_owned();
        marker.close()?;
        let mut daemon = Self {
            child: spawn(library, Some(&stop_path), binary)?,
            stop_path,
        };
        wait_ready(library, Some(&mut daemon.child))?;
        Ok(daemon)
    }
    pub fn try_wait(&mut self) -> Result<Option<ExitStatus>> {
        Ok(self.child.try_wait()?)
    }
    pub fn stop_path(&self) -> &std::path::Path {
        &self.stop_path
    }
    pub fn stop(&mut self) -> Result<ExitStatus> {
        if let Some(status) = self.child.try_wait()? {
            return Ok(status);
        }
        std::fs::write(&self.stop_path, [])?;
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            if let Some(status) = self.child.try_wait()? {
                return Ok(status);
            }
            if Instant::now() >= deadline {
                self.child.kill()?;
                return Ok(self.child.wait()?);
            }
            std::thread::sleep(Duration::from_millis(25));
        }
    }
}
impl Drop for OwnedDaemon {
    fn drop(&mut self) {
        if self.stop().is_err() {
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
        let _ = std::fs::remove_file(&self.stop_path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::daemon;
    use wisp_host::status::DaemonStatus;
    #[test]
    fn process_lock_alone_and_stale_status_do_not_imply_initialized_readiness() {
        use fs2::FileExt;
        let temporary = tempfile::tempdir().unwrap();
        let library = Library::open(Some(temporary.path().to_owned())).unwrap();
        let lock = crate::library::lock_file(&library.root.join("daemon.lock")).unwrap();
        lock.try_lock_exclusive().unwrap();
        assert!(daemon::running(&library).unwrap());
        assert!(!daemon::ready(&library).unwrap());
        let path = library.root.join("daemon.status.json");
        let mut status = DaemonStatus::starting(&Default::default());
        status.write(&path).unwrap();
        assert!(!daemon::ready(&library).unwrap());
        status.ready = true;
        status.write(&path).unwrap();
        assert!(daemon::ready(&library).unwrap());
        status.running = false;
        status.write(&path).unwrap();
        assert!(!daemon::ready(&library).unwrap());
        status.running = true;
        status.write(&path).unwrap();
        drop(lock);
        // A concurrent process spawn may inherit the descriptor until close-on-exec runs.
        let deadline = Instant::now() + Duration::from_secs(3);
        while daemon::running(&library).unwrap() {
            assert!(
                Instant::now() < deadline,
                "released daemon lock remained held"
            );
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(!daemon::ready(&library).unwrap());
        assert!(daemon::status(&library).unwrap().is_none());
    }

    #[test]
    fn owned_start_reports_early_exit_and_cleans_the_unique_stop_marker() {
        let temporary = tempfile::tempdir().unwrap();
        let library = Library::open(Some(temporary.path().to_owned())).unwrap();
        let started = Instant::now();
        // The test runner rejects daemon CLI flags, exercising startup failure on every platform.
        let result =
            OwnedDaemon::start_with_executable(&library, &std::env::current_exe().unwrap());
        let error = result.err().expect("test runner must reject daemon flags");
        assert!(
            error.to_string().contains("exited before initialization"),
            "{error:#}"
        );
        assert!(started.elapsed() < Duration::from_secs(3));
        assert!(!running(&library).unwrap());
        assert!(std::fs::read_dir(&library.root).unwrap().all(|entry| {
            !entry
                .unwrap()
                .file_name()
                .to_string_lossy()
                .ends_with(".stop")
        }));
    }
    #[test]
    fn a_new_lock_owner_cannot_reuse_the_previous_daemons_ready_snapshot() {
        let temporary = tempfile::tempdir().unwrap();
        let library = Library::open(Some(temporary.path().to_owned())).unwrap();
        let path = library.root.join("daemon.status.json");
        let mut snapshot = DaemonStatus::starting(&Default::default());
        snapshot.ready = true;
        snapshot.write(&path).unwrap();
        let published = std::fs::metadata(&path).unwrap().modified().unwrap();
        let lock = lock_file(&library.root.join("daemon.lock")).unwrap();
        lock.try_lock_exclusive().unwrap();
        lock.set_modified(published + Duration::from_secs(2))
            .unwrap();
        assert!(running(&library).unwrap());
        assert!(status(&library).unwrap().is_none());
        assert!(!ready(&library).unwrap());
        lock.set_modified(published - Duration::from_secs(2))
            .unwrap();
        assert!(ready(&library).unwrap());
    }
}
