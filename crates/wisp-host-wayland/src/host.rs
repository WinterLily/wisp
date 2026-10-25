use crate::{Config, Options};
use anyhow::{Context, Result, ensure};
use raw_window_handle::{
    RawDisplayHandle, RawWindowHandle, WaylandDisplayHandle, WaylandWindowHandle,
};
use smithay_client_toolkit::{
    compositor::{CompositorHandler, CompositorState, Region},
    delegate_compositor, delegate_layer, delegate_output, delegate_registry,
    output::{OutputHandler, OutputState},
    registry::{ProvidesRegistryState, RegistryState},
    registry_handlers,
    shell::{
        WaylandSurface,
        wlr_layer::{
            Anchor, KeyboardInteractivity, Layer, LayerShell, LayerShellHandler, LayerSurface,
            LayerSurfaceConfigure,
        },
    },
};
use std::{
    path::PathBuf,
    ptr::NonNull,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant, SystemTime},
};
use wayland_client::{
    Connection, Proxy, QueueHandle,
    globals::registry_queue_init,
    protocol::{wl_output, wl_surface},
};
use wisp_host::{
    Assignment, reconciliation,
    status::{OutputStatus, StatusReporter},
};
use wisp_runtime::{Globals, Gpu, Renderer};

// Declaration order deliberately releases renderer/surface before layer objects.
struct Output {
    instance_id: u64,
    renderer: Option<Renderer>,
    surface: wgpu::Surface<'static>,
    layer: LayerSurface,
    output: wl_output::WlOutput,
    name: String,
    config: Option<wgpu::SurfaceConfiguration>,
    logical_size: (u32, u32),
    scale: i32,
    globals: Globals,
    frame_pending: bool,
    next_frame: Instant,
    last_frame: Instant,
    presented: u64,
    assignment: Assignment,
}
struct Host {
    outputs: Vec<Output>,
    gpu: Option<Gpu>,
    instance: wgpu::Instance,
    registry: RegistryState,
    output_state: OutputState,
    compositor: CompositorState,
    layer_shell: Option<LayerShell>,
    options: Options,
    error: Option<anyhow::Error>,
}

