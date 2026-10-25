#![cfg(target_os = "linux")]
use std::{
    process::{Child, Command},
    time::{Duration, Instant},
};
struct Sink(String);
impl Drop for Sink {
    fn drop(&mut self) {
        let _ = Command::new("pactl")
            .args(["unload-module", &self.0])
            .status();
    }
}
struct Playback(Child);
impl Drop for Playback {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
#[test]
#[ignore = "requires PipeWire/PulseAudio, pactl and paplay; creates an isolated silent test sink"]
fn output_monitor_captures_stereo_tone_and_releases_to_silence() {
    let name = format!("wisp_capture_test_{}", std::process::id());
    let output = Command::new("pactl")
        .args([
            "load-module",
            "module-null-sink",
            &format!("sink_name={name}"),
        ])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let _sink = Sink(String::from_utf8(output.stdout).unwrap().trim().into());
    // Selecting a Linux output ID resolves to its monitor, never the default microphone.
    let service = wisp_audio::Service::new(wisp_audio::Config {
        enabled: true,
        device: Some(format!("PulseAudio:{name}")),
        gain: 1.,
    });
    let deadline = Instant::now() + Duration::from_secs(8);
    while !service.status().message.starts_with("Listening:") {
        assert!(Instant::now() < deadline, "{}", service.status().message);
        std::thread::sleep(Duration::from_millis(25));
    }
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("tone.wav");
    let frames = 48000u32 * 3;
    let bytes = frames * 4;
    let mut wav = Vec::new();
    wav.extend(b"RIFF");
    wav.extend((36 + bytes).to_le_bytes());
    wav.extend(b"WAVEfmt ");
    wav.extend(16u32.to_le_bytes());
    wav.extend(1u16.to_le_bytes());
    wav.extend(2u16.to_le_bytes());
    wav.extend(48000u32.to_le_bytes());
    wav.extend(192000u32.to_le_bytes());
    wav.extend(4u16.to_le_bytes());
    wav.extend(16u16.to_le_bytes());
    wav.extend(b"data");
    wav.extend(bytes.to_le_bytes());
    for i in 0..frames {
        let v = (16384. * (std::f32::consts::TAU * 100. * i as f32 / 48000.).sin()) as i16;
        wav.extend(v.to_le_bytes());
        wav.extend((-v).to_le_bytes());
    }
    std::fs::write(&path, wav).unwrap();
    let mut playback = Playback(
        Command::new("paplay")
            .arg(format!("--device={name}"))
            .arg(path)
            .spawn()
            .unwrap(),
    );
    let mut peak = [0f32; 4];
    while playback.0.try_wait().unwrap().is_none() {
        for (p, v) in peak.iter_mut().zip(service.bands().values()) {
            *p = p.max(v);
        }
        assert!(Instant::now() < deadline + Duration::from_secs(5));
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!((peak[0] - 0.35355).abs() < 0.02, "{peak:?}");
    assert!((peak[3] - 0.35355).abs() < 0.02, "{peak:?}");
    assert!(peak[1] < 0.03 && peak[2] < 0.03, "{peak:?}");
    std::thread::sleep(Duration::from_secs(2));
    assert!(service.bands().volume < 0.001);
    assert_eq!(service.status().dropped_frames, 0);
    service.configure(Default::default()).unwrap();
    let deadline = Instant::now() + Duration::from_secs(3);
    while service.status().message != "Audio disabled" {
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(service.bands(), Default::default());
}
