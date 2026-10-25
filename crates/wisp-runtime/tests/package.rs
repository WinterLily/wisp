mod common;
use wisp_runtime::{EXAMPLE_IMAGE, EXAMPLE_SHADER, Wallpaper};

#[test]
fn validates_package_content_without_gpu() {
    let package = common::package(EXAMPLE_SHADER, EXAMPLE_IMAGE, Some(EXAMPLE_IMAGE));
    let wallpaper = Wallpaper::from_package(&package).unwrap();
    assert_eq!(wallpaper.images()[0].dimensions(), (256, 256));
    for (source, image, preview, diagnostic) in [
        ("invalid shader", EXAMPLE_IMAGE, None, "shader"),
        (EXAMPLE_SHADER, b"invalid image".as_slice(), None, "image"),
        (
            EXAMPLE_SHADER,
            EXAMPLE_IMAGE,
            Some(b"invalid preview".as_slice()),
            "preview",
        ),
    ] {
        let package = common::package(source, image, preview);
        let error = Wallpaper::from_package(&package).err().unwrap().to_string();
        assert!(error.contains(diagnostic), "{error}");
    }
}

#[test]
fn validates_every_image_and_parameter_layout_without_gpu() {
    let package = common::multi_package(EXAMPLE_IMAGE, EXAMPLE_IMAGE);
    let wallpaper = Wallpaper::from_package(&package).unwrap();
    assert_eq!(wallpaper.images().len(), 2);
    let module = naga::front::wgsl::parse_str(wallpaper.shader().source()).unwrap();
    let (_, ty) = module
        .types
        .iter()
        .find(|(_, ty)| ty.name.as_deref() == Some("Parameters"))
        .unwrap();
    let naga::TypeInner::Struct { members, span } = &ty.inner else {
        panic!("expected struct")
    };
    assert_eq!(*span, 112);
    assert_eq!(
        members
            .iter()
            .filter(|m| m
                .name
                .as_ref()
                .is_some_and(|name| !name.starts_with("_padding")))
            .map(|m| m.offset)
            .collect::<Vec<_>>(),
        [0, 16, 32, 48, 64, 80, 96]
    );
    assert!(Wallpaper::from_package(&common::multi_package(EXAMPLE_IMAGE, b"invalid")).is_err());
    assert!(wisp_runtime::Shader::new(common::MULTI_SHADER).is_err());
    for declaration in [
        "@group(1) @binding(4) var extra: texture_2d<f32>;",
        "@group(2) @binding(1) var<uniform> extra: vec4<f32>;",
        "@compute @workgroup_size(1) fn extra() {}",
    ] {
        assert!(
            wisp_runtime::Shader::with_bindings(
                &format!("{}\n{declaration}", common::MULTI_SHADER),
                2,
                &common::parameters()
            )
            .is_err()
        );
    }
}

#[test]
fn multipass_validation_and_configurable_texture_budgets() {
    use wisp_runtime::RuntimeLimits;
    let package = common::multipass_package(EXAMPLE_IMAGE, None);
    let bytes = 256 * 256 * 4 + (7 * 5 + 11 * 3) * 4;
    let limits = RuntimeLimits {
        max_texture_bytes: bytes,
        ..Default::default()
    };
    let mut wallpaper = Wallpaper::from_package_with_limits(&package, &limits).unwrap();
    assert_eq!(wallpaper.pass_count(), 3);
    wallpaper
        .set_parameter("gain", wisp_format::ParameterValue::Float(0.5))
        .unwrap();
    assert!(
        wallpaper
            .set_parameter("gain", wisp_format::ParameterValue::Float(2.0))
            .is_err()
    );
    assert_eq!(
        wallpaper.parameters()[0].default,
        wisp_format::ParameterValue::Float(0.5)
    );
    for strict in [
        RuntimeLimits {
            max_texture_bytes: bytes - 1,
            ..limits.clone()
        },
        RuntimeLimits {
            max_texture_dimension: 255,
            ..limits.clone()
        },
        RuntimeLimits {
            max_passes: 2,
            ..limits.clone()
        },
        RuntimeLimits {
            max_intermediate_targets: 1,
            ..limits.clone()
        },
    ] {
        assert!(Wallpaper::from_package_with_limits(&package, &strict).is_err());
    }
    let invalid = common::multipass_package(EXAMPLE_IMAGE, Some("invalid shader"));
    let error = Wallpaper::from_package(&invalid).err().unwrap();
    assert!(error.to_string().contains("pass 2"));
}

#[test]
fn compute_dependencies_versions_limits_and_shaders_are_validated() {
    use wisp_format::{Package, PackageLimits, Pass, Resource};
    use wisp_runtime::RuntimeLimits;
    let build = |program, assets| {
        let (manifest, _, _) = common::compute_parts();
        Package::build(manifest, program, assets, &PackageLimits::default())
    };
    let (_, program, assets) = common::compute_parts();
    let package = build(program.clone(), assets.clone()).unwrap();
    let bytes = package.to_bytes(&PackageLimits::default()).unwrap();
    let loaded = Package::from_bytes(&bytes, &PackageLimits::default()).unwrap();
    let wallpaper = Wallpaper::from_package(&loaded).unwrap();
    assert_eq!(wallpaper.pass_count(), 3);
    for limits in [
        RuntimeLimits {
            max_resource_bytes: 1511,
            ..Default::default()
        },
        RuntimeLimits {
            max_storage_bytes: 1007,
            ..Default::default()
        },
        RuntimeLimits {
            max_dispatch_groups: 1,
            ..Default::default()
        },
    ] {
        assert!(Wallpaper::from_package_with_limits(&loaded, &limits).is_err());
    }
    for case in 0..10 {
        let mut p = program.clone();
        match case {
            0 => p.version = 3,
            1 => p.shader_abi_version = 2,
            2 => {
                if let Pass::Compute { dispatch, .. } = &mut p.passes[0] {
                    *dispatch = [0, 1, 1];
                }
            }
            3 => {
                if let Pass::Compute { dispatch, .. } = &mut p.passes[0] {
                    *dispatch = [65535, 65535, 65535];
                }
            }
            4 => {
                if let Pass::Compute { buffers, .. } = &mut p.passes[0] {
                    buffers[0].writable = false;
                }
            }
            5 => {
                if let Pass::Compute { buffers, .. } = &mut p.passes[1] {
                    buffers[0].writable = true;
                }
            }
            6 => {
                if let Pass::Compute { images, .. } = &mut p.passes[0] {
                    images.push("first".into());
                }
            }
            7 => {
                if let Pass::Compute { buffers, .. } = &mut p.passes[0] {
                    buffers.push(buffers[0].clone());
                }
            }
            8 => {
                p.resources[0] = Resource::StorageBuffer {
                    id: "data".into(),
                    elements: 0,
                }
            }
            9 => p.passes.swap(0, 1),
            _ => unreachable!(),
        }
        assert!(
            build(p, assets.clone()).is_err(),
            "malformed compute case {case}"
        );
    }
    for source in [
        "not wgsl",
        "@compute @workgroup_size(1) fn injected() {}",
        "@group(3) @binding(5) var<storage, read_write> extra: array<f32>; fn compute(id: vec3<u32>) {}",
    ] {
        let mut files = assets.clone();
        files.insert("shaders/first.wgsl".into(), source.as_bytes().to_vec());
        let package = build(program.clone(), files).unwrap();
        assert!(Wallpaper::from_package(&package).is_err());
    }
}