pub fn run(options: Options) -> Result<()> {
    let mut reporter = StatusReporter::new(options.status_path.clone(), &options.config)?;
    let connection = Connection::connect_to_env().context("could not connect to Wayland")?;
    let (globals, mut queue) = registry_queue_init(&connection)?;
    let qh = queue.handle();
    let mut host = Host {
        outputs: Vec::new(),
        gpu: None,
        instance: wgpu::Instance::new(&wgpu::InstanceDescriptor::from_env_or_default()),
        registry: RegistryState::new(&globals),
        output_state: OutputState::new(&globals, &qh),
        compositor: CompositorState::bind(&globals, &qh)?,
        layer_shell: if options.list_outputs {
            None
        } else {
            Some(LayerShell::bind(&globals, &qh).context(
                "compositor does not support wlr-layer-shell (Niri or Hyprland required)",
            )?)
        },
        options,
        error: None,
    };
    queue.roundtrip(&mut host)?;
    if host.options.list_outputs {
        if host.options.json {
            let monitors: Vec<_> = host
                .output_state
                .outputs()
                .filter_map(|output| host.output_state.info(&output))
                .map(|info| {
                    let (width, height) = info.logical_size.unwrap_or((0, 0));
                    wisp_host::MonitorInfo {
                        id: output_name(&info),
                        name: output_name(&info),
                        description: format!("{} {}", info.make, info.model),
                        width: width.max(0) as u32,
                        height: height.max(0) as u32,
                        scale: f64::from(info.scale_factor),
                    }
                })
                .collect();
            println!("{}", serde_json::to_string(&monitors)?);
            return Ok(());
        }
        for output in host.output_state.outputs() {
            if let Some(info) = host.output_state.info(&output) {
                println!(
                    "{}\t{} {}\tlogical {:?}\tscale {}",
                    output_name(&info),
                    info.make,
                    info.model,
                    info.logical_size,
                    info.scale_factor
                );
            }
        }
        return Ok(());
    }
    if let Some(error) = host.error.take() {
        return Err(error);
    }
    let audio = wisp_audio::Service::new(host.options.config.audio.clone());
    let mut audio_config = host.options.config.audio.clone();
    let stop = Arc::new(AtomicBool::new(false));
    let signal = stop.clone();
    ctrlc::set_handler(move || signal.store(true, Ordering::Relaxed))?;
    let mut event_loop: calloop::EventLoop<Host> = calloop::EventLoop::try_new()?;
    calloop_wayland_source::WaylandSource::new(connection.clone(), queue)
        .insert(event_loop.handle())
        .map_err(|e| anyhow::anyhow!("could not register Wayland event source: {e}"))?;
    let mut modified = config_modified(&host.options.config_path);
    let mut next_reload = Instant::now() + Duration::from_secs(1);
    while !stop.load(Ordering::Relaxed)
        && !host
            .options
            .stop_path
            .as_ref()
            .is_some_and(|path| path.exists())
    {
        let now = Instant::now();
        if audio_config != host.options.config.audio {
            audio_config = host.options.config.audio.clone();
            audio.configure(audio_config.clone())?;
        }
        let bands = audio.bands().values();
        for output in &mut host.outputs {
            output.globals.set_audio(bands);
            if output.renderer.is_some()
                && output.config.is_some()
                && !output.frame_pending
                && now >= output.next_frame
            {
                output.draw(host.gpu.as_ref().unwrap(), &qh)?;
            }
        }
        reporter.update(true, host.output_status(), false)?;
        if host.options.frames.is_some_and(|count| {
            !host.outputs.is_empty() && host.outputs.iter().all(|o| o.presented >= count)
        }) {
            reporter.update(true, host.output_status(), true)?;
            eprintln!(
                "Wisp: presented requested frames on {} output(s)",
                host.outputs.len()
            );
            break;
        }
        if now >= next_reload {
            let current = config_modified(&host.options.config_path);
            if current != modified {
                modified = current;
                if let Some(path) = &host.options.config_path {
                    let result =
                        Config::load(path).and_then(|config| host.reload(config, &connection, &qh));
                    reporter.reloaded(&result, &host.options.config);
                    if let Err(error) = result {
                        eprintln!("Wisp: keeping previous assignments: {error:#}");
                    }
                }
            }
            next_reload = now + Duration::from_secs(1);
        }
        let timeout = host
            .outputs
            .iter()
            .filter(|o| o.renderer.is_some() && o.config.is_some() && !o.frame_pending)
            .map(|o| o.next_frame.saturating_duration_since(Instant::now()))
            .min()
            .unwrap_or(Duration::from_millis(100))
            .min(Duration::from_millis(100));
        event_loop.dispatch(timeout, &mut host)?;
        if let Some(error) = host.error.take() {
            return Err(error);
        }
    }
    // Explicitly release GPU/EGL resources while the display connection is alive.
    host.outputs.clear();
    host.gpu = None;
    Ok(())
}
fn config_modified(path: &Option<PathBuf>) -> Option<SystemTime> {
    path.as_ref()
        .and_then(|p| std::fs::metadata(p).ok()?.modified().ok())
}
fn output_name(info: &smithay_client_toolkit::output::OutputInfo) -> String {
    info.name
        .clone()
        .unwrap_or_else(|| format!("{}-{}-{}", info.make, info.model, info.id))
}

