use anyhow::{Result, ensure};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) enum PaintTool {
    #[default]
    HardRound,
    Airbrush,
    PolygonalLasso,
    MagicWand,
}
impl PaintTool {
    pub const ALL: [Self; 4] = [
        Self::HardRound,
        Self::Airbrush,
        Self::PolygonalLasso,
        Self::MagicWand,
    ];

    pub fn is_brush(self) -> bool {
        matches!(self, Self::HardRound | Self::Airbrush)
    }
}
impl std::fmt::Display for PaintTool {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::HardRound => "Hard round",
            Self::Airbrush => "Airbrush",
            Self::PolygonalLasso => "Polygonal lasso",
            Self::MagicWand => "Magic wand",
        })
    }
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct MagicWandOptions {
    pub tolerance: u8,
    pub all_matching: bool,
}
impl Default for MagicWandOptions {
    fn default() -> Self {
        Self {
            tolerance: 32,
            all_matching: false,
        }
    }
}

pub(crate) fn fill_magic_wand(
    mask: &mut image::GrayImage,
    source: &image::RgbaImage,
    point: [f32; 2],
    options: MagicWandOptions,
    value: u8,
) -> Result<[u32; 4]> {
    ensure!(
        point
            .into_iter()
            .all(|v| v.is_finite() && (0.0..=1.0).contains(&v)),
        "invalid magic wand coordinates"
    );
    ensure!(
        source.width() > 0 && source.height() > 0 && mask.width() > 0 && mask.height() > 0,
        "magic wand needs a source image and mask"
    );
    let width = source.width() as usize;
    let height = source.height() as usize;
    let seed_x = ((point[0] * width as f32) as usize).min(width - 1);
    let seed_y = ((point[1] * height as f32) as usize).min(height - 1);
    // Invisible RGB values must not split an otherwise uniform transparent region.
    let premultiplied = |pixel: &image::Rgba<u8>| {
        let [r, g, b, a] = pixel.0;
        let channel = |v| (u16::from(v) * u16::from(a) / 255) as u8;
        [channel(r), channel(g), channel(b), a]
    };
    let seed_color = premultiplied(source.get_pixel(seed_x as u32, seed_y as u32));
    // Comparing to the seed prevents a gentle gradient from leaking across the entire image.
    let matches = |index: usize| {
        premultiplied(source.get_pixel((index % width) as u32, (index / width) as u32))
            .into_iter()
            .zip(seed_color)
            .all(|(channel, seed)| channel.abs_diff(seed) <= options.tolerance)
    };
    let mut selected = vec![false; width * height];
    if options.all_matching {
        for (index, selected) in selected.iter_mut().enumerate() {
            *selected = matches(index);
        }
    } else {
        let seed = seed_y * width + seed_x;
        selected[seed] = true;
        let mut pending = vec![seed];
        while let Some(index) = pending.pop() {
            let x = index % width;
            let y = index / width;
            let neighbors = [
                (x > 0).then(|| index - 1),
                (x + 1 < width).then_some(index + 1),
                (y > 0).then(|| index - width),
                (y + 1 < height).then_some(index + width),
            ];
            for neighbor in neighbors.into_iter().flatten() {
                if !selected[neighbor] && matches(neighbor) {
                    selected[neighbor] = true;
                    pending.push(neighbor);
                }
            }
        }
    }
    let mask_width = u64::from(mask.width());
    let mask_height = u64::from(mask.height());
    let mut region = [mask.width(), mask.height(), 0, 0];
    for (x, y, pixel) in mask.enumerate_pixels_mut() {
        // Selection uses the displayed source's topology before mapping to the square mask.
        let source_x = ((u64::from(x) * 2 + 1) * width as u64 / (mask_width * 2)) as usize;
        let source_y = ((u64::from(y) * 2 + 1) * height as u64 / (mask_height * 2)) as usize;
        if selected[source_y * width + source_x] {
            pixel.0[0] = value;
            region[0] = region[0].min(x);
            region[1] = region[1].min(y);
            region[2] = region[2].max(x + 1);
            region[3] = region[3].max(y + 1);
        }
    }
    Ok(if region[2] == 0 { [0; 4] } else { region })
}

