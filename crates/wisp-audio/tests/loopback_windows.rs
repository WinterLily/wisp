#![cfg(target_os = "windows")]

use cpal::{
    FromSample, SizedSample,
    traits::{DeviceTrait, HostTrait, StreamTrait},
};
use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::{Duration, Instant},
};

fn tone_stream<Sample: SizedSample + FromSample<f32>>(
    device: &cpal::Device,
    config: cpal::StreamConfig,
    failed: Arc<AtomicBool>,
) -> cpal::Stream {
    let channels = usize::from(config.channels);
    let phase_increment = std::f32::consts::TAU * 100. / config.sample_rate as f32;
    let mut phase = 0.;
    device
        .build_output_stream(
            config,
            move |data: &mut [Sample], _| {
                for frame in data.chunks_exact_mut(channels) {
                    let value = 0.1 * f32::sin(phase);
                    phase = (phase + phase_increment) % std::f32::consts::TAU;
                    for sample in frame {
                        *sample = Sample::from_sample(value);
                    }
                }
            },
            move |_| failed.store(true, Ordering::Relaxed),
            Some(Duration::from_secs(2)),
        )
        .unwrap()
}

#[test]
#[ignore = "requires a quiet interactive Windows audio output; plays a three-second 100 Hz tone"]
fn system_output_loopback_captures_tone_and_releases_to_silence() {
    let host = cpal::default_host();
    let device = host
        .default_output_device()
        .expect("no Windows audio output");
    let supported = device.default_output_config().unwrap();
    let service = wisp_audio::Service::new(wisp_audio::Config {
        enabled: true,
        ..Default::default()
    });
    let deadline = Instant::now() + Duration::from_secs(10);
    while !service.status().message.starts_with("Listening:") {
        assert!(Instant::now() < deadline, "{}", service.status().message);
        thread::sleep(Duration::from_millis(25));
    }

    let failed = Arc::new(AtomicBool::new(false));
    let config: cpal::StreamConfig = supported.into();
    macro_rules! stream {
        ($sample:ty) => {
            tone_stream::<$sample>(&device, config, failed.clone())
        };
    }
    use cpal::SampleFormat::*;
    let stream = match supported.sample_format() {
        I8 => stream!(i8),
        I16 => stream!(i16),
        I24 => stream!(cpal::I24),
        I32 => stream!(i32),
        I64 => stream!(i64),
        U8 => stream!(u8),
        U16 => stream!(u16),
        U24 => stream!(cpal::U24),
        U32 => stream!(u32),
        U64 => stream!(u64),
        F32 => stream!(f32),
        F64 => stream!(f64),
        format => panic!("unsupported Windows output format: {format}"),
    };
    stream.play().unwrap();
    let deadline = Instant::now() + Duration::from_secs(3);
    let mut bass_peak = 0f32;
    while Instant::now() < deadline {
        bass_peak = bass_peak.max(service.bands().bass);
        assert!(
            !failed.load(Ordering::Relaxed),
            "test playback stream failed"
        );
        thread::sleep(Duration::from_millis(20));
    }
    drop(stream);
    assert!(
        bass_peak > 0.025,
        "output loopback did not capture the tone: {bass_peak}"
    );
    let deadline = Instant::now() + Duration::from_secs(5);
    while service.bands().volume >= 0.005 {
        assert!(
            Instant::now() < deadline,
            "output did not return to silence"
        );
        thread::sleep(Duration::from_millis(25));
    }
    service.configure(Default::default()).unwrap();
    let deadline = Instant::now() + Duration::from_secs(3);
    while service.status().message != "Audio disabled" {
        assert!(Instant::now() < deadline);
        thread::sleep(Duration::from_millis(25));
    }
    assert_eq!(service.bands(), Default::default());
}