impl Host {
    fn add_output(
        &mut self,
        connection: &Connection,
        qh: &QueueHandle<Self>,
        output: wl_output::WlOutput,
    ) -> Result<()> {
        if self.options.list_outputs || self.outputs.iter().any(|o| o.output == output) {
            return Ok(());
        }
        let Some(info) = self.output_state.info(&output) else {
            return Ok(());
        };
        let name = output_name(&info);
        let Some(assignment) = self.options.config.assignment(&name) else {
            return Ok(());
        };
        ensure!(
            self.outputs.len() < 16,
            "at most 16 active outputs are supported"
        );
        let assignment = assignment.clone();
        let prepared = self.prepare_output(connection, qh, output, assignment)?;
        eprintln!(
            "Wisp: assigned {} to {}",
            prepared.assignment.package.display(),
            prepared.name
        );
        self.outputs.push(prepared);
        Ok(())
    }
    fn prepare_output(
        &mut self,
        connection: &Connection,
        qh: &QueueHandle<Self>,
        output: wl_output::WlOutput,
        assignment: Assignment,
    ) -> Result<Output> {
        let info = self
            .output_state
            .info(&output)
            .context("output disappeared")?;
        let name = output_name(&info);
        let wallpaper = assignment.load()?;
        let wl_surface = self.compositor.create_surface(qh);
        let region = Region::new(&self.compositor)?;
        wl_surface.set_input_region(Some(region.wl_region()));
        let layer = self.layer_shell.as_ref().unwrap().create_layer_surface(
            qh,
            wl_surface,
            Layer::Background,
            Some("wisp"),
            Some(&output),
        );
        layer.set_anchor(Anchor::TOP | Anchor::BOTTOM | Anchor::LEFT | Anchor::RIGHT);
        layer.set_size(0, 0);
        layer.set_exclusive_zone(-1);
        layer.set_keyboard_interactivity(KeyboardInteractivity::None);
        // SAFETY: the connection lives until after Host drops, and Output drops the
        // GPU surface before its layer/wl_surface on every success and error path.
        let surface = unsafe {
            self.instance
                .create_surface_unsafe(wgpu::SurfaceTargetUnsafe::RawHandle {
                    raw_display_handle: RawDisplayHandle::Wayland(WaylandDisplayHandle::new(
                        NonNull::new(connection.backend().display_ptr().cast())
                            .context("null Wayland display")?,
                    )),
                    raw_window_handle: RawWindowHandle::Wayland(WaylandWindowHandle::new(
                        NonNull::new(layer.wl_surface().id().as_ptr().cast())
                            .context("null Wayland surface")?,
                    )),
                })?
        };
        if self.gpu.is_none() {
            self.gpu = Some(pollster::block_on(Gpu::new(
                &self.instance,
                Some(&surface),
            ))?);
        }
        let gpu = self.gpu.as_ref().unwrap();
        let format = surface
            .get_capabilities(&gpu.adapter)
            .formats
            .into_iter()
            .find(|f| {
                matches!(
                    f,
                    wgpu::TextureFormat::Rgba8UnormSrgb | wgpu::TextureFormat::Bgra8UnormSrgb
                )
            })
            .context("output lacks a compatible sRGB GPU surface")?;
        let renderer = pollster::block_on(Renderer::from_wallpaper(gpu, &wallpaper, format))?;
        let now = Instant::now();
        layer.commit();
        Ok(Output {
            instance_id: wisp_host::status::next_output_identity(),
            renderer: Some(renderer),
            surface,
            layer,
            output,
            name,
            config: None,
            logical_size: (0, 0),
            scale: info.scale_factor.max(1),
            globals: Globals::new(1, 1),
            frame_pending: false,
            next_frame: now,
            last_frame: now,
            presented: 0,
            assignment,
        })
    }
    fn output_status(&self) -> Vec<OutputStatus> {
        self.outputs
            .iter()
            .map(|output| OutputStatus {
                instance_id: output.instance_id,
                id: output.name.clone(),
                assignment: output.assignment.clone(),
                initialized: output.config.is_some(),
                presented: output.presented,
            })
            .collect()
    }
    fn reload(
        &mut self,
        config: Config,
        connection: &Connection,
        qh: &QueueHandle<Self>,
    ) -> Result<()> {
        config.validate()?;
        let desired: Vec<_> = self
            .output_state
            .outputs()
            .filter_map(|output| {
                let info = self.output_state.info(&output)?;
                let assignment = config.assignment(&output_name(&info))?.clone();
                Some((output, assignment))
            })
            .collect();
        ensure!(
            desired.len() <= 16,
            "at most 16 active outputs are supported"
        );
        // Preparation needs exclusive access to the shared GPU, while old surfaces stay alive.
        let mut outputs = std::mem::take(&mut self.outputs);
        let mut previous = self.options.config.clone();
        let result = reconciliation::reconcile(
            &mut outputs,
            &mut previous,
            Some(config),
            |_| Ok(desired),
            |output, (target, assignment)| {
                output.output == *target && output.assignment == *assignment
            },
            |(output, assignment)| {
                self.prepare_output(connection, qh, output.clone(), assignment.clone())
            },
        );
        self.outputs = outputs;
        result?;
        self.options.config = previous;
        eprintln!("Wisp: reloaded wallpaper assignments");
        Ok(())
    }
}

