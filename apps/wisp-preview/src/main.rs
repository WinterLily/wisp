use std::{
    fs::File,
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, Instant},
};

use anyhow::{Context, Result, ensure};
use clap::Parser;
use winit::{
    application::ApplicationHandler,
    dpi::PhysicalSize,
    event::{ElementState, WindowEvent},
    event_loop::{ActiveEventLoop, ControlFlow, EventLoop},
    keyboard::{Key, NamedKey},
    window::{Window, WindowId},
};
use wisp_format::{Manifest, Output, Package, PackageLimits, Pass, Program, Resource};
use wisp_runtime::{
    EXAMPLE_IMAGE, EXAMPLE_SHADER, Globals, Gpu, Renderer, Shader, Wallpaper, WallpaperImage,
};

#[derive(Debug, Parser)]
#[command(
    version,
    about = "Wisp GPU wallpaper preview and portable package tool"
)]
struct Args {
    /// Capture system output audio in the preview window
    #[arg(long, conflicts_with_all = ["check", "export", "output"])]
    audio: bool,
    #[arg(long, requires = "audio")]
    audio_device: Option<String>,
    /// Snapshot bass, mid, treble, volume (comma separated, each 0..=1)
    #[arg(long, requires = "output", value_delimiter = ',', num_args = 1)]
    audio_bands: Vec<f32>,
    /// Advance simulation at 60 Hz before a snapshot
    #[arg(long, requires = "output", default_value_t = 0, value_parser = clap::value_parser!(u32).range(0..=3600))]
    warmup: u32,
    /// Portable .wisp package to validate or render
    #[arg(long, conflicts_with_all = ["shader", "image", "export"])]
    package: Option<PathBuf>,
    /// Override a package parameter: --param speed=0.5 (vectors/colors use JSON arrays)
    #[arg(long = "param", requires = "package")]
    parameters: Vec<String>,
    /// Export the shader and image to a new .wisp package without requesting a GPU
    #[arg(long, conflicts_with_all = ["check", "output", "frames", "time"])]
    export: Option<PathBuf>,
    /// Wallpaper identifier for export (default: local.wallpaper)
    #[arg(long, requires = "export")]
    id: Option<String>,
    /// Wallpaper display name for export (default: Wisp Wallpaper)
    #[arg(long, requires = "export")]
    name: Option<String>,
    /// Wallpaper author for export (default: Local Author)
    #[arg(long, requires = "export")]
    author: Option<String>,
    /// WGSL file defining fn wallpaper(uv: vec2<f32>) -> vec4<f32>
    #[arg(long)]
    shader: Option<PathBuf>,
    /// Background texture (PNG, JPEG or WebP)
    #[arg(long)]
    image: Option<PathBuf>,
    #[arg(long, default_value_t = 1280, value_parser = clap::value_parser!(u32).range(1..=4096))]
    width: u32,
    #[arg(long, default_value_t = 720, value_parser = clap::value_parser!(u32).range(1..=4096))]
    height: u32,
    /// Maximum preview frame rate
    #[arg(long, default_value_t = 60, value_parser = clap::value_parser!(u32).range(1..=240))]
    fps: u32,
    /// Validate the package or shader/image without requesting a GPU
    #[arg(long, conflicts_with_all = ["output", "frames", "time"])]
    check: bool,
    /// Render one frame to a PNG without opening a window
    #[arg(long, conflicts_with = "frames")]
    output: Option<PathBuf>,
    /// Animation time in seconds for a snapshot (defaults to zero)
    #[arg(long, requires = "output")]
    time: Option<f32>,
    /// Exit the window after this many presented frames (for smoke testing)
    #[arg(long, value_parser = clap::value_parser!(u64).range(1..))]
    frames: Option<u64>,
}

fn read_bounded(path: &Path, limit: u64) -> Result<Vec<u8>> {
    let mut data = Vec::new();
    File::open(path)
        .with_context(|| format!("could not open {}", path.display()))?
        .take(limit + 1)
        .read_to_end(&mut data)?;
    ensure!(
        data.len() as u64 <= limit,
        "{} exceeds its {} byte input limit",
        path.display(),
        limit
    );
    Ok(data)
}

