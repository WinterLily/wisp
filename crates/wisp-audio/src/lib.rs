//! Host-owned capture and analysis. Wallpaper packages never select capture devices.
mod capture;
use anyhow::{Result, ensure};
pub use capture::{Service, Status, devices};
use rustfft::{Fft, FftPlanner, num_complex::Complex};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    pub enabled: bool,
    /// None selects system output loopback. An explicit ID may select an input.
    pub device: Option<String>,
    pub gain: f32,
}
impl Default for Config {
    fn default() -> Self {
        Self {
            enabled: false,
            device: None,
            gain: 1.0,
        }
    }
}
impl Config {
    pub fn validate(&self) -> Result<()> {
        ensure!(
            self.gain.is_finite() && (0.0..=10.0).contains(&self.gain),
            "audio gain must be 0..=10"
        );
        ensure!(
            self.device
                .as_ref()
                .is_none_or(|s| !s.trim().is_empty() && s.len() <= 1024),
            "invalid audio device ID"
        );
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Bands {
    pub bass: f32,
    pub mid: f32,
    pub treble: f32,
    pub volume: f32,
}
impl Bands {
    pub fn values(self) -> [f32; 4] {
        [self.bass, self.mid, self.treble, self.volume]
    }
}
const N: usize = 4096;
const HOP: usize = 1024;
pub(crate) const MAX_CHANNELS: usize = 8;
/// Hann-windowed, DC-rejected, channel-power FFT; stereo antiphase does not cancel.
/// Preallocates all FFT scratch and sample storage. Output is linear RMS in 0..=1.
pub struct Analyzer {
    rate: f32,
    channels: usize,
    gain: f32,
    samples: Vec<[f32; MAX_CHANNELS]>,
    cursor: usize,
    filled: usize,
    hop: usize,
    window: Vec<f32>,
    window_power: f32,
    fft: Arc<dyn Fft<f32>>,
    buffer: Vec<Complex<f32>>,
    scratch: Vec<Complex<f32>>,
    bands: Bands,
}
impl Analyzer {
    pub fn new(rate: u32, channels: usize, gain: f32) -> Result<Self> {
        ensure!(
            (8000..=192000).contains(&rate),
            "unsupported audio sample rate"
        );
        ensure!(
            (1..=MAX_CHANNELS).contains(&channels),
            "unsupported audio channel count"
        );
        Config {
            gain,
            ..Default::default()
        }
        .validate()?;
        let fft = FftPlanner::new().plan_fft_forward(N);
        let scratch = vec![Complex::default(); fft.get_inplace_scratch_len()];
        let window: Vec<_> = (0..N)
            .map(|i| 0.5 - 0.5 * (std::f32::consts::TAU * i as f32 / N as f32).cos())
            .collect();
        let window_power = window.iter().map(|v| v * v).sum();
        Ok(Self {
            rate: rate as f32,
            channels,
            gain,
            samples: vec![[0.; MAX_CHANNELS]; N],
            cursor: 0,
            filled: 0,
            hop: 0,
            window,
            window_power,
            fft,
            buffer: vec![Complex::default(); N],
            scratch,
            bands: Bands::default(),
        })
    }
    /// Discard pre-gap history when capture stops delivering samples.
    pub fn reset(&mut self) {
        self.samples.fill([0.; MAX_CHANNELS]);
        self.cursor = 0;
        self.filled = 0;
        self.hop = 0;
        self.bands = Bands::default();
    }
    pub fn push(&mut self, frame: &[f32]) -> Option<Bands> {
        for c in 0..self.channels {
            let v = frame.get(c).copied().unwrap_or(0.);
            self.samples[self.cursor][c] = if v.is_finite() { v.clamp(-1., 1.) } else { 0. };
        }
        self.cursor = (self.cursor + 1) % N;
        self.filled = (self.filled + 1).min(N);
        self.hop += 1;
        if self.filled < N || self.hop < HOP {
            return None;
        }
        self.hop = 0;
        let mut powers = [0.; 4];
        for c in 0..self.channels {
            let mean = self.samples.iter().map(|s| s[c]).sum::<f32>() / N as f32;
            for i in 0..N {
                let v = self.samples[(self.cursor + i) % N][c] - mean;
                powers[3] += v * v / (N * self.channels) as f32;
                self.buffer[i] = Complex::new(v * self.window[i], 0.);
            }
            self.fft
                .process_with_scratch(&mut self.buffer, &mut self.scratch);
            for k in 1..N / 2 {
                let hz = k as f32 * self.rate / N as f32;
                let band = if (20.0..250.).contains(&hz) {
                    Some(0)
                } else if (250.0..4000.).contains(&hz) {
                    Some(1)
                } else if (4000.0..20000.).contains(&hz) {
                    Some(2)
                } else {
                    None
                };
                if let Some(b) = band {
                    powers[b] += 2. * self.buffer[k].norm_sqr()
                        / (N as f32 * self.window_power * self.channels as f32);
                }
            }
        }
        let mut values = self.bands.values();
        for i in 0..4 {
            let target = (powers[i].sqrt() * self.gain).clamp(0., 1.);
            let tau = if target > values[i] { 0.025 } else { 0.2 };
            let a = 1. - (-(HOP as f32) / self.rate / tau).exp();
            values[i] += (target - values[i]) * a;
            if values[i] < 0.00001 {
                values[i] = 0.;
            }
        }
        self.bands = Bands {
            bass: values[0],
            mid: values[1],
            treble: values[2],
            volume: values[3],
        };
        Some(self.bands)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn tone(hz: f32, rate: u32, channels: usize) -> Bands {
        let mut a = Analyzer::new(rate, channels, 1.).unwrap();
        let mut out = Bands::default();
        for i in 0..rate {
            let v = 0.5 * (std::f32::consts::TAU * hz * i as f32 / rate as f32).sin();
            if let Some(b) = a.push(&[v, -v]) {
                out = b;
            }
        }
        out
    }
    #[test]
    fn fft_separates_bands_and_preserves_stereo_power() {
        for rate in [44100, 48000, 96000] {
            for (i, hz) in [100., 1000., 8000.].into_iter().enumerate() {
                let b = tone(hz, rate, 2).values();
                assert!((b[i] - 0.35355).abs() < 0.008, "{rate} {hz} {b:?}");
                assert!((b[3] - 0.35355).abs() < 0.008);
                for (j, v) in b[..3].iter().enumerate() {
                    if i != j {
                        assert!(*v < 0.005);
                    }
                }
            }
            assert!((tone(1000., rate, 1).volume - tone(1000., rate, 2).volume).abs() < 0.001);
        }
    }
    #[test]
    fn silence_dc_invalid_samples_gain_and_release_are_bounded() {
        assert!(Analyzer::new(0, 2, 1.).is_err());
        assert!(Analyzer::new(48000, 9, 1.).is_err());
        assert!(Analyzer::new(48000, 2, f32::NAN).is_err());
        let mut a = Analyzer::new(48000, 1, 10.).unwrap();
        for _ in 0..10000 {
            if let Some(b) = a.push(&[0.5]) {
                assert_eq!(b, Bands::default());
            }
        }
        for i in 0..48000 {
            a.push(&[(i as f32 * 0.13).sin()]);
        }
        assert!(a.bands.volume > 0.99);
        for _ in 0..192000 {
            a.push(&[f32::NAN]);
        }
        assert_eq!(a.bands, Bands::default());
        a.reset();
        assert_eq!(a.push(&[0.5]), None);
    }
}
