use super::*;
use std::io::{Cursor, Write};
use zip::{CompressionMethod, ZipWriter, write::SimpleFileOptions};

fn fixture() -> (Manifest, Program, BTreeMap<String, Vec<u8>>) {
    (
        Manifest {
            format_version: 1,
            id: "example.test".into(),
            name: "Test wallpaper".into(),
            author: "Test author".into(),
            description: String::new(),
            preview: None,
            program: "program.json".into(),
            minimum_wisp_version: "0.1.0".into(),
            gpu_profile: 1,
            tags: vec![],
        },
        Program {
            version: 1,
            shader_abi_version: 1,
            parameters: vec![],
            resources: vec![Resource::Image {
                id: "background".into(),
                path: "assets/image.png".into(),
            }],
            passes: vec![Pass::Fragment {
                shader: "shaders/pass.wgsl".into(),
                background: "background".into(),
                images: vec![],
                output: Output::Presentation,
            }],
        },
        [
            ("assets/image.png".into(), vec![1, 2, 3]),
            (
                "shaders/pass.wgsl".into(),
                b"shader validation belongs to the runtime".to_vec(),
            ),
        ]
        .into(),
    )
}
fn package() -> Package {
    let (manifest, program, assets) = fixture();
    Package::build(manifest, program, assets, &PackageLimits::default()).unwrap()
}
fn zip(files: &BTreeMap<String, Vec<u8>>, compression: CompressionMethod) -> Vec<u8> {
    let mut writer = ZipWriter::new(Cursor::new(Vec::new()));
    for (name, bytes) in files {
        writer
            .start_file(
                name,
                SimpleFileOptions::default().compression_method(compression),
            )
            .unwrap();
        writer.write_all(bytes).unwrap();
    }
    writer.finish().unwrap().into_inner()
}
fn error(bytes: &[u8], limits: &PackageLimits) -> String {
    Package::from_bytes(bytes, limits)
        .err()
        .expect("expected invalid package")
        .to_string()
}

#[test]
fn stored_and_deflated_roundtrip() {
    let package = package();
    for compression in [CompressionMethod::Stored, CompressionMethod::Deflated] {
        let bytes = zip(&package.files, compression);
        let decoded = Package::from_bytes(&bytes, &PackageLimits::default()).unwrap();
        assert_eq!(decoded.manifest().id, "example.test");
        assert_eq!(decoded.shader_source(), package.shader_source());
        assert_eq!(decoded.image_bytes(), &[1, 2, 3]);
        assert_eq!(
            decoded.to_bytes(&PackageLimits::default()).unwrap(),
            package.to_bytes(&PackageLimits::default()).unwrap()
        );
    }
}

#[test]
fn rejects_each_independent_version_and_future_minimum() {
    for field in [
        "format_version",
        "gpu_profile",
        "minimum_wisp_version",
        "version",
        "shader_abi_version",
    ] {
        let mut files = package().files;
        let path = if matches!(field, "version" | "shader_abi_version") {
            "program.json"
        } else {
            "manifest.json"
        };
        let mut json: serde_json::Value = serde_json::from_slice(&files[path]).unwrap();
        json[field] = if field == "minimum_wisp_version" {
            "999.0.0".into()
        } else {
            999.into()
        };
        files.insert(path.into(), serde_json::to_vec(&json).unwrap());
        assert!(
            Package::from_files(files, &PackageLimits::default()).is_err(),
            "{field}"
        );
    }
}

#[test]
fn rejects_missing_files_unknown_fields_and_invalid_references() {
    for missing in package().files.keys() {
        let mut files = package().files;
        files.remove(missing);
        assert!(
            Package::from_files(files, &PackageLimits::default()).is_err(),
            "{missing}"
        );
    }
    for (path, field, value) in [
        ("manifest.json", "typo", serde_json::json!(true)),
        (
            "manifest.json",
            "program",
            serde_json::json!("../program.json"),
        ),
        ("program.json", "typo", serde_json::json!([])),
        (
            "program.json",
            "passes",
            serde_json::json!([{ "type": "compute" }]),
        ),
        ("program.json", "passes", serde_json::json!([])),
    ] {
        let mut files = package().files;
        let mut json: serde_json::Value = serde_json::from_slice(&files[path]).unwrap();
        json[field] = value;
        files.insert(path.into(), serde_json::to_vec(&json).unwrap());
        assert!(Package::from_files(files, &PackageLimits::default()).is_err());
    }
    let (manifest, mut program, assets) = fixture();
    let Pass::Fragment { background, .. } = &mut program.passes[0] else {
        unreachable!()
    };
    *background = "missing".into();
    assert!(Package::build(manifest, program, assets, &PackageLimits::default()).is_err());
}

