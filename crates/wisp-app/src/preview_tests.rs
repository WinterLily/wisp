use super::*;
use iced::widget::shader::{Pipeline as _, Primitive as _};
use wisp_runtime::Gpu;

fn read_texture(gpu: &Gpu, texture: &wgpu::Texture) -> Vec<u8> {
    let size = texture.size();
    let row = (size.width * 4).div_ceil(256) * 256;
    let buffer = gpu.device.create_buffer(&wgpu::BufferDescriptor {
        label: None,
        size: u64::from(row) * u64::from(size.height),
        usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let mut encoder = gpu.device.create_command_encoder(&Default::default());
    encoder.copy_texture_to_buffer(
        texture.as_image_copy(),
        wgpu::TexelCopyBufferInfo {
            buffer: &buffer,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(row),
                rows_per_image: Some(size.height),
            },
        },
        size,
    );
    gpu.queue.submit([encoder.finish()]);
    let (send, receive) = std::sync::mpsc::channel();
    buffer
        .slice(..)
        .map_async(wgpu::MapMode::Read, move |result| {
            send.send(result).unwrap()
        });
    gpu.device
        .poll(wgpu::PollType::wait_indefinitely())
        .unwrap();
    receive.recv().unwrap().unwrap();
    let mapped = buffer.slice(..).get_mapped_range();
    let pixels = mapped
        .chunks_exact(row as usize)
        .flat_map(|row| row[..size.width as usize * 4].iter().copied())
        .collect();
    drop(mapped);
    buffer.unmap();
    pixels
}
fn frame(
    gpu: &Gpu,
    pipeline: &mut Pipeline,
    preview: Preview,
    bounds: Rectangle,
    mouse: [f32; 2],
) -> Vec<u8> {
    let viewport = Viewport::with_physical_size(iced::Size::new(96, 64), 1.0);
    let primitive = Primitive { preview, mouse };
    primitive.prepare(pipeline, &gpu.device, &gpu.queue, &bounds, &viewport);
    let target = gpu.device.create_texture(&wgpu::TextureDescriptor {
        label: None,
        size: wgpu::Extent3d {
            width: 96,
            height: 64,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8UnormSrgb,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let view = target.create_view(&Default::default());
    let mut encoder = gpu.device.create_command_encoder(&Default::default());
    drop(encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
            view: &view,
            depth_slice: None,
            resolve_target: None,
            ops: wgpu::Operations {
                load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                store: wgpu::StoreOp::Store,
            },
        })],
        ..Default::default()
    }));
    primitive.render(
        pipeline,
        &mut encoder,
        &view,
        &Rectangle {
            x: 0,
            y: 0,
            width: 96,
            height: 64,
        },
    );
    gpu.queue.submit([encoder.finish()]);
    read_texture(gpu, &target)
}
#[test]
#[ignore = "requires a working graphics adapter"]
fn embedded_preview_matches_runtime_and_preserves_clipping_pause_and_live_parameters() {
    let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor::from_env_or_default());
    let gpu = pollster::block_on(Gpu::new(&instance, None)).unwrap();
    let compilation = wisp_compiler::compile_project(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/graph-blur"),
        &Default::default(),
    )
    .unwrap();
    let wallpaper = Arc::new(Wallpaper::from_package(&compilation.package).unwrap());
    let parameters = Arc::new(
        wallpaper
            .parameters()
            .iter()
            .map(|p| (p.id.clone(), p.default.clone()))
            .collect(),
    );
    let mut globals = Globals::new(65, 33);
    globals.time = 3.0;
    globals.mouse = [0.8, 0.2];
    let mut preview = Preview {
        generation: 1,
        wallpaper: wallpaper.clone(),
        parameters,
        globals,
        status: Arc::new(Mutex::new(Status::default())),
    };
    let mut pipeline = Pipeline::new(&gpu.device, &gpu.queue, wgpu::TextureFormat::Rgba8UnormSrgb);
    let bounds = Rectangle {
        x: 7.0,
        y: 9.0,
        width: 65.0,
        height: 33.0,
    };
    let first = frame(&gpu, &mut pipeline, preview.clone(), bounds, globals.mouse);
    assert!(preview.status.lock().unwrap().error.is_none());
    let mut reference = pollster::block_on(Renderer::from_wallpaper(
        &gpu,
        &wallpaper,
        wgpu::TextureFormat::Rgba8UnormSrgb,
    ))
    .unwrap();
    let expected = reference.render_rgba(&gpu, 65, 33, &globals).unwrap();
    for y in 0..64 {
        for x in 0..96 {
            let pixel = &first[(y * 96 + x) * 4..(y * 96 + x + 1) * 4];
            if (7..72).contains(&x) && (9..42).contains(&y) {
                let i = ((y - 9) * 65 + x - 7) * 4;
                assert!(
                    pixel
                        .iter()
                        .zip(&expected[i..i + 4])
                        .all(|(a, b)| a.abs_diff(*b) <= 1),
                    "pixel mismatch {x},{y}"
                );
            } else {
                assert_eq!(pixel, [0, 0, 0, 255]);
            }
        }
    }
    let paused = frame(&gpu, &mut pipeline, preview.clone(), bounds, [0.1, 0.9]);
    assert_eq!(first, paused);
    assert_eq!(preview.status.lock().unwrap().frames, 1);
    Arc::make_mut(&mut preview.parameters).insert("radius".into(), ParameterValue::Float(4.0));
    preview.globals.frame += 1;
    let updated = frame(&gpu, &mut pipeline, preview.clone(), bounds, globals.mouse);
    assert_ne!(updated, first);
    reference
        .set_parameter(&gpu.queue, "radius", ParameterValue::Float(4.0))
        .unwrap();
    let expected = reference.render_rgba(&gpu, 65, 33, &globals).unwrap();
    for y in 0..33 {
        for x in 0..65 {
            let i = ((y + 9) * 96 + x + 7) * 4;
            let j = (y * 65 + x) * 4;
            assert!(
                updated[i..i + 4]
                    .iter()
                    .zip(&expected[j..j + 4])
                    .all(|(a, b)| a.abs_diff(*b) <= 1)
            );
        }
    }
    // Scrolling moves the top of a widget outside the window; UVs must remain cropped.
    let clipped = frame(
        &gpu,
        &mut pipeline,
        preview.clone(),
        Rectangle { y: -9.0, ..bounds },
        globals.mouse,
    );
    for y in 0..24 {
        for x in 0..65 {
            let i = (y * 96 + x + 7) * 4;
            let j = ((y + 9) * 65 + x) * 4;
            assert!(
                clipped[i..i + 4]
                    .iter()
                    .zip(&expected[j..j + 4])
                    .all(|(a, b)| a.abs_diff(*b) <= 1)
            );
        }
    }
    assert!(preview.status.lock().unwrap().error.is_none());
}

