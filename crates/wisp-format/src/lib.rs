//! Portable Wisp package data and bounded ZIP loading. No GPU or editor dependencies.
mod archive;
mod model;
mod parameter;
pub use parameter::*;

use anyhow::{Context, Result, ensure};
pub use model::*;
use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
};

/// Host-selected limits, applied before decompression and JSON deserialization.
#[derive(Clone, Debug)]
pub struct PackageLimits {
    pub max_archive_bytes: u64,
    pub max_uncompressed_bytes: u64,
    pub max_file_bytes: u64,
    pub max_metadata_bytes: u64,
    pub max_shader_bytes: u64,
    pub max_entries: usize,
}
impl Default for PackageLimits {
    fn default() -> Self {
        Self {
            max_archive_bytes: 64 * 1024 * 1024,
            max_uncompressed_bytes: 64 * 1024 * 1024,
            max_file_bytes: 32 * 1024 * 1024,
            max_metadata_bytes: 1024 * 1024,
            max_shader_bytes: 1024 * 1024,
            max_entries: 128,
        }
    }
}

/// Structurally validated package. Shader and image validation belong to the runtime.
/// Private fields prevent callers from mutating metadata after validation.
pub struct Package {
    manifest: Manifest,
    program: Program,
    files: BTreeMap<String, Vec<u8>>,
}
impl Package {
    pub fn load(path: impl AsRef<Path>, limits: &PackageLimits) -> Result<Self> {
        let path = path.as_ref();
        let bytes = wisp_io::read_bounded(path, limits.max_archive_bytes)
            .with_context(|| format!("could not read package {}", path.display()))?;
        Self::from_bytes(&bytes, limits)
            .with_context(|| format!("invalid package {}", path.display()))
    }

    pub fn from_bytes(bytes: &[u8], limits: &PackageLimits) -> Result<Self> {
        Self::from_files(archive::decode(bytes, limits)?, limits)
    }

    /// Build a package from metadata and shader/image assets (excluding metadata files).
    pub fn build(
        manifest: Manifest,
        program: Program,
        mut assets: BTreeMap<String, Vec<u8>>,
        limits: &PackageLimits,
    ) -> Result<Self> {
        ensure!(
            !assets.contains_key("manifest.json") && !assets.contains_key(&manifest.program),
            "asset collides with metadata path"
        );
        assets.insert(
            manifest.program.clone(),
            serde_json::to_vec_pretty(&program)?,
        );
        assets.insert(
            "manifest.json".into(),
            serde_json::to_vec_pretty(&manifest)?,
        );
        Self::from_files(assets, limits)
    }

    pub fn to_bytes(&self, limits: &PackageLimits) -> Result<Vec<u8>> {
        // Reapply possibly stricter export limits, including the entry and JSON limits.
        validate_files(&self.files, limits)?;
        ensure!(
            self.files["manifest.json"].len() as u64 <= limits.max_metadata_bytes
                && self.files[&self.manifest.program].len() as u64 <= limits.max_metadata_bytes,
            "metadata exceeds byte limit"
        );
        for i in 0..self.program.passes.len() {
            ensure!(
                self.shader_source_at(i).expect("validated pass").len() as u64
                    <= limits.max_shader_bytes,
                "shader exceeds byte limit"
            );
        }
        archive::encode(&self.files, limits)
    }

