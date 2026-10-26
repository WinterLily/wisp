use std::{collections::BTreeMap, io::Cursor};
use wisp_compiler::{Asset, CompileOptions, compile};
use wisp_format::{Manifest, Parameter, ParameterValue};
use wisp_ir::{Graph, Node, Op, WeatherKind};
use wisp_runtime::{Globals, Gpu, Renderer, Shader, Wallpaper};

fn gpu() -> Gpu {
    pollster::block_on(Gpu::new(
        &wgpu::Instance::new(&wgpu::InstanceDescriptor::from_env_or_default()),
        None,
    ))
    .unwrap()
}

fn parameters() -> Vec<Parameter> {
    [
        ("density", 10.),
        ("size", 0.25),
        ("speed", 1.),
        ("strength", 1.),
        ("mask", 1.),
    ]
    .into_iter()
    .map(|(id, value)| Parameter {
        id: id.into(),
        default: ParameterValue::Float(value),
        min: Some(0.),
        max: Some(32.),
    })
    .collect()
}

fn surface_renderer(gpu: &Gpu, shape: [f32; 2]) -> Renderer {
    let source = format!(
        "{}\n{}\nfn wallpaper(uv: vec2<f32>) -> vec4<f32> {{
            let water = wisp_glass_droplets(uv, params.p0, params.p1, params.p2, vec2<f32>({}, {}));
            return vec4<f32>(water.w, water.z * 32.0, 0.0, 1.0);
        }}",
        include_str!("../src/effects.wgsl"),
        include_str!("../src/droplets.wgsl"),
        shape[0],
        shape[1],
    );
    let shader = Shader::with_bindings(&source, 1, &parameters()).unwrap();
    pollster::block_on(Renderer::new(
        gpu,
        &shader,
        &wisp_runtime::WallpaperImage::decode(wisp_runtime::EXAMPLE_IMAGE).unwrap(),
        wgpu::TextureFormat::Rgba8UnormSrgb,
    ))
    .unwrap()
}

fn set(renderer: &mut Renderer, gpu: &Gpu, id: &str, value: f32) {
    renderer
        .set_parameter(&gpu.queue, id, ParameterValue::Float(value))
        .unwrap();
}

fn component_areas(pixels: &[u8], width: usize) -> Vec<usize> {
    let mut wet: Vec<_> = pixels.chunks_exact(4).map(|p| p[0] > 128).collect();
    let mut areas = vec![];
    for start in 0..wet.len() {
        if !wet[start] {
            continue;
        }
        wet[start] = false;
        let mut pending = vec![start];
        let mut area = 0;
        while let Some(index) = pending.pop() {
            area += 1;
            for neighbor in [
                (index % width > 0).then(|| index - 1),
                (index % width + 1 < width).then_some(index + 1),
                (index >= width).then(|| index - width),
                (index + width < wet.len()).then_some(index + width),
            ]
            .into_iter()
            .flatten()
            {
                if wet[neighbor] {
                    wet[neighbor] = false;
                    pending.push(neighbor);
                }
            }
        }
        areas.push(area);
    }
    areas
}

#[test]
#[ignore = "requires a working graphics adapter"]
fn droplets_have_fine_condensation_resting_beads_and_independent_motion() {
    let gpu = gpu();
    let mut renderer = surface_renderer(&gpu, [1., 1.]);
    let mut globals = Globals::new(1024, 576);
    globals.time = 3.;
    let first = renderer.render_rgba(&gpu, 1024, 576, &globals).unwrap();
    let areas = component_areas(&first, 1024);
    let fine = areas.iter().filter(|&&area| area <= 6).count();
    let large = areas.iter().filter(|&&area| area >= 30).count();
    assert!(fine > 500, "missing fine condensation: {fine} beads");
    assert!(large > 20, "missing larger lenses: {large} beads");
    globals.time += 0.4;
    let next = renderer.render_rgba(&gpu, 1024, 576, &globals).unwrap();
    let mut pinned = 0;
    let mut moved = 0;
    for (a, b) in first.chunks_exact(4).zip(next.chunks_exact(4)) {
        if a[0] > 128 && a == b {
            pinned += 1;
        }
        if a[0].abs_diff(b[0]) > 64 {
            moved += 1;
        }
    }
    assert!(
        pinned > moved,
        "condensation should stay pinned while runners move"
    );
    assert!(moved > 100, "sliding drops stopped moving");
    set(&mut renderer, &gpu, "speed", 0.);
    let frozen = renderer.render_rgba(&gpu, 1024, 576, &globals).unwrap();
    globals.time = 100.;
    assert_eq!(
        frozen,
        renderer.render_rgba(&gpu, 1024, 576, &globals).unwrap()
    );
}

#[test]
#[ignore = "requires a working graphics adapter"]
fn droplets_keep_physical_proportions_and_respond_to_size_and_axis_controls() {
    let gpu = gpu();
    let mut renderer = surface_renderer(&gpu, [1., 1.]);
    let wide = renderer
        .render_rgba(&gpu, 1024, 576, &Globals::new(1024, 576))
        .unwrap();
    let square = renderer
        .render_rgba(&gpu, 576, 576, &Globals::new(576, 576))
        .unwrap();
    for y in 0..576 {
        for x in 0..576 {
            let a = (y * 1024 + x + 224) * 4;
            let b = (y * 576 + x) * 4;
            assert!(
                wide[a].abs_diff(square[b]) <= 1,
                "aspect changed bead shape at {x},{y}"
            );
        }
    }
    let coverage = |pixels: &[u8]| pixels.chunks_exact(4).map(|p| u64::from(p[0])).sum::<u64>();
    set(&mut renderer, &gpu, "size", 0.12);
    let small = renderer
        .render_rgba(&gpu, 576, 576, &Globals::new(576, 576))
        .unwrap();
    assert!(coverage(&square) > coverage(&small) * 2);
    let thin = surface_renderer(&gpu, [0.5, 1.]);
    let broad = surface_renderer(&gpu, [1.5, 1.]);
    let thin = thin
        .render_rgba(&gpu, 576, 576, &Globals::new(576, 576))
        .unwrap();
    let broad = broad
        .render_rgba(&gpu, 576, 576, &Globals::new(576, 576))
        .unwrap();
    assert!(coverage(&broad) > coverage(&thin) * 2);
}

