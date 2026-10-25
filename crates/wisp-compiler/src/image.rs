//! Local authoring sources keep their original bytes; portable images are prepared
//! separately. Package loading continues to use the bounded runtime decoder.
use crate::Asset;
use anyhow::{Context, Result, ensure};
use image::{DynamicImage, ImageFormat, ImageReader, imageops::FilterType};
use std::{
    fs::File,
    io::{Cursor, Read},
    path::Path,
};

/// Read a local source image without a file-size cap. Reject devices and FIFOs.
pub fn read_source_image(path: &Path) -> Result<Vec<u8>> {
    ensure!(
        std::fs::metadata(path)?.is_file(),
        "image must be a regular file"
    );
    let mut file = File::open(path)?;
    ensure!(file.metadata()?.is_file(), "image must be a regular file");
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)?;
    Ok(bytes)
}

/// Decode a local authoring image without fixed dimension or allocation caps.
/// Actual allocation/codec errors still propagate to the editor.
pub fn decode_source_image(bytes: &[u8]) -> Result<DynamicImage> {
    let mut reader = ImageReader::new(Cursor::new(bytes)).with_guessed_format()?;
    ensure!(
        matches!(
            reader.format(),
            Some(ImageFormat::Png | ImageFormat::Jpeg | ImageFormat::WebP)
        ),
        "choose a PNG, JPEG or WebP image"
    );
    reader.no_limits();
    let image = reader.decode().context("could not decode source image")?;
    ensure!(
        image.width() > 0 && image.height() > 0,
        "image dimensions must be nonzero"
    );
    Ok(image)
}

/// Produce a runtime-compatible copy, retaining alpha and aspect ratio. Sources
/// that already fit are copied verbatim. Budgets apply only to generated assets.
pub(crate) fn prepare(
    source: &Asset,
    max_dimension: u32,
    max_pixels: u64,
    max_encoded_bytes: u64,
) -> Result<(Asset, bool)> {
    ensure!(
        max_dimension > 0 && max_pixels > 0,
        "no texture budget available for images"
    );
    let max_dimension = max_dimension.min(4096);
    let max_encoded_bytes = max_encoded_bytes.min(32 * 1024 * 1024);
    if source.bytes.len() as u64 <= max_encoded_bytes
        && let Ok(image) = wisp_runtime::WallpaperImage::decode(&source.bytes)
    {
        let (w, h) = image.dimensions();
        if w <= max_dimension && h <= max_dimension && u64::from(w) * u64::from(h) <= max_pixels {
            return Ok((source.clone(), false));
        }
    }
    let decoded = decode_source_image(&source.bytes)?;
    let (w, h) = (decoded.width(), decoded.height());
    let scale = (f64::from(max_dimension) / f64::from(w.max(h)))
        .min((max_pixels as f64 / (f64::from(w) * f64::from(h))).sqrt())
        .min(1.);
    let mut width = (f64::from(w) * scale).floor().max(1.) as u32;
    let mut height = (f64::from(h) * scale).floor().max(1.) as u32;
    // Extremely thin images can round a sub-pixel side up to one pixel.
    width = width.min(max_pixels.min(u64::from(u32::MAX)) as u32);
    height = height.min(max_pixels.min(u64::from(u32::MAX)) as u32);
    let original = DynamicImage::ImageRgba8(decoded.into_rgba8());
    loop {
        let resized = original.resize_exact(width, height, FilterType::Lanczos3);
        let mut bytes = Cursor::new(Vec::new());
        resized.write_to(&mut bytes, ImageFormat::Png)?;
        if bytes.get_ref().len() as u64 <= max_encoded_bytes {
            return Ok((
                Asset {
                    extension: "png".into(),
                    bytes: bytes.into_inner(),
                },
                (width, height) != (w, h),
            ));
        }
        ensure!(
            width > 1 || height > 1,
            "package byte budget is too small for an image"
        );
        // Re-encode at a smaller size if lossless PNG exceeds the portable budget.
        // Always resample the original, avoiding cumulative filtering artifacts.
        let scale = (max_encoded_bytes as f64 / bytes.get_ref().len() as f64)
            .sqrt()
            .min(0.9);
        width = (f64::from(width) * scale).floor().max(1.) as u32;
        height = (f64::from(height) * scale).floor().max(1.) as u32;
    }
}

