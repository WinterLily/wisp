//! Persistent application data. Installed packages are immutable, addressed by content.
use anyhow::{Context, Result, ensure};
use fs2::FileExt;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};
use wisp_format::{Manifest, Package, PackageLimits, Parameter, Resource};
use wisp_host::{Assignment, Config};
use wisp_runtime::Wallpaper;

#[derive(Clone, Debug)]
pub struct Library {
    pub root: PathBuf,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Record {
    pub key: String,
    pub source: String,
    pub imported_at: u64,
}
#[derive(Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Catalog {
    version: u32,
    entries: Vec<Record>,
}
#[derive(Clone, Debug)]
pub struct Item {
    pub record: Record,
    pub manifest: Manifest,
    pub parameters: Vec<Parameter>,
    pub passes: usize,
    pub resource_bytes: u64,
    pub thumbnail: Option<(u32, u32, Vec<u8>)>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Settings {
    pub version: u32,
    pub desktop_fps: u32,
    pub preview_fps: u32,
    pub dark_theme: bool,
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            version: 1,
            desktop_fps: 60,
            preview_fps: 30,
            dark_theme: true,
        }
    }
}
impl Settings {
    pub fn validate(&self) -> Result<()> {
        ensure!(self.version == 1, "unsupported settings version");
        ensure!(
            (1..=240).contains(&self.desktop_fps),
            "desktop FPS must be 1–240"
        );
        ensure!(
            (1..=60).contains(&self.preview_fps),
            "preview FPS must be 1–60"
        );
        Ok(())
    }
}
impl Library {
    pub fn open(root: Option<PathBuf>) -> Result<Self> {
        let root = root
            .or_else(|| {
                directories::ProjectDirs::from("", "", "wisp")
                    .map(|p| p.data_local_dir().to_owned())
            })
            .context("could not determine application data directory; use --data-dir")?;
        fs::create_dir_all(root.join("packages"))?;
        Ok(Self {
            root: root.canonicalize()?,
        })
    }
    pub fn lock(&self) -> Result<File> {
        let file = lock_file(&self.root.join("app.lock"))?;
        file.try_lock_exclusive()
            .context("another Wisp application is using this library")?;
        Ok(file)
    }
    pub fn config_path(&self) -> PathBuf {
        self.root.join("assignments.json")
    }
    pub fn package_path(&self, key: &str) -> Result<PathBuf> {
        ensure!(
            key.len() == 64
                && key
                    .bytes()
                    .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase()),
            "invalid library key"
        );
        Ok(self.root.join("packages").join(format!("{key}.wisp")))
    }
    fn catalog(&self) -> Result<Catalog> {
        let path = self.root.join("library.json");
        if !path.exists() {
            return Ok(Catalog {
                version: 1,
                entries: vec![],
            });
        }
        let catalog: Catalog = serde_json::from_slice(&read_bounded(&path, 1024 * 1024)?)?;
        ensure!(catalog.version == 1, "unsupported library version");
        ensure!(
            catalog.entries.len() <= 4096,
            "library entry limit exceeded"
        );
        let mut keys = std::collections::BTreeSet::new();
        for record in &catalog.entries {
            self.package_path(&record.key)?;
            ensure!(keys.insert(&record.key), "duplicate library entry");
        }
        Ok(catalog)
    }
    pub fn scan(&self) -> Result<(Vec<Item>, Vec<String>)> {
        let mut items = vec![];
        let mut errors = vec![];
        for record in self.catalog()?.entries {
            match self.item(record.clone()) {
                Ok(item) => items.push(item),
                Err(error) => errors.push(format!("{}: {error:#}", record.key)),
            }
        }
        items.sort_by_key(|item| item.manifest.name.to_lowercase());
        Ok((items, errors))
    }
    pub fn load(&self, key: &str) -> Result<Package> {
        let bytes = read_bounded(
            &self.package_path(key)?,
            PackageLimits::default().max_archive_bytes,
        )?;
        ensure!(
            format!("{:x}", Sha256::digest(&bytes)) == key,
            "installed package changed on disk; import it again"
        );
        Package::from_bytes(&bytes, &PackageLimits::default())
    }
    fn item(&self, record: Record) -> Result<Item> {
        let package = self.load(&record.key)?;
        let wallpaper = Wallpaper::from_package(&package)?;
        let thumbnail = package
            .preview_bytes()
            .map(|bytes| -> Result<_> {
                let decoded = image::load_from_memory(bytes)?
                    .thumbnail(320, 180)
                    .to_rgba8();
                Ok((decoded.width(), decoded.height(), decoded.into_raw()))
            })
            .transpose()?;
        let image_bytes: u64 = wallpaper
            .images()
            .iter()
            .map(|i| {
                let (w, h) = i.dimensions();
                u64::from(w) * u64::from(h) * 4
            })
            .sum();
        let other_bytes: u64 = package
            .program()
            .resources
            .iter()
            .map(|r| match r {
                Resource::RenderTarget { width, height, .. } => {
                    u64::from(*width) * u64::from(*height) * 4
                }
                Resource::StorageBuffer { elements, .. } => u64::from(*elements) * 16,
                _ => 0,
            })
            .sum();
        Ok(Item {
            record,
            manifest: package.manifest().clone(),
            parameters: package.program().parameters.clone(),
            passes: wallpaper.pass_count(),
            resource_bytes: image_bytes + other_bytes,
            thumbnail,
        })
    }
    pub fn import(&self, path: &Path) -> Result<Item> {
        let (package, source) = if path.is_dir() {
            (
                wisp_compiler::compile_project(path, &Default::default())?.package,
                "Local project",
            )
        } else {
            let bytes = read_bounded(path, PackageLimits::default().max_archive_bytes)?;
            (
                Package::from_bytes(&bytes, &PackageLimits::default())?,
                "Imported package",
            )
        };
        self.install(&package, source)
    }
    pub fn install(&self, package: &Package, source: &str) -> Result<Item> {
        Wallpaper::from_package(package).context("wallpaper validation failed")?;
        let bytes = package.to_bytes(&PackageLimits::default())?;
        let key = format!("{:x}", Sha256::digest(&bytes));
        let mut catalog = self.catalog()?;
        if let Some(record) = catalog.entries.iter().find(|r| r.key == key) {
            return self.item(record.clone());
        }
        ensure!(catalog.entries.len() < 4096, "library entry limit exceeded");
        let record = Record {
            key,
            source: source.into(),
            imported_at: SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs(),
        };
        let destination = self.package_path(&record.key)?;
        // Never overwrite an installed object. A previous interrupted import can leave an orphan.
        if destination.exists() {
            ensure!(
                read_bounded(&destination, PackageLimits::default().max_archive_bytes)? == bytes,
                "library object collision"
            );
        } else {
            atomic_write(&destination, &bytes)?;
        }
        catalog.entries.push(record.clone());
        atomic_json(&self.root.join("library.json"), &catalog)?;
        self.item(record)
    }
    pub fn export(&self, key: &str, destination: &Path) -> Result<()> {
        let package = self.load(key)?;
        Wallpaper::from_package(&package)?;
        let bytes = package.to_bytes(&PackageLimits::default())?;
        let parent = destination
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        let mut temporary = tempfile::NamedTempFile::new_in(parent)?;
        temporary.write_all(&bytes)?;
        temporary.as_file().sync_all()?;
        temporary
            .persist_noclobber(destination)
            .context("export requires a new destination")?;
        Ok(())
    }
    pub fn settings(&self) -> Result<Settings> {
        let path = self.root.join("settings.json");
        let settings: Settings = if path.exists() {
            serde_json::from_slice(&read_bounded(&path, 65536)?)?
        } else {
            Settings::default()
        };
        settings.validate()?;
        Ok(settings)
    }
    pub fn save_settings(&self, settings: &Settings) -> Result<()> {
        settings.validate()?;
        atomic_json(&self.root.join("settings.json"), settings)
    }
    pub fn config(&self) -> Result<Config> {
        if self.config_path().exists() {
            Config::load(&self.config_path())
        } else {
            Ok(Config::default())
        }
    }
    pub fn save_config(&self, config: &Config) -> Result<()> {
        config.validate()?;
        atomic_json(&self.config_path(), config)
    }
    pub fn assign(&self, monitor: Option<&str>, assignment: Option<Assignment>) -> Result<()> {
        let mut config = self.config()?;
        match (monitor, assignment) {
            (None, assignment) => {
                config.default = assignment;
                config.outputs.clear();
                config.disabled_outputs.clear();
            }
            (Some(id), Some(assignment)) => {
                config.disabled_outputs.remove(id);
                config.outputs.insert(id.into(), assignment);
            }
            (Some(id), None) => {
                config.outputs.remove(id);
                config.disabled_outputs.insert(id.into());
            }
        }
        self.save_config(&config)
    }
}
pub fn lock_file(path: &Path) -> Result<File> {
    Ok(OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(path)?)
}
pub fn read_bounded(path: &Path, limit: u64) -> Result<Vec<u8>> {
    ensure!(
        fs::metadata(path)?.is_file(),
        "expected a regular file: {}",
        path.display()
    );
    let file = File::open(path)?;
    ensure!(file.metadata()?.is_file(), "expected a regular file");
    let mut bytes = vec![];
    file.take(limit + 1).read_to_end(&mut bytes)?;
    ensure!(bytes.len() as u64 <= limit, "file exceeds byte limit");
    Ok(bytes)
}
fn atomic_json(path: &Path, value: &impl Serialize) -> Result<()> {
    let bytes = serde_json::to_vec_pretty(value)?;
    ensure!(bytes.len() <= 1024 * 1024, "application data exceeds 1 MiB");
    atomic_write(path, &bytes)
}
fn atomic_write(path: &Path, bytes: &[u8]) -> Result<()> {
    let mut file =
        tempfile::NamedTempFile::new_in(path.parent().context("missing parent directory")?)?;
    file.write_all(bytes)?;
    file.as_file().sync_all()?;
    file.persist(path)
        .context("could not commit application data")?;
    Ok(())
}
