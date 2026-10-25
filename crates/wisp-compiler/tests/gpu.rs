use std::{collections::BTreeMap, io::Cursor, path::Path};
use wisp_compiler::{Asset, CompileOptions, compile, compile_project};
use wisp_format::{Manifest, Package, PackageLimits, Parameter, ParameterValue};
use wisp_ir::{Graph, Node, Op};
use wisp_runtime::{Globals, Gpu, Renderer, Shader, Wallpaper, WallpaperImage};

#[test]
#[ignore = "requires a working graphics adapter"]
fn fused_graph_matches_handwritten_shader_and_live_parameters() {
    pollster::block_on(async {
        let compilation = compile_project(
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/graph-fused"),
            &CompileOptions::default(),
        )
        .unwrap();
        assert_eq!(compilation.report.passes, 1);
        let bytes = compilation
            .package
            .to_bytes(&PackageLimits::default())
            .unwrap();
        let package = Package::from_bytes(&bytes, &PackageLimits::default()).unwrap();
        let wallpaper = Wallpaper::from_package(&package).unwrap();
        let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor::from_env_or_default());
        let gpu = Gpu::new(&instance, None).await.unwrap();
        let mut generated =
            Renderer::from_wallpaper(&gpu, &wallpaper, wgpu::TextureFormat::Rgba8UnormSrgb)
                .await
                .unwrap();
        let shader=Shader::with_bindings(r#"
fn wallpaper(uv: vec2<f32>) -> vec4<f32> {
    let strength = params.p0;
    let zoom = 1.0 + abs(strength);
    let shifted = vec2<f32>(0.5) + (uv - vec2<f32>(0.5)
        + (clamp(globals.mouse, vec2<f32>(0.0), vec2<f32>(1.0)) - vec2<f32>(0.5)) * strength) / zoom;
    let image = textureSample(background, background_sampler, shifted);
    let bright = vec4<f32>(image.rgb * params.p1, image.a);
    let tinted = bright * params.p2;
    return vec4<f32>(tinted.rgb * clamp(1.0 - dot(uv - vec2<f32>(0.5), uv - vec2<f32>(0.5)) * 1.2, 0.0, 1.0), tinted.a);
}"#,1,&package.program().parameters).unwrap();
        let image = WallpaperImage::decode(package.image_bytes()).unwrap();
        let mut reference =
            Renderer::new(&gpu, &shader, &image, wgpu::TextureFormat::Rgba8UnormSrgb)
                .await
                .unwrap();
        let mut globals = Globals::new(65, 33);
        globals.mouse = [0.8, 0.2];
        globals.time = 3.0;
        let initial = generated.render_rgba(&gpu, 65, 33, &globals).unwrap();
        assert_eq!(
            initial,
            reference.render_rgba(&gpu, 65, 33, &globals).unwrap()
        );
        generated
            .set_parameter(&gpu.queue, "brightness", ParameterValue::Float(0.3))
            .unwrap();
        reference
            .set_parameter(&gpu.queue, "brightness", ParameterValue::Float(0.3))
            .unwrap();
        let changed = generated.render_rgba(&gpu, 65, 33, &globals).unwrap();
        assert_ne!(initial, changed);
        assert_eq!(
            changed,
            reference.render_rgba(&gpu, 65, 33, &globals).unwrap()
        );
    });
}
#[test]
#[ignore = "requires a working graphics adapter"]
fn compiled_blur_matches_cpu_kernel_and_radius_updates() {
    pollster::block_on(async {
        let mut image = image::RgbaImage::from_pixel(9, 9, image::Rgba([0, 0, 0, 255]));
        image.put_pixel(4, 4, image::Rgba([255; 4]));
        let mut encoded = Cursor::new(vec![]);
        image
            .write_to(&mut encoded, image::ImageFormat::Png)
            .unwrap();
        let nodes = [
            ("uv", Op::Uv {}),
            (
                "image",
                Op::Image {
                    asset: "image".into(),
                    uv: "uv".into(),
                },
            ),
            (
                "radius",
                Op::Parameter {
                    parameter: "radius".into(),
                },
            ),
            (
                "blur",
                Op::Blur {
                    color: "image".into(),
                    radius: "radius".into(),
                    width: 9,
                    height: 9,
                },
            ),
            (
                "output",
                Op::Output {
                    color: "blur".into(),
                },
            ),
        ]
        .into_iter()
        .map(|(id, op)| Node { id: id.into(), op })
        .collect();
        let graph = Graph {
            version: 1,
            nodes,
            output: "output".into(),
            parameters: vec![Parameter {
                id: "radius".into(),
                default: ParameterValue::Float(1.0),
                min: Some(0.0),
                max: Some(64.0),
            }],
        }
        .check()
        .unwrap();
        let assets = BTreeMap::from([(
            "image".into(),
            Asset {
                extension: "png".into(),
                bytes: encoded.into_inner(),
            },
        )]);
        let manifest = Manifest {
            format_version: 1,
            id: "test.blur".into(),
            name: "Blur".into(),
            author: "Test".into(),
            description: String::new(),
            preview: None,
            program: "program.json".into(),
            minimum_wisp_version: "0.1.0".into(),
            gpu_profile: 1,
            tags: vec![],
        };
        let compiled = compile(&graph, &assets, manifest, &CompileOptions::default()).unwrap();
        assert_eq!(compiled.report.passes, 3);
        let wallpaper = Wallpaper::from_package(&compiled.package).unwrap();
        let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor::from_env_or_default());
        let gpu = Gpu::new(&instance, None).await.unwrap();
        let mut renderer =
            Renderer::from_wallpaper(&gpu, &wallpaper, wgpu::TextureFormat::Rgba8UnormSrgb)
                .await
                .unwrap();
        let globals = Globals::new(9, 9);
        let pixels = renderer.render_rgba(&gpu, 9, 9, &globals).unwrap();
        let kernel = [0.0f32, 0.0, 0.0625, 0.25, 0.375, 0.25, 0.0625, 0.0, 0.0];
        for y in 0..9 {
            for x in 0..9 {
                // The horizontal target quantizes linear RGB to 8 bits; vertical
                // filtering is fused into presentation and encoded directly to sRGB.
                let linear = (kernel[x] * 255.0).round() / 255.0 * kernel[y];
                let srgb = if linear <= 0.0031308 {
                    linear * 12.92
                } else {
                    1.055 * linear.powf(1.0 / 2.4) - 0.055
                };
                let expected = (srgb * 255.0).round() as u8;
                let pixel = &pixels[(y * 9 + x) * 4..(y * 9 + x) * 4 + 4];
                assert!(
                    pixel[..3].iter().all(|v| v.abs_diff(expected) <= 2),
                    "pixel {x},{y}: {pixel:?}, expected {expected}"
                );
                assert_eq!(pixel[3], 255);
            }
        }
        renderer
            .set_parameter(&gpu.queue, "radius", ParameterValue::Float(0.0))
            .unwrap();
        assert_eq!(
            renderer.render_rgba(&gpu, 9, 9, &globals).unwrap(),
            image.into_raw()
        );
        assert!(
            renderer
                .set_parameter(&gpu.queue, "radius", ParameterValue::Float(-1.0))
                .is_err()
        );
    });
}