    fn from_files(files: BTreeMap<String, Vec<u8>>, limits: &PackageLimits) -> Result<Self> {
        validate_files(&files, limits)?;
        let metadata = |path: &str| -> Result<&[u8]> {
            let bytes = files
                .get(path)
                .with_context(|| format!("missing required file: {path}"))?;
            ensure!(
                bytes.len() as u64 <= limits.max_metadata_bytes,
                "metadata exceeds byte limit: {path}"
            );
            Ok(bytes)
        };
        let manifest: Manifest =
            serde_json::from_slice(metadata("manifest.json")?).context("invalid manifest.json")?;
        ensure!(
            manifest.format_version == FORMAT_VERSION,
            "unsupported package format version: {}",
            manifest.format_version
        );
        ensure!(
            manifest.gpu_profile == GPU_PROFILE,
            "unsupported GPU profile: {}",
            manifest.gpu_profile
        );
        ensure!(
            manifest.program == "program.json",
            "manifest program must be program.json"
        );
        let minimum = semver::Version::parse(&manifest.minimum_wisp_version)
            .context("minimum_wisp_version must be a full semantic version")?;
        ensure!(
            !minimum
                .cmp_precedence(&semver::Version::parse(env!("CARGO_PKG_VERSION"))?)
                .is_gt(),
            "package requires Wisp {minimum} or newer"
        );
        identifier(&manifest.id)?;
        ensure!(
            !manifest.name.trim().is_empty() && manifest.name.len() <= 256,
            "wallpaper name must contain 1–256 bytes"
        );
        ensure!(
            !manifest.author.trim().is_empty() && manifest.author.len() <= 256,
            "author must contain 1–256 bytes"
        );
        ensure!(
            manifest.description.len() <= 4096
                && manifest.tags.len() <= 32
                && manifest
                    .tags
                    .iter()
                    .all(|tag| !tag.trim().is_empty() && tag.len() <= 64),
            "description or tags exceed limits"
        );
        let program: Program =
            serde_json::from_slice(metadata(&manifest.program)?).context("invalid program.json")?;
        ensure!(
            matches!(
                (program.version, program.shader_abi_version),
                (1, 1) | (2, 2) | (3, 2) | (4, 3)
            ),
            "unsupported program/shader ABI version pair: {}/{}",
            program.version,
            program.shader_abi_version
        );
        ensure!(
            (1..=MAX_PASSES).contains(&program.passes.len()),
            "program requires 1..={MAX_PASSES} passes"
        );
        if program.version < 3 {
            ensure!(
                program.passes.len() == 1,
                "exactly one fragment pass is required before version 3"
            );
        }
        validate_parameters(&program.parameters)?;
        let mut resources = BTreeMap::new();
        let mut available = BTreeSet::new();
        let mut required = BTreeSet::from(["manifest.json", "program.json"]);
        let mut image_count = 0;
        let mut target_count = 0;
        let mut buffer_count = 0;
        for resource in &program.resources {
            let id = resource.id();
            identifier(id)?;
            ensure!(
                resources.insert(id, resource).is_none(),
                "duplicate resource: {id}"
            );
            match resource {
                Resource::Image { path, .. } => {
                    image_count += 1;
                    available.insert(id);
                    archive::validate_path(path)?;
                    ensure!(
                        path.starts_with("assets/") && image_path(path),
                        "image resource must be PNG, JPEG or WebP under assets/"
                    );
                    required.insert(path);
                }
                Resource::StorageBuffer { elements, .. } => {
                    ensure!(program.version == 4, "storage buffers require version 4");
                    ensure!(
                        (1..=MAX_STORAGE_ELEMENTS).contains(elements),
                        "invalid storage buffer size"
                    );
                    buffer_count += 1;
                }
                Resource::RenderTarget { width, height, .. } => {
                    ensure!(
                        program.version >= 3,
                        "render targets require program version 3"
                    );
                    ensure!(
                        (1..=MAX_TARGET_DIMENSION).contains(width)
                            && (1..=MAX_TARGET_DIMENSION).contains(height),
                        "render target dimensions must be between 1 and {MAX_TARGET_DIMENSION}"
                    );
                    target_count += 1;
                }
            }
        }
        ensure!(
            image_count <= MAX_IMAGES && (image_count > 0 || program.version == 4),
            "program requires 1..={MAX_IMAGES} images"
        );
        ensure!(
            target_count <= MAX_RENDER_TARGETS,
            "too many render targets"
        );
        ensure!(
            buffer_count <= MAX_STORAGE_BUFFERS,
            "too many storage buffers"
        );
        if program.version == 1 {
            let Pass::Fragment { images, .. } = &program.passes[0] else {
                anyhow::bail!("version 1 requires fragment pass")
            };
            ensure!(
                image_count == 1 && images.is_empty() && program.parameters.is_empty(),
                "program version 1 requires one image and no parameters"
            );
        }
        let mut used = BTreeSet::new();
        let mut written = BTreeSet::new();
        for (index, pass) in program.passes.iter().enumerate() {
            let shader = pass.shader();
            let compute_output;
            let output = match pass {
                Pass::Fragment { output, .. } => output,
                Pass::Compute {
                    output,
                    dispatch,
                    buffers,
                    ..
                } => {
                    ensure!(program.version == 4, "compute requires program version 4");
                    ensure!(
                        dispatch.iter().all(|n| (1..=65535).contains(n))
                            && dispatch.iter().map(|n| u64::from(*n)).product::<u64>()
                                <= MAX_DISPATCH_GROUPS,
                        "compute dispatch exceeds limits"
                    );
                    ensure!(
                        buffers.len() <= MAX_STORAGE_BUFFERS,
                        "too many storage bindings"
                    );
                    let mut bindings = BTreeSet::new();
                    for binding in buffers {
                        let id = binding.resource.as_str();
                        ensure!(bindings.insert(id), "duplicate storage binding: {id}");
                        ensure!(
                            matches!(resources.get(id), Some(Resource::StorageBuffer { .. })),
                            "unknown storage buffer: {id}"
                        );
                        if binding.writable {
                            ensure!(written.insert(id), "multiple producers for buffer: {id}");
                            available.insert(id);
                        } else {
                            ensure!(available.contains(id), "buffer read before producer: {id}");
                        }
                        used.insert(id);
                    }
                    compute_output = Output::Target(output.clone());
                    &compute_output
                }
            };
            ensure!(
                pass.inputs().count() <= MAX_IMAGES,
                "pass {index} exceeds texture bindings"
            );
            let mut bound = BTreeSet::new();
            for id in pass.inputs() {
                ensure!(
                    matches!(
                        resources.get(id),
                        Some(Resource::Image { .. } | Resource::RenderTarget { .. })
                    ),
                    "pass {index} references unknown resource: {id}"
                );
                ensure!(bound.insert(id), "duplicate image binding: {id}");
                ensure!(
                    available.contains(id),
                    "pass {index} reads target before its producer: {id}"
                );
                used.insert(id);
            }
            match &output {
                Output::Presentation => ensure!(
                    index + 1 == program.passes.len(),
                    "only the final pass may output to presentation"
                ),
                Output::Target(id) => {
                    ensure!(
                        program.version >= 3,
                        "target outputs require program version 3"
                    );
                    ensure!(
                        matches!(
                            resources.get(id.as_str()),
                            Some(Resource::RenderTarget { .. })
                        ),
                        "pass {index} output is not a declared render target: {id}"
                    );
                    ensure!(
                        !bound.contains(id.as_str()),
                        "pass {index} samples its own output: {id}"
                    );
                    let id = resources
                        .get_key_value(id.as_str())
                        .expect("validated target")
                        .0;
                    ensure!(written.insert(*id), "multiple producers for target: {id}");
                    available.insert(*id);
                    ensure!(
                        index + 1 < program.passes.len(),
                        "final pass must output to presentation"
                    );
                }
            }
            archive::validate_path(shader)?;
            ensure!(
                shader.starts_with("shaders/") && shader.ends_with(".wgsl"),
                "fragment shader must be a .wgsl file under shaders/"
            );
            let source = files
                .get(shader)
                .with_context(|| format!("missing required file: {shader}"))?;
            ensure!(
                source.len() as u64 <= limits.max_shader_bytes,
                "shader exceeds byte limit: {shader}"
            );
            std::str::from_utf8(source).context("shader is not UTF-8")?;
            required.insert(shader);
        }
        ensure!(
            used.len() == resources.len(),
            "unreferenced image resource or render target"
        );
        if let Some(preview) = &manifest.preview {
            archive::validate_path(preview)?;
            ensure!(image_path(preview), "preview must be PNG, JPEG or WebP");
            required.insert(preview);
        }
        for path in &required {
            ensure!(files.contains_key(*path), "missing required file: {path}");
        }
        for path in files.keys() {
            ensure!(
                required.contains(path.as_str()),
                "unreferenced package file: {path}"
            );
        }
        Ok(Self {
            manifest,
            program,
            files,
        })
    }

