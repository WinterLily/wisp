//! Persistent application data. Installed packages are immutable, addressed by content.
use anyhow::{Context, Result, ensure};
use fs2::FileExt;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File, OpenOptions},
    io::Write,
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
        write_new(destination, &bytes).context("export requires a new destination")?;
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
    wisp_io::read_bounded(path, limit)
}

fn atomic_json(path: &Path, value: &impl Serialize) -> Result<()> {
    let bytes = serde_json::to_vec_pretty(value)?;
    ensure!(bytes.len() <= 1024 * 1024, "application data exceeds 1 MiB");
    atomic_write(path, &bytes)
}
pub(crate) fn atomic_write(path: &Path, bytes: &[u8]) -> Result<()> {
    write_atomically(path, bytes, false)
}

pub(crate) fn write_new(path: &Path, bytes: &[u8]) -> Result<()> {
    write_atomically(path, bytes, true)
}

fn write_atomically(path: &Path, bytes: &[u8], require_new: bool) -> Result<()> {
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let mut temporary = tempfile::NamedTempFile::new_in(parent)?;
    #[cfg(test)]
    persistence_tests::inject_failure(path, persistence_tests::Stage::Write, &mut temporary)?;
    temporary.write_all(bytes)?;
    #[cfg(test)]
    persistence_tests::inject_failure(path, persistence_tests::Stage::Sync, &mut temporary)?;
    temporary.as_file().sync_all()?;
    #[cfg(test)]
    persistence_tests::inject_failure(path, persistence_tests::Stage::Persist, &mut temporary)?;
    if require_new {
        temporary.persist_noclobber(path)
    } else {
        temporary.persist(path)
    }
    .context("could not commit application data")?;
    Ok(())
}

#[cfg(test)]
mod persistence_tests {
    use super::*;
    use std::cell::RefCell;

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub(super) enum Stage {
        Write,
        Sync,
        Persist,
    }

    thread_local! {
        static FAILURE: RefCell<Option<(PathBuf, Stage)>> = const { RefCell::new(None) };
    }

    struct FailureGuard;
    impl FailureGuard {
        fn at(path: &Path, stage: Stage) -> Self {
            FAILURE.with(|failure| {
                assert!(failure.borrow().is_none());
                *failure.borrow_mut() = Some((path.to_owned(), stage));
            });
            Self
        }
    }
    impl Drop for FailureGuard {
        fn drop(&mut self) {
            FAILURE.with(|failure| *failure.borrow_mut() = None);
        }
    }

    pub(super) fn inject_failure(
        path: &Path,
        stage: Stage,
        temporary: &mut tempfile::NamedTempFile,
    ) -> Result<()> {
        let should_fail = FAILURE.with(|failure| {
            let mut failure = failure.borrow_mut();
            if failure
                .as_ref()
                .is_some_and(|(target, target_stage)| target == path && *target_stage == stage)
            {
                failure.take();
                true
            } else {
                false
            }
        });
        if should_fail {
            if stage == Stage::Write {
                temporary.write_all(b"partial write")?;
            }
            anyhow::bail!("injected persistence {stage:?} failure");
        }
        Ok(())
    }

    fn assert_injected(result: Result<()>, stage: Stage) {
        let error = format!("{:#}", result.unwrap_err());
        assert!(
            error.contains(&format!("injected persistence {stage:?} failure")),
            "{error}"
        );
    }

    fn draft(root: &Path) -> crate::designer::Draft {
        let image = root.join("source.png");
        fs::write(&image, wisp_runtime::EXAMPLE_IMAGE).unwrap();
        crate::designer::Draft::from_image(&image).unwrap()
    }

    #[test]
    fn failed_replace_keeps_previous_bytes_and_removes_temporary_files() {
        for stage in [Stage::Write, Stage::Sync, Stage::Persist] {
            let directory = tempfile::tempdir().unwrap();
            let path = directory.path().join("settings.json");
            let library = Library {
                root: directory.path().to_owned(),
            };
            library.save_settings(&Settings::default()).unwrap();
            let previous = fs::read(&path).unwrap();
            let _failure = FailureGuard::at(&path, stage);
            assert_injected(
                library.save_settings(&Settings {
                    desktop_fps: 24,
                    ..Default::default()
                }),
                stage,
            );
            assert_eq!(fs::read(&path).unwrap(), previous);
            assert_eq!(fs::read_dir(directory.path()).unwrap().count(), 1);
            library
                .save_settings(&Settings {
                    desktop_fps: 24,
                    ..Default::default()
                })
                .unwrap();
            assert_eq!(library.settings().unwrap().desktop_fps, 24);
        }
    }

