use anyhow::{Context, Result, ensure};
use wgpu::util::DeviceExt;

use crate::{Globals, Shader, WallpaperImage};

/// Shared GPU context. A host may use the same device for several renderers.
pub struct Gpu {
    pub adapter: wgpu::Adapter,
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
}

impl Gpu {
    pub async fn new(
        instance: &wgpu::Instance,
        surface: Option<&wgpu::Surface<'_>>,
    ) -> Result<Self> {
        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::LowPower,
                compatible_surface: surface,
                force_fallback_adapter: false,
            })
            .await
            .context("no compatible GPU adapter found")?;
        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor {
                label: Some("Wisp GPU"),
                required_features: wgpu::Features::empty(),
                required_limits: wgpu::Limits::downlevel_defaults()
                    .using_resolution(adapter.limits()),
                ..Default::default()
            })
            .await
            .context("could not create Wisp GPU device")?;
        Ok(Self {
            adapter,
            device,
            queue,
        })
    }
}

// Borrowed graphics context also supports host-owned devices (e.g. Iced).
struct DeviceContext<'a> {
    device: &'a wgpu::Device,
    queue: &'a wgpu::Queue,
}

enum Pipeline {
    Fragment(wgpu::RenderPipeline),
    Compute(wgpu::ComputePipeline),
}

/// A prebuilt GPU pass. All persistent resources are allocated at load time.
struct PassRenderer {
    pipeline: Pipeline,
    storage_bind_group: Option<wgpu::BindGroup>,
    globals_buffer: wgpu::Buffer,
    globals_bind_group: wgpu::BindGroup,
    image_bind_group: wgpu::BindGroup,
    parameter_buffer: Option<wgpu::Buffer>,
    parameter_bind_group: Option<wgpu::BindGroup>,
    parameters: Vec<wisp_format::Parameter>,
}

