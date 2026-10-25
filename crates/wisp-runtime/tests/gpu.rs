use std::io::Cursor;
mod common;
use wisp_runtime::{EXAMPLE_IMAGE, EXAMPLE_SHADER, Globals, Gpu, Renderer, Shader, WallpaperImage};

#[test]
#[ignore = "requires a working graphics adapter; run with cargo test -p wisp-runtime --test gpu -- --ignored"]
fn fullscreen_texture_orientation_readback_and_uniform_updates() {
    pollster::block_on(async {
        let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor::from_env_or_default());
        let gpu = Gpu::new(&instance, None).await.unwrap();
        let pixels = image::RgbaImage::from_raw(
            2,
            2,
            vec![
                255, 0, 0, 255, 0, 255, 0, 255, 0, 0, 255, 255, 255, 255, 255, 255,
            ],
        )
        .unwrap();
        let mut encoded = Cursor::new(Vec::new());
        pixels
            .write_to(&mut encoded, image::ImageFormat::Png)
            .unwrap();
        let image = WallpaperImage::decode(encoded.get_ref()).unwrap();
        let shader = Shader::new("fn wallpaper(uv: vec2<f32>) -> vec4<f32> { return textureSample(background, background_sampler, uv); }").unwrap();
        let renderer = Renderer::new(&gpu, &shader, &image, wgpu::TextureFormat::Rgba8UnormSrgb)
            .await
            .unwrap();
        // Deliberately unaligned width exercises 256-byte readback row padding.
        let globals = Globals::new(65, 33);
        let pixels = renderer.render_rgba(&gpu, 65, 33, &globals).unwrap();
        assert_eq!(pixels.len(), 65 * 33 * 4);
        for (x, y, expected) in [
            (0, 0, [255, 0, 0, 255]),
            (64, 0, [0, 255, 0, 255]),
            (0, 32, [0, 0, 255, 255]),
            (64, 32, [255, 255, 255, 255]),
        ] {
            let offset = (y * 65 + x) * 4;
            assert_eq!(&pixels[offset..offset + 4], &expected);
        }
        assert!(renderer.render_rgba(&gpu, 0, 33, &globals).is_err());
        assert!(renderer.render_rgba(&gpu, 4097, 33, &globals).is_err());

        let shader = Shader::new(EXAMPLE_SHADER).unwrap();
        let image = WallpaperImage::decode(EXAMPLE_IMAGE).unwrap();
        let renderer = Renderer::new(&gpu, &shader, &image, wgpu::TextureFormat::Rgba8UnormSrgb)
            .await
            .unwrap();
        let first = renderer.render_rgba(&gpu, 65, 33, &globals).unwrap();
        let limits = wisp_format::PackageLimits::default();
        let package = common::package(EXAMPLE_SHADER, EXAMPLE_IMAGE, None);
        let bytes = package.to_bytes(&limits).unwrap();
        let package = wisp_format::Package::from_bytes(&bytes, &limits).unwrap();
        let wallpaper = wisp_runtime::Wallpaper::from_package(&package).unwrap();
        let packaged = Renderer::new(
            &gpu,
            wallpaper.shader(),
            &wallpaper.images()[0],
            wgpu::TextureFormat::Rgba8UnormSrgb,
        )
        .await
        .unwrap();
        assert_eq!(
            first,
            packaged.render_rgba(&gpu, 65, 33, &globals).unwrap(),
            "package roundtrip must preserve rendered pixels"
        );
        let mut later = globals;
        later.time = 3.0;
        let animated = renderer.render_rgba(&gpu, 65, 33, &later).unwrap();
        later.mouse = [0.0, 1.0];
        let moved = renderer.render_rgba(&gpu, 65, 33, &later).unwrap();
        assert_ne!(first, animated, "time must affect the example");
        assert_ne!(animated, moved, "mouse must affect the example");
        assert!(moved.chunks_exact(4).all(|pixel| pixel[3] == 255));
    });
}

