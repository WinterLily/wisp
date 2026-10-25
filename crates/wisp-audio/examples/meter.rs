//! Bounded capture diagnostic: cargo run -p wisp-audio --example meter -- [device ID] [seconds]
use std::time::{Duration, Instant};
fn main() -> anyhow::Result<()> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    if args.first().is_some_and(|s| s == "--list") {
        for (id, name) in wisp_audio::devices()? {
            println!("{id}\t{name}");
        }
        return Ok(());
    }
    let device = args.first().filter(|s| s.as_str() != "default").cloned();
    let seconds: u64 = args.get(1).map(|v| v.parse()).transpose()?.unwrap_or(5);
    anyhow::ensure!(
        (1..=60).contains(&seconds),
        "duration must be 1..=60 seconds"
    );
    let service = wisp_audio::Service::new(wisp_audio::Config {
        enabled: true,
        device,
        ..Default::default()
    });
    let start = Instant::now();
    while start.elapsed() < Duration::from_secs(seconds) {
        std::thread::sleep(Duration::from_millis(100));
        let s = service.status();
        println!(
            "{}: {:.4?}, dropped {}",
            s.message,
            s.bands.values(),
            s.dropped_frames
        );
    }
    anyhow::ensure!(
        service.status().message.starts_with("Listening:"),
        "{}",
        service.status().message
    );
    Ok(())
}