fn main() -> Result<()> {
    let args = Args::parse();
    ensure!(
        args.time.is_none_or(|time| time.is_finite() && time >= 0.0),
        "snapshot time must be finite and nonnegative"
    );
    ensure!(
        (args.audio_bands.is_empty() || args.audio_bands.len() == 4)
            && args
                .audio_bands
                .iter()
                .all(|v| v.is_finite() && (0.0..=1.0).contains(v)),
        "provide four finite audio bands, each 0..=1"
    );
    let mut wallpaper = if let Some(path) = &args.package {
        let package = Package::load(path, &PackageLimits::default())?;
        let wallpaper = Wallpaper::from_package(&package)?;
        eprintln!(
            "Wisp: {} by {} ({})",
            package.manifest().name,
            package.manifest().author,
            package.manifest().id
        );
        wallpaper
    } else {
        let source = match &args.shader {
            Some(path) => String::from_utf8(read_bounded(path, 1024 * 1024)?)
                .context("shader is not UTF-8")?,
            None => EXAMPLE_SHADER.to_owned(),
        };
        let shader = Shader::new(&source).context("wallpaper shader validation failed")?;
        let image_bytes = match &args.image {
            Some(path) => read_bounded(path, 32 * 1024 * 1024)?,
            None => EXAMPLE_IMAGE.to_vec(),
        };
        let image = WallpaperImage::decode(&image_bytes)?;
        if let Some(path) = &args.export {
            export(&args, path, source, image_bytes)?;
            return Ok(());
        }
        Wallpaper::new(shader, image)?
    };
    for assignment in &args.parameters {
        let (id, text) = assignment
            .split_once('=')
            .context("parameter override must be ID=VALUE")?;
        let parameter = wallpaper
            .parameters()
            .iter()
            .find(|p| p.id == id)
            .with_context(|| format!("unknown parameter: {id}"))?;
        use wisp_format::ParameterValue::*;
        let value = match parameter.default {
            Float(_) => Float(serde_json::from_str(text)?),
            Integer(_) => Integer(serde_json::from_str(text)?),
            Boolean(_) => Boolean(serde_json::from_str(text)?),
            Color(_) => Color(serde_json::from_str(text)?),
            Vector2(_) => Vector2(serde_json::from_str(text)?),
            Vector3(_) => Vector3(serde_json::from_str(text)?),
            Vector4(_) => Vector4(serde_json::from_str(text)?),
        };
        wallpaper.set_parameter(id, value)?;
    }
    if args.check {
        println!(
            "Valid: fullscreen WGSL wallpaper, {} image(s), {} parameter(s), {} pass(es)",
            wallpaper.images().len(),
            wallpaper.parameters().len(),
            wallpaper.pass_count()
        );
        return Ok(());
    }
    if let Some(path) = &args.output {
        return pollster::block_on(snapshot(&args, path, &wallpaper));
    }
    let event_loop = EventLoop::new().context("could not connect to the window system")?;
    let mut app = Preview {
        args,
        wallpaper,
        state: None,
        error: None,
    };
    event_loop.run_app(&mut app)?;
    if let Some(error) = app.error {
        return Err(error);
    }
    Ok(())
}

