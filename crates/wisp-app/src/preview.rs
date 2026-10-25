//! Iced only composites a texture; wallpaper rendering uses the desktop runtime.
use iced::{
    ContentFit, Element, Length, Rectangle, Size, mouse,
    widget::shader::{self, Viewport},
    widget::{Shader, container, responsive},
};
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
};
use wisp_format::ParameterValue;
use wisp_runtime::{Globals, Renderer, Wallpaper};

#[derive(Default, Debug)]
pub struct Status {
    pub frames: u64,
    pub rendered_generation: Option<u64>,
    pub error: Option<String>,
}
#[derive(Clone)]
pub struct Preview {
    pub generation: u64,
    pub wallpaper: Arc<Wallpaper>,
    pub parameters: Arc<BTreeMap<String, ParameterValue>>,
    pub globals: Globals,
    pub status: Arc<Mutex<Status>>,
}
impl Preview {
    /// Use the source aspect ratio so opening a panel never stretches the image.
    pub fn canvas<Message: 'static>(&self) -> Element<'static, Message> {
        let dimensions = self
            .wallpaper
            .images()
            .first()
            .map(|image| image.dimensions())
            .unwrap_or((16, 9));
        fitted(self.clone(), dimensions)
    }

    pub fn view<Message: 'static>(&self, max_height: f32) -> Element<'static, Message> {
        container(self.canvas())
            .width(Length::Fill)
            .height(max_height)
            .into()
    }
}

/// Keep authoring dimensions explicit because layer assets can have different sizes.
pub(crate) fn fitted<'a, Message: 'a>(
    preview: Preview,
    dimensions: (u32, u32),
) -> Element<'a, Message> {
    let source = Size::new(dimensions.0.max(1) as f32, dimensions.1.max(1) as f32);
    responsive(move |available| {
        let size = ContentFit::Contain.fit(source, available);
        container(
            Shader::new(preview.clone())
                .width(size.width)
                .height(size.height),
        )
        .center(Length::Fill)
        .into()
    })
    .into()
}

