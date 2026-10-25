//! Versioned observations distinguish process ownership from initialized desktop presentation.
use crate::{Assignment, Config};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use std::{
    io::Write,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

pub fn next_output_identity() -> u64 {
    static NEXT_OUTPUT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
    NEXT_OUTPUT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct OutputStatus {
    pub id: String,
    pub instance_id: u64,
    pub assignment: Assignment,
    pub initialized: bool,
    pub presented: u64,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct DaemonStatus {
    pub schema_version: u32,
    pub pid: u32,
    pub running: bool,
    pub ready: bool,
    pub accepted_generation: u64,
    pub attempted_generation: u64,
    pub config: Config,
    pub outputs: Vec<OutputStatus>,
    pub rejection: Option<String>,
}
impl DaemonStatus {
    pub fn starting(config: &Config) -> Self {
        Self {
            schema_version: 1,
            pid: std::process::id(),
            running: true,
            ready: false,
            accepted_generation: 1,
            attempted_generation: 1,
            config: config.clone(),
            outputs: Vec::new(),
            rejection: None,
        }
    }
    pub fn read(path: &Path) -> Result<Self> {
        let bytes = wisp_io::read_bounded(path, 2 * 1024 * 1024)?;
        let status: Self = serde_json::from_slice(&bytes).context("invalid daemon status")?;
        ensure!(
            status.schema_version == 1,
            "unsupported daemon status version"
        );
        Ok(status)
    }
    pub fn write(&self, path: &Path) -> Result<()> {
        let parent = path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        let mut file = tempfile::NamedTempFile::new_in(parent)?;
        file.write_all(&serde_json::to_vec(self)?)?;
        file.as_file().sync_all()?;
        file.persist(path)
            .context("could not publish daemon status")?;
        Ok(())
    }
}

pub struct StatusReporter {
    path: Option<PathBuf>,
    pub status: DaemonStatus,
    next_publish: Instant,
}
impl StatusReporter {
    pub fn new(path: Option<PathBuf>, config: &Config) -> Result<Self> {
        let mut reporter = Self {
            path,
            status: DaemonStatus::starting(config),
            next_publish: Instant::now(),
        };
        reporter.publish()?;
        Ok(reporter)
    }
    pub fn reloaded(&mut self, result: &Result<()>, config: &Config) {
        self.status.attempted_generation += 1;
        match result {
            Ok(()) => {
                self.status.accepted_generation = self.status.attempted_generation;
                self.status.config = config.clone();
                self.status.rejection = None;
            }
            Err(error) => self.status.rejection = Some(format!("{error:#}")),
        }
        self.next_publish = Instant::now();
    }
    pub fn update(
        &mut self,
        initialized: bool,
        outputs: Vec<OutputStatus>,
        force: bool,
    ) -> Result<()> {
        self.status.ready = initialized
            && outputs
                .iter()
                .all(|output| output.initialized && output.presented > 0);
        self.status.outputs = outputs;
        if force || Instant::now() >= self.next_publish {
            self.publish()?;
        }
        Ok(())
    }
    fn publish(&mut self) -> Result<()> {
        if let Some(path) = &self.path {
            self.status.write(path)?;
        }
        self.next_publish = Instant::now() + Duration::from_millis(100);
        Ok(())
    }
}
impl Drop for StatusReporter {
    fn drop(&mut self) {
        self.status.running = false;
        self.status.ready = false;
        let _ = self.publish();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn status_publication_replaces_complete_snapshots_and_cleans_temporary_files() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("status.json");
        let mut status = DaemonStatus::starting(&Config::default());
        status.write(&path).unwrap();
        assert!(!DaemonStatus::read(&path).unwrap().ready);
        status.ready = true;
        status.write(&path).unwrap();
        assert!(DaemonStatus::read(&path).unwrap().ready);
        assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 1);
        status.schema_version = 2;
        status.write(&path).unwrap();
        assert!(
            DaemonStatus::read(&path)
                .unwrap_err()
                .to_string()
                .contains("version")
        );
        assert!(status.write(directory.path()).is_err());
        assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 1);
        {
            let _reporter = StatusReporter::new(Some(path.clone()), &Config::default()).unwrap();
        }
        let stopped = DaemonStatus::read(&path).unwrap();
        assert!(!stopped.running);
        assert!(!stopped.ready);
    }
    #[test]
    fn rejection_retains_accepted_generation_and_configuration_until_recovery() {
        let config = Config::default();
        let mut reporter = StatusReporter::new(None, &config).unwrap();
        reporter.update(false, Vec::new(), true).unwrap();
        assert!(!reporter.status.ready);
        reporter.update(true, Vec::new(), true).unwrap();
        assert!(reporter.status.ready);
        let mut next = config.clone();
        next.audio.enabled = true;
        reporter.reloaded(&Err(anyhow::anyhow!("second renderer failed")), &config);
        assert_eq!(reporter.status.accepted_generation, 1);
        assert_eq!(reporter.status.attempted_generation, 2);
        assert!(!reporter.status.config.audio.enabled);
        assert!(
            reporter
                .status
                .rejection
                .as_ref()
                .unwrap()
                .contains("second renderer")
        );
        reporter.reloaded(&Ok(()), &next);
        assert_eq!(reporter.status.accepted_generation, 3);
        assert_eq!(reporter.status.attempted_generation, 3);
        assert!(reporter.status.config.audio.enabled);
        assert!(reporter.status.rejection.is_none());
    }
    #[test]
    fn configured_outputs_must_present_before_readiness() {
        let mut reporter = StatusReporter::new(None, &Config::default()).unwrap();
        let mut output = OutputStatus {
            id: "monitor".into(),
            instance_id: 1,
            assignment: Assignment::new("test.wisp".into(), 30),
            initialized: true,
            presented: 0,
        };
        reporter.update(true, vec![output.clone()], true).unwrap();
        assert!(!reporter.status.ready);
        output.presented = 1;
        reporter.update(true, vec![output], true).unwrap();
        assert!(reporter.status.ready);
    }
}