#[test]
#[ignore = "requires a working graphics adapter"]
fn materialized_branch_multiple_images_and_pruned_parameter_bindings() {
    pollster::block_on(async {
        let graph: Graph = serde_json::from_value(serde_json::json!({
            "version":1,"output":"out",
            "parameters":[
                {"id":"unused","default":{"type":"integer","value":123}},
                {"id":"blend","default":{"type":"float","value":0.25},"min":0,"max":1}
            ],
            "nodes":[
                {"id":"uv","op":{"type":"uv"}},
                {"id":"red","op":{"type":"image","asset":"z_red","uv":"uv"}},
                {"id":"blue","op":{"type":"image","asset":"a_blue","uv":"uv"}},
                {"id":"cached","op":{"type":"materialize","color":"red","width":7,"height":5}},
                {"id":"factor","op":{"type":"parameter","parameter":"blend"}},
                {"id":"mixed","op":{"type":"mix","a":"cached","b":"blue","factor":"factor"}},
                {"id":"out","op":{"type":"output","color":"mixed"}}
            ]
        }))
        .unwrap();
        let image_asset = |color| {
            let image = image::RgbaImage::from_pixel(2, 2, image::Rgba(color));
            let mut bytes = Cursor::new(vec![]);
            image.write_to(&mut bytes, image::ImageFormat::Png).unwrap();
            Asset {
                extension: "png".into(),
                bytes: bytes.into_inner(),
            }
        };
        let assets = BTreeMap::from([
            ("z_red".into(), image_asset([255, 0, 0, 255])),
            ("a_blue".into(), image_asset([0, 0, 255, 255])),
        ]);
        let manifest = Manifest {
            format_version: 1,
            id: "test.materialize".into(),
            name: "Materialize".into(),
            author: "Test".into(),
            description: String::new(),
            preview: None,
            program: "program.json".into(),
            minimum_wisp_version: "0.1.0".into(),
            gpu_profile: 1,
            tags: vec![],
        };
        let compiled = compile(
            &graph.check().unwrap(),
            &assets,
            manifest,
            &CompileOptions::default(),
        )
        .unwrap();
        assert_eq!(compiled.report.passes, 2);
        assert_eq!(compiled.report.intermediate_targets, 1);
        assert_eq!(compiled.report.parameters, 1);
        let wallpaper = Wallpaper::from_package(&compiled.package).unwrap();
        let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor::from_env_or_default());
        let gpu = Gpu::new(&instance, None).await.unwrap();
        let mut renderer =
            Renderer::from_wallpaper(&gpu, &wallpaper, wgpu::TextureFormat::Rgba8UnormSrgb)
                .await
                .unwrap();
        let globals = Globals::new(65, 33);
        let pixels = renderer.render_rgba(&gpu, 65, 33, &globals).unwrap();
        assert!(pixels.chunks_exact(4).all(|p| p[0].abs_diff(225) <= 1
            && p[1] == 0
            && p[2].abs_diff(137) <= 1
            && p[3] == 255));
        renderer
            .set_parameter(&gpu.queue, "blend", ParameterValue::Float(1.0))
            .unwrap();
        assert!(
            renderer
                .render_rgba(&gpu, 65, 33, &globals)
                .unwrap()
                .chunks_exact(4)
                .all(|p| p == [0, 0, 255, 255])
        );
    });
}