#[allow(dead_code)]
#[path = "../../wisp-runtime/tests/common/mod.rs"]
mod fixtures;

#[test]
#[ignore = "requires a working graphics adapter"]
fn compute_preview_uses_four_groups_and_live_values() {
    let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor::from_env_or_default());
    let gpu = pollster::block_on(Gpu::new(&instance, None)).unwrap();
    let (manifest, program, files) = fixtures::compute_parts();
    let package =
        wisp_format::Package::build(manifest, program, files, &Default::default()).unwrap();
    let wallpaper = Arc::new(Wallpaper::from_package(&package).unwrap());
    let mut globals = Globals::new(65, 33);
    globals.time = 0.25;
    let mut preview = Preview {
        generation: 1,
        wallpaper: wallpaper.clone(),
        parameters: Arc::new(BTreeMap::from([(
            "gain".into(),
            ParameterValue::Float(0.5),
        )])),
        globals,
        status: Arc::new(Mutex::new(Status::default())),
    };
    let mut pipeline = Pipeline::new(&gpu.device, &gpu.queue, wgpu::TextureFormat::Rgba8UnormSrgb);
    let bounds = Rectangle {
        x: 0.0,
        y: 0.0,
        width: 65.0,
        height: 33.0,
    };
    let first = frame(&gpu, &mut pipeline, preview.clone(), bounds, [0.5, 0.5]);
    assert!(preview.status.lock().unwrap().error.is_none());
    let mut reference = pollster::block_on(Renderer::from_wallpaper(
        &gpu,
        &wallpaper,
        wgpu::TextureFormat::Rgba8UnormSrgb,
    ))
    .unwrap();
    reference
        .set_parameter(&gpu.queue, "gain", ParameterValue::Float(0.5))
        .unwrap();
    let expected = reference.render_rgba(&gpu, 65, 33, &globals).unwrap();
    assert!(
        first[..4]
            .iter()
            .zip(&expected[..4])
            .all(|(a, b)| a.abs_diff(*b) <= 1)
    );
    Arc::make_mut(&mut preview.parameters).insert("gain".into(), ParameterValue::Float(0.25));
    preview.globals.frame += 1;
    let updated = frame(&gpu, &mut pipeline, preview.clone(), bounds, [0.5, 0.5]);
    assert_ne!(updated, first);
    assert!(preview.status.lock().unwrap().error.is_none());
    let paused = frame(&gpu, &mut pipeline, preview.clone(), bounds, [0.5, 0.5]);
    assert_eq!(paused, updated);
    assert_eq!(preview.status.lock().unwrap().frames, 2);
}

