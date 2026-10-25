//! Shared daemon assignments and options; desktop presentation lives in host adapters.
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs::File,
    io::Read,
    path::{Path, PathBuf},
};
use wisp_format::{Package, PackageLimits, ParameterValue};
use wisp_runtime::Wallpaper;

pub struct Options {
    pub config: Config,
    pub config_path: Option<PathBuf>,
    pub list_outputs: bool,
    pub json: bool,
    pub frames: Option<u64>,
}

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Assignment {
    pub package: PathBuf,
    #[serde(default)]
    pub parameters: BTreeMap<String, ParameterValue>,
    #[serde(default = "default_fps")]
    pub fps: u32,
}
fn default_fps() -> u32 {
    60
}
impl Assignment {
    pub fn new(package: PathBuf, fps: u32) -> Self {
        Self {
            package,
            parameters: BTreeMap::new(),
            fps,
        }
    }
    pub fn load(&self) -> Result<Wallpaper> {
        ensure!(
            (1..=240).contains(&self.fps),
            "FPS must be between 1 and 240"
        );
        let package = Package::load(&self.package, &PackageLimits::default())?;
        let mut wallpaper = Wallpaper::from_package(&package)?;
        for (id, value) in &self.parameters {
            wallpaper.set_parameter(id, value.clone())?;
        }
        Ok(wallpaper)
    }
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    #[serde(default)]
    pub audio: wisp_audio::Config,
    #[serde(default)]
    pub default: Option<Assignment>,
    #[serde(default)]
    pub outputs: BTreeMap<String, Assignment>,
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    pub disabled_outputs: BTreeSet<String>,
}
impl Config {
    /// Audio-only reloads preserve all renderer-owned persistent state.
    pub fn audio_only_change(&self, next: &Self) -> bool {
        self.audio != next.audio
            && self.default == next.default
            && self.outputs == next.outputs
            && self.disabled_outputs == next.disabled_outputs
    }

    pub fn load(path: &Path) -> Result<Self> {
        let mut bytes = Vec::new();
        File::open(path)?
            .take(1024 * 1024 + 1)
            .read_to_end(&mut bytes)?;
        ensure!(
            bytes.len() <= 1024 * 1024,
            "assignment config exceeds 1 MiB"
        );
        let mut config: Self =
            serde_json::from_slice(&bytes).context("invalid assignment config")?;
        ensure!(config.outputs.len() <= 64, "too many output assignments");
        for assignment in config.default.iter_mut().chain(config.outputs.values_mut()) {
            if assignment.package.is_relative() {
                assignment.package = path
                    .parent()
                    .unwrap_or(Path::new("."))
                    .join(&assignment.package);
            }
        }
        Ok(config)
    }
    pub fn assignment(&self, name: &str) -> Option<&Assignment> {
        if self.disabled_outputs.contains(name) {
            return None;
        }
        self.outputs.get(name).or(self.default.as_ref())
    }
    pub fn validate(&self) -> Result<()> {
        self.audio.validate()?;
        ensure!(
            self.outputs.len() <= 64 && self.disabled_outputs.len() <= 64,
            "too many output assignments"
        );
        ensure!(
            self.disabled_outputs
                .iter()
                .all(|name| !name.trim().is_empty() && !self.outputs.contains_key(name)),
            "disabled output is empty or also assigned"
        );
        ensure!(
            self.outputs.keys().all(|name| !name.trim().is_empty()),
            "output names must not be empty"
        );
        for assignment in self.default.iter().chain(self.outputs.values()) {
            assignment.load()?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn explicit_assignment_wins_and_unknown_outputs_use_default() {
        let mut config = Config {
            default: Some(Assignment::new("default.wisp".into(), 30)),
            ..Default::default()
        };
        config
            .outputs
            .insert("DP-1".into(), Assignment::new("specific.wisp".into(), 60));
        assert_eq!(config.assignment("DP-1").unwrap().fps, 60);
        assert_eq!(config.assignment("DP-2").unwrap().fps, 30);
        let mut next = config.clone();
        next.audio.enabled = true;
        assert!(config.audio_only_change(&next));
        next.default.as_mut().unwrap().fps = 24;
        assert!(!config.audio_only_change(&next));
        next = config.clone();
        next.audio.enabled = true;
        next.disabled_outputs.insert("DP-2".into());
        assert!(!config.audio_only_change(&next));
        assert!(!config.audio_only_change(&config));
        config.default = None;
        assert!(config.assignment("DP-2").is_none());
        assert!(serde_json::from_str::<Config>(r#"{"typo":true}"#).is_err());
        assert!(
            Assignment::new("missing.wisp".into(), 0)
                .load()
                .err()
                .unwrap()
                .to_string()
                .contains("FPS")
        );
    }
}

/// Machine-readable desktop output identity. Dimensions are logical on Wayland and physical on Windows.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct MonitorInfo {
    pub id: String,
    pub name: String,
    pub description: String,
    pub width: u32,
    pub height: u32,
    pub scale: f64,
}