fn export(args: &Args, path: &Path, source: String, image_bytes: Vec<u8>) -> Result<()> {
    let extension = match image::guess_format(&image_bytes)? {
        image::ImageFormat::Png => "png",
        image::ImageFormat::Jpeg => "jpg",
        image::ImageFormat::WebP => "webp",
        _ => anyhow::bail!("unsupported image format"),
    };
    let image_path = format!("assets/background.{extension}");
    let shader_path = "shaders/pass_0.wgsl".to_owned();
    let limits = PackageLimits::default();
    let package = Package::build(
        Manifest {
            format_version: wisp_format::FORMAT_VERSION,
            id: args.id.clone().unwrap_or_else(|| "local.wallpaper".into()),
            name: args.name.clone().unwrap_or_else(|| "Wisp Wallpaper".into()),
            author: args.author.clone().unwrap_or_else(|| "Local Author".into()),
            description: String::new(),
            preview: Some(image_path.clone()),
            program: "program.json".into(),
            minimum_wisp_version: env!("CARGO_PKG_VERSION").into(),
            gpu_profile: wisp_format::GPU_PROFILE,
            tags: Vec::new(),
        },
        Program {
            version: 1,
            shader_abi_version: 1,
            parameters: vec![],
            resources: vec![Resource::Image {
                id: "background".into(),
                path: image_path.clone(),
            }],
            passes: vec![Pass::Fragment {
                shader: shader_path.clone(),
                background: "background".into(),
                images: vec![],
                output: Output::Presentation,
            }],
        },
        [
            (shader_path, source.into_bytes()),
            (image_path, image_bytes),
        ]
        .into(),
        &limits,
    )?;
    let bytes = package.to_bytes(&limits)?;
    let mut file = File::create_new(path).with_context(|| {
        format!(
            "could not create {} (export requires a new file)",
            path.display()
        )
    })?;
    if let Err(error) = file.write_all(&bytes).and_then(|()| file.sync_all()) {
        drop(file);
        let _ = std::fs::remove_file(path);
        return Err(error).context("could not write package");
    }
    println!("Exported {} to {}", package.manifest().name, path.display());
    Ok(())
}

fn instance() -> wgpu::Instance {
    wgpu::Instance::new(&wgpu::InstanceDescriptor::from_env_or_default())
}

fn report_adapter(gpu: &Gpu) {
    let info = gpu.adapter.get_info();
    eprintln!(
        "Wisp: {} ({:?}, {:?})",
        info.name, info.backend, info.device_type
    );
}

async fn snapshot(args: &Args, path: &Path, wallpaper: &Wallpaper) -> Result<()> {
    let gpu = Gpu::new(&instance(), None).await?;
    report_adapter(&gpu);
    let renderer =
        Renderer::from_wallpaper(&gpu, wallpaper, wgpu::TextureFormat::Rgba8UnormSrgb).await?;
    let mut globals = Globals::new(args.width, args.height);
    globals.time = args.time.unwrap_or(0.0);
    if !args.audio_bands.is_empty() {
        globals.set_audio(args.audio_bands.clone().try_into().unwrap());
    }
    // Use render_rgba for the final readback only; warmup submits ordinary GPU passes.
    let target = gpu.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("Wisp simulation warmup"),
        size: wgpu::Extent3d {
            width: args.width,
            height: args.height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8UnormSrgb,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
        view_formats: &[],
    });
    let view = target.create_view(&Default::default());
    for frame in 0..args.warmup {
        globals.frame = frame;
        globals.delta_time = 1.0 / 60.0;
        globals.time = args.time.unwrap_or(0.0) + frame as f32 / 60.0;
        let mut encoder = gpu.device.create_command_encoder(&Default::default());
        renderer.render(&gpu.queue, &mut encoder, &view, &globals);
        gpu.queue.submit([encoder.finish()]);
    }
    globals.frame = args.warmup;
    globals.time = args.time.unwrap_or(0.0) + args.warmup as f32 / 60.0;
    globals.delta_time = if args.warmup > 0 { 1.0 / 60.0 } else { 0.0 };
    let pixels = renderer.render_rgba(&gpu, args.width, args.height, &globals)?;
    image::save_buffer_with_format(
        path,
        &pixels,
        args.width,
        args.height,
        image::ColorType::Rgba8,
        image::ImageFormat::Png,
    )
    .with_context(|| format!("could not write {}", path.display()))?;
    println!(
        "Rendered {} × {} to {}",
        args.width,
        args.height,
        path.display()
    );
    Ok(())
}

