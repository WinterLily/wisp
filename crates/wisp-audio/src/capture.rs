use crate::{Analyzer, Bands, Config, MAX_CHANNELS};
use anyhow::{Context, Result};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use crossbeam_queue::ArrayQueue;
use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc,
    },
    thread,
    time::{Duration, Instant},
};

#[derive(Clone, Debug)]
pub struct Status {
    pub message: String,
    pub bands: Bands,
    pub dropped_frames: u64,
    updated: Instant,
}
impl Default for Status {
    fn default() -> Self {
        Self {
            message: "Audio disabled".into(),
            bands: Bands::default(),
            dropped_frames: 0,
            updated: Instant::now(),
        }
    }
}
/// One capture/FFT worker per host process; readers only copy the latest result.
/// Dropping the service stops capture and joins the worker.
pub struct Service {
    commands: mpsc::Sender<Config>,
    status: Arc<Mutex<Status>>,
    worker: Option<thread::JoinHandle<()>>,
}
impl Service {
    pub fn new(config: Config) -> Self {
        let (commands, rx) = mpsc::channel();
        let status = Arc::new(Mutex::new(Status::default()));
        let shared = status.clone();
        let worker = thread::spawn(move || run(config, rx, shared));
        Self {
            commands,
            status,
            worker: Some(worker),
        }
    }
    pub fn configure(&self, config: Config) -> Result<()> {
        config.validate()?;
        self.commands.send(config).context("audio worker stopped")
    }
    pub fn status(&self) -> Status {
        let mut status = self.status.lock().unwrap().clone();
        if status.updated.elapsed() > Duration::from_millis(250) {
            status.bands = Bands::default();
        }
        status
    }
    pub fn bands(&self) -> Bands {
        let status = self.status.lock().unwrap();
        if status.updated.elapsed() > Duration::from_millis(250) {
            Bands::default()
        } else {
            status.bands
        }
    }
}
impl Drop for Service {
    fn drop(&mut self) {
        // Disconnect before joining, including when the worker is waiting while disabled.
        let (unused, _) = mpsc::channel();
        drop(std::mem::replace(&mut self.commands, unused));
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}
fn host() -> Result<cpal::Host> {
    #[cfg(target_os = "linux")]
    {
        Ok(cpal::host_from_id(cpal::HostId::PulseAudio)?)
    }
    #[cfg(not(target_os = "linux"))]
    {
        Ok(cpal::default_host())
    }
}
pub fn devices() -> Result<Vec<(String, String)>> {
    let host = host()?;
    host.devices()?
        .map(|d| Ok((d.id()?.to_string(), d.description()?.name().to_owned())))
        .collect()
}
fn device(host: &cpal::Host, config: &Config) -> Result<cpal::Device> {
    let output = if let Some(id) = &config.device {
        let device = host
            .device_by_id(&id.parse()?)
            .context("audio device is unavailable")?;
        if device.supports_input() {
            return Ok(device);
        }
        device
    } else {
        host.default_output_device()
            .context("no default audio output")?
    };
    #[cfg(target_os = "linux")]
    {
        let id = cpal::DeviceId::new(
            cpal::HostId::PulseAudio,
            format!("{}.monitor", output.id()?.id()),
        );
        host.device_by_id(&id)
            .context("default output monitor unavailable; select a monitor device")
    }
    #[cfg(not(target_os = "linux"))]
    {
        Ok(output)
    }
}
struct Capture {
    _stream: cpal::Stream,
    // Keep the host/client alive for the stream's lifetime.
    _host: cpal::Host,
    device_id: cpal::DeviceId,
    queue: Arc<ArrayQueue<[f32; MAX_CHANNELS]>>,
    failed: Arc<AtomicBool>,
    dropped: Arc<AtomicU64>,
    analyzer: Analyzer,
    last_sample: Instant,
}
fn start(config: &Config, status: &Mutex<Status>) -> Result<Capture> {
    config.validate()?;
    let host = host()?;
    let device = device(&host, config)?;
    let supported = if device.supports_input() {
        device.default_input_config()?
    } else {
        device.default_output_config()?
    };
    let channels = usize::from(supported.channels());
    let analyzer = Analyzer::new(supported.sample_rate(), channels, config.gain)?;
    let queue = Arc::new(ArrayQueue::new(16384));
    let failed = Arc::new(AtomicBool::new(false));
    let dropped = Arc::new(AtomicU64::new(0));
    let format = supported.sample_format();
    let stream_config: cpal::StreamConfig = supported.into();
    #[cfg(target_os = "linux")]
    let stream_config = cpal::StreamConfig {
        buffer_size: cpal::BufferSize::Fixed(512),
        ..stream_config
    };
    macro_rules! build {
        ($ty:ty) => {{
            let q = queue.clone();
            let f = failed.clone();
            let d = dropped.clone();
            device.build_input_stream(
                stream_config,
                move |data: &[$ty], _: &cpal::InputCallbackInfo| {
                    for samples in data.chunks_exact(channels) {
                        let mut frame = [0.; MAX_CHANNELS];
                        for (out, sample) in frame.iter_mut().zip(samples) {
                            *out = cpal::Sample::to_sample::<f32>(*sample);
                        }
                        if q.push(frame).is_err() {
                            d.fetch_add(1, Ordering::Relaxed);
                        }
                    }
                },
                move |error| {
                    if !matches!(
                        error.kind(),
                        cpal::ErrorKind::Xrun | cpal::ErrorKind::RealtimeDenied
                    ) {
                        f.store(true, Ordering::Relaxed);
                    }
                },
                Some(Duration::from_secs(2)),
            )?
        }};
    }
    use cpal::SampleFormat::*;
    let stream = match format {
        I8 => build!(i8),
        I16 => build!(i16),
        I24 => build!(cpal::I24),
        I32 => build!(i32),
        I64 => build!(i64),
        U8 => build!(u8),
        U16 => build!(u16),
        U24 => build!(cpal::U24),
        U32 => build!(u32),
        U64 => build!(u64),
        F32 => build!(f32),
        F64 => build!(f64),
        _ => anyhow::bail!("unsupported audio sample format: {format}"),
    };
    stream.play()?;
    status.lock().unwrap().message = format!("Listening: {}", device.description()?.name());
    Ok(Capture {
        _stream: stream,
        _host: host,
        device_id: device.id()?,
        queue,
        failed,
        dropped,
        analyzer,
        last_sample: Instant::now(),
    })
}
fn run(mut config: Config, rx: mpsc::Receiver<Config>, status: Arc<Mutex<Status>>) {
    let mut capture: Option<Capture> = None;
    let mut retry = Instant::now();
    let mut next_device_check = Instant::now() + Duration::from_secs(5);
    loop {
        if config.enabled && capture.is_none() && Instant::now() >= retry {
            status.lock().unwrap().message = "Connecting to audio source…".into();
            match start(&config, &status) {
                Ok(c) => capture = Some(c),
                Err(e) => {
                    let mut s = status.lock().unwrap();
                    s.message = format!("Audio unavailable (retrying): {e:#}");
                    eprintln!("Wisp: {}", s.message);
                    s.bands = Bands::default();
                }
            }
            retry = Instant::now() + Duration::from_secs(5);
        }
        if let Some(c) = &mut capture {
            if Instant::now() >= next_device_check {
                // Follow default output changes and detect explicit device removal.
                if device(&c._host, &config)
                    .and_then(|d| Ok(d.id()?))
                    .map_or(true, |id| id != c.device_id)
                {
                    c.failed.store(true, Ordering::Relaxed);
                }
                next_device_check = Instant::now() + Duration::from_secs(5);
            }
            let mut latest = None;
            if !c.queue.is_empty() {
                if c.last_sample.elapsed() > Duration::from_millis(250) {
                    c.analyzer.reset();
                }
                c.last_sample = Instant::now();
            }
            // Bounded drain keeps configuration/shutdown responsive even under overload.
            for _ in 0..16384 {
                let Some(frame) = c.queue.pop() else { break };
                if let Some(b) = c.analyzer.push(&frame) {
                    latest = Some(b);
                }
            }
            if let Some(bands) = latest {
                let mut s = status.lock().unwrap();
                s.bands = bands;
                s.updated = Instant::now();
                s.dropped_frames = c.dropped.load(Ordering::Relaxed);
            }
            if c.failed.load(Ordering::Relaxed) {
                capture = None;
                let mut s = status.lock().unwrap();
                s.bands = Bands::default();
                s.message = "Audio disconnected (retrying)".into();
            }
        }
        let timeout = if capture.is_some() {
            Duration::from_millis(10)
        } else if config.enabled {
            retry.saturating_duration_since(Instant::now())
        } else {
            Duration::from_secs(3600)
        };
        match rx.recv_timeout(timeout) {
            Ok(next) => {
                if config != next {
                    config = next;
                    capture = None;
                    *status.lock().unwrap() = Status::default();
                    retry = Instant::now();
                }
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
            Err(mpsc::RecvTimeoutError::Timeout) => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn disabled_service_is_silent_and_stale_samples_expire() {
        let service = Service::new(Config::default());
        assert_eq!(service.bands(), Bands::default());
        {
            let mut s = service.status.lock().unwrap();
            s.bands.volume = 1.;
            s.updated = Instant::now() - Duration::from_secs(1);
        }
        assert_eq!(service.bands(), Bands::default());
        assert_eq!(service.status().bands, Bands::default());
        assert!(
            service
                .configure(Config {
                    gain: -1.,
                    ..Default::default()
                })
                .is_err()
        );
    }
}
