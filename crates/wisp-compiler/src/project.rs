use crate::{Asset, Compilation, CompileOptions, compile};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    path::{Component, Path},
};
use wisp_format::Manifest;
use wisp_ir::{Graph, Op};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Project {
    pub version: u32,
    pub id: String,
    pub name: String,
    pub author: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub assets: BTreeMap<String, ProjectAsset>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProjectAsset {
    pub path: String,
}

fn read_bounded(path: &Path, limit: u64) -> Result<Vec<u8>> {
    wisp_io::read_bounded(path, limit)
        .with_context(|| format!("could not read project input {}", path.display()))
}

fn contained(root: &Path, path: &str) -> Result<std::path::PathBuf> {
    ensure!(
        !path.contains('\\')
            && !path.contains(':')
            && !path.is_empty()
            && Path::new(path)
                .components()
                .all(|c| matches!(c, Component::Normal(_))),
        "unsafe project path: {path}"
    );
    let resolved = root
        .join(path)
        .canonicalize()
        .with_context(|| format!("missing project file: {path}"))?;
    ensure!(
        resolved.starts_with(root),
        "project path escapes project directory: {path}"
    );
    Ok(resolved)
}
/// Load project.json and graph.json from a directory. Paths are relative to that
/// directory and canonicalized to reject escaping symlinks. Metadata is bounded;
/// original image sources have no authoring size cap.
pub fn compile_project(
    directory: impl AsRef<Path>,
    options: &CompileOptions,
) -> Result<Compilation> {
    let root = directory
        .as_ref()
        .canonicalize()
        .context("could not open project directory")?;
    let metadata_limit = options.package_limits.max_metadata_bytes;
    let project: Project = serde_json::from_slice(&read_bounded(
        &contained(&root, "project.json")?,
        metadata_limit,
    )?)
    .context("invalid project.json")?;
    ensure!(
        project.version == 1,
        "unsupported project version: {}",
        project.version
    );
    ensure!(
        project.assets.len() <= wisp_ir::MAX_NODES,
        "too many project assets"
    );
    let graph: Graph = serde_json::from_slice(&read_bounded(
        &contained(&root, "graph.json")?,
        metadata_limit,
    )?)
    .context("invalid graph.json")?;
    let graph = graph.check()?;
    let mut assets = BTreeMap::new();
    for &i in graph.live_order() {
        if let Op::Image { asset, .. } = &graph.graph().nodes[i].op {
            if assets.contains_key(asset) {
                continue;
            }
            let file = project
                .assets
                .get(asset)
                .with_context(|| format!("undeclared project asset: {asset}"))?;
            ensure!(
                file.path.starts_with("assets/"),
                "image must be under project assets/: {}",
                file.path
            );
            let extension = Path::new(&file.path)
                .extension()
                .and_then(|s| s.to_str())
                .context("image requires extension")?
                .to_owned();
            ensure!(
                ["png", "jpg", "jpeg", "webp"].contains(&extension.as_str()),
                "unsupported image extension"
            );
            let bytes = crate::read_source_image(&contained(&root, &file.path)?)?;
            assets.insert(asset.clone(), Asset { extension, bytes });
        }
    }
    compile(
        &graph,
        &assets,
        Manifest {
            format_version: 1,
            id: project.id,
            name: project.name,
            author: project.author,
            description: project.description,
            tags: project.tags,
            preview: None,
            program: "program.json".into(),
            minimum_wisp_version: env!("CARGO_PKG_VERSION").into(),
            gpu_profile: 1,
        },
        options,
    )
}