#[test]
#[ignore = "requires a working graphics adapter"]
fn multiple_images_and_typed_parameter_updates() {
    pollster::block_on(async {
        let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor::from_env_or_default());
        let gpu = Gpu::new(&instance, None).await.unwrap();
        let png = |color| {
            let mut encoded = std::io::Cursor::new(Vec::new());
            image::RgbaImage::from_pixel(2, 2, image::Rgba(color))
                .write_to(&mut encoded, image::ImageFormat::Png)
                .unwrap();
            encoded.into_inner()
        };
        let package = common::multi_package(&png([255, 0, 0, 255]), &png([0, 0, 255, 255]));
        let limits = wisp_format::PackageLimits::default();
        let package =
            wisp_format::Package::from_bytes(&package.to_bytes(&limits).unwrap(), &limits).unwrap();
        let mut wallpaper = wisp_runtime::Wallpaper::from_package(&package).unwrap();
        let mut renderer =
            Renderer::from_wallpaper(&gpu, &wallpaper, wgpu::TextureFormat::Rgba8UnormSrgb)
                .await
                .unwrap();
        let globals = Globals::new(65, 33);
        let assert_color = |renderer: &Renderer, color: [u8; 4]| {
            let pixels = renderer.render_rgba(&gpu, 65, 33, &globals).unwrap();
            assert!(
                pixels.chunks_exact(4).all(|pixel| pixel == color),
                "expected {color:?}, got {:?}",
                &pixels[..4]
            );
        };
        assert_color(&renderer, [255, 0, 0, 255]);
        use wisp_format::ParameterValue::*;
        renderer
            .set_parameter(&gpu.queue, "value0", Float(1.0))
            .unwrap();
        assert_color(&renderer, [0, 0, 255, 255]);
        for (id, value) in [
            ("value0", Float(2.0)),
            ("value0", Float(f32::NAN)),
            ("value0", Integer(0)),
            ("missing", Float(0.0)),
        ] {
            assert!(renderer.set_parameter(&gpu.queue, id, value).is_err());
            assert_color(&renderer, [0, 0, 255, 255]);
        }
        renderer
            .set_parameter(&gpu.queue, "value2", Boolean(false))
            .unwrap();
        assert_color(&renderer, [0, 0, 0, 255]);
        renderer
            .set_parameter(&gpu.queue, "value2", Boolean(true))
            .unwrap();
        renderer
            .set_parameter(&gpu.queue, "value5", Color([1.0, 1.0, 0.0, 1.0]))
            .unwrap();
        assert_color(&renderer, [0, 0, 0, 255]);
        renderer
            .set_parameter(&gpu.queue, "value5", Color([1.0; 4]))
            .unwrap();
        renderer
            .set_parameter(&gpu.queue, "value0", Float(0.5))
            .unwrap();
        assert_color(&renderer, [188, 0, 188, 255]); // Linear blend, sRGB output.
        wallpaper.set_parameter("value0", Float(1.0)).unwrap();
        let overridden =
            Renderer::from_wallpaper(&gpu, &wallpaper, wgpu::TextureFormat::Rgba8UnormSrgb)
                .await
                .unwrap();
        assert_color(&overridden, [0, 0, 255, 255]);
        assert!(
            Renderer::new(
                &gpu,
                wallpaper.shader(),
                &wallpaper.images()[0],
                wgpu::TextureFormat::Rgba8UnormSrgb
            )
            .await
            .is_err()
        );
    });
}

