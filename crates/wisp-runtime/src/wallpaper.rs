use anyhow::{Context, Result, ensure};
use wisp_format::{Parameter, ParameterValue, Resource};

use crate::{Shader, WallpaperImage};

/// Host-selected limits on persistent texture storage, checked before GPU allocation.
/// These estimates exclude driver overhead, pipelines, uniforms and host-owned output.
#[derive(Clone, Debug)]
pub struct RuntimeLimits {
    pub max_texture_dimension: u32,
    pub max_texture_bytes: u64,
    pub max_passes: usize,
    pub max_storage_bytes: u64,
    pub max_resource_bytes: u64,
    pub max_dispatch_groups: u64,
    pub max_intermediate_targets: usize,
}

impl Default for RuntimeLimits {
    fn default() -> Self {
        Self {
            max_texture_dimension: 4096,
            max_texture_bytes: 256 * 1024 * 1024,
            max_passes: wisp_format::MAX_PASSES,
            max_storage_bytes: 128 * 1024 * 1024,
            max_resource_bytes: 256 * 1024 * 1024,
            max_dispatch_groups: wisp_format::MAX_DISPATCH_GROUPS,
            max_intermediate_targets: wisp_format::MAX_RENDER_TARGETS,
        }
    }
}

pub(crate) enum TextureSource {
    Image(usize),
    Buffer { elements: u32 },
    Target { width: u32, height: u32 },
}

pub(crate) struct CompiledPass {
    pub shader: Shader,
    pub inputs: Vec<usize>,
    pub output: Option<usize>,
    pub compute: Option<([u32; 3], Vec<usize>)>,
}

/// CPU-validated resources and passes. Private topology keeps validation intact.
pub struct Wallpaper {
    pub(crate) images: Vec<WallpaperImage>,
    pub(crate) resources: Vec<TextureSource>,
    pub(crate) passes: Vec<CompiledPass>,
}

impl Wallpaper {
    pub fn new(shader: Shader, image: WallpaperImage) -> Result<Self> {
        ensure!(
            shader.image_count() == 1,
            "loose wallpaper requires one image binding"
        );
        Ok(Self {
            images: vec![image],
            resources: vec![TextureSource::Image(0)],
            passes: vec![CompiledPass {
                shader,
                inputs: vec![0],
                output: None,
                compute: None,
            }],
        })
    }

    pub fn from_package(package: &wisp_format::Package) -> Result<Self> {
        Self::from_package_with_limits(package, &RuntimeLimits::default())
    }

    pub fn from_package_with_limits(
        package: &wisp_format::Package,
        limits: &RuntimeLimits,
    ) -> Result<Self> {
        let program = package.program();
        ensure!(
            program.passes.len() <= limits.max_passes,
            "pass count exceeds runtime limit"
        );
        let mut wallpaper = Self {
            images: vec![],
            resources: vec![],
            passes: vec![],
        };
        let mut total = 0;
        let mut targets = 0;
        for resource in &program.resources {
            let (source, width, height) = match resource {
                Resource::Image { id, .. } => {
                    let image = WallpaperImage::decode(
                        package.resource_image_bytes(id).expect("validated image"),
                    )
                    .with_context(|| format!("invalid package image {id}"))?;
                    let (width, height) = image.dimensions();
                    let index = wallpaper.images.len();
                    wallpaper.images.push(image);
                    (TextureSource::Image(index), width, height)
                }
                Resource::StorageBuffer { elements, .. } => {
                    wallpaper.resources.push(TextureSource::Buffer {
                        elements: *elements,
                    });
                    continue;
                }
                Resource::RenderTarget { width, height, .. } => {
                    targets += 1;
                    ensure!(
                        targets <= limits.max_intermediate_targets,
                        "intermediate target count exceeds runtime limit"
                    );
                    (
                        TextureSource::Target {
                            width: *width,
                            height: *height,
                        },
                        *width,
                        *height,
                    )
                }
            };
            limits.check_texture(width, height, &mut total)?;
            wallpaper.resources.push(source);
        }
        let resource_index = |id: &str| {
            program
                .resources
                .iter()
                .position(|r| r.id() == id)
                .expect("validated resource")
        };
        for (index, pass) in program.passes.iter().enumerate() {
            let inputs: Vec<_> = pass.inputs().map(resource_index).collect();
            let body = package.shader_source_at(index).expect("validated pass");
            let (shader, output, compute) = match pass {
                wisp_format::Pass::Fragment { output, .. } => {
                    let shader = Shader::with_bindings(body, inputs.len(), &program.parameters);
                    let output = match output {
                        wisp_format::Output::Presentation => None,
                        wisp_format::Output::Target(id) => Some(resource_index(id)),
                    };
                    (shader, output, None)
                }
                wisp_format::Pass::Compute {
                    buffers,
                    dispatch,
                    output,
                    ..
                } => {
                    let shader = Shader::compute(
                        body,
                        inputs.len(),
                        &program.parameters,
                        buffers.iter().map(|b| b.writable).collect(),
                    );
                    (
                        shader,
                        Some(resource_index(output)),
                        Some((
                            *dispatch,
                            buffers
                                .iter()
                                .map(|b| resource_index(&b.resource))
                                .collect(),
                        )),
                    )
                }
            };
            let shader =
                shader.with_context(|| format!("invalid package shader in pass {index}"))?;
            wallpaper.passes.push(CompiledPass {
                shader,
                inputs,
                output,
                compute,
            });
        }
        if let Some(preview) = package.preview_bytes() {
            WallpaperImage::decode(preview).context("invalid package preview")?;
        }
        wallpaper.validate_limits(limits)?;
        Ok(wallpaper)
    }

