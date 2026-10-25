use anyhow::{Context, Result, ensure};
use image::{ImageReader, Limits, RgbaImage};
use std::io::Cursor;

const MAX_ENCODED_BYTES: usize = 32 * 1024 * 1024;
const MAX_DIMENSION: u32 = 4096;

/// Decoded, bounded sRGB image. Decoding happens only while loading a wallpaper.
pub struct WallpaperImage {
    pub(crate) pixels: RgbaImage,
}

impl WallpaperImage {
    pub fn decode(bytes: &[u8]) -> Result<Self> {
        ensure!(
            bytes.len() <= MAX_ENCODED_BYTES,
            "image exceeds the 32 MiB encoded size limit"
        );
        let mut reader = ImageReader::new(Cursor::new(bytes)).with_guessed_format()?;
        let mut limits = Limits::default();
        limits.max_image_width = Some(MAX_DIMENSION);
        limits.max_image_height = Some(MAX_DIMENSION);
        limits.max_alloc = Some(64 * 1024 * 1024);
        reader.limits(limits);
        let pixels = reader
            .decode()
            .context("could not decode wallpaper image (PNG, JPEG or WebP; maximum 4096 × 4096)")?
            .to_rgba8();
        ensure!(
            pixels.width() > 0 && pixels.height() > 0,
            "image dimensions must be nonzero"
        );
        Ok(Self { pixels })
    }

    pub fn dimensions(&self) -> (u32, u32) {
        self.pixels.dimensions()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_bundled_image_and_rejects_invalid_input() {
        assert_eq!(
            WallpaperImage::decode(crate::EXAMPLE_IMAGE)
                .unwrap()
                .dimensions(),
            (256, 256)
        );
        assert!(WallpaperImage::decode(b"not an image").is_err());
    }

    #[test]
    fn rejects_image_dimensions_before_allocating_pixels() {
        let mut bytes = Cursor::new(Vec::new());
        image::DynamicImage::new_rgba8(4097, 1)
            .write_to(&mut bytes, image::ImageFormat::Png)
            .unwrap();
        assert!(WallpaperImage::decode(bytes.get_ref()).is_err());
    }
}