impl PassRenderer {
    async fn new(
        gpu: &DeviceContext<'_>,
        shader: &Shader,
        texture_views: &[&wgpu::TextureView],
        target_format: wgpu::TextureFormat,
        storage: Option<(&wgpu::TextureView, Vec<&wgpu::Buffer>)>,
    ) -> Result<Self> {
        let device = gpu.device;
        validate_bindings(gpu, shader, texture_views.len())?;
        let visibility = if shader.storage.is_some() {
            wgpu::ShaderStages::COMPUTE
        } else {
            wgpu::ShaderStages::FRAGMENT
        };
        device.push_error_scope(wgpu::ErrorFilter::Validation);
        device.push_error_scope(wgpu::ErrorFilter::OutOfMemory);
        let globals_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Wisp globals"),
            contents: bytemuck::bytes_of(&Globals::new(1, 1)),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });
        let globals_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Wisp globals layout"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: wgpu::BufferSize::new(size_of::<Globals>() as u64),
                },
                count: None,
            }],
        });
        let globals_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Wisp globals bindings"),
            layout: &globals_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: globals_buffer.as_entire_binding(),
            }],
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("Wisp image sampler"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            ..Default::default()
        });
        let mut image_layout_entries = Vec::new();
        let mut image_entries = Vec::new();
        for (i, view) in texture_views.iter().enumerate() {
            let binding = i as u32 * 2;
            image_layout_entries.extend([
                wgpu::BindGroupLayoutEntry {
                    binding,
                    visibility,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: binding + 1,
                    visibility,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ]);
            image_entries.extend([
                wgpu::BindGroupEntry {
                    binding,
                    resource: wgpu::BindingResource::TextureView(view),
                },
                wgpu::BindGroupEntry {
                    binding: binding + 1,
                    resource: wgpu::BindingResource::Sampler(&sampler),
                },
            ]);
        }
        let image_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Wisp image layout"),
            entries: &image_layout_entries,
        });
        let image_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Wisp image bindings"),
            layout: &image_layout,
            entries: &image_entries,
        });
        let parameters = shader.parameters().to_vec();
        let mut parameter_buffer = None;
        let mut parameter_layout = None;
        let mut parameter_bind_group = None;
        if !parameters.is_empty() {
            let bytes: Vec<u8> = parameters
                .iter()
                .flat_map(|p| crate::parameters::encode(&p.default))
                .collect();
            let buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("Wisp parameters"),
                contents: &bytes,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            });
            let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("Wisp parameter layout"),
                entries: &[wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: wgpu::BufferSize::new(bytes.len() as u64),
                    },
                    count: None,
                }],
            });
            parameter_bind_group = Some(device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("Wisp parameter bindings"),
                layout: &layout,
                entries: &[wgpu::BindGroupEntry {
                    binding: 0,
                    resource: buffer.as_entire_binding(),
                }],
            }));
            parameter_buffer = Some(buffer);
            parameter_layout = Some(layout);
        }
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Wisp wallpaper shader"),
            source: wgpu::ShaderSource::Wgsl(shader.source().into()),
        });
        let mut layouts = vec![&globals_layout, &image_layout];
        let empty_parameters = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Wisp empty parameters"),
            entries: &[],
        });
        if storage.is_some() && parameter_layout.is_none() {
            parameter_bind_group = Some(device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("Wisp empty parameters"),
                layout: &empty_parameters,
                entries: &[],
            }));
        }
        if parameter_layout.is_some() || storage.is_some() {
            layouts.push(parameter_layout.as_ref().unwrap_or(&empty_parameters));
        }
        let mut storage_layout = None;
        let mut storage_bind_group = None;
        if let Some((output, buffers)) = storage {
            let mut entries = vec![wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility,
                ty: wgpu::BindingType::StorageTexture {
                    access: wgpu::StorageTextureAccess::WriteOnly,
                    format: wgpu::TextureFormat::Rgba8Unorm,
                    view_dimension: wgpu::TextureViewDimension::D2,
                },
                count: None,
            }];
            let mut bindings = vec![wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(output),
            }];
            for (i, buffer) in buffers.iter().enumerate() {
                entries.push(wgpu::BindGroupLayoutEntry {
                    binding: i as u32 + 1,
                    visibility,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage {
                            read_only: !shader.storage.as_ref().unwrap()[i],
                        },
                        has_dynamic_offset: false,
                        min_binding_size: wgpu::BufferSize::new(buffer.size()),
                    },
                    count: None,
                });
                bindings.push(wgpu::BindGroupEntry {
                    binding: i as u32 + 1,
                    resource: buffer.as_entire_binding(),
                });
            }
            let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("Wisp compute storage"),
                entries: &entries,
            });
            storage_bind_group = Some(device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("Wisp compute storage"),
                layout: &layout,
                entries: &bindings,
            }));
            storage_layout = Some(layout);
        }
        layouts.extend(storage_layout.as_ref());
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Wisp pass layout"),
            bind_group_layouts: &layouts,
            push_constant_ranges: &[],
        });
        let pipeline = if shader.storage.is_some() {
            Pipeline::Compute(
                device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                    label: Some("Wisp compute pass"),
                    layout: Some(&layout),
                    module: &module,
                    entry_point: Some("cs_main"),
                    compilation_options: Default::default(),
                    cache: None,
                }),
            )
        } else {
            Pipeline::Fragment(
                device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                    label: Some("Wisp fullscreen pass"),
                    layout: Some(&layout),
                    vertex: wgpu::VertexState {
                        module: &module,
                        entry_point: Some("vs_main"),
                        compilation_options: Default::default(),
                        buffers: &[],
                    },
                    primitive: Default::default(),
                    depth_stencil: None,
                    multisample: Default::default(),
                    fragment: Some(wgpu::FragmentState {
                        module: &module,
                        entry_point: Some("fs_main"),
                        compilation_options: Default::default(),
                        targets: &[Some(wgpu::ColorTargetState {
                            format: target_format,
                            blend: None,
                            write_mask: wgpu::ColorWrites::ALL,
                        })],
                    }),
                    multiview: None,
                    cache: None,
                }),
            )
        };
        let allocation_error = device.pop_error_scope().await;
        let validation_error = device.pop_error_scope().await;
        if let Some(error) = allocation_error.or(validation_error) {
            anyhow::bail!("could not create wallpaper pipeline: {error}");
        }
        Ok(Self {
            pipeline,
            storage_bind_group,
            globals_buffer,
            globals_bind_group,
            image_bind_group,
            parameter_buffer,
            parameter_bind_group,
            parameters,
        })
    }

    /// Update one uniform slot. Submit previously encoded frames before calling.
    /// Invalid changes leave the current value and GPU buffer untouched.
    pub fn set_parameter(
        &mut self,
        queue: &wgpu::Queue,
        id: &str,
        value: wisp_format::ParameterValue,
    ) -> Result<()> {
        let (index, parameter) = self
            .parameters
            .iter_mut()
            .enumerate()
            .find(|(_, p)| p.id == id)
            .with_context(|| format!("unknown parameter: {id}"))?;
        parameter.validate_value(&value)?;
        queue.write_buffer(
            self.parameter_buffer.as_ref().expect("parameter buffer"),
            index as u64 * 16,
            &crate::parameters::encode(&value),
        );
        parameter.default = value;
        Ok(())
    }

    /// Encode one draw into a host-owned target matching the format passed to `new`.
    /// The host submits this encoder before updating globals for another frame.
    pub fn render(
        &self,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        target: &wgpu::TextureView,
        globals: &Globals,
        dispatch: Option<[u32; 3]>,
    ) {
        queue.write_buffer(&self.globals_buffer, 0, bytemuck::bytes_of(globals));
        if let Pipeline::Compute(pipeline) = &self.pipeline {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("Wisp compute pass"),
                timestamp_writes: None,
            });
            pass.set_pipeline(pipeline);
            pass.set_bind_group(0, &self.globals_bind_group, &[]);
            pass.set_bind_group(1, &self.image_bind_group, &[]);
            pass.set_bind_group(2, self.parameter_bind_group.as_ref().unwrap(), &[]);
            pass.set_bind_group(3, self.storage_bind_group.as_ref().unwrap(), &[]);
            let [x, y, z] = dispatch.expect("compute dispatch");
            pass.dispatch_workgroups(x, y, z);
            return;
        }
        let Pipeline::Fragment(pipeline) = &self.pipeline else {
            unreachable!()
        };
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("Wisp wallpaper pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: target,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                    store: wgpu::StoreOp::Store,
                },
            })],
            ..Default::default()
        });
        pass.set_pipeline(pipeline);
        pass.set_bind_group(0, &self.globals_bind_group, &[]);
        pass.set_bind_group(1, &self.image_bind_group, &[]);
        if let Some(bind_group) = &self.parameter_bind_group {
            pass.set_bind_group(2, bind_group, &[]);
        }
        pass.draw(0..3, 0..1);
    }
}