#[test]
#[ignore = "requires a working graphics adapter"]
fn easy_presets_preview_match_exported_defaults_and_animate() {
    use crate::designer::{Draft, Preset};
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("image.png");
    std::fs::write(&source, wisp_runtime::EXAMPLE_IMAGE).unwrap();
    let mut draft = Draft::from_image(&source).unwrap();
    let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor::from_env_or_default());
    let gpu = pollster::block_on(Gpu::new(&instance, None)).unwrap();
    let mut pipeline = Pipeline::new(&gpu.device, &gpu.queue, wgpu::TextureFormat::Rgba8UnormSrgb);
    let bounds = Rectangle {
        x: 0.,
        y: 0.,
        width: 96.,
        height: 64.,
    };
    for (i, preset) in Preset::ALL.into_iter().enumerate() {
        draft.design.preset(preset);
        let wallpaper =
            Arc::new(Wallpaper::from_package(&draft.compile().unwrap().package).unwrap());
        let mut preview = Preview {
            generation: i as u64 + 1,
            parameters: Arc::new(
                wallpaper
                    .parameters()
                    .iter()
                    .map(|p| (p.id.clone(), p.default.clone()))
                    .collect(),
            ),
            wallpaper,
            globals: Globals::new(96, 64),
            status: Arc::new(Mutex::new(Status::default())),
        };
        let initial = frame(&gpu, &mut pipeline, preview.clone(), bounds, [0.5, 0.5]);
        // Author edits a live value, then exports that value as the package default.
        draft.design.values.insert("brightness".into(), 0.7);
        Arc::make_mut(&mut preview.parameters)
            .insert("brightness".into(), ParameterValue::Float(0.7));
        preview.globals.time = 2.;
        preview.globals.frame = 1;
        preview.globals.mouse = [0.9, 0.1];
        let live = frame(
            &gpu,
            &mut pipeline,
            preview.clone(),
            bounds,
            preview.globals.mouse,
        );
        assert_ne!(initial, live);
        let output = temp.path().join(format!("preset-{i}.wisp"));
        draft.export(&output).unwrap();
        let package = wisp_format::Package::load(output, &Default::default()).unwrap();
        let exported = Wallpaper::from_package(&package).unwrap();
        let renderer = pollster::block_on(Renderer::from_wallpaper(
            &gpu,
            &exported,
            wgpu::TextureFormat::Rgba8UnormSrgb,
        ))
        .unwrap();
        let expected = renderer
            .render_rgba(&gpu, 96, 64, &preview.globals)
            .unwrap();
        assert!(
            live.iter().zip(expected).all(|(a, b)| a.abs_diff(b) <= 1),
            "{preset}: preview/export differ"
        );
        let before = live;
        preview.globals.time = 4.;
        preview.globals.frame += 1;
        let animated = frame(&gpu, &mut pipeline, preview.clone(), bounds, [0.9, 0.1]);
        assert_eq!(
            animated != before,
            draft.design.pulse,
            "{preset}: unexpected time response"
        );
        preview.globals.frame += 1;
        let moved = frame(&gpu, &mut pipeline, preview.clone(), bounds, [0.1, 0.9]);
        assert_eq!(
            moved != animated,
            draft.design.parallax,
            "{preset}: unexpected mouse response"
        );
        assert!(preview.status.lock().unwrap().error.is_none());
    }
}

