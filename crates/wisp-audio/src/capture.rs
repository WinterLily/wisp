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
    samples: CaptureSamples,
}
const QUEUED_FRAMES: usize = 16384;
const RETRY_INTERVAL: Duration = Duration::from_secs(5);
const STALE_INTERVAL: Duration = Duration::from_millis(250);

struct CaptureSamples {
    queue: Arc<ArrayQueue<[f32; MAX_CHANNELS]>>,
    failed: Arc<AtomicBool>,
    dropped: Arc<AtomicU64>,
    analyzer: Analyzer,
    last_sample: Instant,
}

struct CaptureUpdate {
    bands: Option<Bands>,
    dropped_frames: u64,
    failed: bool,
}

impl CaptureSamples {
    fn poll(&mut self, now: Instant) -> CaptureUpdate {
        let mut bands = None;
        if !self.queue.is_empty() {
            if now.duration_since(self.last_sample) > STALE_INTERVAL {
                self.analyzer.reset();
            }
            self.last_sample = now;
        }
        // A producer can refill the queue while it drains; cap work so commands still run.
        for _ in 0..QUEUED_FRAMES {
            let Some(frame) = self.queue.pop() else { break };
            if let Some(latest) = self.analyzer.push(&frame) {
                bands = Some(latest);
            }
        }
        CaptureUpdate {
            bands,
            dropped_frames: self.dropped.load(Ordering::Relaxed),
            failed: self.failed.load(Ordering::Relaxed),
        }
    }
}

fn enqueue_frame(
    queue: &ArrayQueue<[f32; MAX_CHANNELS]>,
    dropped: &AtomicU64,
    frame: [f32; MAX_CHANNELS],
) {
    if queue.push(frame).is_err() {
        dropped.fetch_add(1, Ordering::Relaxed);
    }
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
    let queue = Arc::new(ArrayQueue::new(QUEUED_FRAMES));
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
                        enqueue_frame(&q, &d, frame);
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
        samples: CaptureSamples {
            queue,
            failed,
            dropped,
            analyzer,
            last_sample: Instant::now(),
        },
    })
}

trait ActiveCapture {
    fn device_changed(&self, config: &Config) -> bool;
    fn poll(&mut self, now: Instant) -> CaptureUpdate;
}

impl ActiveCapture for Capture {
    fn device_changed(&self, config: &Config) -> bool {
        device(&self._host, config)
            .and_then(|device| Ok(device.id()?))
            .map_or(true, |id| id != self.device_id)
    }

    fn poll(&mut self, now: Instant) -> CaptureUpdate {
        self.samples.poll(now)
    }
}

trait CaptureBackend {
    type Stream: ActiveCapture;
    fn start(&mut self, config: &Config, status: &Mutex<Status>) -> Result<Self::Stream>;
}

struct NativeBackend;
impl CaptureBackend for NativeBackend {
    type Stream = Capture;

    fn start(&mut self, config: &Config, status: &Mutex<Status>) -> Result<Capture> {
        start(config, status)
    }
}

struct Worker<Backend: CaptureBackend> {
    backend: Backend,
    config: Config,
    capture: Option<Backend::Stream>,
    retry: Instant,
    next_device_check: Instant,
}

impl<Backend: CaptureBackend> Worker<Backend> {
    fn new(config: Config, backend: Backend, now: Instant) -> Self {
        Self {
            backend,
            config,
            capture: None,
            retry: now,
            next_device_check: now + RETRY_INTERVAL,
        }
    }

    fn step(&mut self, now: Instant, status: &Mutex<Status>) {
        if self.config.enabled && self.capture.is_none() && now >= self.retry {
            status.lock().unwrap().message = "Connecting to audio source…".into();
            match self.backend.start(&self.config, status) {
                Ok(capture) => {
                    self.capture = Some(capture);
                    self.next_device_check = now + RETRY_INTERVAL;
                }
                Err(e) => {
                    let mut s = status.lock().unwrap();
                    s.message = format!("Audio unavailable (retrying): {e:#}");
                    eprintln!("Wisp: {}", s.message);
                    s.bands = Bands::default();
                }
            }
            self.retry = now + RETRY_INTERVAL;
        }
        if let Some(capture) = &mut self.capture {
            let changed = if now >= self.next_device_check {
                self.next_device_check = now + RETRY_INTERVAL;
                capture.device_changed(&self.config)
            } else {
                false
            };
            let update = capture.poll(now);
            let mut current = status.lock().unwrap();
            current.dropped_frames = update.dropped_frames;
            if let Some(bands) = update.bands {
                current.bands = bands;
                current.updated = now;
            }
            if update.failed || changed {
                self.capture = None;
                self.retry = now + RETRY_INTERVAL;
                current.bands = Bands::default();
                current.message = "Audio disconnected (retrying)".into();
            }
        }
    }