fn simulation_graph(feedback: bool) -> Graph {
    let mut nodes = vec![
        Node {
            id: "mouse".into(),
            op: Op::Mouse {},
        },
        Node {
            id: "color".into(),
            op: Op::Parameter {
                parameter: "color".into(),
            },
        },
        Node {
            id: "speed".into(),
            op: Op::Constant {
                value: ParameterValue::Float(0.3),
            },
        },
        Node {
            id: "size".into(),
            op: Op::Constant {
                value: ParameterValue::Float(0.05),
            },
        },
        Node {
            id: "decay".into(),
            op: Op::Parameter {
                parameter: "decay".into(),
            },
        },
    ];
    let effect = if feedback {
        Op::Feedback {
            color: "color".into(),
            decay: "decay".into(),
            width: 32,
            height: 32,
        }
    } else {
        Op::Particles {
            position: "mouse".into(),
            color: "color".into(),
            speed: "speed".into(),
            size: "size".into(),
            count: 8,
            width: 32,
            height: 32,
        }
    };
    nodes.push(Node {
        id: "effect".into(),
        op: effect,
    });
    nodes.push(Node {
        id: "out".into(),
        op: Op::Output {
            color: "effect".into(),
        },
    });
    Graph {
        version: 1,
        nodes,
        output: "out".into(),
        parameters: vec![
            Parameter {
                id: "color".into(),
                default: ParameterValue::Color([1.; 4]),
                min: None,
                max: None,
            },
            Parameter {
                id: "decay".into(),
                default: ParameterValue::Float(0.5),
                min: Some(0.),
                max: Some(0.9999),
            },
        ],
    }
}
fn simulation_wallpaper(g: Graph) -> Wallpaper {
    let manifest = Manifest {
        format_version: 1,
        id: "test.simulation".into(),
        name: "Simulation".into(),
        author: "Wisp".into(),
        description: String::new(),
        preview: None,
        program: "program.json".into(),
        minimum_wisp_version: "0.1.0".into(),
        gpu_profile: 1,
        tags: vec![],
    };
    let compiled = compile(
        &g.check().unwrap(),
        &BTreeMap::new(),
        manifest,
        &Default::default(),
    )
    .unwrap();
    let bytes = compiled.package.to_bytes(&Default::default()).unwrap();
    Wallpaper::from_package(&Package::from_bytes(&bytes, &Default::default()).unwrap()).unwrap()
}
#[test]
#[ignore = "requires a working graphics adapter"]
fn feedback_retains_float_history_decays_in_seconds_and_isolates_renderers() {
    pollster::block_on(async {
        let gpu = Gpu::new(
            &wgpu::Instance::new(&wgpu::InstanceDescriptor::from_env_or_default()),
            None,
        )
        .await
        .unwrap();
        let wallpaper = simulation_wallpaper(simulation_graph(true));
        let mut renderer =
            Renderer::from_wallpaper(&gpu, &wallpaper, wgpu::TextureFormat::Rgba8UnormSrgb)
                .await
                .unwrap();
        let mut fresh =
            Renderer::from_wallpaper(&gpu, &wallpaper, wgpu::TextureFormat::Rgba8UnormSrgb)
                .await
                .unwrap();
        let mut g = Globals::new(32, 32);
        g.delta_time = 1. / 60.;
        let first = renderer.render_rgba(&gpu, 32, 32, &g).unwrap();
        assert!(first.iter().all(|v| *v == 255));
        renderer
            .set_parameter(&gpu.queue, "color", ParameterValue::Color([0.; 4]))
            .unwrap();
        fresh
            .set_parameter(&gpu.queue, "color", ParameterValue::Color([0.; 4]))
            .unwrap();
        for frame in 1..=6 {
            g.frame = frame;
            let pixels = renderer.render_rgba(&gpu, 32, 32, &g).unwrap();
            let linear = (0.5f32.powi(frame as i32) * 255.).round() / 255.;
            let expected = ((if linear <= 0.0031308 {
                linear * 12.92
            } else {
                1.055 * linear.powf(1. / 2.4) - 0.055
            }) * 255.)
                .round() as u8;
            assert!(
                pixels.chunks_exact(4).all(|p| p[0].abs_diff(expected) <= 1),
                "frame {frame} {} vs {expected}",
                pixels[0]
            );
        }
        assert!(
            fresh
                .render_rgba(&gpu, 32, 32, &g)
                .unwrap()
                .iter()
                .all(|v| *v == 0)
        );
        // No time advance retains state, including a presentation size change.
        g.delta_time = 0.;
        let held = renderer.render_rgba(&gpu, 17, 9, &g).unwrap();
        assert_eq!(held, renderer.render_rgba(&gpu, 17, 9, &g).unwrap());
        // Live decay reset clears history on the next advancing frame.
        renderer
            .set_parameter(&gpu.queue, "decay", ParameterValue::Float(0.))
            .unwrap();
        assert_eq!(held, renderer.render_rgba(&gpu, 17, 9, &g).unwrap());
        g.delta_time = 1. / 60.;
        assert!(
            renderer
                .render_rgba(&gpu, 32, 32, &g)
                .unwrap()
                .iter()
                .all(|v| *v == 0)
        );
    });
}
#[test]
#[ignore = "requires a working graphics adapter"]
fn particles_advance_persist_follow_mouse_and_reset_on_reload() {
    pollster::block_on(async {
        let gpu = Gpu::new(
            &wgpu::Instance::new(&wgpu::InstanceDescriptor::from_env_or_default()),
            None,
        )
        .await
        .unwrap();
        let wallpaper = simulation_wallpaper(simulation_graph(false));
        let mut a = Renderer::from_wallpaper(&gpu, &wallpaper, wgpu::TextureFormat::Rgba8UnormSrgb)
            .await
            .unwrap();
        let b = Renderer::from_wallpaper(&gpu, &wallpaper, wgpu::TextureFormat::Rgba8UnormSrgb)
            .await
            .unwrap();
        let mut g = Globals::new(32, 32);
        g.delta_time = 1. / 60.;
        let first = a.render_rgba(&gpu, 32, 32, &g).unwrap();
        assert!(first.iter().any(|v| *v > 0));
        assert_eq!(first, b.render_rgba(&gpu, 32, 32, &g).unwrap());
        let mut last = first.clone();
        for frame in 1..30 {
            g.frame = frame;
            last = a.render_rgba(&gpu, 32, 32, &g).unwrap();
            assert_eq!(last, b.render_rgba(&gpu, 32, 32, &g).unwrap());
        }
        assert_ne!(first, last);
        g.delta_time = 0.;
        let held = a.render_rgba(&gpu, 32, 32, &g).unwrap();
        assert_eq!(last, held);
        // A new renderer starts a new emitter at the new mouse position.
        let fresh = Renderer::from_wallpaper(&gpu, &wallpaper, wgpu::TextureFormat::Rgba8UnormSrgb)
            .await
            .unwrap();
        g.mouse = [0.15, 0.2];
        g.delta_time = 1. / 60.;
        let shifted = fresh.render_rgba(&gpu, 32, 32, &g).unwrap();
        assert_ne!(first, shifted);
        let fresh = Renderer::from_wallpaper(&gpu, &wallpaper, wgpu::TextureFormat::Rgba8UnormSrgb)
            .await
            .unwrap();
        g.mouse = [0.5, 0.5];
        g.frame = 0;
        assert_eq!(first, fresh.render_rgba(&gpu, 32, 32, &g).unwrap());
        a.set_parameter(&gpu.queue, "color", ParameterValue::Color([0.; 4]))
            .unwrap();
        assert!(
            a.render_rgba(&gpu, 32, 32, &g)
                .unwrap()
                .iter()
                .all(|v| *v == 0)
        );
    });
}