pub struct Primitive {
    preview: Preview,
    mouse: [f32; 2],
}
impl std::fmt::Debug for Primitive {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WispPreview")
            .field("generation", &self.preview.generation)
            .finish()
    }
}
impl<Message> shader::Program<Message> for Preview {
    type State = ();
    type Primitive = Primitive;
    fn draw(&self, _: &(), cursor: mouse::Cursor, bounds: Rectangle) -> Primitive {
        let mouse = cursor
            .position_in(bounds)
            .map(|p| {
                [
                    (p.x / bounds.width.max(1.0)).clamp(0.0, 1.0),
                    (p.y / bounds.height.max(1.0)).clamp(0.0, 1.0),
                ]
            })
            .unwrap_or([0.5, 0.5]);
        Primitive {
            preview: self.clone(),
            mouse,
        }
    }
}
pub struct Pipeline {
    renderer: Option<Renderer>,
    attempted: Option<u64>,
    rendered_generation: Option<u64>,
    runtime_limits: wisp_runtime::RuntimeLimits,
    parameters: BTreeMap<String, ParameterValue>,
    texture: Option<(wgpu::TextureView, wgpu::BindGroup, [u32; 2])>,
    blit: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    queue: wgpu::Queue,
    bounds: Rectangle,
    globals: Globals,
    needs_frame: bool,
    uv_buffer: wgpu::Buffer,
}
impl shader::Pipeline for Pipeline {
    fn new(device: &wgpu::Device, queue: &wgpu::Queue, format: wgpu::TextureFormat) -> Self {
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Wisp preview composite"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::VERTEX,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });
        let uv_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Wisp preview crop"),
            size: 16,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Wisp preview composite"),
            source: wgpu::ShaderSource::Wgsl(include_str!("preview.wgsl").into()),
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: None,
            bind_group_layouts: &[&layout],
            push_constant_ranges: &[],
        });
        let blit = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("Wisp preview composite"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vertex"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fragment"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: Default::default(),
            depth_stencil: None,
            multisample: Default::default(),
            multiview: None,
            cache: None,
        });
        Self {
            renderer: None,
            attempted: None,
            rendered_generation: None,
            runtime_limits: Default::default(),
            parameters: BTreeMap::new(),
            texture: None,
            blit,
            layout,
            sampler,
            queue: queue.clone(),
            bounds: Rectangle::default(),
            globals: Globals::new(1, 1),
            needs_frame: true,
            uv_buffer,
        }
    }
}
impl shader::Primitive for Primitive {
    type Pipeline = Pipeline;
    fn prepare(
        &self,
        pipeline: &mut Pipeline,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        bounds: &Rectangle,
        viewport: &Viewport,
    ) {
        let preview = &self.preview;
        let advancing = pipeline.globals.frame != preview.globals.frame
            || pipeline.attempted != Some(preview.generation);
        pipeline.needs_frame = advancing;
        if pipeline.attempted != Some(preview.generation) {
            pipeline.attempted = Some(preview.generation);
            // Keeping the last valid renderer leaves a usable preview when replacement allocation fails.
            match pollster::block_on(Renderer::from_device(
                device,
                queue,
                &preview.wallpaper,
                wgpu::TextureFormat::Rgba8UnormSrgb,
                &pipeline.runtime_limits,
            )) {
                Ok(renderer) => {
                    pipeline.renderer = Some(renderer);
                    pipeline.rendered_generation = Some(preview.generation);
                    pipeline.needs_frame = true;
                    pipeline.parameters.clear();
                    preview.status.lock().unwrap().error = None;
                }
                Err(error) => {
                    pipeline.needs_frame = false;
                    preview.status.lock().unwrap().error =
                        Some(format!("Preview could not load: {error:#}"))
                }
            }
        }
        if preview.status.lock().unwrap().error.is_some() {
            pipeline.needs_frame = false;
        }
        if preview.status.lock().unwrap().error.is_none()
            && pipeline.parameters != *preview.parameters
            && let Some(renderer) = &mut pipeline.renderer
        {
            pipeline.needs_frame = true;
            for (id, value) in preview.parameters.iter() {
                if pipeline.parameters.get(id) != Some(value)
                    && let Err(error) = renderer.set_parameter(queue, id, value.clone())
                {
                    preview.status.lock().unwrap().error = Some(format!("{error:#}"));
                }
            }
            pipeline.parameters = (*preview.parameters).clone();
        }
        let scale = viewport.scale_factor();
        pipeline.bounds = Rectangle {
            x: bounds.x * scale,
            y: bounds.y * scale,
            width: bounds.width * scale,
            height: bounds.height * scale,
        };
        let max = device.limits().max_texture_dimension_2d.min(4096);
        let size = [
            (bounds.width * scale).round().clamp(1.0, max as f32) as u32,
            (bounds.height * scale).round().clamp(1.0, max as f32) as u32,
        ];
        if pipeline.texture.as_ref().is_none_or(|t| t.2 != size) {
            pipeline.needs_frame = true;
            let texture = device.create_texture(&wgpu::TextureDescriptor {
                label: Some("Wisp live preview"),
                size: wgpu::Extent3d {
                    width: size[0],
                    height: size[1],
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba8UnormSrgb,
                usage: wgpu::TextureUsages::TEXTURE_BINDING
                    | wgpu::TextureUsages::RENDER_ATTACHMENT,
                view_formats: &[],
            });
            let view = texture.create_view(&Default::default());
            let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: None,
                layout: &pipeline.layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureView(&view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::Sampler(&pipeline.sampler),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: pipeline.uv_buffer.as_entire_binding(),
                    },
                ],
            });
            pipeline.texture = Some((view, group, size));
        }
        let bounds = pipeline.bounds;
        let screen = Rectangle {
            x: 0.0,
            y: 0.0,
            width: viewport.physical_size().width as f32,
            height: viewport.physical_size().height as f32,
        };
        pipeline.bounds = bounds.intersection(&screen).unwrap_or_default();
        let visible = pipeline.bounds;
        let uv = [
            (visible.x - bounds.x) / bounds.width.max(1.0),
            (visible.y - bounds.y) / bounds.height.max(1.0),
            visible.width / bounds.width.max(1.0),
            visible.height / bounds.height.max(1.0),
        ];
        let mut bytes = [0u8; 16];
        for (chunk, value) in bytes.chunks_exact_mut(4).zip(uv) {
            chunk.copy_from_slice(&value.to_ne_bytes());
        }
        queue.write_buffer(&pipeline.uv_buffer, 0, &bytes);
        pipeline.globals = preview.globals;
        if !advancing {
            pipeline.globals.delta_time = 0.;
        }
        pipeline.globals.resolution = [size[0] as f32, size[1] as f32];
        pipeline.globals.mouse = self.mouse;
    }
    fn render(
        &self,
        pipeline: &Pipeline,
        encoder: &mut wgpu::CommandEncoder,
        target: &wgpu::TextureView,
        clip: &Rectangle<u32>,
    ) {
        let (Some(renderer), Some((view, group, _))) = (&pipeline.renderer, &pipeline.texture)
        else {
            return;
        };
        if clip.width == 0
            || clip.height == 0
            || pipeline.bounds.width <= 0.0
            || pipeline.bounds.height <= 0.0
        {
            return;
        }
        if pipeline.needs_frame {
            renderer.render(&pipeline.queue, encoder, view, &pipeline.globals);
            let mut status = self.preview.status.lock().unwrap();
            if pipeline.rendered_generation == Some(self.preview.generation)
                && status.error.is_none()
            {
                if status.rendered_generation != pipeline.rendered_generation {
                    status.frames = 0;
                }
                status.frames += 1;
                status.rendered_generation = pipeline.rendered_generation;
            }
        }
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("Wisp preview composite"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: target,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Load,
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
        });
        pass.set_pipeline(&pipeline.blit);
        let b = pipeline.bounds;
        pass.set_viewport(b.x, b.y, b.width, b.height, 0.0, 1.0);
        pass.set_scissor_rect(clip.x, clip.y, clip.width, clip.height);
        pass.set_bind_group(0, group, &[]);
        pass.draw(0..3, 0..1);
    }
}

#[cfg(test)]
#[path = "preview_tests.rs"]
mod tests;