#[test]
#[ignore = "requires a working graphics adapter"]
fn graph_rewire_and_multipass_edit_preview_match_saved_package() {
    use crate::graph::Document;
    let mut doc = Document::default();
    let gain = doc.add("Constant", [0., 0.]).unwrap();
    let effect = doc.add("Brightness", [0., 0.]).unwrap();
    doc.connect("color", &effect, "color").unwrap();
    doc.connect(&gain, &effect, "amount").unwrap();
    doc.connect(&effect, "output", "color").unwrap();
    let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor::from_env_or_default());
    let gpu = pollster::block_on(Gpu::new(&instance, None)).unwrap();
    let mut pipeline = Pipeline::new(&gpu.device, &gpu.queue, wgpu::TextureFormat::Rgba8UnormSrgb);
    let bounds = Rectangle::new(iced::Point::ORIGIN, iced::Size::new(96., 64.));
    let temp = tempfile::tempdir().unwrap();
    let mut previous = vec![];
    for generation in 1..=2 {
        if generation == 2 {
            doc.set_field(
                &gain,
                "value",
                serde_json::to_value(ParameterValue::Float(0.3)).unwrap(),
            )
            .unwrap();
            let blur = doc.add("Blur", [0., 0.]).unwrap();
            doc.connect(&effect, &blur, "color").unwrap();
            doc.connect(&gain, &blur, "radius").unwrap();
            doc.connect(&blur, "output", "color").unwrap();
            doc.set_field(&blur, "width", 65.into()).unwrap();
            doc.set_field(&blur, "height", 33.into()).unwrap();
        }
        let wallpaper = Arc::new(Wallpaper::from_package(&doc.compile().unwrap().package).unwrap());
        let preview = Preview {
            generation,
            wallpaper,
            parameters: Arc::new(BTreeMap::new()),
            globals: Globals::new(96, 64),
            status: Arc::new(Mutex::new(Status::default())),
        };
        let pixels = frame(&gpu, &mut pipeline, preview.clone(), bounds, [0.5, 0.5]);
        assert!(preview.status.lock().unwrap().error.is_none());
        let path = temp.path().join(format!("graph{generation}.wisp"));
        doc.export(&path).unwrap();
        let exported = Wallpaper::from_package(
            &wisp_format::Package::load(path, &Default::default()).unwrap(),
        )
        .unwrap();
        let renderer = pollster::block_on(Renderer::from_wallpaper(
            &gpu,
            &exported,
            wgpu::TextureFormat::Rgba8UnormSrgb,
        ))
        .unwrap();
        let expected = renderer
            .render_rgba(&gpu, 96, 64, &preview.globals)
            .unwrap();
        assert_eq!(pixels.len(), expected.len());
        assert!(pixels.iter().zip(expected).all(|(a, b)| a.abs_diff(b) <= 1));
        assert_ne!(pixels, previous);
        previous = pixels;
    }
}