#[test]
fn rejects_nonportable_archive_paths() {
    for path in [
        "../escape",
        "/absolute",
        "C:/drive",
        "a\\b",
        "a//b",
        "a/./b",
        "a/../b",
        "a/CON.txt",
        "a/LPT1",
        "a/NUL",
        "a/trailing.",
        "a/space name",
        "a/é",
    ] {
        let bytes = zip(&[(path.into(), vec![1])].into(), CompressionMethod::Stored);
        assert!(
            error(&bytes, &PackageLimits::default()).contains("path"),
            "{path}"
        );
    }
}

#[test]
fn rejects_duplicates_case_collisions_symlinks_and_encryption() {
    let files = [("a.wgsl".into(), vec![1]), ("b.wgsl".into(), vec![2])].into();
    let bytes = zip(&files, CompressionMethod::Stored);
    // Change both occurrences of the second name: zip's reader would silently coalesce it.
    for replacement in [b"a.wgsl", b"A.wgsl"] {
        let mut duplicate = bytes.clone();
        for offset in 0..duplicate.len() - 5 {
            if &duplicate[offset..offset + 6] == b"b.wgsl" {
                duplicate[offset..offset + 6].copy_from_slice(replacement);
            }
        }
        assert!(error(&duplicate, &PackageLimits::default()).contains("colliding"));
    }
    let mut writer = ZipWriter::new(Cursor::new(Vec::new()));
    writer
        .add_symlink("link", "/outside", SimpleFileOptions::default())
        .unwrap();
    assert!(
        error(
            &writer.finish().unwrap().into_inner(),
            &PackageLimits::default()
        )
        .contains("non-regular")
    );
    let mut encrypted = bytes;
    for i in 0..encrypted.len() - 46 {
        if &encrypted[i..i + 4] == b"PK\x01\x02" {
            encrypted[i + 8] |= 1;
        }
    }
    assert!(Package::from_bytes(&encrypted, &PackageLimits::default()).is_err());
}

#[test]
fn enforces_archive_entry_file_total_metadata_and_shader_limits() {
    let package = package();
    let bytes = package.to_bytes(&PackageLimits::default()).unwrap();
    for limits in [
        PackageLimits {
            max_archive_bytes: bytes.len() as u64 - 1,
            ..Default::default()
        },
        PackageLimits {
            max_entries: 3,
            ..Default::default()
        },
        PackageLimits {
            max_file_bytes: 2,
            ..Default::default()
        },
        PackageLimits {
            max_uncompressed_bytes: 10,
            ..Default::default()
        },
        PackageLimits {
            max_metadata_bytes: 2,
            ..Default::default()
        },
        PackageLimits {
            max_shader_bytes: 2,
            ..Default::default()
        },
    ] {
        assert!(Package::from_bytes(&bytes, &limits).is_err());
        assert!(package.to_bytes(&limits).is_err());
    }
    let compressed = zip(
        &[("bomb".into(), vec![0; 1024 * 1024])].into(),
        CompressionMethod::Deflated,
    );
    let limits = PackageLimits {
        max_file_bytes: 1024,
        ..Default::default()
    };
    assert!(error(&compressed, &limits).contains("byte limit"));
}

#[test]
fn rejects_corruption_truncation_zip64_and_unreferenced_content() {
    let bytes = package().to_bytes(&PackageLimits::default()).unwrap();
    for end in [0, 4, 21, bytes.len() / 2, bytes.len() - 1] {
        assert!(Package::from_bytes(&bytes[..end], &PackageLimits::default()).is_err());
    }
    let mut corrupt = bytes.clone();
    let data_start = 30 + u16::from_le_bytes(corrupt[26..28].try_into().unwrap()) as usize;
    corrupt[data_start] ^= 1;
    assert!(error(&corrupt, &PackageLimits::default()).contains("decode ZIP"));
    let mut zip64 = bytes;
    let end = zip64.len() - 22;
    zip64[end + 8..end + 12].fill(255);
    assert!(error(&zip64, &PackageLimits::default()).contains("ZIP64"));
    let mut files = package().files;
    files.insert("unreferenced.exe".into(), vec![0]);
    assert!(
        Package::from_files(files, &PackageLimits::default())
            .err()
            .unwrap()
            .to_string()
            .contains("unreferenced")
    );
}