#[test]
#[ignore = "requires a working graphics adapter"]
fn layer_blends_match_straight_alpha_reference_and_transforms_clip() {
    use wisp_ir::BlendMode;
    pollster::block_on(async {
        let gpu = Gpu::new(
            &wgpu::Instance::new(&wgpu::InstanceDescriptor::from_env_or_default()),
            None,
        )
        .await
        .unwrap();
        let encode = |v: f32| -> u8 {
            ((if v <= 0.0031308 {
                v * 12.92
            } else {
                1.055 * v.powf(1. / 2.4) - 0.055
            }) * 255.)
                .round() as u8
        };
        for mode in BlendMode::ALL {
            for (ab, af) in [(0.6, 0.4), (0., 1.), (1., 0.), (0., 0.)] {
                let back = [0.2f32, 0.55, 0.8, ab];
                let front = [0.7f32, 0.35, 0.1, af];
                let graph = Graph {
                    version: 1,
                    parameters: vec![],
                    output: "out".into(),
                    nodes: vec![
                        Node {
                            id: "a".into(),
                            op: Op::Constant {
                                value: ParameterValue::Color(back),
                            },
                        },
                        Node {
                            id: "b".into(),
                            op: Op::Constant {
                                value: ParameterValue::Color(front),
                            },
                        },
                        Node {
                            id: "blend".into(),
                            op: Op::Blend {
                                a: "a".into(),
                                b: "b".into(),
                                mode,
                            },
                        },
                        Node {
                            id: "out".into(),
                            op: Op::Output {
                                color: "blend".into(),
                            },
                        },
                    ],
                };
                let wallpaper = simulation_wallpaper(graph);
                let renderer =
                    Renderer::from_wallpaper(&gpu, &wallpaper, wgpu::TextureFormat::Rgba8UnormSrgb)
                        .await
                        .unwrap();
                let pixels = renderer
                    .render_rgba(&gpu, 3, 2, &Globals::new(3, 2))
                    .unwrap();
                let alpha = af + ab * (1. - af);
                let mut expected = [0u8; 4];
                expected[3] = (alpha * 255.).round() as u8;
                for i in 0..3 {
                    let b = back[i];
                    let f = front[i];
                    let blend = match mode {
                        BlendMode::Alpha => f,
                        BlendMode::Add => b + f,
                        BlendMode::Multiply => b * f,
                        BlendMode::Screen => 1. - (1. - b) * (1. - f),
                        BlendMode::Overlay => {
                            if b <= 0.5 {
                                2. * b * f
                            } else {
                                1. - 2. * (1. - b) * (1. - f)
                            }
                        }
                    };
                    expected[i] = encode(
                        ((1. - af) * ab * b + af * ((1. - ab) * f + ab * blend))
                            / alpha.max(0.000001),
                    );
                }
                assert!(
                    pixels
                        .chunks_exact(4)
                        .all(|p| p.iter().zip(expected).all(|(a, b)| a.abs_diff(b) <= 1)),
                    "{mode:?} {ab} {af}: {:?} != {expected:?}",
                    &pixels[..4]
                );
            }
        }
        let graph: Graph = serde_json::from_value(serde_json::json!({"version":1,"output":"out","nodes":[
            {"id":"uv","op":{"type":"uv"}},
            {"id":"pos","op":{"type":"constant","value":{"type":"vector2","value":[0.25,0.0]}}},
            {"id":"scale","op":{"type":"constant","value":{"type":"vector2","value":[0.5,0.5]}}},
            {"id":"pivot","op":{"type":"constant","value":{"type":"vector2","value":[0.5,0.5]}}},
            {"id":"angle","op":{"type":"constant","value":{"type":"float","value":0.0}}},
            {"id":"one","op":{"type":"constant","value":{"type":"float","value":1.0}}},
            {"id":"color","op":{"type":"constant","value":{"type":"color","value":[1.,0.,0.,1.]}}},
            {"id":"transform","op":{"type":"transform","uv":"uv","position":"pos","scale":"scale","rotation":"angle","pivot":"pivot"}},
            {"id":"layer","op":{"type":"layer","uv":"transform","color":"color","mask":"one","opacity":"one"}},
            {"id":"out","op":{"type":"output","color":"layer"}}
        ]})).unwrap();
        let wallpaper = simulation_wallpaper(graph);
        let renderer =
            Renderer::from_wallpaper(&gpu, &wallpaper, wgpu::TextureFormat::Rgba8UnormSrgb)
                .await
                .unwrap();
        let pixels = renderer
            .render_rgba(&gpu, 8, 8, &Globals::new(8, 8))
            .unwrap();
        for y in 0..8 {
            for x in 0..8 {
                assert_eq!(
                    pixels[(y * 8 + x) * 4 + 3],
                    if x >= 4 && (2..6).contains(&y) {
                        255
                    } else {
                        0
                    },
                    "{x},{y}"
                );
            }
        }
    });
}