impl Output {
    fn configure(&mut self, gpu: &Gpu, size: (u32, u32)) -> Result<()> {
        self.logical_size = size;
        let (width, height) = physical_size(
            size,
            self.scale,
            gpu.device.limits().max_texture_dimension_2d,
        )?;
        let mut config = self
            .surface
            .get_default_config(&gpu.adapter, width, height)
            .context("unsupported output surface")?;
        config.format = self
            .surface
            .get_capabilities(&gpu.adapter)
            .formats
            .into_iter()
            .find(|f| {
                matches!(
                    f,
                    wgpu::TextureFormat::Rgba8UnormSrgb | wgpu::TextureFormat::Bgra8UnormSrgb
                )
            })
            .context("no sRGB format")?;
        config.present_mode = wgpu::PresentMode::Fifo;
        config.desired_maximum_frame_latency = 2;
        self.layer.wl_surface().set_buffer_scale(self.scale);
        self.surface.configure(&gpu.device, &config);
        self.globals.resolution = [width as f32, height as f32];
        self.config = Some(config);
        self.next_frame = Instant::now();
        Ok(())
    }
    fn draw(&mut self, gpu: &Gpu, qh: &QueueHandle<Host>) -> Result<()> {
        self.next_frame =
            Instant::now() + Duration::from_secs_f64(1.0 / f64::from(self.assignment.fps));
        let frame = match self.surface.get_current_texture() {
            Ok(frame) => frame,
            Err(wgpu::SurfaceError::Lost | wgpu::SurfaceError::Outdated) => {
                self.configure(gpu, self.logical_size)?;
                return Ok(());
            }
            Err(wgpu::SurfaceError::Timeout) => return Ok(()),
            Err(error) => return Err(error).context("could not acquire wallpaper surface"),
        };
        let now = Instant::now();
        self.globals.delta_time = now.duration_since(self.last_frame).as_secs_f32().min(0.25);
        self.globals.time += self.globals.delta_time;
        self.last_frame = now;
        let mut encoder = gpu.device.create_command_encoder(&Default::default());
        self.renderer.as_ref().unwrap().render(
            &gpu.queue,
            &mut encoder,
            &frame.texture.create_view(&Default::default()),
            &self.globals,
        );
        gpu.queue.submit([encoder.finish()]);
        self.layer
            .wl_surface()
            .frame(qh, self.layer.wl_surface().clone());
        self.frame_pending = true;
        self.layer
            .wl_surface()
            .damage_buffer(0, 0, i32::MAX, i32::MAX);
        frame.present();
        self.globals.frame = self.globals.frame.wrapping_add(1);
        self.presented += 1;
        Ok(())
    }
}
fn physical_size(size: (u32, u32), scale: i32, limit: u32) -> Result<(u32, u32)> {
    ensure!(scale > 0, "invalid output scale");
    let width = size
        .0
        .checked_mul(scale as u32)
        .context("output width overflow")?;
    let height = size
        .1
        .checked_mul(scale as u32)
        .context("output height overflow")?;
    ensure!(
        width > 0 && height > 0 && width <= limit && height <= limit,
        "output dimensions exceed GPU limits"
    );
    Ok((width, height))
}