    /// Decoded images in resource declaration order.
    pub fn images(&self) -> &[WallpaperImage] {
        &self.images
    }
    /// Validated shader for the first pass.
    pub fn shader(&self) -> &Shader {
        &self.passes[0].shader
    }
    pub fn parameters(&self) -> &[Parameter] {
        self.shader().parameters()
    }
    pub fn pass_count(&self) -> usize {
        self.passes.len()
    }

    /// Apply a validated initial value to every pass before renderer creation.
    pub fn set_parameter(&mut self, id: &str, value: ParameterValue) -> Result<()> {
        let parameter = self
            .parameters()
            .iter()
            .find(|p| p.id == id)
            .with_context(|| format!("unknown parameter: {id}"))?;
        parameter.validate_value(&value)?;
        for pass in &mut self.passes {
            pass.shader.set_parameter(id, value.clone())?;
        }
        Ok(())
    }

    pub(crate) fn validate_limits(&self, limits: &RuntimeLimits) -> Result<()> {
        ensure!(
            self.passes.len() <= limits.max_passes,
            "pass count exceeds runtime limit"
        );
        let mut total = 0;
        let mut targets = 0;
        let mut storage = 0u64;
        for pass in &self.passes {
            if let Some((dispatch, _)) = &pass.compute {
                ensure!(
                    dispatch.iter().map(|n| u64::from(*n)).product::<u64>()
                        <= limits.max_dispatch_groups,
                    "compute dispatch exceeds runtime limit"
                );
            }
        }
        for resource in &self.resources {
            let (width, height) = match resource {
                TextureSource::Buffer { elements } => {
                    storage += u64::from(*elements) * 16;
                    ensure!(
                        storage <= limits.max_storage_bytes,
                        "storage buffers exceed runtime byte limit"
                    );
                    continue;
                }
                TextureSource::Image(index) => self.images[*index].dimensions(),
                TextureSource::Target { width, height } => {
                    targets += 1;
                    (*width, *height)
                }
            };
            limits.check_texture(width, height, &mut total)?;
        }
        ensure!(
            total + storage <= limits.max_resource_bytes,
            "combined GPU resources exceed runtime byte limit"
        );
        ensure!(
            targets <= limits.max_intermediate_targets,
            "intermediate target count exceeds runtime limit"
        );
        Ok(())
    }
}

impl RuntimeLimits {
    pub(crate) fn check_texture(&self, width: u32, height: u32, total: &mut u64) -> Result<()> {
        ensure!(
            width > 0
                && height > 0
                && width <= self.max_texture_dimension
                && height <= self.max_texture_dimension,
            "texture dimensions exceed runtime limit"
        );
        *total = total
            .checked_add(u64::from(width) * u64::from(height) * 4)
            .context("texture memory estimate overflow")?;
        ensure!(
            *total <= self.max_texture_bytes,
            "texture storage exceeds runtime byte limit"
        );
        Ok(())
    }
}