    fn timeout(&self, now: Instant) -> Duration {
        if self.capture.is_some() {
            Duration::from_millis(10)
        } else if self.config.enabled {
            self.retry.saturating_duration_since(now)
        } else {
            Duration::from_secs(3600)
        }
    }

    fn configure(&mut self, next: Config, now: Instant, status: &Mutex<Status>) {
        if self.config != next {
            self.config = next;
            self.capture = None;
            *status.lock().unwrap() = Status::default();
            self.retry = now;
        }
    }
}

fn run(config: Config, rx: mpsc::Receiver<Config>, status: Arc<Mutex<Status>>) {
    run_with_backend(config, rx, status, NativeBackend);
}

fn run_with_backend<Backend: CaptureBackend>(
    config: Config,
    rx: mpsc::Receiver<Config>,
    status: Arc<Mutex<Status>>,
    backend: Backend,
) {
    let mut worker = Worker::new(config, backend, Instant::now());
    loop {
        worker.step(Instant::now(), &status);
        match rx.recv_timeout(worker.timeout(Instant::now())) {
            Ok(next) => worker.configure(next, Instant::now(), &status),
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
            Err(mpsc::RecvTimeoutError::Timeout) => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{collections::VecDeque, sync::atomic::AtomicUsize};

    #[derive(Default)]
    struct StreamSignals {
        failed: AtomicBool,
        changed: AtomicBool,
        dropped: AtomicU64,
        released: AtomicUsize,
    }

    struct FakeCapture {
        signals: Arc<StreamSignals>,
        bands: Option<Bands>,
    }
    impl FakeCapture {
        fn new(signals: Arc<StreamSignals>) -> Self {
            Self {
                signals,
                bands: Some(Bands {
                    volume: 0.5,
                    ..Default::default()
                }),
            }
        }
    }
    impl ActiveCapture for FakeCapture {
        fn device_changed(&self, _: &Config) -> bool {
            self.signals.changed.load(Ordering::Relaxed)
        }

        fn poll(&mut self, _: Instant) -> CaptureUpdate {
            CaptureUpdate {
                bands: self.bands.take(),
                dropped_frames: self.signals.dropped.load(Ordering::Relaxed),
                failed: self.signals.failed.load(Ordering::Relaxed),
            }
        }
    }
    impl Drop for FakeCapture {
        fn drop(&mut self) {
            self.signals.released.fetch_add(1, Ordering::Relaxed);
        }
    }

    #[derive(Default)]
    struct FakeBackend {
        results: VecDeque<Result<FakeCapture>>,
        attempts: Vec<Config>,
        started: Option<mpsc::Sender<()>>,
    }
    impl CaptureBackend for FakeBackend {
        type Stream = FakeCapture;
        fn start(&mut self, config: &Config, status: &Mutex<Status>) -> Result<FakeCapture> {
            self.attempts.push(config.clone());
            if let Some(started) = &self.started {
                let _ = started.send(());
            }
            let result = self
                .results
                .pop_front()
                .expect("unexpected connection attempt");
            if result.is_ok() {
                status.lock().unwrap().message = "Listening: test output".into();
            }
            result
        }
    }

    fn enabled_config() -> Config {
        Config {
            enabled: true,
            ..Default::default()
        }
    }

    #[test]
    fn failed_connection_retries_and_configuration_interrupts_the_wait() {
        let now = Instant::now();
        let status = Mutex::new(Status::default());
        let signals = Arc::new(StreamSignals::default());
        let backend = FakeBackend {
            results: [
                Err(anyhow::anyhow!("output unplugged")),
                Ok(FakeCapture::new(signals.clone())),
            ]
            .into(),
            ..Default::default()
        };
        let mut worker = Worker::new(enabled_config(), backend, now);
        worker.step(now, &status);
        assert!(status.lock().unwrap().message.contains("output unplugged"));
        assert_eq!(status.lock().unwrap().bands, Bands::default());
        worker.step(now + Duration::from_secs(4), &status);
        assert_eq!(worker.backend.attempts.len(), 1);
        let replacement = Config {
            device: Some("replacement".into()),
            gain: 2.,
            ..enabled_config()
        };
        worker.configure(replacement.clone(), now + Duration::from_secs(4), &status);
        worker.step(now + Duration::from_secs(4), &status);
        assert_eq!(worker.backend.attempts[1], replacement);
        assert_eq!(status.lock().unwrap().bands.volume, 0.5);
        worker.configure(Config::default(), now + Duration::from_secs(4), &status);
        assert_eq!(signals.released.load(Ordering::Relaxed), 1);
        worker.step(now + Duration::from_secs(20), &status);
        assert_eq!(worker.backend.attempts.len(), 2);
        assert_eq!(status.lock().unwrap().message, "Audio disabled");
    }

    #[test]
    fn stream_errors_and_device_changes_clear_bands_then_reconnect() {
        let now = Instant::now();
        let status = Mutex::new(Status::default());
        let signals: Vec<_> = (0..3).map(|_| Arc::new(StreamSignals::default())).collect();
        let backend = FakeBackend {
            results: signals
                .iter()
                .map(|s| Ok(FakeCapture::new(s.clone())))
                .collect(),
            ..Default::default()
        };
        let mut worker = Worker::new(enabled_config(), backend, now);
        worker.step(now, &status);
        signals[0].failed.store(true, Ordering::Relaxed);
        worker.step(now + Duration::from_secs(1), &status);
        assert_eq!(signals[0].released.load(Ordering::Relaxed), 1);
        assert_eq!(status.lock().unwrap().bands, Bands::default());
        worker.step(now + Duration::from_secs(5), &status);
        assert_eq!(worker.backend.attempts.len(), 1);
        worker.step(now + Duration::from_secs(6), &status);
        assert_eq!(status.lock().unwrap().bands.volume, 0.5);
        signals[1].changed.store(true, Ordering::Relaxed);
        worker.step(now + Duration::from_secs(11), &status);
        assert_eq!(signals[1].released.load(Ordering::Relaxed), 1);
        assert_eq!(
            status.lock().unwrap().message,
            "Audio disconnected (retrying)"
        );
        assert_eq!(status.lock().unwrap().bands, Bands::default());
        worker.step(now + Duration::from_secs(16), &status);
        assert_eq!(worker.backend.attempts.len(), 3);
        assert_eq!(status.lock().unwrap().bands.volume, 0.5);
    }

    #[test]
    fn overflow_is_reported_without_waiting_for_a_complete_fft_window() {
        let now = Instant::now();
        let mut samples = CaptureSamples {
            queue: Arc::new(ArrayQueue::new(2)),
            failed: Arc::new(AtomicBool::new(false)),
            dropped: Arc::new(AtomicU64::new(0)),
            analyzer: Analyzer::new(48000, 1, 1.).unwrap(),
            last_sample: now,
        };
        for _ in 0..3 {
            enqueue_frame(&samples.queue, &samples.dropped, [0.; MAX_CHANNELS]);
        }
        let update = samples.poll(now);
        assert_eq!(update.dropped_frames, 1);
        assert!(update.bands.is_none());
        assert!(samples.queue.is_empty());

        samples.queue = Arc::new(ArrayQueue::new(QUEUED_FRAMES + 1));
        for _ in 0..=QUEUED_FRAMES {
            enqueue_frame(&samples.queue, &samples.dropped, [0.; MAX_CHANNELS]);
        }
        let _ = samples.poll(now + Duration::from_secs(1));
        assert_eq!(samples.queue.len(), 1);
    }

    #[test]
    fn disconnecting_commands_stops_a_worker_waiting_to_retry() {
        let (commands, receiver) = mpsc::channel();
        let (started, ready) = mpsc::channel();
        let (finished, completion) = mpsc::channel();
        let worker = thread::spawn(move || {
            run_with_backend(
                enabled_config(),
                receiver,
                Arc::new(Mutex::new(Status::default())),
                FakeBackend {
                    results: [Err(anyhow::anyhow!("unavailable"))].into(),
                    started: Some(started),
                    ..Default::default()
                },
            );
            finished.send(()).unwrap();
        });
        ready.recv_timeout(Duration::from_secs(1)).unwrap();
        drop(commands);
        completion.recv_timeout(Duration::from_secs(1)).unwrap();
        worker.join().unwrap();
    }

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