impl OutputHandler for Host {
    fn output_state(&mut self) -> &mut OutputState {
        &mut self.output_state
    }
    fn new_output(
        &mut self,
        connection: &Connection,
        qh: &QueueHandle<Self>,
        output: wl_output::WlOutput,
    ) {
        if let Err(error) = self.add_output(connection, qh, output) {
            self.error = Some(error);
        }
    }
    fn update_output(
        &mut self,
        connection: &Connection,
        qh: &QueueHandle<Self>,
        output: wl_output::WlOutput,
    ) {
        if let Err(error) = self.add_output(connection, qh, output.clone()) {
            self.error = Some(error);
        }
        if let Some(info) = self.output_state.info(&output)
            && let Some(state) = self.outputs.iter_mut().find(|o| o.output == output)
        {
            state.scale = info.scale_factor.max(1);
            if state.config.is_some()
                && let Err(error) = state.configure(self.gpu.as_ref().unwrap(), state.logical_size)
            {
                self.error = Some(error);
            }
        }
    }
    fn output_destroyed(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        output: wl_output::WlOutput,
    ) {
        self.outputs.retain(|o| o.output != output);
    }
}
impl LayerShellHandler for Host {
    fn closed(&mut self, _: &Connection, _: &QueueHandle<Self>, layer: &LayerSurface) {
        self.outputs.retain(|o| &o.layer != layer);
    }
    fn configure(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        layer: &LayerSurface,
        configure: LayerSurfaceConfigure,
        _: u32,
    ) {
        if let Some(output) = self.outputs.iter_mut().find(|o| &o.layer == layer) {
            let fallback = self
                .output_state
                .info(&output.output)
                .and_then(|i| i.logical_size)
                .unwrap_or((1, 1));
            let size = (
                if configure.new_size.0 == 0 {
                    fallback.0.max(1) as u32
                } else {
                    configure.new_size.0
                },
                if configure.new_size.1 == 0 {
                    fallback.1.max(1) as u32
                } else {
                    configure.new_size.1
                },
            );
            if let Err(error) = output.configure(self.gpu.as_ref().unwrap(), size) {
                self.error = Some(error);
            }
        }
    }
}
impl CompositorHandler for Host {
    fn scale_factor_changed(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        surface: &wl_surface::WlSurface,
        scale: i32,
    ) {
        if let Some(output) = self
            .outputs
            .iter_mut()
            .find(|o| o.layer.wl_surface() == surface)
        {
            output.scale = scale.max(1);
            if output.config.is_some()
                && let Err(error) =
                    output.configure(self.gpu.as_ref().unwrap(), output.logical_size)
            {
                self.error = Some(error);
            }
        }
    }
    fn transform_changed(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_surface::WlSurface,
        _: wl_output::Transform,
    ) {
    }
    fn frame(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        surface: &wl_surface::WlSurface,
        _: u32,
    ) {
        if let Some(output) = self
            .outputs
            .iter_mut()
            .find(|o| o.layer.wl_surface() == surface)
        {
            output.frame_pending = false;
        }
    }
    fn surface_enter(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_surface::WlSurface,
        _: &wl_output::WlOutput,
    ) {
    }
    fn surface_leave(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_surface::WlSurface,
        _: &wl_output::WlOutput,
    ) {
    }
}
delegate_compositor!(Host);
delegate_output!(Host);
delegate_layer!(Host);
delegate_registry!(Host);
impl ProvidesRegistryState for Host {
    fn registry(&mut self) -> &mut RegistryState {
        &mut self.registry
    }
    registry_handlers![OutputState];
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn scaled_outputs_are_checked_before_surface_configuration() {
        assert_eq!(physical_size((1920, 1080), 2, 8192).unwrap(), (3840, 2160));
        for (size, scale) in [
            ((0, 1080), 1),
            ((4096, 4096), 3),
            ((u32::MAX, 1), 2),
            ((1, 1), 0),
        ] {
            assert!(physical_size(size, scale, 8192).is_err());
        }
    }
}
