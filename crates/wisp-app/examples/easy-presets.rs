//! Reproducible editable projects and packages for documentation and parity checks.
use anyhow::{Context, Result};
use std::path::PathBuf;
use wisp_app::designer::{Draft, Preset};
fn main() -> Result<()> {
    let destination = PathBuf::from(
        std::env::args_os()
            .nth(1)
            .context("usage: easy-presets NEW_DIRECTORY")?,
    );
    std::fs::create_dir(&destination)?;
    let source = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples/shader-basic/background.png");
    let mut draft = Draft::from_image(&source)?;
    for (preset, name) in [
        (Preset::Still, "clean"),
        (Preset::Gentle, "gentle"),
        (Preset::Dreamy, "dreamy"),
        (Preset::Pulse, "pulse"),
    ] {
        draft.design.preset(preset);
        draft.design.name = preset.to_string();
        draft.save_project(&destination.join(name))?;
        draft.export(&destination.join(format!("{name}.wisp")))?;
    }
    Ok(())
}
