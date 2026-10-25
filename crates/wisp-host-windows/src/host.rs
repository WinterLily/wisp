use crate::{
    Config, Options,
    monitor::Monitor,
    native::{self, Desktop, Window},
};
use anyhow::{Context, Result, ensure};
use raw_window_handle::{RawDisplayHandle, RawWindowHandle, WindowsDisplayHandle};
use std::{
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant, SystemTime},
};
use wisp_host::{
    Assignment,
    reconciliation::{self, DesktopAction},
    status::{OutputStatus, StatusReporter},
};
use wisp_runtime::{Globals, Gpu, Renderer};

// Field order releases each surface before its HWND, and all outputs before GPU/instance.
struct Output {
    instance_id: u64,
    renderer: Renderer,
    surface: wgpu::Surface<'static>,
    window: Window,
    surface_config: wgpu::SurfaceConfiguration,
    monitor: Monitor,
    assignment: Assignment,
    globals: Globals,
    last_frame: Instant,
    next_frame: Instant,
    presented: u64,
}
struct Host {
    outputs: Vec<Output>,
    gpu: Option<Gpu>,
    instance: wgpu::Instance,
    desktop: Option<Desktop>,
    config: Config,
}

pub fn run(options: Options) -> Result<()> {
    let mut reporter = StatusReporter::new(options.status_path.clone(), &options.config)?;
    let _dpi = native::DpiContext::physical()?;
    let monitors = native::monitors()?;
    if options.list_outputs {
        if options.json {
            let monitors = monitors
                .iter()
                .map(|monitor| {
                    let (width, height) = monitor.rect.size(i32::MAX as u32)?;
                    Ok(wisp_host::MonitorInfo {
                        id: monitor.id.clone(),
                        name: monitor.name.clone(),
                        description: monitor.description.clone(),
                        width,
                        height,
                        scale: f64::from(monitor.dpi.0) / 96.0,
                    })
                })
                .collect::<Result<Vec<_>>>()?;
            println!("{}", serde_json::to_string(&monitors)?);
            return Ok(());
        }
        for monitor in monitors {
            let (width, height) = monitor.rect.size(i32::MAX as u32)?;
            println!(
                "{}\t{}\t{}x{} at {},{}{}\nid: {}",
                monitor.name,
                monitor.description,
                width,
                height,
                monitor.rect.left,
                monitor.rect.top,
                if monitor.primary { " (primary)" } else { "" },
                monitor.id
            );
        }
        return Ok(());
    }
    options.config.validate()?;
    let audio = wisp_audio::Service::new(options.config.audio.clone());
    let mut audio_config = options.config.audio.clone();
    let _class = native::WindowClass::register()?;
    let stop = Arc::new(AtomicBool::new(false));
    let signal = stop.clone();
    ctrlc::set_handler(move || signal.store(true, Ordering::Relaxed))?;
    let mut host = Host {
        outputs: Vec::new(),
        gpu: None,
        instance: wgpu::Instance::new(&wgpu::InstanceDescriptor::from_env_or_default()),
        desktop: Some(Desktop::discover()?),
        config: options.config,
    };
    host.reconcile(&monitors, None)?;
    let mut modified = config_modified(options.config_path.as_deref());
    let mut next_scan = Instant::now() + Duration::from_secs(1);
    let mut reported_shell_loss = false;
    while !stop.load(Ordering::Relaxed)
        && !options.stop_path.as_ref().is_some_and(|path| path.exists())
        && native::pump_messages()
    {
        let now = Instant::now();
        let desktop_action = reconciliation::desktop_action(
            host.desktop.map(|desktop| desktop.valid()),
            host.desktop.is_some_and(|desktop| {
                host.outputs
                    .iter()
                    .all(|output| output.window.valid(desktop))
            }),
        );
        if desktop_action == DesktopAction::Release {
            // Stop using swapchains whose native parents disappeared. Discovery
            // below recreates our child windows after Explorer returns.
            host.outputs.clear();
            host.desktop = None;
            next_scan = now;
        }
        if now >= next_scan {
            if host.desktop.is_none() {
                match Desktop::discover() {
                    Ok(desktop) => {
                        host.desktop = Some(desktop);
                        reported_shell_loss = false;
                        modified = None;
                    }
                    Err(error) if !reported_shell_loss => {
                        eprintln!("Wisp: waiting for Explorer desktop: {error:#}");
                        reported_shell_loss = true;
                    }
                    Err(_) => {}
                }
            }
            if let Some(path) = options.config_path.as_deref() {
                let current = config_modified(Some(path));
                if current != modified {
                    modified = current;
                    let result = Config::load(path).and_then(|config| {
                        config.validate()?;
                        host.reconcile(&native::monitors()?, Some(config))
                    });
                    reporter.reloaded(&result, &host.config);
                    match result {
                        Ok(()) => eprintln!("Wisp: reloaded wallpaper assignments"),
                        Err(error) => eprintln!("Wisp: keeping previous assignments: {error:#}"),
                    }
                }
            }
            if host.desktop.is_some() {
                match native::monitors() {
                    Ok(monitors) => {
                        // Disconnected monitors must release resources even if a newly
                        // connected monitor's wallpaper fails preparation.
                        host.outputs
                            .retain(|o| monitors.iter().any(|m| m.id == o.monitor.id));
                        if let Err(error) = host.reconcile(&monitors, None) {
                            eprintln!("Wisp: could not update monitor assignments: {error:#}");
                        }
                    }
                    Err(error) => eprintln!("Wisp: could not refresh monitors: {error:#}"),
                }
            }
            next_scan = Instant::now() + Duration::from_secs(1);
        }
        if audio_config != host.config.audio {
            audio_config = host.config.audio.clone();
            audio.configure(audio_config.clone())?;
        }
        let bands = audio.bands().values();
        let cursor = native::cursor();
        for output in &mut host.outputs {
            output.globals.set_audio(bands);
            if now >= output.next_frame {
                output.draw(host.gpu.as_ref().unwrap(), cursor)?;
            }
        }
        reporter.update(host.desktop.is_some(), host.output_status(), false)?;
        if options.frames.is_some_and(|count| {
            !host.outputs.is_empty() && host.outputs.iter().all(|o| o.presented >= count)
        }) {
            reporter.update(host.desktop.is_some(), host.output_status(), true)?;
            eprintln!(
                "Wisp: presented requested frames on {} output(s)",
                host.outputs.len()
            );
            break;
        }
        let timeout = host
            .outputs
            .iter()
            .map(|o| o.next_frame)
            .chain(std::iter::once(next_scan))
            .min()
            .unwrap()
            .saturating_duration_since(Instant::now());
        native::wait(timeout);
    }
    // Explicit order also covers WM_CLOSE and Ctrl+C. Host's field order covers errors.
    host.outputs.clear();
    host.gpu = None;
    Ok(())
}
fn config_modified(path: Option<&Path>) -> Option<SystemTime> {
    path.and_then(|p| std::fs::metadata(p).ok()?.modified().ok())
}
impl Host {
    fn output_status(&self) -> Vec<OutputStatus> {
        self.outputs
            .iter()
            .map(|output| OutputStatus {
                instance_id: output.instance_id,
                id: output.monitor.id.clone(),
                assignment: output.assignment.clone(),
                initialized: true,
                presented: output.presented,
            })
            .collect()
    }
    /// Prepare hidden windows and GPU resources for every changed/new output,
    /// then commit together. Failed reloads leave the old visible outputs intact.
    fn reconcile(&mut self, monitors: &[Monitor], replacement: Option<Config>) -> Result<()> {
        let desktop = self.desktop.context("Explorer desktop is unavailable")?;
        reconciliation::reconcile(
            &mut self.outputs,
            &mut self.config,
            replacement,
            |config| {
                let desired: Vec<_> = monitors
                    .iter()
                    .filter_map(|monitor| {
                        monitor
                            .assignment(config)
                            .map(|assignment| (monitor.clone(), assignment.clone()))
                    })
                    .collect();
                ensure!(
                    desired.len() <= 16,
                    "at most 16 active outputs are supported"
                );
                Ok(desired)
            },
            |output, (monitor, assignment)| {
                output.monitor == *monitor && output.assignment == *assignment
            },
            |(monitor, assignment)| {
                Output::new(
                    &self.instance,
                    &mut self.gpu,
                    desktop,
                    monitor.clone(),
                    assignment.clone(),
                )
            },
        )?;
        for output in &self.outputs {
            output.window.show();
        }
        // Explorer can reorder its raised desktop children independently.
        for output in &self.outputs {
            if let Err(error) = output.window.place(desktop) {
                eprintln!("Wisp: could not restore wallpaper position: {error:#}");
            }
        }
        Ok(())
    }
}
impl Output {
    fn new(
        instance: &wgpu::Instance,
        shared_gpu: &mut Option<Gpu>,
        desktop: Desktop,
        monitor: Monitor,
        assignment: Assignment,
    ) -> Result<Self> {
        let wallpaper = assignment.load()?;
        let window = Window::create(desktop, monitor.rect)?;
        // SAFETY: window is owned here and moves into Output after surface creation.
        // Surface drops before Window, including all early error paths. All calls
        // and destruction stay on this HWND's creating thread.
        let surface = unsafe {
            instance.create_surface_unsafe(wgpu::SurfaceTargetUnsafe::RawHandle {
                raw_display_handle: RawDisplayHandle::Windows(WindowsDisplayHandle::new()),
                raw_window_handle: RawWindowHandle::Win32(window.raw_handle()),
            })?
        };
        if shared_gpu.is_none() {
            *shared_gpu = Some(pollster::block_on(Gpu::new(instance, Some(&surface)))?);
        }
        let gpu = shared_gpu.as_ref().unwrap();
        let (width, height) = monitor
            .rect
            .size(gpu.device.limits().max_texture_dimension_2d)?;
        let mut config = surface
            .get_default_config(&gpu.adapter, width, height)
            .context("GPU cannot present to this monitor")?;
        config.format = surface
            .get_capabilities(&gpu.adapter)
            .formats
            .into_iter()
            .find(|f| {
                matches!(
                    f,
                    wgpu::TextureFormat::Rgba8UnormSrgb | wgpu::TextureFormat::Bgra8UnormSrgb
                )
            })
            .context("monitor lacks an sRGB GPU surface")?;
        config.present_mode = wgpu::PresentMode::Fifo;
        config.desired_maximum_frame_latency = 2;
        let renderer =
            pollster::block_on(Renderer::from_wallpaper(gpu, &wallpaper, config.format))?;
        configure(&surface, gpu, &config)?;
        let now = Instant::now();
        Ok(Self {
            instance_id: wisp_host::status::next_output_identity(),
            renderer,
            surface,
            window,
            surface_config: config,
            monitor,
            assignment,
            globals: Globals::new(width, height),
            last_frame: now,
            next_frame: now,
            presented: 0,
        })
    }
    fn draw(&mut self, gpu: &Gpu, cursor: Option<(i32, i32)>) -> Result<()> {
        self.next_frame =
            Instant::now() + Duration::from_secs_f64(1.0 / f64::from(self.assignment.fps));
        let frame = match self.surface.get_current_texture() {
            Ok(frame) => frame,
            Err(wgpu::SurfaceError::Lost | wgpu::SurfaceError::Outdated) => {
                configure(&self.surface, gpu, &self.surface_config)?;
                return Ok(());
            }
            Err(wgpu::SurfaceError::Timeout) => return Ok(()),
            Err(error) => return Err(error).context("could not acquire Windows wallpaper surface"),
        };
        let now = Instant::now();
        self.globals.delta_time = now.duration_since(self.last_frame).as_secs_f32().min(0.25);
        self.globals.time += self.globals.delta_time;
        self.last_frame = now;
        if let Some((x, y)) = cursor {
            self.globals.mouse = self.monitor.rect.mouse(x, y);
        }
        let mut encoder = gpu.device.create_command_encoder(&Default::default());
        self.renderer.render(
            &gpu.queue,
            &mut encoder,
            &frame.texture.create_view(&Default::default()),
            &self.globals,
        );
        gpu.queue.submit([encoder.finish()]);
        frame.present();
        self.globals.frame = self.globals.frame.wrapping_add(1);
        self.presented += 1;
        Ok(())
    }
}
fn configure(
    surface: &wgpu::Surface<'_>,
    gpu: &Gpu,
    config: &wgpu::SurfaceConfiguration,
) -> Result<()> {
    gpu.device.push_error_scope(wgpu::ErrorFilter::Validation);
    gpu.device.push_error_scope(wgpu::ErrorFilter::OutOfMemory);
    surface.configure(&gpu.device, config);
    let memory = pollster::block_on(gpu.device.pop_error_scope());
    let validation = pollster::block_on(gpu.device.pop_error_scope());
    if let Some(error) = memory.or(validation) {
        anyhow::bail!("could not configure Windows surface: {error}");
    }
    Ok(())
}