#[test]
#[ignore = "requires a working graphics adapter"]
fn interactive_preview_and_export_share_audio_mouse_and_persistent_state() {
    let mut doc = crate::graph::Document::open(
        &std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/audio-reactive"),
    )
    .unwrap();
    for id in ["particles", "trails"] {
        doc.set_field(id, "width", 96.into()).unwrap();
        doc.set_field(id, "height", 64.into()).unwrap();
    }
    let temp = tempfile::tempdir().unwrap();
    let package = temp.path().join("interactive.wisp");
    doc.export(&package).unwrap();
    let exported =
        Wallpaper::from_package(&wisp_format::Package::load(package, &Default::default()).unwrap())
            .unwrap();
    let gpu = pollster::block_on(Gpu::new(
        &wgpu::Instance::new(&wgpu::InstanceDescriptor::from_env_or_default()),
        None,
    ))
    .unwrap();
    let mut reference = pollster::block_on(Renderer::from_wallpaper(
        &gpu,
        &exported,
        wgpu::TextureFormat::Rgba8UnormSrgb,
    ))
    .unwrap();
    let mut pipeline = Pipeline::new(&gpu.device, &gpu.queue, wgpu::TextureFormat::Rgba8UnormSrgb);
    let mut preview = Preview {
        generation: 1,
        wallpaper: Arc::new(Wallpaper::from_package(&doc.compile().unwrap().package).unwrap()),
        parameters: Arc::new(BTreeMap::new()),
        globals: Globals::new(96, 64),
        status: Arc::new(Mutex::new(Status::default())),
    };
    let bounds = Rectangle::new(iced::Point::ORIGIN, iced::Size::new(96., 64.));
    let mut analyzer = wisp_audio::Analyzer::new(48000, 2, 1.).unwrap();
    let mut bands = wisp_audio::Bands::default();
    for i in 0..48000 {
        let v = 0.6 * (std::f32::consts::TAU * 100. * i as f32 / 48000.).sin();
        if let Some(b) = analyzer.push(&[v, -v]) {
            bands = b;
        }
    }
    assert!(bands.bass > 0.4);
    let mut previous = vec![];
    for i in 0..24 {
        preview.globals.frame = i;
        preview.globals.delta_time = 1. / 60.;
        preview.globals.time = i as f32 / 60.;
        preview.globals.mouse = [0.3, 0.65];
        preview
            .globals
            .set_audio(if i < 12 { [0.; 4] } else { bands.values() });
        let pixels = frame(
            &gpu,
            &mut pipeline,
            preview.clone(),
            bounds,
            preview.globals.mouse,
        );
        let expected = reference
            .render_rgba(&gpu, 96, 64, &preview.globals)
            .unwrap();
        assert!(preview.status.lock().unwrap().error.is_none());
        assert!(
            pixels
                .iter()
                .zip(&expected)
                .all(|(a, b)| a.abs_diff(*b) <= 1),
            "frame {i}"
        );
        previous = pixels;
    }
    // Paused repaint reuses the completed frame. Parameter edits redraw with zero dt.
    assert_eq!(
        previous,
        frame(&gpu, &mut pipeline, preview.clone(), bounds, [0.8, 0.2])
    );
    Arc::make_mut(&mut preview.parameters).insert("size".into(), ParameterValue::Float(0.03));
    reference
        .set_parameter(&gpu.queue, "size", ParameterValue::Float(0.03))
        .unwrap();
    let mut globals = preview.globals;
    globals.delta_time = 0.;
    let changed = frame(&gpu, &mut pipeline, preview.clone(), bounds, globals.mouse);
    let expected = reference.render_rgba(&gpu, 96, 64, &globals).unwrap();
    assert!(
        changed
            .iter()
            .zip(expected)
            .all(|(a, b)| a.abs_diff(b) <= 1)
    );
    assert_eq!(pipeline.globals.delta_time, 0.);
    assert_ne!(previous, changed);
}

