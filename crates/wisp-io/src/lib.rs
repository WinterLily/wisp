//! Bounded input for regular files shared by package loaders and authoring tools.
use anyhow::{Context, Result, ensure};
use std::{
    fs::{File, OpenOptions},
    io::{self, Read},
    path::Path,
};

/// Reject special files and read at most `limit` bytes, checking growth during the read.
pub fn read_bounded(path: &Path, limit: u64) -> Result<Vec<u8>> {
    read_bounded_with(path, limit, open_nonblocking)
}

fn open_nonblocking(path: &Path) -> io::Result<File> {
    let mut options = OpenOptions::new();
    options.read(true);
    // A regular path can be replaced by a FIFO after metadata inspection. The opened
    // handle must be checked without waiting for a writer on that replacement.
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NONBLOCK | libc::O_NOCTTY);
    }
    options.open(path)
}

fn read_bounded_with(
    path: &Path,
    limit: u64,
    open: impl FnOnce(&Path) -> io::Result<File>,
) -> Result<Vec<u8>> {
    let metadata =
        std::fs::metadata(path).with_context(|| format!("could not inspect {}", path.display()))?;
    ensure!(
        metadata.is_file(),
        "expected a regular file: {}",
        path.display()
    );
    let file = open(path).with_context(|| format!("could not open {}", path.display()))?;
    let metadata = file.metadata()?;
    ensure!(
        metadata.is_file(),
        "expected a regular file: {}",
        path.display()
    );
    ensure!(
        metadata.len() <= limit,
        "{} exceeds its {limit} byte input limit",
        path.display()
    );
    let mut bytes = Vec::new();
    file.take(limit.saturating_add(1))
        .read_to_end(&mut bytes)
        .with_context(|| format!("could not read {}", path.display()))?;
    ensure!(
        bytes.len() as u64 <= limit,
        "{} exceeds its {limit} byte input limit",
        path.display()
    );
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn regular_files_obey_exact_zero_and_maximum_limits() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("input");
        fs::write(&path, b"abc").unwrap();
        assert_eq!(read_bounded(&path, 3).unwrap(), b"abc");
        assert_eq!(read_bounded(&path, u64::MAX).unwrap(), b"abc");
        assert!(read_bounded(&path, 2).is_err());
        assert!(read_bounded(&path, 0).is_err());
        fs::write(&path, []).unwrap();
        assert!(read_bounded(&path, 0).unwrap().is_empty());
        assert!(read_bounded(directory.path(), 100).is_err());
        assert!(read_bounded(&path.with_extension("missing"), 100).is_err());
    }

    #[test]
    fn growth_between_inspection_and_open_cannot_escape_the_limit() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("input");
        fs::write(&path, b"abc").unwrap();
        let error = read_bounded_with(&path, 3, |path| {
            fs::write(path, b"abcd")?;
            open_nonblocking(path)
        })
        .unwrap_err();
        assert!(error.to_string().contains("3 byte input limit"));
    }

    #[cfg(unix)]
    #[test]
    fn fifo_and_replacement_are_rejected_before_deadline() {
        use std::{
            process::Command,
            thread,
            time::{Duration, Instant},
        };
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("input");
        for scenario in ["fifo", "replacement"] {
            let mut child = Command::new(std::env::current_exe().unwrap())
                .args(["--exact", "tests::fifo_child", "--nocapture"])
                .env("WISP_IO_FIFO_PATH", &path)
                .env("WISP_IO_FIFO_SCENARIO", scenario)
                .spawn()
                .unwrap();
            let deadline = Instant::now() + Duration::from_secs(5);
            loop {
                if let Some(status) = child.try_wait().unwrap() {
                    assert!(status.success(), "FIFO child failed for {scenario}");
                    break;
                }
                if Instant::now() >= deadline {
                    child.kill().unwrap();
                    child.wait().unwrap();
                    panic!("FIFO read hung for {scenario}");
                }
                thread::sleep(Duration::from_millis(10));
            }
            fs::remove_file(&path).unwrap();
        }
    }

    #[cfg(unix)]
    #[test]
    fn fifo_child() {
        let Some(path) = std::env::var_os("WISP_IO_FIFO_PATH") else {
            return;
        };
        let path = Path::new(&path);
        let make_fifo = |path: &Path| {
            assert!(
                std::process::Command::new("mkfifo")
                    .arg(path)
                    .status()
                    .unwrap()
                    .success()
            );
        };
        let result = if std::env::var("WISP_IO_FIFO_SCENARIO").unwrap() == "replacement" {
            fs::write(path, b"regular").unwrap();
            read_bounded_with(path, 100, |path| {
                fs::remove_file(path)?;
                make_fifo(path);
                open_nonblocking(path)
            })
        } else {
            make_fifo(path);
            read_bounded(path, 100)
        };
        assert!(result.unwrap_err().to_string().contains("regular file"));
    }
}