    #[test]
    fn failed_new_file_write_cleans_up_and_never_overwrites_existing_exports() {
        for stage in [Stage::Write, Stage::Sync, Stage::Persist] {
            let directory = tempfile::tempdir().unwrap();
            let path = directory.path().join("export.wisp");
            let _failure = FailureGuard::at(&path, stage);
            assert_injected(write_new(&path, b"new"), stage);
            assert!(!path.exists());
            assert_eq!(fs::read_dir(directory.path()).unwrap().count(), 0);
            fs::write(&path, b"previous").unwrap();
            assert!(write_new(&path, b"new").is_err());
            assert_eq!(fs::read(&path).unwrap(), b"previous");
            assert_eq!(fs::read_dir(directory.path()).unwrap().count(), 1);
        }
    }

    #[test]
    fn all_exports_use_the_same_failure_safe_commit() {
        let directory = tempfile::tempdir().unwrap();
        let draft = draft(directory.path());
        let project = directory.path().join("project");
        draft.save_project(&project).unwrap();
        let document = crate::graph::Document::open(&project).unwrap();
        let library = Library::open(Some(directory.path().join("library"))).unwrap();
        let item = library.import(&project).unwrap();
        for stage in [Stage::Write, Stage::Sync, Stage::Persist] {
            let destination = directory.path().join("export.wisp");
            let exports: [&dyn Fn() -> Result<()>; 3] = [
                &|| draft.export(&destination),
                &|| document.export(&destination),
                &|| library.export(&item.record.key, &destination),
            ];
            for export in exports {
                let _failure = FailureGuard::at(&destination, stage);
                assert_injected(export(), stage);
                assert!(!destination.exists());
            }
        }
    }

    #[test]
    fn failed_project_writes_remove_only_the_new_project() {
        let directory = tempfile::tempdir().unwrap();
        let mut draft = draft(directory.path());
        let existing = directory.path().join("existing");
        draft.save_project(&existing).unwrap();
        let document = crate::graph::Document::open(&existing).unwrap();
        let original = fs::read(existing.join("project.json")).unwrap();
        assert!(draft.save_project(&existing).is_err());
        assert!(document.save(&existing).is_err());
        assert_eq!(fs::read(existing.join("project.json")).unwrap(), original);
        for stage in [Stage::Write, Stage::Sync, Stage::Persist] {
            let destination = directory.path().join("new-project");
            let target = destination.join("graph.json");
            {
                let _failure = FailureGuard::at(&target, stage);
                assert_injected(draft.save_project(&destination), stage);
                assert!(!destination.exists());
            }
            {
                let _failure = FailureGuard::at(&target, stage);
                assert_injected(document.save(&destination), stage);
                assert!(!destination.exists());
            }
        }
        draft.layers = Some(crate::layers::Stack::new(
            draft.assets()["background"].clone(),
        ));
        for filename in ["designer.json", "layers.json"] {
            for stage in [Stage::Write, Stage::Sync, Stage::Persist] {
                let destination = directory.path().join("layered-project");
                let _failure = FailureGuard::at(&destination.join(filename), stage);
                assert_injected(draft.save_project(&destination), stage);
                assert!(!destination.exists());
            }
        }
        assert_eq!(fs::read(existing.join("project.json")).unwrap(), original);
    }

    #[test]
    fn installed_package_survives_failed_catalog_commit_and_reimports() {
        let directory = tempfile::tempdir().unwrap();
        let draft = draft(directory.path());
        let project = directory.path().join("project");
        draft.save_project(&project).unwrap();
        let package_bytes = draft
            .compile()
            .unwrap()
            .package
            .to_bytes(&Default::default())
            .unwrap();
        let key = format!("{:x}", Sha256::digest(&package_bytes));
        for stage in [Stage::Write, Stage::Sync, Stage::Persist] {
            let library =
                Library::open(Some(directory.path().join(format!("library-{stage:?}")))).unwrap();
            let catalog_path = library.root.join("library.json");
            atomic_json(
                &catalog_path,
                &Catalog {
                    version: 1,
                    entries: vec![],
                },
            )
            .unwrap();
            let previous = fs::read(&catalog_path).unwrap();
            {
                let _failure = FailureGuard::at(&catalog_path, stage);
                assert_injected(library.import(&project).map(|_| ()), stage);
            }
            assert_eq!(fs::read(&catalog_path).unwrap(), previous);
            assert_eq!(
                fs::read(library.package_path(&key).unwrap()).unwrap(),
                package_bytes
            );
            assert!(library.scan().unwrap().0.is_empty());
            let item = library.import(&project).unwrap();
            assert_eq!(item.record.key, key);
            assert_eq!(library.scan().unwrap().0.len(), 1);
            library.import(&project).unwrap();
            assert_eq!(library.scan().unwrap().0.len(), 1);
        }
    }
}