#[test]
#[ignore = "requires a working graphics adapter"]
fn masked_layer_effects_preview_export_animation_and_anchor_parity() {
    use crate::{
        designer::{Draft, Preset},
        layers::Effect,
    };
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("source.png");
    std::fs::write(&source, wisp_runtime::EXAMPLE_IMAGE).unwrap();
    let mut draft = Draft::from_image(&source).unwrap();
    draft.design.preset(Preset::Still);
    let stack = draft.enable_layers();
    stack.blank_mask(0, false).unwrap();
    // Lower half selected, upper half black: pixel checks below are far from the feather edge.
    for y in [0.7, 0.9] {
        stack.paint(0, [0., y], [1., y], 0.2, 255).unwrap();
    }
    stack.layers[0].mask.as_mut().unwrap().feather = 0.;
    stack.layers[0].displacement = Some("background".into());
    let gpu = pollster::block_on(Gpu::new(
        &wgpu::Instance::new(&wgpu::InstanceDescriptor::from_env_or_default()),
        None,
    ))
    .unwrap();
    let mut pipeline = Pipeline::new(&gpu.device, &gpu.queue, wgpu::TextureFormat::Rgba8UnormSrgb);
    let bounds = Rectangle {
        x: 0.,
        y: 0.,
        width: 96.,
        height: 64.,
    };
    for (i, effect) in Effect::ALL.into_iter().enumerate() {
        let stack = draft.layers.as_mut().unwrap();
        stack.layers[0].preset(effect);
        stack.layers[0].amplitude = 0.08;
        let wallpaper =
            Arc::new(Wallpaper::from_package(&draft.compile().unwrap().package).unwrap());
        let mut preview = Preview {
            generation: i as u64 + 1,
            parameters: Arc::new(
                wallpaper
                    .parameters()
                    .iter()
                    .map(|p| (p.id.clone(), p.default.clone()))
                    .collect(),
            ),
            wallpaper,
            globals: Globals::new(96, 64),
            status: Arc::new(Mutex::new(Status::default())),
        };
        preview.globals.time = 0.5;
        let first = frame(&gpu, &mut pipeline, preview.clone(), bounds, [0.5, 0.5]);
        preview.globals.time = 2.;
        preview.globals.frame += 1;
        let animated = frame(&gpu, &mut pipeline, preview.clone(), bounds, [0.5, 0.5]);
        assert_eq!(
            &first[..96 * 20 * 4],
            &animated[..96 * 20 * 4],
            "{effect:?}: black anchors must stay still"
        );
        if !matches!(effect, Effect::None | Effect::Displacement) {
            assert_ne!(first, animated, "{effect:?} must animate");
        }
        let output = temp.path().join(format!("{i}.wisp"));
        draft.export(&output).unwrap();
        let package = wisp_format::Package::load(&output, &Default::default()).unwrap();
        let renderer = pollster::block_on(Renderer::from_wallpaper(
            &gpu,
            &Wallpaper::from_package(&package).unwrap(),
            wgpu::TextureFormat::Rgba8UnormSrgb,
        ))
        .unwrap();
        let expected = renderer
            .render_rgba(&gpu, 96, 64, &preview.globals)
            .unwrap();
        assert!(
            animated
                .iter()
                .zip(expected)
                .all(|(a, b)| a.abs_diff(b) <= 1),
            "{effect:?}: preview/export differ"
        );
        assert!(preview.status.lock().unwrap().error.is_none());
    }
}