#[test]
fn rejects_malformed_json_duplicate_fields_and_non_utf8_shader() {
    for contents in [
        b"{".as_slice(),
        b"{\"format_version\":1,\"format_version\":1}",
        b"[]",
    ] {
        let mut files = package().files;
        files.insert("manifest.json".into(), contents.to_vec());
        assert!(Package::from_files(files, &PackageLimits::default()).is_err());
    }
    let mut files = package().files;
    files.insert("shaders/pass.wgsl".into(), vec![255]);
    assert!(Package::from_files(files, &PackageLimits::default()).is_err());
}

#[test]
fn rejects_conflicting_local_headers_overlaps_and_actual_decompression_overruns() {
    let bytes = package().to_bytes(&PackageLimits::default()).unwrap();
    for offset in [6, 14, 18, 22, 30] {
        let mut conflicting = bytes.clone();
        conflicting[offset] ^= 1;
        assert!(error(&conflicting, &PackageLimits::default()).contains("mismatch"));
    }
    let compressed = zip(
        &[("bomb".into(), vec![0; 1024 * 1024])].into(),
        CompressionMethod::Deflated,
    );
    let mut dishonest = compressed;
    dishonest[22..26].copy_from_slice(&1u32.to_le_bytes());
    let central = dishonest
        .windows(4)
        .position(|part| part == b"PK\x01\x02")
        .unwrap();
    dishonest[central + 24..central + 28].copy_from_slice(&1u32.to_le_bytes());
    assert!(
        error(
            &dishonest,
            &PackageLimits {
                max_file_bytes: 1024,
                ..Default::default()
            }
        )
        .contains("byte limit exceeded")
    );

    let mut overlapping = zip(
        &[("first".into(), vec![1; 8]), ("second".into(), vec![2; 8])].into(),
        CompressionMethod::Stored,
    );
    let positions: Vec<_> = overlapping
        .windows(4)
        .enumerate()
        .filter_map(|(i, value)| (value == b"PK\x01\x02").then_some(i))
        .collect();
    let local_offset = |central: usize| {
        u32::from_le_bytes(overlapping[central + 42..central + 46].try_into().unwrap()) as usize
    };
    let first_header = local_offset(positions[0]);
    let second_header = local_offset(positions[1]);
    let name_length = u16::from_le_bytes(
        overlapping[first_header + 26..first_header + 28]
            .try_into()
            .unwrap(),
    ) as usize;
    let extra_length = u16::from_le_bytes(
        overlapping[first_header + 28..first_header + 30]
            .try_into()
            .unwrap(),
    ) as usize;
    let data_start = first_header + 30 + name_length + extra_length;
    let overlapping_size = (second_header + 1 - data_start) as u32;
    // Matching headers ensure the fixture reaches the interval-overlap check.
    for offset in [
        first_header + 18,
        first_header + 22,
        positions[0] + 20,
        positions[0] + 24,
    ] {
        overlapping[offset..offset + 4].copy_from_slice(&overlapping_size.to_le_bytes());
    }
    assert!(error(&overlapping, &PackageLimits::default()).contains("overlapping ZIP entries"));
}

#[test]
fn accepts_empty_directories_but_counts_them_toward_limits() {
    let mut writer = ZipWriter::new(Cursor::new(Vec::new()));
    let options = SimpleFileOptions::default().compression_method(CompressionMethod::Stored);
    writer.add_directory("assets/", options).unwrap();
    for (name, bytes) in package().files {
        writer.start_file(name, options).unwrap();
        writer.write_all(&bytes).unwrap();
    }
    let bytes = writer.finish().unwrap().into_inner();
    Package::from_bytes(&bytes, &PackageLimits::default()).unwrap();
    assert!(
        Package::from_bytes(
            &bytes,
            &PackageLimits {
                max_entries: 4,
                ..Default::default()
            }
        )
        .is_err()
    );
}

fn expanded_fixture() -> (Manifest, Program, BTreeMap<String, Vec<u8>>) {
    let (manifest, mut program, mut assets) = fixture();
    program.version = 2;
    program.shader_abi_version = 2;
    program.resources.insert(
        0,
        Resource::Image {
            id: "overlay".into(),
            path: "assets/overlay.png".into(),
        },
    );
    let Pass::Fragment { images, .. } = &mut program.passes[0] else {
        unreachable!()
    };
    images.push("overlay".into());
    program.parameters.push(Parameter {
        id: "mix".into(),
        default: ParameterValue::Float(0.5),
        min: Some(0.0),
        max: Some(1.0),
    });
    assets.insert("assets/overlay.png".into(), vec![4, 5, 6]);
    (manifest, program, assets)
}