#[test]
#[ignore = "requires a working graphics adapter"]
fn compiled_droplets_refract_the_scene_preserve_alpha_and_disable_exactly() {
    let gpu = gpu();
    let image = image::RgbaImage::from_fn(256, 128, |x, y| {
        let alpha = if x < 32 { 0 } else { (y * 2 + 1) as u8 };
        if (x / 4 + y / 4) % 2 == 0 {
            image::Rgba([230, 35, 25, alpha])
        } else {
            image::Rgba([20, 65, 230, alpha])
        }
    });
    let mut encoded = Cursor::new(vec![]);
    image
        .write_to(&mut encoded, image::ImageFormat::Png)
        .unwrap();
    let assets = BTreeMap::from([(
        "image".into(),
        Asset {
            extension: "png".into(),
            bytes: encoded.into_inner(),
        },
    )]);
    let mut nodes = vec![
        Node {
            id: "uv".into(),
            op: Op::Uv {},
        },
        Node {
            id: "image".into(),
            op: Op::Image {
                asset: "image".into(),
                uv: "uv".into(),
            },
        },
    ];
    nodes.extend(parameters().iter().map(|p| Node {
        id: p.id.clone(),
        op: Op::Parameter {
            parameter: p.id.clone(),
        },
    }));
    nodes.push(Node {
        id: "water".into(),
        op: Op::Weather {
            color: "image".into(),
            uv: "uv".into(),
            mask: "mask".into(),
            density: "density".into(),
            size: "size".into(),
            speed: "speed".into(),
            strength: "strength".into(),
            kind: WeatherKind::Droplets,
            rain_direction: 0.,
            size_x: 1.,
            size_y: 1.,
        },
    });
    nodes.push(Node {
        id: "out".into(),
        op: Op::Output {
            color: "water".into(),
        },
    });
    let mut graph = Graph {
        version: 1,
        nodes,
        output: "out".into(),
        parameters: parameters(),
    };
    let manifest = Manifest {
        format_version: 1,
        id: "test.droplets".into(),
        name: "Droplets".into(),
        author: "Wisp".into(),
        description: String::new(),
        preview: None,
        program: "program.json".into(),
        minimum_wisp_version: "0.1.0".into(),
        gpu_profile: 1,
        tags: vec![],
    };
    let package = compile(
        &graph.clone().check().unwrap(),
        &assets,
        manifest.clone(),
        &CompileOptions::default(),
    )
    .unwrap()
    .package;
    let mut renderer = pollster::block_on(Renderer::from_wallpaper(
        &gpu,
        &Wallpaper::from_package(&package).unwrap(),
        wgpu::TextureFormat::Rgba8UnormSrgb,
    ))
    .unwrap();
    let mut globals = Globals::new(256, 128);
    globals.time = 3.;
    let wet = renderer.render_rgba(&gpu, 256, 128, &globals).unwrap();
    let flat_source = package
        .shader_source()
        .replace("uv + offset * amount", "uv");
    assert_ne!(flat_source, package.shader_source());
    let flat_shader =
        Shader::with_bindings(&flat_source, 1, &package.program().parameters).unwrap();
    let flat = pollster::block_on(Renderer::new(
        &gpu,
        &flat_shader,
        &wisp_runtime::WallpaperImage::decode(package.image_bytes()).unwrap(),
        wgpu::TextureFormat::Rgba8UnormSrgb,
    ))
    .unwrap();
    let flat = flat.render_rgba(&gpu, 256, 128, &globals).unwrap();
    let displaced = wet
        .chunks_exact(4)
        .zip(flat.chunks_exact(4))
        .filter(|(a, b)| a[..3].iter().zip(&b[..3]).any(|(a, b)| a.abs_diff(*b) > 15))
        .count();
    assert!(displaced > 100, "lenses displaced only {displaced} pixels");
    graph.nodes.last_mut().unwrap().op = Op::Output {
        color: "image".into(),
    };
    let dry = compile(
        &graph.check().unwrap(),
        &assets,
        manifest,
        &CompileOptions::default(),
    )
    .unwrap();
    let dry = pollster::block_on(Renderer::from_wallpaper(
        &gpu,
        &Wallpaper::from_package(&dry.package).unwrap(),
        wgpu::TextureFormat::Rgba8UnormSrgb,
    ))
    .unwrap()
    .render_rgba(&gpu, 256, 128, &globals)
    .unwrap();
    assert!(
        wet.chunks_exact(4)
            .zip(dry.chunks_exact(4))
            .all(|(a, b)| a[3] == b[3]),
        "glass changed layer alpha"
    );
    set(&mut renderer, &gpu, "strength", 0.);
    assert_eq!(dry, renderer.render_rgba(&gpu, 256, 128, &globals).unwrap());
    set(&mut renderer, &gpu, "strength", 1.);
    set(&mut renderer, &gpu, "mask", 0.);
    assert_eq!(dry, renderer.render_rgba(&gpu, 256, 128, &globals).unwrap());
}