struct Preview {
    args: Args,
    wallpaper: Wallpaper,
    state: Option<WindowState>,
    error: Option<anyhow::Error>,
}

struct WindowState {
    // Drop GPU resources and the surface before the native window they reference.
    renderer: Renderer,
    gpu: Gpu,
    surface: wgpu::Surface<'static>,
    window: Arc<Window>,
    config: wgpu::SurfaceConfiguration,
    globals: Globals,
    audio: wisp_audio::Service,
    last_frame: Instant,
    next_frame: Instant,
    presented: u64,
    occluded: bool,
}

impl WindowState {
    async fn new(window: Arc<Window>, wallpaper: &Wallpaper, args: &Args) -> Result<Self> {
        let instance = instance();
        let surface = instance
            .create_surface(window.clone())
            .context("could not create preview surface")?;
        let gpu = Gpu::new(&instance, Some(&surface)).await?;
        report_adapter(&gpu);
        let size = window.inner_size();
        let mut config = surface
            .get_default_config(&gpu.adapter, size.width.max(1), size.height.max(1))
            .context("GPU does not support this window surface")?;
        config.format = surface
            .get_capabilities(&gpu.adapter)
            .formats
            .into_iter()
            .find(|format| {
                matches!(
                    format,
                    wgpu::TextureFormat::Bgra8UnormSrgb | wgpu::TextureFormat::Rgba8UnormSrgb
                )
            })
            .context("surface does not support an 8-bit sRGB format")?;
        config.present_mode = wgpu::PresentMode::Fifo;
        config.desired_maximum_frame_latency = 2;
        let renderer = Renderer::from_wallpaper(&gpu, wallpaper, config.format).await?;
        let now = Instant::now();
        let mut state = Self {
            window,
            surface,
            config,
            gpu,
            renderer,
            globals: Globals::new(size.width, size.height),
            audio: wisp_audio::Service::new(wisp_audio::Config {
                enabled: args.audio,
                device: args.audio_device.clone(),
                ..Default::default()
            }),
            last_frame: now,
            next_frame: now,
            presented: 0,
            occluded: false,
        };
        state.resize(size)?;
        Ok(state)
    }

    fn resize(&mut self, size: PhysicalSize<u32>) -> Result<()> {
        if size.width == 0 || size.height == 0 {
            return Ok(());
        }
        let limit = self.gpu.device.limits().max_texture_dimension_2d;
        ensure!(
            size.width <= limit && size.height <= limit,
            "window size exceeds GPU texture limits ({limit})"
        );
        self.config.width = size.width;
        self.config.height = size.height;
        self.globals.resolution = [size.width as f32, size.height as f32];
        self.surface.configure(&self.gpu.device, &self.config);
        self.last_frame = Instant::now();
        self.next_frame = self.last_frame;
        Ok(())
    }

    fn active(&self) -> bool {
        let size = self.window.inner_size();
        !self.occluded && size.width > 0 && size.height > 0
    }

    fn draw(&mut self) -> Result<()> {
        let output = match self.surface.get_current_texture() {
            Ok(output) => output,
            Err(wgpu::SurfaceError::Lost | wgpu::SurfaceError::Outdated) => {
                self.resize(self.window.inner_size())?;
                return Ok(());
            }
            Err(wgpu::SurfaceError::Timeout) => return Ok(()),
            Err(error) => return Err(error).context("could not acquire wallpaper frame"),
        };
        let now = Instant::now();
        self.globals.set_audio(self.audio.bands().values());
        self.globals.delta_time = (now - self.last_frame).as_secs_f32().min(0.25);
        self.globals.time += self.globals.delta_time;
        self.last_frame = now;
        let view = output.texture.create_view(&Default::default());
        let mut encoder = self.gpu.device.create_command_encoder(&Default::default());
        self.renderer
            .render(&self.gpu.queue, &mut encoder, &view, &self.globals);
        self.gpu.queue.submit([encoder.finish()]);
        self.window.pre_present_notify();
        output.present();
        self.globals.frame = self.globals.frame.wrapping_add(1);
        self.presented += 1;
        Ok(())
    }
}

