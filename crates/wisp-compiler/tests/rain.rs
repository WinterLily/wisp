use std::collections::BTreeMap;
use wisp_compiler::{CompileOptions, compile};
use wisp_format::{Manifest, ParameterValue};
use wisp_ir::Graph;
use wisp_runtime::{Globals, Gpu, Renderer, Wallpaper};

fn rain_wallpaper(direction: f32, density: f32, size_x: f32, size_y: f32) -> Wallpaper {
    let graph: Graph = serde_json::from_value(serde_json::json!({
        "version": 1, "output": "out",
        "parameters": [
            {"id":"speed","default":{"type":"float","value":1.0}},
            {"id":"strength","default":{"type":"float","value":1.0}},
            {"id":"mask","default":{"type":"float","value":1.0}}
        ],
        "nodes": [
            {"id":"uv","op":{"type":"uv"}},
            {"id":"color","op":{"type":"constant","value":{"type":"color","value":[0.0,0.0,0.0,0.6]}}},
            {"id":"speed","op":{"type":"parameter","parameter":"speed"}},
            {"id":"strength","op":{"type":"parameter","parameter":"strength"}},
            {"id":"mask","op":{"type":"parameter","parameter":"mask"}},
            {"id":"density","op":{"type":"constant","value":{"type":"float","value":density}}},
            {"id":"size","op":{"type":"constant","value":{"type":"float","value":0.25}}},
            {"id":"rain","op":{"type":"weather","kind":"rain","color":"color","uv":"uv",
                "mask":"mask","strength":"strength","speed":"speed","density":"density","size":"size",
                "rain_direction":direction,"size_x":size_x,"size_y":size_y}},
            {"id":"out","op":{"type":"output","color":"rain"}}
        ]
    }))
    .unwrap();
    let manifest = Manifest {
        format_version: 1,
        id: "test.rain".into(),
        name: "Rain".into(),
        author: "Wisp".into(),
        description: String::new(),
        preview: None,
        program: "program.json".into(),
        minimum_wisp_version: "0.1.0".into(),
        gpu_profile: 1,
        tags: vec![],
    };
    let compilation = compile(
        &graph.check().unwrap(),
        &BTreeMap::new(),
        manifest,
        &CompileOptions::default(),
    )
    .unwrap();
    Wallpaper::from_package(&compilation.package).unwrap()
}

#[test]
fn rain_graph_compiles_with_direction_and_axis_extremes() {
    for direction in [-180., -90., 0., 20., 90., 180.] {
        for density in [1., 32.] {
            for (size_x, size_y) in [(0.2, 2.), (2., 0.2)] {
                rain_wallpaper(direction, density, size_x, size_y);
            }
        }
    }
}

// Correlation measures actual image motion without depending on individual random drop seeds.
fn motion_offset(before: &[u8], after: &[u8], width: usize, height: usize) -> usize {
    (0..40)
        .max_by_key(|offset| {
            let mut correlation = 0u64;
            for y in 40..height - 40 {
                for x in 0..width {
                    correlation += u64::from(before[(y * width + x) * 4])
                        * u64::from(after[((y + offset) * width + x) * 4]);
                }
            }
            correlation
        })
        .unwrap()
}

#[test]
#[ignore = "requires a working graphics adapter"]
fn rain_renders_thin_fast_streaks_and_respects_rotation_mask_strength_and_pause() {
    pollster::block_on(async {
        let gpu = Gpu::new(
            &wgpu::Instance::new(&wgpu::InstanceDescriptor::from_env_or_default()),
            None,
        )
        .await
        .unwrap();
        let render = |wallpaper: &Wallpaper| {
            pollster::block_on(Renderer::from_wallpaper(
                &gpu,
                wallpaper,
                wgpu::TextureFormat::Rgba8UnormSrgb,
            ))
            .unwrap()
        };
        let width = 512;
        let height = 512;
        let mut globals = Globals::new(width, height);
        globals.time = 3.;
        let mut vertical = render(&rain_wallpaper(0., 10., 1., 1.));
        let first = vertical.render_rgba(&gpu, width, height, &globals).unwrap();
        assert!(first.chunks_exact(4).all(|pixel| pixel[3] == 153));
        let mut horizontal_edges = 0u64;
        let mut vertical_edges = 0u64;
        for y in 1..height as usize {
            for x in 1..width as usize {
                let index = (y * width as usize + x) * 4;
                horizontal_edges += u64::from(first[index].abs_diff(first[index - 4]));
                vertical_edges +=
                    u64::from(first[index].abs_diff(first[index - width as usize * 4]));
            }
        }
        assert!(
            horizontal_edges > vertical_edges * 6,
            "rain became rounded instead of long and narrow: horizontal edges {horizontal_edges}, vertical edges {vertical_edges}"
        );

        let mut short_streaks = 0;
        let mut long_streaks = 0;
        for x in 0..width as usize {
            let mut run = 0;
            for y in 0..height as usize {
                if first[(y * width as usize + x) * 4] > 24 {
                    run += 1;
                } else {
                    if (3..=12).contains(&run) {
                        short_streaks += 1;
                    }
                    if run >= 20 {
                        long_streaks += 1;
                    }
                    run = 0;
                }
            }
        }
        assert!(
            short_streaks > 50 && long_streaks > 20 && short_streaks > long_streaks,
            "rain lost depth variation: {short_streaks} short, {long_streaks} long streaks"
        );

        let rotated = render(&rain_wallpaper(90., 10., 1., 1.))
            .render_rgba(&gpu, width, height, &globals)
            .unwrap();
        for y in 0..height as usize {
            for x in 0..width as usize {
                let index = (y * width as usize + x) * 4;
                let reference = (x * width as usize + height as usize - 1 - y) * 4;
                assert!(
                    rotated[index].abs_diff(first[reference]) <= 2,
                    "direction did not rotate the streaks"
                );
            }
        }

        globals.time += 1. / 30.;
        let moved = vertical.render_rgba(&gpu, width, height, &globals).unwrap();
        let offset = motion_offset(&first, &moved, width as usize, height as usize);
        assert!(
            (14..=36).contains(&offset),
            "rain moved {offset} pixels in 1/30 second"
        );
        let sparse = render(&rain_wallpaper(0., 4., 1., 1.));
        globals.time = 3.;
        let sparse_first = sparse.render_rgba(&gpu, width, height, &globals).unwrap();
        globals.time += 1. / 30.;
        let sparse_moved = sparse.render_rgba(&gpu, width, height, &globals).unwrap();
        let offset = motion_offset(
            &sparse_first,
            &sparse_moved,
            width as usize,
            height as usize,
        );
        assert!(
            (14..=36).contains(&offset),
            "density slowed rain to {offset} pixels in 1/30 second"
        );

        vertical
            .set_parameter(&gpu.queue, "speed", ParameterValue::Float(0.))
            .unwrap();
        let paused = vertical.render_rgba(&gpu, width, height, &globals).unwrap();
        globals.time += 1.;
        assert_eq!(
            paused,
            vertical.render_rgba(&gpu, width, height, &globals).unwrap()
        );
        for parameter in ["mask", "strength"] {
            vertical
                .set_parameter(&gpu.queue, parameter, ParameterValue::Float(0.))
                .unwrap();
            let disabled = vertical.render_rgba(&gpu, width, height, &globals).unwrap();
            assert!(
                disabled
                    .chunks_exact(4)
                    .all(|pixel| pixel == [0, 0, 0, 153])
            );
            vertical
                .set_parameter(&gpu.queue, parameter, ParameterValue::Float(1.))
                .unwrap();
        }
    });
}