#[test]
fn expanded_packages_preserve_binding_order_and_parameters() {
    let (manifest, program, assets) = expanded_fixture();
    let limits = PackageLimits::default();
    let package = Package::build(manifest, program, assets, &limits).unwrap();
    let package = Package::from_bytes(&package.to_bytes(&limits).unwrap(), &limits).unwrap();
    assert_eq!(package.image_count(), 2);
    assert_eq!(package.image_bytes(), &[1, 2, 3]);
    assert_eq!(package.image_bytes_at(1), Some([4, 5, 6].as_slice()));
    assert_eq!(package.image_bytes_at(2), None);
    assert_eq!(
        package.program().parameters[0].default,
        ParameterValue::Float(0.5)
    );
}

#[test]
fn rejects_invalid_expanded_bindings_and_legacy_extensions() {
    for case in 0..9 {
        let (manifest, mut program, assets) = expanded_fixture();
        match case {
            0 => {
                program.version = 1;
                program.shader_abi_version = 1;
            }
            1 => program.shader_abi_version = 1,
            2 => program.resources.push(program.resources[0].clone()),
            3 => {
                let Pass::Fragment { images, .. } = &mut program.passes[0] else {
                    unreachable!()
                };
                images[0] = "missing".into();
            }
            4 => {
                let Pass::Fragment { images, .. } = &mut program.passes[0] else {
                    unreachable!()
                };
                images[0] = "background".into();
            }
            5 => {
                let Pass::Fragment { images, .. } = &mut program.passes[0] else {
                    unreachable!()
                };
                images.clear();
            }
            6 => program.parameters.push(program.parameters[0].clone()),
            7 => program.parameters[0].default = ParameterValue::Float(2.0),
            8 => program.resources = vec![program.resources[0].clone(); MAX_IMAGES + 1],
            _ => unreachable!(),
        }
        assert!(
            Package::build(manifest, program, assets, &PackageLimits::default()).is_err(),
            "case {case}"
        );
    }
}

#[test]
fn validates_parameter_types_finite_values_and_inclusive_bounds() {
    let mut p = Parameter {
        id: "speed".into(),
        default: ParameterValue::Float(0.5),
        min: Some(0.0),
        max: Some(1.0),
    };
    p.validate().unwrap();
    for value in [ParameterValue::Float(0.0), ParameterValue::Float(1.0)] {
        p.validate_value(&value).unwrap();
    }
    for value in [
        ParameterValue::Float(-0.01),
        ParameterValue::Float(f32::NAN),
        ParameterValue::Float(f32::INFINITY),
        ParameterValue::Integer(1),
    ] {
        assert!(p.validate_value(&value).is_err());
    }
    p.min = Some(2.0);
    assert!(p.validate().is_err());
    p.min = Some(f64::NAN);
    assert!(p.validate().is_err());
    p.default = ParameterValue::Integer(0);
    p.min = Some(-0.5);
    assert!(p.validate().is_err());
    p.min = Some(i32::MIN as f64);
    p.max = Some(i32::MAX as f64);
    p.validate().unwrap();
    p.validate_value(&ParameterValue::Integer(i32::MIN))
        .unwrap();
    p.validate_value(&ParameterValue::Integer(i32::MAX))
        .unwrap();
    p.min = None;
    p.max = None;
    for value in [
        ParameterValue::Color([1.0; 4]),
        ParameterValue::Vector2([-2.0, 2.0]),
        ParameterValue::Vector3([2.0; 3]),
        ParameterValue::Vector4([2.0; 4]),
        ParameterValue::Boolean(true),
    ] {
        p.default = value;
        p.validate().unwrap();
        p.max = Some(1.0);
        assert!(p.validate().is_err());
        p.max = None;
    }
    for value in [
        ParameterValue::Color([1.1; 4]),
        ParameterValue::Vector2([f32::INFINITY; 2]),
        ParameterValue::Vector3([f32::NAN; 3]),
    ] {
        p.default = value;
        assert!(p.validate().is_err());
    }
    assert!(validate_parameters(&vec![p; MAX_PARAMETERS + 1]).is_err());
}

#[test]
fn float_bounds_use_the_same_precision_as_values() {
    Parameter {
        id: "decimal".into(),
        default: ParameterValue::Float(0.1),
        min: Some(0.1),
        max: Some(0.1),
    }
    .validate()
    .unwrap();
}