impl Preview {
    fn fail(&mut self, event_loop: &ActiveEventLoop, error: anyhow::Error) {
        self.error = Some(error);
        event_loop.exit();
    }
}

impl ApplicationHandler for Preview {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.state.is_some() {
            return;
        }
        let result = (|| {
            let window = Arc::new(
                event_loop.create_window(
                    Window::default_attributes()
                        .with_title("Wisp · Renderer Preview")
                        .with_inner_size(PhysicalSize::new(self.args.width, self.args.height)),
                )?,
            );
            pollster::block_on(WindowState::new(window, &self.wallpaper, &self.args))
        })();
        match result {
            Ok(state) => self.state = Some(state),
            Err(error) => self.fail(event_loop, error),
        }
    }

    fn suspended(&mut self, _event_loop: &ActiveEventLoop) {
        self.state = None;
    }

    fn exiting(&mut self, _event_loop: &ActiveEventLoop) {
        // run_app consumes the event loop. Release GPU/EGL state while its
        // Wayland display connection is still alive, including on error exits.
        self.state = None;
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        window_id: WindowId,
        event: WindowEvent,
    ) {
        let Some(state) = &mut self.state else {
            return;
        };
        if state.window.id() != window_id {
            return;
        }
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::KeyboardInput { event, .. }
                if event.state == ElementState::Pressed
                    && event.logical_key == Key::Named(NamedKey::Escape) =>
            {
                event_loop.exit()
            }
            WindowEvent::Resized(size) => {
                if let Err(error) = state.resize(size) {
                    self.fail(event_loop, error);
                }
            }
            WindowEvent::Occluded(occluded) => {
                state.occluded = occluded;
                state.last_frame = Instant::now();
                state.next_frame = state.last_frame;
            }
            WindowEvent::CursorMoved { position, .. } => {
                let size = state.window.inner_size();
                state.globals.mouse = [
                    (position.x as f32 / size.width.max(1) as f32).clamp(0.0, 1.0),
                    (position.y as f32 / size.height.max(1) as f32).clamp(0.0, 1.0),
                ];
            }
            WindowEvent::RedrawRequested if state.active() => {
                let frame_started = Instant::now();
                if frame_started < state.next_frame {
                    return;
                }
                if let Err(error) = state.draw() {
                    self.fail(event_loop, error);
                    return;
                }
                state.next_frame =
                    frame_started + Duration::from_secs_f64(1.0 / f64::from(self.args.fps));
                if self
                    .args
                    .frames
                    .is_some_and(|frames| state.presented >= frames)
                {
                    eprintln!("Wisp: presented {} frames", state.presented);
                    event_loop.exit();
                }
            }
            _ => {}
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        let Some(state) = &self.state else {
            return;
        };
        if state.active() {
            if Instant::now() >= state.next_frame {
                state.window.request_redraw();
                event_loop.set_control_flow(ControlFlow::Wait);
            } else {
                event_loop.set_control_flow(ControlFlow::WaitUntil(state.next_frame));
            }
        } else {
            event_loop.set_control_flow(ControlFlow::Wait);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn snapshot_audio_arguments_accept_one_comma_separated_value() {
        let args = Args::try_parse_from([
            "wisp-preview",
            "--output",
            "test.png",
            "--warmup",
            "120",
            "--audio-bands",
            "0.4,0.1,0.05,0.5",
        ])
        .unwrap();
        assert_eq!(args.audio_bands, [0.4, 0.1, 0.05, 0.5]);
        assert_eq!(args.warmup, 120);
        assert!(Args::try_parse_from(["wisp-preview", "--check", "--audio"]).is_err());
        assert!(Args::try_parse_from(["wisp-preview", "--output", "test.png", "--audio"]).is_err());
        assert!(
            Args::try_parse_from(["wisp-preview", "--output", "test.png", "--warmup", "3601"])
                .is_err()
        );
    }
}