struct PreparedPass {
    renderer: PassRenderer,
    target: Option<(wgpu::TextureView, [u32; 2])>,
    dispatch: Option<[u32; 3]>,
}

/// Prebuilt ordered GPU passes. Persistent textures and bindings are allocated at load.
/// Intermediate targets have package-defined dimensions and survive output resizing.
pub struct Renderer {
    passes: Vec<PreparedPass>,
    target_format: wgpu::TextureFormat,
}

impl Renderer {
    pub async fn new(
        gpu: &Gpu,
        shader: &Shader,
        image: &WallpaperImage,
        target_format: wgpu::TextureFormat,
    ) -> Result<Self> {
        Self::with_images(gpu, shader, std::slice::from_ref(image), target_format).await
    }

    pub async fn with_images(
        gpu: &Gpu,
        shader: &Shader,
        images: &[WallpaperImage],
        target_format: wgpu::TextureFormat,
    ) -> Result<Self> {
        let gpu = &DeviceContext {
            device: &gpu.device,
            queue: &gpu.queue,
        };
        validate_format(target_format)?;
        validate_bindings(gpu, shader, images.len())?;
        let limits = device_limits(gpu, &crate::RuntimeLimits::default());
        let mut total = 0;
        for image in images {
            let (width, height) = image.dimensions();
            limits.check_texture(width, height, &mut total)?;
        }
        let device = gpu.device;
        device.push_error_scope(wgpu::ErrorFilter::Validation);
        device.push_error_scope(wgpu::ErrorFilter::OutOfMemory);
        let views: Vec<_> = images
            .iter()
            .map(|image| upload_image(gpu, image))
            .collect();
        finish_resource_scope(device).await?;
        let views: Vec<_> = views.iter().collect();
        let renderer = PassRenderer::new(gpu, shader, &views, target_format, None).await?;
        Ok(Self {
            passes: vec![PreparedPass {
                renderer,
                target: None,
                dispatch: None,
            }],
            target_format,
        })
    }