fn multipass_fixture() -> (Manifest, Program, BTreeMap<String, Vec<u8>>) {
    let (manifest, mut program, mut assets) = fixture();
    program.version = 3;
    program.shader_abi_version = 2;
    program.resources.insert(
        0,
        Resource::RenderTarget {
            id: "intermediate".into(),
            width: 65,
            height: 33,
        },
    );
    let Pass::Fragment { output, .. } = &mut program.passes[0] else {
        unreachable!()
    };
    *output = Output::Target("intermediate".into());
    program.passes.push(Pass::Fragment {
        shader: "shaders/final.wgsl".into(),
        background: "intermediate".into(),
        images: vec!["background".into()],
        output: Output::Presentation,
    });
    assets.insert("shaders/final.wgsl".into(), vec![b' '; 128]);
    (manifest, program, assets)
}

#[test]
fn multipass_roundtrip_and_limits_apply_to_every_shader() {
    let (manifest, program, assets) = multipass_fixture();
    let limits = PackageLimits::default();
    let package =
        Package::build(manifest.clone(), program.clone(), assets.clone(), &limits).unwrap();
    let bytes = package.to_bytes(&limits).unwrap();
    let decoded = Package::from_bytes(&bytes, &limits).unwrap();
    assert_eq!(decoded.program().passes.len(), 2);
    assert_eq!(decoded.shader_source_at(1), Some(" ".repeat(128).as_str()));
    assert_eq!(decoded.shader_source_at(2), None);
    assert_eq!(decoded.resource_image_bytes("intermediate"), None);
    let strict = PackageLimits {
        max_shader_bytes: 64,
        ..limits
    };
    assert!(Package::build(manifest, program, assets, &strict).is_err());
    assert!(Package::from_bytes(&bytes, &strict).is_err());
    assert!(decoded.to_bytes(&strict).is_err());
}

#[test]
fn rejects_invalid_pass_dependencies_targets_and_versions() {
    for case in 0..18 {
        let (manifest, mut program, mut assets) = multipass_fixture();
        match case {
            0 => program.version = 2,
            1 => program.shader_abi_version = 3,
            2 => program.passes.swap(0, 1),
            3 => {
                // Read before write / self feedback.
                let Pass::Fragment { background, .. } = &mut program.passes[0] else {
                    unreachable!()
                };
                *background = "intermediate".into();
            }
            4 => {
                // Multiple producers.
                program.passes.insert(1, program.passes[0].clone());
            }
            5 => {
                let Pass::Fragment { output, .. } = &mut program.passes[0] else {
                    unreachable!()
                };
                *output = Output::Target("background".into());
            }
            6 => {
                let Pass::Fragment { output, .. } = &mut program.passes[0] else {
                    unreachable!()
                };
                *output = Output::Target("missing".into());
            }
            7 => {
                let Pass::Fragment { output, .. } = &mut program.passes[0] else {
                    unreachable!()
                };
                *output = Output::Presentation;
            }
            8 => {
                program.passes.pop();
            }
            9 => {
                let Pass::Fragment {
                    background, images, ..
                } = &mut program.passes[1]
                else {
                    unreachable!()
                };
                *background = "background".into();
                images.clear(); // Unused target.
            }
            10 => program.resources.push(Resource::RenderTarget {
                id: "unused".into(),
                width: 1,
                height: 1,
            }),
            11 => {
                let Resource::RenderTarget { width, .. } = &mut program.resources[0] else {
                    unreachable!()
                };
                *width = 0;
            }
            12 => {
                let Resource::RenderTarget { height, .. } = &mut program.resources[0] else {
                    unreachable!()
                };
                *height = MAX_TARGET_DIMENSION + 1;
            }
            13 => {
                assets.remove("shaders/final.wgsl");
            }
            14 => {
                assets.insert("shaders/final.wgsl".into(), vec![255]);
            }
            15 => program.passes = vec![program.passes[0].clone(); MAX_PASSES + 1],
            16 => {
                let Pass::Fragment { images, .. } = &mut program.passes[1] else {
                    unreachable!()
                };
                images.push("intermediate".into());
            }
            17 => {
                let Pass::Fragment { background, .. } = &mut program.passes[1] else {
                    unreachable!()
                };
                *background = "missing".into();
            }
            _ => unreachable!(),
        }
        assert!(
            Package::build(manifest, program, assets, &PackageLimits::default()).is_err(),
            "case {case}"
        );
    }
}
