use anyhow::{Result, ensure};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) enum PaintTool {
    #[default]
    HardRound,
    Airbrush,
    PolygonalLasso,
}
impl PaintTool {
    pub const ALL: [Self; 3] = [Self::HardRound, Self::Airbrush, Self::PolygonalLasso];
}
impl std::fmt::Display for PaintTool {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::HardRound => "Hard round",
            Self::Airbrush => "Airbrush",
            Self::PolygonalLasso => "Polygonal lasso",
        })
    }
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
            pixel.0[0] = (pixel.0[0] as f32 * (1. - alpha) + value as f32 * alpha).round() as u8;
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
