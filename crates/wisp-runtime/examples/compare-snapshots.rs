//! Compare deterministic PNG snapshots from different GPU backends/platforms.
use anyhow::{Context, Result, ensure};

fn main() -> Result<()> {
    let paths: Vec<_> = std::env::args_os().skip(1).collect();
    ensure!(
        paths.len() == 2,
        "usage: compare-snapshots LEFT.png RIGHT.png"
    );
    let left = image::open(&paths[0])
        .context("read left snapshot")?
        .into_rgba8();
    let right = image::open(&paths[1])
        .context("read right snapshot")?
        .into_rgba8();
    ensure!(
        left.dimensions() == right.dimensions(),
        "snapshot dimensions differ"
    );
    ensure!(!left.is_empty(), "empty snapshots");
    let mut maximum = 0u8;
    let mut total = 0u64;
    for (a, b) in left.as_raw().iter().zip(right.as_raw()) {
        let error = a.abs_diff(*b);
        maximum = maximum.max(error);
        total += u64::from(error);
    }
    let mean = total as f64 / left.as_raw().len() as f64;
    println!(
        "{}x{}: maximum channel error {maximum}/255, mean {mean:.6}/255",
        left.width(),
        left.height()
    );
    // Allow sRGB conversion / floating point rounding, while catching orientation,
    // ABI, missing passes, and visible color discrepancies. Includes alpha.
    ensure!(
        maximum <= 3 && mean <= 0.25,
        "snapshots exceed parity tolerance (max 3, mean 0.25)"
    );
    Ok(())
}