#[test]
#[ignore = "requires a working graphics adapter"]
fn maximum_image_and_parameter_bindings_render() {
    pollster::block_on(async {
        let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor::from_env_or_default());
        let gpu = Gpu::new(&instance, None).await.unwrap();
        let parameters: Vec<_> = (0..wisp_format::MAX_PARAMETERS)
            .map(|i| wisp_format::Parameter {
                id: format!("p{i}"),
                default: wisp_format::ParameterValue::Float(1.0),
                min: None,
                max: None,
            })
            .collect();
        let shader = Shader::with_bindings(
            "fn wallpaper(uv: vec2<f32>) -> vec4<f32> { return textureSample(image_7, image_7_sampler, uv) * params.p63; }",
            wisp_format::MAX_IMAGES, &parameters,
        ).unwrap();
        let images: Vec<_> = (0..wisp_format::MAX_IMAGES)
            .map(|_| WallpaperImage::decode(EXAMPLE_IMAGE).unwrap())
            .collect();
        let renderer =
            Renderer::with_images(&gpu, &shader, &images, wgpu::TextureFormat::Rgba8UnormSrgb)
                .await
                .unwrap();
        let reference = Shader::new("fn wallpaper(uv: vec2<f32>) -> vec4<f32> { return textureSample(background, background_sampler, uv); }").unwrap();
        let reference = Renderer::new(
            &gpu,
            &reference,
            &images[0],
            wgpu::TextureFormat::Rgba8UnormSrgb,
        )
        .await
        .unwrap();
        let globals = Globals::new(65, 33);
        assert!(
            renderer.render_rgba(&gpu, 65, 33, &globals).unwrap()
                == reference.render_rgba(&gpu, 65, 33, &globals).unwrap(),
            "last image and parameter bindings must match the reference"
        );
    });
}

#[test]
#[ignore = "requires a working graphics adapter"]
fn multipass_dependencies_resolution_color_and_parameter_updates() {
    pollster::block_on(async {
        let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor::from_env_or_default());
        let gpu = Gpu::new(&instance, None).await.unwrap();
        let mut png = Cursor::new(Vec::new());
        image::RgbaImage::from_pixel(2, 2, image::Rgba([0, 0, 255, 255]))
            .write_to(&mut png, image::ImageFormat::Png)
            .unwrap();
        let package = common::multipass_package(png.get_ref(), None);
        let limits = wisp_format::PackageLimits::default();
        let package =
            wisp_format::Package::from_bytes(&package.to_bytes(&limits).unwrap(), &limits).unwrap();
        let mut wallpaper = wisp_runtime::Wallpaper::from_package(&package).unwrap();
        let mut renderer =
            Renderer::from_wallpaper(&gpu, &wallpaper, wgpu::TextureFormat::Rgba8UnormSrgb)
                .await
                .unwrap();
        let mut globals = Globals::new(65, 33);
        globals.time = 3.0;
        let assert_color =
            |renderer: &Renderer, width, height, globals: &Globals, expected: [u8; 4]| {
                let pixels = renderer.render_rgba(&gpu, width, height, globals).unwrap();
                assert!(
                    pixels.chunks_exact(4).all(|pixel| pixel
                        .iter()
                        .zip(expected)
                        .all(|(actual, expected)| actual.abs_diff(expected) <= 1)),
                    "expected {expected:?}, got {:?}",
                    &pixels[..4]
                );
            };
        assert_color(&renderer, 65, 33, &globals, [255, 137, 255, 255]);
        renderer
            .set_parameter(&gpu.queue, "gain", wisp_format::ParameterValue::Float(0.5))
            .unwrap();
        assert_color(&renderer, 65, 33, &globals, [137, 137, 255, 255]);
        assert!(
            renderer
                .set_parameter(&gpu.queue, "gain", wisp_format::ParameterValue::Float(2.0))
                .is_err()
        );
        assert_color(&renderer, 19, 9, &globals, [137, 137, 255, 255]);
        globals.time = 0.0;
        assert_color(&renderer, 65, 33, &globals, [137, 0, 255, 255]);
        wallpaper
            .set_parameter("gain", wisp_format::ParameterValue::Float(0.0))
            .unwrap();
        let overridden =
            Renderer::from_wallpaper(&gpu, &wallpaper, wgpu::TextureFormat::Rgba8UnormSrgb)
                .await
                .unwrap();
        assert_color(&overridden, 65, 33, &globals, [0, 0, 255, 255]);
        assert_color(&renderer, 65, 33, &globals, [137, 0, 255, 255]);
        let strict = wisp_runtime::RuntimeLimits {
            max_texture_bytes: (2 * 2 + 7 * 5 + 11 * 3) * 4 - 1,
            ..Default::default()
        };
        assert!(
            Renderer::from_wallpaper_with_limits(
                &gpu,
                &wallpaper,
                wgpu::TextureFormat::Rgba8UnormSrgb,
                &strict
            )
            .await
            .is_err()
        );
    });
}