// Source aspect keeps brushes circular even though the retained mask is square.
pub(crate) fn paint_brush(
    image: &mut image::GrayImage,
    from: [f32; 2],
    to: [f32; 2],
    radius: f32,
    value: u8,
    aspect: f32,
    flow: Option<f32>,
) -> Result<[u32; 4]> {
    ensure!(
        from.into_iter()
            .chain(to)
            .all(|v| v.is_finite() && (0.0..=1.0).contains(&v))
            && radius.is_finite()
            && (0.005..=0.25).contains(&radius),
        "invalid brush stroke"
    );
    ensure!(aspect.is_finite() && aspect > 0., "invalid brush aspect");
    ensure!(
        flow.is_none_or(|flow| flow.is_finite() && (0.01..=0.5).contains(&flow)),
        "invalid airbrush flow"
    );
    let extent = [aspect.max(1.), (1. / aspect).max(1.)];
    let dimensions = [image.width() as f32, image.height() as f32];
    let a = [from[0] * extent[0], from[1] * extent[1]];
    let ab = [(to[0] - from[0]) * extent[0], (to[1] - from[1]) * extent[1]];
    let length2 = ab[0] * ab[0] + ab[1] * ab[1];
    let edge = (extent[0] / dimensions[0]).max(extent[1] / dimensions[1]);
    let low = [0, 1].map(|i| {
        ((from[i].min(to[i]) - radius / extent[i]) * dimensions[i])
            .floor()
            .max(0.) as u32
    });
    let high = [0, 1].map(|i| {
        ((from[i].max(to[i]) + radius / extent[i]) * dimensions[i])
            .ceil()
            .min(dimensions[i]) as u32
    });
    // Overlapping dabs keep fast airbrush strokes connected without hard capsule edges.
    let steps = (length2.sqrt() / (radius * 0.25)).ceil().max(1.) as usize;
    for y in low[1]..high[1] {
        for x in low[0]..high[0] {
            let q = [
                (x as f32 + 0.5) / dimensions[0] * extent[0] - a[0],
                (y as f32 + 0.5) / dimensions[1] * extent[1] - a[1],
            ];
            let t = ((q[0] * ab[0] + q[1] * ab[1]) / length2.max(0.0000001)).clamp(0., 1.);
            let distance = ((q[0] - t * ab[0]).powi(2) + (q[1] - t * ab[1]).powi(2)).sqrt();
            let alpha = if let Some(flow) = flow {
                let mut remaining = 1.;
                for step in 0..=if length2 > 0. { steps } else { 0 } {
                    let fraction = step as f32 / steps as f32;
                    let distance = ((q[0] - fraction * ab[0]).powi(2)
                        + (q[1] - fraction * ab[1]).powi(2))
                    .sqrt();
                    remaining *= 1. - flow * (1. - distance / radius).max(0.).powi(2);
                }
                1. - remaining
            } else {
                ((radius - distance) / edge).clamp(0., 1.)
            };
            let pixel = image.get_pixel_mut(x, y);
            let previous = pixel.0[0];
            let blended = (previous as f32 * (1. - alpha) + value as f32 * alpha).round() as u8;
            // Low flow must keep responding near black and white despite the mask's 8-bit precision.
            pixel.0[0] = if flow.is_some() && alpha > 0. && blended == previous {
                match previous.cmp(&value) {
                    std::cmp::Ordering::Less => previous + 1,
                    std::cmp::Ordering::Greater => previous - 1,
                    std::cmp::Ordering::Equal => previous,
                }
            } else {
                blended
            };
        }
    }
    Ok([low[0], low[1], high[0], high[1]])
}