    pub async fn from_wallpaper(
        gpu: &Gpu,
        wallpaper: &crate::Wallpaper,
        target_format: wgpu::TextureFormat,
    ) -> Result<Self> {
        Self::from_wallpaper_with_limits(
            gpu,
            wallpaper,
            target_format,
            &crate::RuntimeLimits::default(),
        )
        .await
    }

    pub async fn from_wallpaper_with_limits(
        gpu: &Gpu,
        wallpaper: &crate::Wallpaper,
        target_format: wgpu::TextureFormat,
        limits: &crate::RuntimeLimits,
    ) -> Result<Self> {
        if wallpaper.passes.iter().any(|p| p.compute.is_some()) {
            ensure!(
                gpu.adapter
                    .get_downlevel_capabilities()
                    .flags
                    .contains(wgpu::DownlevelFlags::COMPUTE_SHADERS),
                "GPU does not support Wisp compute profile"
            );
        }
        Self::from_device(&gpu.device, &gpu.queue, wallpaper, target_format, limits).await
    }

    /// Prepare the same runtime on a host-owned device. All device/resource limits
    /// and GPU validation scopes apply; the host must enable compute limits to use compute wallpapers.
    pub async fn from_device(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        wallpaper: &crate::Wallpaper,
        target_format: wgpu::TextureFormat,
        limits: &crate::RuntimeLimits,
    ) -> Result<Self> {
        let gpu = &DeviceContext { device, queue };
        use crate::wallpaper::TextureSource;
        validate_format(target_format)?;
        wallpaper.validate_limits(&device_limits(gpu, limits))?;
        for pass in &wallpaper.passes {
            validate_bindings(gpu, &pass.shader, pass.inputs.len())?;
        }
        let device = gpu.device;
        let has_compute = wallpaper.passes.iter().any(|p| p.compute.is_some());
        if has_compute {
            let limits = device.limits();
            ensure!(
                limits.max_compute_invocations_per_workgroup >= 64
                    && limits.max_compute_workgroup_size_x >= 8
                    && limits.max_compute_workgroup_size_y >= 8
                    && limits.max_storage_textures_per_shader_stage >= 1,
                "GPU does not support Wisp compute profile"
            );
            for pass in &wallpaper.passes {
                if let Some((dispatch, buffers)) = &pass.compute {
                    ensure!(
                        dispatch
                            .iter()
                            .all(|n| *n <= limits.max_compute_workgroups_per_dimension)
                            && buffers.len()
                                <= limits.max_storage_buffers_per_shader_stage as usize,
                        "compute pass exceeds GPU limits"
                    );
                }
            }
            for resource in &wallpaper.resources {
                if let TextureSource::Buffer { elements } = resource {
                    let size = u64::from(*elements) * 16;
                    ensure!(
                        size <= limits.max_buffer_size
                            && size <= u64::from(limits.max_storage_buffer_binding_size),
                        "storage buffer exceeds GPU limits"
                    );
                }
            }
        }
        device.push_error_scope(wgpu::ErrorFilter::Validation);
        device.push_error_scope(wgpu::ErrorFilter::OutOfMemory);
        let buffers: Vec<_> = wallpaper
            .resources
            .iter()
            .map(|resource| match resource {
                TextureSource::Buffer { elements } => {
                    Some(device.create_buffer(&wgpu::BufferDescriptor {
                        label: Some("Wisp storage buffer"),
                        size: u64::from(*elements) * 16,
                        usage: wgpu::BufferUsages::STORAGE,
                        mapped_at_creation: false,
                    }))
                }
                _ => None,
            })
            .collect();
        let views: Vec<_> = wallpaper
            .resources
            .iter()
            .map(|resource| match resource {
                TextureSource::Buffer { .. } => None,
                TextureSource::Image(index) => Some(upload_image(gpu, &wallpaper.images[*index])),
                TextureSource::Target { width, height } => Some(
                    device
                        .create_texture(&wgpu::TextureDescriptor {
                            label: Some("Wisp intermediate target"),
                            size: wgpu::Extent3d {
                                width: *width,
                                height: *height,
                                depth_or_array_layers: 1,
                            },
                            mip_level_count: 1,
                            sample_count: 1,
                            dimension: wgpu::TextureDimension::D2,
                            // Linear 8-bit storage: sampling does not apply an sRGB conversion.
                            format: wgpu::TextureFormat::Rgba8Unorm,
                            usage: wgpu::TextureUsages::TEXTURE_BINDING
                                | wgpu::TextureUsages::RENDER_ATTACHMENT
                                | if has_compute {
                                    wgpu::TextureUsages::STORAGE_BINDING
                                } else {
                                    wgpu::TextureUsages::empty()
                                },
                            view_formats: &[],
                        })
                        .create_view(&Default::default()),
                ),
            })
            .collect();
        finish_resource_scope(device).await?;
        let mut passes = Vec::with_capacity(wallpaper.passes.len());
        for pass in &wallpaper.passes {
            let inputs: Vec<_> = pass
                .inputs
                .iter()
                .map(|index| views[*index].as_ref().unwrap())
                .collect();
            let target = pass.output.map(|index| {
                let TextureSource::Target { width, height } = wallpaper.resources[index] else {
                    unreachable!("validated output")
                };
                (views[index].as_ref().unwrap().clone(), [width, height])
            });
            let format = if target.is_some() {
                wgpu::TextureFormat::Rgba8Unorm
            } else {
                target_format
            };
            let storage = pass.compute.as_ref().map(|(_, indices)| {
                (
                    &target.as_ref().unwrap().0,
                    indices
                        .iter()
                        .map(|index| buffers[*index].as_ref().unwrap())
                        .collect(),
                )
            });
            let renderer = PassRenderer::new(gpu, &pass.shader, &inputs, format, storage)
                .await
                .with_context(|| format!("could not prepare pass {}", passes.len()))?;
            passes.push(PreparedPass {
                renderer,
                target,
                dispatch: pass.compute.as_ref().map(|(d, _)| *d),
            });
        }
        Ok(Self {
            passes,
            target_format,
        })
    }

