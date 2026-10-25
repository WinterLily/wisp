//! Generate deterministic layer/mask fixtures and portable parity packages.
use anyhow::{Context, Result};
use std::path::PathBuf;
use wisp_app::{
    designer::{Draft, Preset},
    layers::Effect,
};
fn main() -> Result<()> {
    let destination = PathBuf::from(
        std::env::args_os()
            .nth(1)
            .context("usage: layer-effects NEW_DIRECTORY")?,
    );
    std::fs::create_dir(&destination)?;
    let source = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples/shader-basic/background.png");
    // Prepared transparent character parts, deliberately simple test artwork.
    let character = image::RgbaImage::from_fn(256, 256, |x, y| {
        let x = x as f32 / 256.;
        let y = y as f32 / 256.;
        let hair = ((x - 0.5) / 0.23).powi(2) + ((y - 0.37) / 0.3).powi(2) < 1.;
        let face = ((x - 0.5) / 0.12).powi(2) + ((y - 0.3) / 0.15).powi(2) < 1.;
        let body = y > 0.48 && y < 0.94 && (x - 0.5).abs() < 0.14 + (y - 0.48) * 0.25;
        if face {
            image::Rgba([245, 193, 160, 255])
        } else if body {
            image::Rgba([35 + (x * 100.) as u8, 145, 165 + (y * 60.) as u8, 255])
        } else if hair {
            image::Rgba([65 + (x * 120.) as u8, 35 + (y * 90.) as u8, 130, 255])
        } else {
            image::Rgba([0, 0, 0, 0])
        }
    });
    let character_path = destination.join("character.png");
    character.save(&character_path)?;
    let mut draft = Draft::from_image(&source)?;
    draft.design.preset(Preset::Still);
    let stack = draft.enable_layers();
    stack.import(&character_path, None)?;
    stack.layers[1].name = "Character".into();
    stack.blank_mask(1, false)?;
    for x in [0.32, 0.68] {
        stack.paint(1, [x, 0.23], [x, 0.47], 0.075, 255)?;
    }
    stack.layers[1].preset(Effect::Hair);
    for (name, effect) in [
        ("layers-hair", Effect::Hair),
        ("layers-clothing", Effect::Clothing),
        ("layers-wave", Effect::Wave),
        ("layers-warp", Effect::Warp),
        ("layers-displacement", Effect::Displacement),
        ("layers-droplets", Effect::Droplets),
        ("layers-rain", Effect::Rain),
        ("layers-ripples", Effect::Ripples),
    ] {
        let mut current = draft.clone();
        current.design.name = name.into();
        let stack = current.layers.as_mut().unwrap();
        if matches!(effect, Effect::Droplets | Effect::Rain | Effect::Ripples) {
            stack.layers[0].preset(effect);
        } else {
            stack.layers[1].preset(effect);
            if effect == Effect::Clothing {
                stack.blank_mask(1, false)?;
                stack.paint(1, [0.4, 0.75], [0.6, 0.85], 0.15, 255)?;
            }
            if effect == Effect::Displacement {
                stack.layers[1].displacement = Some("background".into());
            }
        }
        current.save_project(&destination.join(name))?;
        current.export(&destination.join(format!("{name}.wisp")))?;
    }
    std::fs::remove_file(character_path)?;
    Ok(())
}