    pub fn manifest(&self) -> &Manifest {
        &self.manifest
    }
    pub fn program(&self) -> &Program {
        &self.program
    }
    /// Source for the first pass. Use `shader_source_at` for subsequent passes.
    pub fn shader_source(&self) -> &str {
        self.shader_source_at(0).expect("validated first pass")
    }
    pub fn shader_source_at(&self, index: usize) -> Option<&str> {
        let shader = self.program.passes.get(index)?.shader();
        Some(std::str::from_utf8(&self.files[shader]).expect("validated UTF-8"))
    }
    /// Encoded asset bytes for an image resource; render targets have no file.
    pub fn resource_image_bytes(&self, id: &str) -> Option<&[u8]> {
        self.program
            .resources
            .iter()
            .find_map(|resource| match resource {
                Resource::Image {
                    id: resource_id,
                    path,
                } if resource_id == id => Some(self.files[path].as_slice()),
                _ => None,
            })
    }
    pub fn image_bytes(&self) -> &[u8] {
        self.image_bytes_at(0).expect("validated background")
    }
    /// Texture binding count for the first pass, independent of resource order.
    pub fn image_count(&self) -> usize {
        self.program.passes[0].inputs().count()
    }
    pub fn image_bytes_at(&self, index: usize) -> Option<&[u8]> {
        let id = self.program.passes[0].inputs().nth(index)?;
        self.resource_image_bytes(id)
    }
    pub fn preview_bytes(&self) -> Option<&[u8]> {
        self.manifest
            .preview
            .as_ref()
            .map(|path| self.files[path].as_slice())
    }
}

fn image_path(path: &str) -> bool {
    [".png", ".jpg", ".jpeg", ".webp"]
        .iter()
        .any(|suffix| path.ends_with(suffix))
}
fn identifier(value: &str) -> Result<()> {
    ensure!(
        !value.is_empty()
            && value.len() <= 128
            && value
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || b"-_.".contains(&c)),
        "invalid identifier: {value:?}"
    );
    Ok(())
}
fn validate_files(files: &BTreeMap<String, Vec<u8>>, limits: &PackageLimits) -> Result<()> {
    ensure!(
        files.len() <= limits.max_entries && files.len() < 65535,
        "package entry count exceeds limit"
    );
    let mut names = BTreeSet::new();
    let mut total = 0u64;
    for (path, bytes) in files {
        archive::validate_path(path)?;
        ensure!(
            names.insert(path.to_ascii_lowercase()),
            "case-colliding package path: {path}"
        );
        ensure!(
            bytes.len() as u64 <= limits.max_file_bytes,
            "file exceeds byte limit: {path}"
        );
        total = total
            .checked_add(bytes.len() as u64)
            .context("package size overflow")?;
        ensure!(
            total <= limits.max_uncompressed_bytes,
            "package exceeds total uncompressed byte limit"
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests;