#[test]
#[ignore = "requires a GPU with compute support"]
fn compute_storage_dependencies_parameters_and_renderer_isolation() {
    pollster::block_on(async {
        let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor::from_env_or_default());
        let gpu = Gpu::new(&instance, None).await.unwrap();
        let (manifest, program, assets) = common::compute_parts();
        let package = wisp_format::Package::build(
            manifest,
            program,
            assets,
            &wisp_format::PackageLimits::default(),
        )
        .unwrap();
        let wallpaper = wisp_runtime::Wallpaper::from_package(&package).unwrap();
        let mut renderer =
            Renderer::from_wallpaper(&gpu, &wallpaper, wgpu::TextureFormat::Rgba8UnormSrgb)
                .await
                .unwrap();
        let other = Renderer::from_wallpaper(&gpu, &wallpaper, wgpu::TextureFormat::Rgba8UnormSrgb)
            .await
            .unwrap();
        // Empty image, storage-buffer and parameter groups also form a valid pipeline.
        let (manifest, mut program, mut assets) = common::compute_parts();
        program.parameters.clear();
        program.resources.retain(|r| r.id() == "second");
        program.passes.remove(1);
        if let wisp_format::Pass::Compute {
            output, buffers, ..
        } = &mut program.passes[0]
        {
            *output = "second".into();
            buffers.clear();
        }
        assets.remove("shaders/second.wgsl");
        assets.insert("shaders/first.wgsl".into(), b"fn compute(id: vec3<u32>) { if id.x < 9u && id.y < 7u { textureStore(output_image, vec2<i32>(id.xy), vec4<f32>(0.0, 0.0, 1.0, 1.0)); } }".to_vec());
        let simple = wisp_format::Package::build(
            manifest,
            program,
            assets,
            &wisp_format::PackageLimits::default(),
        )
        .unwrap();
        let simple = wisp_runtime::Wallpaper::from_package(&simple).unwrap();
        let simple = Renderer::from_wallpaper(&gpu, &simple, wgpu::TextureFormat::Rgba8UnormSrgb)
            .await
            .unwrap();
        assert!(
            simple
                .render_rgba(&gpu, 9, 7, &Globals::new(9, 7))
                .unwrap()
                .chunks_exact(4)
                .all(|p| p == [0, 0, 255, 255])
        );
        let mut globals = Globals::new(9, 7);
        let red = renderer.render_rgba(&gpu, 9, 7, &globals).unwrap();
        assert!(red.chunks_exact(4).all(|p| p == [255, 0, 0, 255]));
        renderer
            .set_parameter(&gpu.queue, "gain", wisp_format::ParameterValue::Float(0.0))
            .unwrap();
        globals.time = 1.0;
        let green = renderer.render_rgba(&gpu, 9, 7, &globals).unwrap();
        assert!(green.chunks_exact(4).all(|p| p == [0, 255, 0, 255]));
        assert!(
            renderer
                .set_parameter(&gpu.queue, "gain", wisp_format::ParameterValue::Float(-1.0))
                .is_err()
        );
        assert_eq!(green, renderer.render_rgba(&gpu, 9, 7, &globals).unwrap());
        globals.time = 0.0;
        assert_eq!(red, other.render_rgba(&gpu, 9, 7, &globals).unwrap());
    });
}