#[test]
#[ignore = "requires a working graphics adapter"]
fn grayscale_masks_preserve_coverage_invert_and_feather() {
    use crate::designer::{Draft, Preset};
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("source.png");
    image::RgbaImage::from_pixel(64, 64, image::Rgba([255; 4]))
        .save(&source)
        .unwrap();
    let mask_path = temp.path().join("mask.png");
    image::GrayImage::from_fn(64, 64, |_, y| image::Luma([if y < 32 { 0 } else { 255 }]))
        .save(&mask_path)
        .unwrap();
    let mut draft = Draft::from_image(&source).unwrap();
    draft.design.preset(Preset::Still);
    let stack = draft.enable_layers();
    stack.import(&mask_path, Some((0, false))).unwrap();
    stack.layers[0].mask.as_mut().unwrap().composition = true;
    let gpu = pollster::block_on(Gpu::new(
        &wgpu::Instance::new(&wgpu::InstanceDescriptor::from_env_or_default()),
        None,
    ))
    .unwrap();
    let render = |d: &Draft| {
        let compiled = d.compile().unwrap();
        let wallpaper = Wallpaper::from_package(&compiled.package).unwrap();
        let renderer = pollster::block_on(Renderer::from_wallpaper(
            &gpu,
            &wallpaper,
            wgpu::TextureFormat::Rgba8UnormSrgb,
        ))
        .unwrap();
        renderer
            .render_rgba(&gpu, 64, 64, &Globals::new(64, 64))
            .unwrap()
    };
    let alpha = |pixels: &[u8], y: usize| pixels[(y * 64 + 32) * 4 + 3];
    let initial = render(&draft);
    assert_eq!(alpha(&initial, 10), 0);
    assert_eq!(alpha(&initial, 50), 255);
    draft.layers.as_mut().unwrap().layers[0]
        .mask
        .as_mut()
        .unwrap()
        .invert = true;
    let inverted = render(&draft);
    assert_eq!(alpha(&inverted, 10), 255);
    assert_eq!(alpha(&inverted, 50), 0);
    draft.layers.as_mut().unwrap().layers[0]
        .mask
        .as_mut()
        .unwrap()
        .feather = 0.05;
    let feathered = render(&draft);
    assert!(alpha(&feathered, 31) > 0 && alpha(&feathered, 31) < 255);
    assert!(alpha(&feathered, 32) > 0 && alpha(&feathered, 32) < 255);
    assert_eq!(alpha(&feathered, 10), 255);
    assert_eq!(alpha(&feathered, 50), 0);
    image::GrayImage::from_pixel(64, 64, image::Luma([128]))
        .save(&mask_path)
        .unwrap();
    let stack = draft.layers.as_mut().unwrap();
    stack.import(&mask_path, Some((0, false))).unwrap();
    stack.layers[0].mask.as_mut().unwrap().composition = true;
    let middle = render(&draft);
    assert!(
        alpha(&middle, 20).abs_diff(128) <= 1,
        "grayscale coverage must not be gamma-darkened"
    );
}

#[test]
#[ignore = "requires a working graphics adapter"]
fn water_refracts_lower_layers_through_a_transparent_foreground() {
    use crate::{
        designer::{Draft, Preset},
        layers::Effect,
    };
    use wisp_runtime::{Shader, WallpaperImage};
    let temp = tempfile::tempdir().unwrap();
    let background = temp.path().join("grid.png");
    ::image::RgbaImage::from_fn(128, 128, |x, y| {
        if (x / 4 + y / 4) % 2 == 0 {
            ::image::Rgba([230, 35, 25, 255])
        } else {
            ::image::Rgba([20, 65, 230, 255])
        }
    })
    .save(&background)
    .unwrap();
    let clear = temp.path().join("clear.png");
    ::image::RgbaImage::from_pixel(2, 2, ::image::Rgba([0; 4]))
        .save(&clear)
        .unwrap();
    let mut draft = Draft::from_image(&background).unwrap();
    draft.design.preset(Preset::Still);
    let stack = draft.enable_layers();
    stack.import(&clear, None).unwrap();
    stack.layers[1].preset(Effect::Droplets);
    stack.layers[1].size = 0.4;
    stack.layers[1].strength = 1.;
    stack.layers[1].density = 6.;
    let compiled = draft.compile().unwrap();
    let package = &compiled.package;
    let gpu = pollster::block_on(Gpu::new(
        &wgpu::Instance::new(&wgpu::InstanceDescriptor::from_env_or_default()),
        None,
    ))
    .unwrap();
    let renderer = pollster::block_on(Renderer::from_wallpaper(
        &gpu,
        &Wallpaper::from_package(package).unwrap(),
        wgpu::TextureFormat::Rgba8UnormSrgb,
    ))
    .unwrap();
    let mut globals = Globals::new(256, 128);
    globals.time = 3.;
    let wet = renderer.render_rgba(&gpu, 256, 128, &globals).unwrap();
    // Same lighting with the optical offset disabled isolates actual refraction
    // from highlights painted over the unchanged background.
    let source = package
        .shader_source()
        .replace("uv + offset * amount", "uv");
    assert_ne!(source, package.shader_source());
    let shader = Shader::with_bindings(
        &source,
        package.image_count(),
        &package.program().parameters,
    )
    .unwrap();
    let images: Vec<_> = (0..package.image_count())
        .map(|i| WallpaperImage::decode(package.image_bytes_at(i).unwrap()).unwrap())
        .collect();
    let flat = pollster::block_on(Renderer::with_images(
        &gpu,
        &shader,
        &images,
        wgpu::TextureFormat::Rgba8UnormSrgb,
    ))
    .unwrap()
    .render_rgba(&gpu, 256, 128, &globals)
    .unwrap();
    let displaced = wet
        .chunks_exact(4)
        .zip(flat.chunks_exact(4))
        .filter(|(a, b)| a.iter().zip(*b).any(|(a, b)| a.abs_diff(*b) > 15))
        .count();
    assert!(
        displaced > 150,
        "expected lower-layer refraction, found {displaced} displaced pixels"
    );
    assert!(
        wet.chunks_exact(4).all(|p| p[3] == 255),
        "water must not punch transparent holes at viewport edges"
    );
    draft.layers.as_mut().unwrap().layers[1].strength = 0.;
    let zero = draft.compile().unwrap();
    draft.layers.as_mut().unwrap().layers[1].effect = Effect::None;
    let dry = draft.compile().unwrap();
    let render = |p: &wisp_format::Package| {
        pollster::block_on(Renderer::from_wallpaper(
            &gpu,
            &Wallpaper::from_package(p).unwrap(),
            wgpu::TextureFormat::Rgba8UnormSrgb,
        ))
        .unwrap()
        .render_rgba(&gpu, 256, 128, &globals)
        .unwrap()
    };
    assert_eq!(render(&zero.package), render(&dry.package));
}