/// Share budgets according to actual image costs, so small masks do not force a
/// large background down to an unnecessarily small equal share of GPU memory.
pub(crate) fn prepare_all(
    sources: &[&Asset],
    max_dimension: u32,
    pixel_budget: u64,
    file_budget: u64,
    encoded_budget: u64,
) -> Result<Vec<(Asset, bool)>> {
    let mut prepared = sources
        .iter()
        .map(|source| {
            prepare(
                source,
                max_dimension,
                pixel_budget,
                file_budget.min(encoded_budget),
            )
        })
        .collect::<Result<Vec<_>>>()?;
    let pixels = prepared
        .iter()
        .map(|(asset, _)| {
            let (w, h) = ImageReader::new(Cursor::new(&asset.bytes))
                .with_guessed_format()?
                .into_dimensions()?;
            Ok(u64::from(w) * u64::from(h))
        })
        .collect::<Result<Vec<_>>>()?;
    let total_pixels = pixels.iter().sum::<u64>();
    let total_encoded = prepared
        .iter()
        .map(|(asset, _)| asset.bytes.len() as u64)
        .sum::<u64>();
    if total_pixels > pixel_budget || total_encoded > encoded_budget {
        let count = sources.len() as u64;
        ensure!(
            pixel_budget >= count,
            "no texture budget available for images"
        );
        // A one-pixel mask still needs one pixel and a complete PNG header. Leave
        // room for these minima instead of rounding its proportional share to zero.
        let encoded_minima: Vec<_> = prepared
            .iter()
            .map(|(asset, _)| (asset.bytes.len() as u64).min(128))
            .collect();
        let minimum_encoded = encoded_minima.iter().sum::<u64>();
        ensure!(
            encoded_budget >= minimum_encoded,
            "package byte budget is too small for images"
        );
        for (i, copy) in prepared.iter_mut().enumerate() {
            let pixels = if total_pixels > pixel_budget {
                1 + (pixels[i] - 1) * (pixel_budget - count) / (total_pixels - count)
            } else {
                pixel_budget
            };
            // A resized JPEG may grow when encoded as PNG. Allocate the total
            // byte budget even when only the texture budget triggered this pass.
            let encoded = if total_encoded > minimum_encoded {
                encoded_minima[i]
                    + (u128::from(copy.0.bytes.len() as u64 - encoded_minima[i])
                        * u128::from(encoded_budget - minimum_encoded)
                        / u128::from(total_encoded - minimum_encoded)) as u64
            } else {
                encoded_budget / count
            };
            // Resample the source, never an already resized copy.
            *copy = prepare(sources[i], max_dimension, pixels, encoded.min(file_budget))
                .with_context(|| format!("image {} exceeds available playback budget", i + 1))?;
        }
    }
    Ok(prepared)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runtime_copies_fit_both_memory_and_encoded_budgets() {
        let pixels = image::RgbaImage::from_fn(513, 257, |x, y| {
            let n = (x + y * 513).wrapping_mul(2_654_435_761);
            image::Rgba([n as u8, (n >> 8) as u8, (n >> 16) as u8, 170])
        });
        let mut bytes = Cursor::new(Vec::new());
        pixels.write_to(&mut bytes, ImageFormat::Png).unwrap();
        let source = Asset {
            extension: "png".into(),
            bytes: bytes.into_inner(),
        };
        let (copy, resized) = prepare(&source, 128, 4096, 1000).unwrap();
        assert!(resized);
        assert!(copy.bytes.len() <= 1000);
        let pixels = wisp_runtime::WallpaperImage::decode(&copy.bytes).unwrap();
        let (w, h) = pixels.dimensions();
        assert!(w <= 128 && h <= 128 && u64::from(w) * u64::from(h) <= 4096);
        assert!((w as f64 / h as f64 - 2.).abs() < 0.3);
        let decoded = decode_source_image(&copy.bytes).unwrap().into_rgba8();
        assert!(decoded.pixels().all(|p| p[3] == 170));
        assert_eq!(decode_source_image(&source.bytes).unwrap().width(), 513);
        assert!(prepare(&source, 128, 0, 1000).is_err());
        assert!(prepare(&source, 128, 4096, 1).is_err());
    }

    #[test]
    fn compatible_images_keep_their_original_encoding() {
        let source = Asset {
            extension: "png".into(),
            bytes: wisp_runtime::EXAMPLE_IMAGE.to_vec(),
        };
        let (copy, resized) = prepare(&source, 4096, 4096 * 4096, 32 * 1024 * 1024).unwrap();
        assert!(!resized);
        assert_eq!(copy.extension, source.extension);
        assert_eq!(copy.bytes, source.bytes);
    }

    #[test]
    fn small_masks_leave_available_budget_for_larger_images() {
        let source = Asset {
            extension: "png".into(),
            bytes: wisp_runtime::EXAMPLE_IMAGE.to_vec(),
        };
        let mask = Asset {
            extension: "png".into(),
            bytes: include_bytes!("fallback.png").to_vec(),
        };
        let prepared = prepare_all(
            &[&source, &mask],
            4096,
            256 * 256 + 1,
            32 * 1024 * 1024,
            64 * 1024 * 1024,
        )
        .unwrap();
        assert_eq!(prepared[0].0.bytes, source.bytes);
        assert!(!prepared[0].1);
        assert_eq!(prepared[1].0.bytes, mask.bytes);

        let prepared = prepare_all(
            &[&source, &mask],
            4096,
            1025,
            32 * 1024 * 1024,
            64 * 1024 * 1024,
        )
        .unwrap();
        assert!(prepared[0].1);
        assert_eq!(prepared[1].0.bytes, mask.bytes);
        let byte_budget = 128 + mask.bytes.len() as u64;
        let prepared = prepare_all(
            &[&source, &mask],
            4096,
            256 * 256 + 1,
            32 * 1024 * 1024,
            byte_budget,
        )
        .unwrap();
        assert!(
            prepared
                .iter()
                .map(|(asset, _)| asset.bytes.len() as u64)
                .sum::<u64>()
                <= byte_budget
        );
        assert_eq!(prepared[1].0.bytes, mask.bytes);
    }

    #[test]
    fn jpeg_resampling_cannot_overrun_the_combined_encoded_budget() {
        let pixels = image::RgbImage::from_fn(512, 512, |x, y| {
            let n = (x + y * 512).wrapping_mul(2_654_435_761);
            image::Rgb([n as u8, (n >> 8) as u8, (n >> 16) as u8])
        });
        let mut bytes = Vec::new();
        image::codecs::jpeg::JpegEncoder::new_with_quality(&mut bytes, 10)
            .encode_image(&pixels)
            .unwrap();
        let source = Asset {
            extension: "jpg".into(),
            bytes,
        };
        let byte_budget = source.bytes.len() as u64 * 2 + 10_000;
        let pixel_budget = 256 * 256 * 2;
        let prepared = prepare_all(
            &[&source, &source],
            4096,
            pixel_budget,
            byte_budget,
            byte_budget,
        )
        .unwrap();
        assert!(prepared.iter().all(|(_, resized)| *resized));
        assert!(
            prepared
                .iter()
                .map(|(asset, _)| asset.bytes.len() as u64)
                .sum::<u64>()
                <= byte_budget
        );
        let used_pixels = prepared
            .iter()
            .map(|(asset, _)| {
                let decoded = decode_source_image(&asset.bytes).unwrap();
                u64::from(decoded.width()) * u64::from(decoded.height())
            })
            .sum::<u64>();
        assert!(used_pixels <= pixel_budget);
    }
}