    /// Updates the same parameter slot in every pass; invalid values change nothing.
    /// Submit previously encoded frames before updating parameters or globals.
    pub fn set_parameter(
        &mut self,
        queue: &wgpu::Queue,
        id: &str,
        value: wisp_format::ParameterValue,
    ) -> Result<()> {
        // All passes have identical parameter declarations, enforced by Wallpaper.
        for pass in &mut self.passes {
            pass.renderer.set_parameter(queue, id, value.clone())?;
        }
        Ok(())
    }

    /// Execute passes in package order. Resolution describes each pass's output;
    /// other globals are identical across passes. The host submits before another frame.
    pub fn render(
        &self,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        target: &wgpu::TextureView,
        globals: &Globals,
    ) {
        for pass in &self.passes {
            let mut pass_globals = *globals;
            let output = if let Some((view, size)) = &pass.target {
                pass_globals.resolution = [size[0] as f32, size[1] as f32];
                view
            } else {
                target
            };
            pass.renderer
                .render(queue, encoder, output, &pass_globals, pass.dispatch);
        }
    }

    /// Render and read back a frame for previews and diagnostics, not the live loop.
    pub fn render_rgba(
        &self,
        gpu: &Gpu,
        width: u32,
        height: u32,
        globals: &Globals,
    ) -> Result<Vec<u8>> {
        ensure!(
            self.target_format == wgpu::TextureFormat::Rgba8UnormSrgb,
            "readback requires an RGBA sRGB renderer"
        );
        ensure!(
            width > 0 && height > 0 && width <= 4096 && height <= 4096,
            "snapshot dimensions must be between 1 and 4096"
        );
        ensure!(
            width <= gpu.device.limits().max_texture_dimension_2d
                && height <= gpu.device.limits().max_texture_dimension_2d,
            "snapshot exceeds GPU texture dimensions"
        );
        let texture = gpu.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("Wisp snapshot"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: self.target_format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let padded_row = (width * 4).div_ceil(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT)
            * wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
        let readback = gpu.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Wisp snapshot readback"),
            size: u64::from(padded_row) * u64::from(height),
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut encoder = gpu.device.create_command_encoder(&Default::default());
        let mut globals = *globals;
        globals.resolution = [width as f32, height as f32];
        self.render(
            &gpu.queue,
            &mut encoder,
            &texture.create_view(&Default::default()),
            &globals,
        );
        encoder.copy_texture_to_buffer(
            texture.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &readback,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(padded_row),
                    rows_per_image: Some(height),
                },
            },
            texture.size(),
        );
        gpu.queue.submit([encoder.finish()]);
        let slice = readback.slice(..);
        let (send, receive) = std::sync::mpsc::sync_channel(1);
        slice.map_async(wgpu::MapMode::Read, move |result| {
            let _ = send.send(result);
        });
        gpu.device.poll(wgpu::PollType::Wait {
            submission_index: None,
            timeout: Some(std::time::Duration::from_secs(30)),
        })?;
        receive
            .recv_timeout(std::time::Duration::from_secs(30))
            .context("GPU readback timed out")??;
        let mapped = slice.get_mapped_range();
        let mut pixels = Vec::with_capacity((width * height * 4) as usize);
        for row in mapped.chunks_exact(padded_row as usize) {
            pixels.extend_from_slice(&row[..(width * 4) as usize]);
        }
        drop(mapped);
        readback.unmap();
        Ok(pixels)
    }
}