pub(crate) fn fill_polygon(
    image: &mut image::GrayImage,
    vertices: &[[f32; 2]],
    value: u8,
) -> Result<[u32; 4]> {
    ensure!(
        vertices.len() >= 3,
        "a polygon needs at least three vertices"
    );
    ensure!(
        vertices
            .iter()
            .flatten()
            .all(|v| v.is_finite() && (0.0..=1.0).contains(v)),
        "invalid polygon coordinates"
    );
    let dimensions = [image.width() as f32, image.height() as f32];
    let low = [0, 1].map(|axis| {
        (vertices.iter().map(|v| v[axis]).fold(1., f32::min) * dimensions[axis]).floor() as u32
    });
    let high = [0, 1].map(|axis| {
        (vertices.iter().map(|v| v[axis]).fold(0., f32::max) * dimensions[axis]).ceil() as u32
    });
    let mut crossings = Vec::with_capacity(vertices.len());
    for y in low[1]..high[1] {
        let scan_y = (y as f32 + 0.5) / dimensions[1];
        crossings.clear();
        // Even-odd filling supports concave outlines and keeps shared vertices from counting twice.
        for (a, b) in vertices.iter().zip(vertices.iter().cycle().skip(1)) {
            if (a[1] > scan_y) != (b[1] > scan_y) {
                crossings.push(a[0] + (scan_y - a[1]) * (b[0] - a[0]) / (b[1] - a[1]));
            }
        }
        crossings.sort_by(f32::total_cmp);
        for pair in crossings.chunks_exact(2) {
            let start = ((pair[0] * dimensions[0] - 0.5).ceil() as u32).max(low[0]);
            let end = ((pair[1] * dimensions[0] - 0.5).ceil() as u32).min(high[0]);
            for x in start..end {
                image.put_pixel(x, y, image::Luma([value]));
            }
        }
    }
    Ok([low[0], low[1], high[0], high[1]])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wand_selects_connected_colors_without_diagonal_or_row_wrapping_leaks() {
        let red = image::Rgba([180, 20, 30, 255]);
        let blue = image::Rgba([20, 30, 180, 255]);
        let mut source = image::RgbaImage::from_pixel(3, 3, blue);
        for (x, y) in [(0, 0), (1, 0), (2, 1), (0, 2)] {
            source.put_pixel(x, y, red);
        }
        let mut mask = image::GrayImage::from_pixel(3, 3, image::Luma([80]));
        let options = MagicWandOptions {
            tolerance: 0,
            all_matching: false,
        };
        assert_eq!(
            fill_magic_wand(&mut mask, &source, [0., 0.], options, 255).unwrap(),
            [0, 0, 2, 1]
        );
        assert_eq!(mask.as_raw(), &[255, 255, 80, 80, 80, 80, 80, 80, 80]);
        fill_magic_wand(
            &mut mask,
            &source,
            [0., 0.],
            MagicWandOptions {
                all_matching: true,
                ..options
            },
            0,
        )
        .unwrap();
        assert_eq!(mask.as_raw(), &[0, 0, 80, 80, 80, 0, 0, 80, 80]);
    }

    #[test]
    fn wand_tolerance_is_inclusive_relative_to_the_seed_and_includes_alpha() {
        let source = image::RgbaImage::from_fn(5, 1, |x, _| {
            image::Rgba(match x {
                0 => [100, 20, 30, 255],
                1 => [110, 20, 30, 255],
                2 => [120, 20, 30, 255],
                3 => [100, 20, 30, 0],
                _ => [100, 20, 30, 255],
            })
        });
        for (tolerance, expected) in [
            (0, vec![255, 0, 0, 0, 0]),
            (10, vec![255, 255, 0, 0, 0]),
            (20, vec![255, 255, 255, 0, 0]),
            (255, vec![255; 5]),
        ] {
            let mut mask = image::GrayImage::new(5, 1);
            fill_magic_wand(
                &mut mask,
                &source,
                [0.; 2],
                MagicWandOptions {
                    tolerance,
                    all_matching: false,
                },
                255,
            )
            .unwrap();
            assert_eq!(mask.into_raw(), expected);
        }
        let transparent =
            image::RgbaImage::from_fn(2, 1, |x, _| image::Rgba([x as u8 * 255, 40, 90, 0]));
        let mut mask = image::GrayImage::new(2, 1);
        fill_magic_wand(
            &mut mask,
            &transparent,
            [1.; 2],
            MagicWandOptions {
                tolerance: 0,
                all_matching: false,
            },
            255,
        )
        .unwrap();
        assert_eq!(mask.into_raw(), vec![255, 255]);
    }

    #[test]
    fn wand_maps_source_regions_to_different_mask_sizes_and_accepts_image_edges() {
        let source = image::RgbaImage::from_fn(4, 2, |x, _| {
            image::Rgba([if x < 2 { 0 } else { 255 }, 0, 0, 255])
        });
        let mut mask = image::GrayImage::new(8, 8);
        assert_eq!(
            fill_magic_wand(
                &mut mask,
                &source,
                [1., 1.],
                MagicWandOptions::default(),
                255
            )
            .unwrap(),
            [4, 0, 8, 8]
        );
        assert!(
            mask.enumerate_pixels()
                .all(|(x, _, p)| p.0[0] == if x < 4 { 0 } else { 255 })
        );
        let mut smaller = image::GrayImage::new(2, 1);
        fill_magic_wand(
            &mut smaller,
            &source,
            [0., 0.],
            MagicWandOptions::default(),
            255,
        )
        .unwrap();
        assert_eq!(smaller.into_raw(), vec![255, 0]);
    }

    #[test]
    fn invalid_wand_inputs_leave_the_mask_unchanged() {
        let source = image::RgbaImage::new(2, 2);
        let mut mask = image::GrayImage::from_pixel(2, 2, image::Luma([80]));
        for point in [[f32::NAN, 0.], [0., f32::INFINITY], [-0.1, 0.], [0., 1.1]] {
            assert!(
                fill_magic_wand(&mut mask, &source, point, MagicWandOptions::default(), 255)
                    .is_err()
            );
        }
        assert!(
            fill_magic_wand(
                &mut mask,
                &image::RgbaImage::new(0, 0),
                [0.; 2],
                MagicWandOptions::default(),
                255
            )
            .is_err()
        );
        assert_eq!(mask.into_raw(), vec![80; 4]);
        assert!(
            fill_magic_wand(
                &mut image::GrayImage::new(0, 0),
                &source,
                [0.; 2],
                MagicWandOptions::default(),
                255
            )
            .is_err()
        );
    }

    #[test]
    fn airbrush_has_soft_falloff_builds_up_and_erases() {
        let mut image = image::GrayImage::new(200, 200);
        let dab = |image: &mut image::GrayImage, value| {
            paint_brush(image, [0.5, 0.5], [0.5, 0.5], 0.2, value, 1., Some(0.2)).unwrap()
        };
        assert_eq!(dab(&mut image, 255), [60, 60, 140, 140]);
        let center = image.get_pixel(100, 100).0[0];
        let edge = image.get_pixel(130, 100).0[0];
        assert!(center > edge && edge > 0 && center < 255);
        assert_eq!(image.get_pixel(145, 100).0[0], 0);
        dab(&mut image, 255);
        let built_up = image.get_pixel(100, 100).0[0];
        assert!(built_up > center);
        dab(&mut image, 0);
        assert!(image.get_pixel(100, 100).0[0] < built_up);
    }

    #[test]
    fn low_flow_reaches_white_and_black_without_stalling_at_rounding_thresholds() {
        let mut image = image::GrayImage::new(16, 16);
        for value in [255, 0] {
            for _ in 0..300 {
                paint_brush(&mut image, [0.5; 2], [0.5; 2], 0.25, value, 1., Some(0.01)).unwrap();
            }
            assert_eq!(image.get_pixel(8, 8).0[0], value);
            assert_eq!(image.get_pixel(0, 0).0[0], 0);
        }
    }

    #[test]
    fn brushes_remain_circular_on_wide_and_tall_sources_and_connect_fast_moves() {
        for aspect in [2., 0.5] {
            for flow in [None, Some(0.2)] {
                let mut image = image::GrayImage::new(400, 400);
                paint_brush(&mut image, [0.5, 0.5], [0.5, 0.5], 0.2, 255, aspect, flow).unwrap();
                let horizontal = image.get_pixel(if aspect > 1. { 220 } else { 240 }, 200).0[0];
                let vertical = image.get_pixel(200, if aspect > 1. { 240 } else { 220 }).0[0];
                assert!((i16::from(horizontal) - i16::from(vertical)).abs() <= 2);
                paint_brush(&mut image, [0.1, 0.2], [0.9, 0.2], 0.03, 255, aspect, flow).unwrap();
                assert!((40..360).all(|x| image.get_pixel(x, 80).0[0] > 0));
            }
        }
    }

    #[test]
    fn polygon_fills_concave_regions_and_erases_without_touching_the_exterior() {
        let vertices = [
            [0.1, 0.1],
            [0.9, 0.1],
            [0.9, 0.4],
            [0.4, 0.4],
            [0.4, 0.9],
            [0.1, 0.9],
        ];
        for reversed in [false, true] {
            let mut vertices = vertices.to_vec();
            if reversed {
                vertices.reverse();
            }
            let mut image = image::GrayImage::new(100, 100);
            assert_eq!(
                fill_polygon(&mut image, &vertices, 255).unwrap(),
                [10, 10, 90, 90]
            );
            assert_eq!(image.get_pixel(20, 70).0[0], 255);
            assert_eq!(image.get_pixel(70, 20).0[0], 255);
            assert_eq!(image.get_pixel(70, 70).0[0], 0);
            assert_eq!(image.get_pixel(5, 5).0[0], 0);
            fill_polygon(&mut image, &vertices, 0).unwrap();
            assert!(image.pixels().all(|p| p.0[0] == 0));
        }
    }

    #[test]
    fn invalid_inputs_leave_the_raster_unchanged_and_edge_polygons_are_safe() {
        let mut image = image::GrayImage::new(16, 16);
        for vertices in [
            vec![],
            vec![[0., 0.]; 2],
            vec![[f32::NAN, 0.]; 3],
            vec![[1.1, 0.]; 3],
        ] {
            assert!(fill_polygon(&mut image, &vertices, 255).is_err());
        }
        for flow in [Some(f32::NAN), Some(0.), Some(1.)] {
            assert!(paint_brush(&mut image, [0.5; 2], [0.5; 2], 0.1, 255, 1., flow).is_err());
        }
        assert!(image.pixels().all(|p| p.0[0] == 0));
        fill_polygon(&mut image, &[[0., 0.], [1., 0.], [1., 1.], [0., 1.]], 255).unwrap();
        assert!(image.pixels().all(|p| p.0[0] == 255));
    }
}