#[test]
#[ignore = "requires a working graphics adapter"]
fn rejected_gpu_replacement_preserves_pixels_without_claiming_success_and_recovers() {
    let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor::from_env_or_default());
    let gpu = pollster::block_on(Gpu::new(&instance, None)).unwrap();
    let compilation = wisp_compiler::compile_project(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/graph-blur"),
        &Default::default(),
    )
    .unwrap();
    let wallpaper = Arc::new(Wallpaper::from_package(&compilation.package).unwrap());
    let mut preview = Preview {
        generation: 1,
        parameters: Arc::new(
            wallpaper
                .parameters()
                .iter()
                .map(|p| (p.id.clone(), p.default.clone()))
                .collect(),
        ),
        wallpaper,
        globals: Globals::new(65, 33),
        status: Arc::new(Mutex::new(Status::default())),
    };
    let mut pipeline = Pipeline::new(&gpu.device, &gpu.queue, wgpu::TextureFormat::Rgba8UnormSrgb);
    let bounds = Rectangle {
        x: 7.,
        y: 9.,
        width: 65.,
        height: 33.,
    };
    let original = frame(&gpu, &mut pipeline, preview.clone(), bounds, [0.5, 0.5]);
    assert_eq!(preview.status.lock().unwrap().rendered_generation, Some(1));
    pipeline.runtime_limits.max_texture_dimension = 1;
    preview.generation = 2;
    preview.globals.time = 20.;
    preview.status = Arc::new(Mutex::new(Status::default()));
    for _ in 0..3 {
        preview.globals.frame += 1;
        assert_eq!(
            frame(&gpu, &mut pipeline, preview.clone(), bounds, [0.1, 0.9]),
            original
        );
        let status = preview.status.lock().unwrap();
        assert!(
            status
                .error
                .as_ref()
                .is_some_and(|error| error.contains("limit"))
        );
        assert_eq!(status.frames, 0);
        assert_eq!(status.rendered_generation, None);
    }
    pipeline.runtime_limits = Default::default();
    preview.generation = 3;
    frame(&gpu, &mut pipeline, preview.clone(), bounds, [0.5, 0.5]);
    let status = preview.status.lock().unwrap();
    assert!(status.error.is_none());
    assert_eq!(status.frames, 1);
    assert_eq!(status.rendered_generation, Some(3));
}