#[test]
#[ignore = "requires a working graphics adapter"]
fn parallax_keeps_image_detail_at_every_edge_and_zero_strength_preserves_framing() {
    let pixels =
        image::RgbaImage::from_fn(256, 256, |x, y| image::Rgba([x as u8, y as u8, 128, 255]));
    let mut encoded = Cursor::new(vec![]);
    pixels
        .write_to(&mut encoded, image::ImageFormat::Png)
        .unwrap();
    let graph: Graph = serde_json::from_value(serde_json::json!({
        "version": 1, "output": "output",
        "parameters": [{"id":"strength","default":{"type":"float","value":0.0},"min":-1,"max":1}],
        "nodes": [
            {"id":"uv","op":{"type":"uv"}},
            {"id":"strength","op":{"type":"parameter","parameter":"strength"}},
            {"id":"parallax","op":{"type":"parallax","uv":"uv","strength":"strength"}},
            {"id":"image","op":{"type":"image","asset":"source","uv":"parallax"}},
            {"id":"output","op":{"type":"output","color":"image"}}
        ]
    }))
    .unwrap();
    let assets = BTreeMap::from([(
        "source".into(),
        Asset {
            extension: "png".into(),
            bytes: encoded.into_inner(),
        },
    )]);
    let manifest = Manifest {
        format_version: 1,
        id: "test.parallax".into(),
        name: "Parallax".into(),
        author: "Test".into(),
        description: String::new(),
        tags: vec![],
        preview: None,
        program: "program.json".into(),
        minimum_wisp_version: "0.1.0".into(),
        gpu_profile: 1,
    };
    let compiled = compile(
        &graph.check().unwrap(),
        &assets,
        manifest,
        &Default::default(),
    )
    .unwrap();
    let wallpaper = Wallpaper::from_package(&compiled.package).unwrap();
    let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor::from_env_or_default());
    let gpu = pollster::block_on(Gpu::new(&instance, None)).unwrap();
    let mut renderer = pollster::block_on(Renderer::from_wallpaper(
        &gpu,
        &wallpaper,
        wgpu::TextureFormat::Rgba8UnormSrgb,
    ))
    .unwrap();
    let shader = Shader::new("fn wallpaper(uv: vec2<f32>) -> vec4<f32> { return textureSample(background, background_sampler, uv); }").unwrap();
    let reference = pollster::block_on(Renderer::new(
        &gpu,
        &shader,
        &wallpaper.images()[0],
        wgpu::TextureFormat::Rgba8UnormSrgb,
    ))
    .unwrap();
    for (width, height) in [(64, 48), (48, 64)] {
        let mut globals = Globals::new(width, height);
        let original = reference
            .render_rgba(&gpu, width, height, &globals)
            .unwrap();
        for strength in [0., 0.03, 0.15, 1., -0.15, -1.] {
            renderer
                .set_parameter(&gpu.queue, "strength", ParameterValue::Float(strength))
                .unwrap();
            globals.mouse = [0.5, 0.5];
            let centered = renderer.render_rgba(&gpu, width, height, &globals).unwrap();
            for mouse in [[0., 0.], [0., 1.], [1., 0.], [1., 1.], [0.5, 0.5]] {
                globals.mouse = mouse;
                let frame = renderer.render_rgba(&gpu, width, height, &globals).unwrap();
                if strength == 0. {
                    assert_eq!(frame, original, "zero strength changed the image framing");
                } else if mouse != [0.5, 0.5] {
                    assert_ne!(
                        frame, centered,
                        "parallax no longer responds to the pointer"
                    );
                }
                // A coordinate gradient exposes stretched border texels as repeated rows or columns.
                for y in 0..height as usize {
                    for x in 0..width as usize {
                        let index = (y * width as usize + x) * 4;
                        assert_eq!(frame[index + 3], 255);
                        if x > 0 {
                            assert!(
                                frame[index] > frame[index - 4],
                                "horizontal edge stretch: strength {strength}, mouse {mouse:?}, pixel {x},{y}"
                            );
                        }
                        if y > 0 {
                            assert!(
                                frame[index + 1] > frame[index + 1 - width as usize * 4],
                                "vertical edge stretch: strength {strength}, mouse {mouse:?}, pixel {x},{y}"
                            );
                        }
                    }
                }
            }
            globals.mouse = [0., 1.];
            let corner = renderer.render_rgba(&gpu, width, height, &globals).unwrap();
            globals.mouse = [-2., 3.];
            assert_eq!(
                corner,
                renderer.render_rgba(&gpu, width, height, &globals).unwrap(),
                "pointer outside the viewport exceeded the reserved image area"
            );
        }
    }
}