fn validate_format(format: wgpu::TextureFormat) -> Result<()> {
    ensure!(
        matches!(
            format,
            wgpu::TextureFormat::Rgba8UnormSrgb | wgpu::TextureFormat::Bgra8UnormSrgb
        ),
        "Wisp requires an 8-bit sRGB output target"
    );
    Ok(())
}

fn validate_bindings(gpu: &DeviceContext<'_>, shader: &Shader, count: usize) -> Result<()> {
    ensure!(
        count == shader.image_count(),
        "image count does not match shader bindings"
    );
    ensure!(
        count <= gpu.device.limits().max_sampled_textures_per_shader_stage as usize
            && count <= gpu.device.limits().max_samplers_per_shader_stage as usize,
        "image bindings exceed GPU limits"
    );
    Ok(())
}

fn device_limits(gpu: &DeviceContext<'_>, limits: &crate::RuntimeLimits) -> crate::RuntimeLimits {
    crate::RuntimeLimits {
        max_texture_dimension: limits
            .max_texture_dimension
            .min(gpu.device.limits().max_texture_dimension_2d),
        ..limits.clone()
    }
}

async fn finish_resource_scope(device: &wgpu::Device) -> Result<()> {
    let allocation = device.pop_error_scope().await;
    let validation = device.pop_error_scope().await;
    if let Some(error) = allocation.or(validation) {
        anyhow::bail!("could not allocate wallpaper textures: {error}");
    }
    Ok(())
}

fn upload_image(gpu: &DeviceContext<'_>, image: &WallpaperImage) -> wgpu::TextureView {
    let (width, height) = image.dimensions();
    let texture = gpu.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("Wisp image"),
        size: wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8UnormSrgb,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    gpu.queue.write_texture(
        texture.as_image_copy(),
        image.pixels.as_raw(),
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(width * 4),
            rows_per_image: Some(height),
        },
        texture.size(),
    );
    texture.create_view(&Default::default())
}
