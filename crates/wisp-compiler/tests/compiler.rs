use serde_json::{Value, json};
use std::collections::BTreeMap;
use wisp_compiler::{Asset, CompileOptions, compile, compile_project};
use wisp_format::{Manifest, Package, PackageLimits, Resource};
use wisp_ir::Graph;

fn graph(mut nodes: Value) -> Graph {
    for node in nodes.as_array_mut().unwrap() {
        let id = node.as_object_mut().unwrap().remove("id").unwrap();
        *node = json!({"id":id,"op":node.take()});
    }
    serde_json::from_value(json!({"version":1,"nodes":nodes,"output":"out"})).unwrap()
}
fn manifest() -> Manifest {
    Manifest {
        format_version: 1,
        id: "test.compiler".into(),
        name: "Compiler test".into(),
        author: "Wisp".into(),
        description: String::new(),
        preview: None,
        program: "program.json".into(),
        minimum_wisp_version: "0.1.0".into(),
        gpu_profile: 1,
        tags: vec![],
    }
}
fn constant_graph() -> Graph {
    graph(json!([
        {"id":"color","type":"constant","value":{"type":"color","value":[0.2,0.4,0.8,1.0]}},
        {"id":"out","type":"output","color":"color"}
    ]))
}
fn example(name: &str) -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples")
        .join(name)
}
#[test]
fn examples_fuse_prune_validate_and_roundtrip_deterministically() {
    for (name, passes, targets) in [("graph-fused", 1, 0), ("graph-blur", 3, 2)] {
        let result = compile_project(example(name), &CompileOptions::default()).unwrap();
        assert_eq!(result.report.passes, passes);
        assert_eq!(result.report.intermediate_targets, targets);
        assert_eq!(result.report.eliminated_nodes, 1);
        assert_eq!(result.report.image_assets, 1);
        assert!(!result.package.shader_source().contains("unused"));
        let limits = PackageLimits::default();
        let bytes = result.package.to_bytes(&limits).unwrap();
        let reloaded = Package::from_bytes(&bytes, &limits).unwrap();
        wisp_runtime::Wallpaper::from_package(&reloaded).unwrap();
        assert_eq!(
            bytes,
            compile_project(example(name), &CompileOptions::default())
                .unwrap()
                .package
                .to_bytes(&limits)
                .unwrap()
        );
    }
}
#[test]
fn invalid_graphs_report_nodes_edges_types_and_versions() {
    let mut cases = vec![];
    let mut g = constant_graph();
    g.version = 2;
    cases.push((g, "version"));
    let mut g = constant_graph();
    g.nodes.push(g.nodes[0].clone());
    cases.push((g, "duplicate node"));
    let mut g = constant_graph();
    g.output = "missing".into();
    cases.push((g, "missing graph output"));
    let mut g = constant_graph();
    g.output = "color".into();
    cases.push((g, "Output"));
    cases.push((
        graph(json!([{"id":"out","type":"output","color":"missing"}])),
        "missing node",
    ));
    cases.push((
        graph(
            json!([{"id":"a","type":"sin","input":"a"},{"id":"out","type":"output","color":"a"}]),
        ),
        "cycle",
    ));
    cases.push((
        graph(json!([{"id":"a","type":"time"},{"id":"out","type":"output","color":"a"}])),
        "expected Color",
    ));
    cases.push((graph(json!([{"id":"a","type":"parameter","parameter":"missing"},{"id":"out","type":"output","color":"a"}])),"unknown parameter"));
    let mut g = constant_graph();
    g.nodes.push(wisp_ir::Node {
        id: "dead".into(),
        op: wisp_ir::Op::Sin {
            input: "dead".into(),
        },
    });
    cases.push((g, "cycle"));
    let mut g = constant_graph();
    g.nodes[0].id = "bad\nWGSL".into();
    cases.push((g, "identifier"));
    let mut g = constant_graph();
    g.nodes = vec![g.nodes[0].clone(); 257];
    cases.push((g, "256"));
    for (g, expected) in cases {
        let error = g.check().err().expect("graph should fail").to_string();
        assert!(error.contains(expected), "expected {expected}: {error}");
    }
    for op in [
        json!({"type":"time","unexpected":true}),
        json!({"type":"constant","value":{"type":"float","value":1},"unexpected":true}),
        json!({"type":"not_a_node"}),
    ] {
        assert!(
            serde_json::from_value::<Graph>(
                json!({"version":1,"nodes":[{"id":"a","op":op}],"output":"a"})
            )
            .is_err()
        );
    }
}
#[test]
fn procedural_graphs_and_dead_parameters_need_no_external_assets() {
    let mut g = constant_graph();
    g.parameters.push(wisp_format::Parameter {
        id: "unused".into(),
        default: wisp_format::ParameterValue::Float(1.0),
        min: None,
        max: None,
    });
    let result = compile(
        &g.check().unwrap(),
        &BTreeMap::new(),
        manifest(),
        &CompileOptions::default(),
    )
    .unwrap();
    assert_eq!(result.report.image_assets, 0);
    assert_eq!(result.report.parameters, 0);
    assert_eq!(result.package.program().resources.len(), 1);
}
#[test]
fn shared_and_nested_boundaries_are_topologically_lowered_once() {
    let mut g = constant_graph();
    g.nodes.extend([
        wisp_ir::Node {
            id: "radius".into(),
            op: wisp_ir::Op::Constant {
                value: wisp_format::ParameterValue::Float(1.0),
            },
        },
        wisp_ir::Node {
            id: "blur".into(),
            op: wisp_ir::Op::Blur {
                color: "color".into(),
                radius: "radius".into(),
                width: 9,
                height: 7,
            },
        },
        wisp_ir::Node {
            id: "mix".into(),
            op: wisp_ir::Op::Mix {
                a: "blur".into(),
                b: "blur".into(),
                factor: "radius".into(),
            },
        },
    ]);
    g.nodes[1].op = wisp_ir::Op::Output {
        color: "mix".into(),
    };
    let first = compile(
        &g.clone().check().unwrap(),
        &BTreeMap::new(),
        manifest(),
        &CompileOptions::default(),
    )
    .unwrap();
    assert_eq!(first.report.passes, 3);
    assert_eq!(first.report.intermediate_bytes, 9 * 7 * 4 * 2);
    assert_eq!(
        first
            .package
            .shader_source_at(2)
            .unwrap()
            .matches("// node blur\n")
            .count(),
        1
    );
    g.nodes.push(wisp_ir::Node {
        id: "blur2".into(),
        op: wisp_ir::Op::Blur {
            color: "mix".into(),
            radius: "radius".into(),
            width: 5,
            height: 3,
        },
    });
    g.nodes[1].op = wisp_ir::Op::Output {
        color: "blur2".into(),
    };
    assert_eq!(
        compile(
            &g.check().unwrap(),
            &BTreeMap::new(),
            manifest(),
            &CompileOptions::default()
        )
        .unwrap()
        .report
        .passes,
        5
    );
}
#[test]
fn limits_and_asset_validation_apply_before_export() {
    let path = example("graph-blur");
    let mut options = CompileOptions::default();
    options.runtime_limits.max_passes = 2;
    assert!(
        compile_project(&path, &options)
            .err()
            .unwrap()
            .to_string()
            .contains("pass limit")
    );
    let mut options = CompileOptions::default();
    options.runtime_limits.max_texture_bytes = 100;
    assert!(
        compile_project(&path, &options)
            .err()
            .unwrap()
            .to_string()
            .contains("memory limit")
    );
    let mut options = CompileOptions::default();
    options.runtime_limits.max_intermediate_targets = 1;
    assert!(
        compile_project(&path, &options)
            .err()
            .unwrap()
            .to_string()
            .contains("target limit")
    );
    let mut options = CompileOptions::default();
    options.package_limits.max_shader_bytes = 10;
    assert!(compile_project(&path, &options).is_err());
    let mut options = CompileOptions::default();
    options.package_limits.max_archive_bytes = 10;
    assert!(compile_project(&path, &options).is_err());
    let g: Graph =
        serde_json::from_slice(&std::fs::read(example("graph-fused").join("graph.json")).unwrap())
            .unwrap();
    let g = g.check().unwrap();
    assert!(
        compile(&g, &BTreeMap::new(), manifest(), &CompileOptions::default())
            .err()
            .unwrap()
            .to_string()
            .contains("missing image")
    );
    let assets = BTreeMap::from([(
        "background".into(),
        Asset {
            extension: "png".into(),
            bytes: vec![0; 10],
        },
    )]);
    assert!(compile(&g, &assets, manifest(), &CompileOptions::default()).is_err());
}

#[test]
fn image_preparation_reserves_intermediate_texture_memory() {
    let mut options = CompileOptions::default();
    let targets = 640 * 360 * 4 * 2;
    options.runtime_limits.max_texture_bytes = targets + 32 * 32 * 4;
    options.runtime_limits.max_resource_bytes = options.runtime_limits.max_texture_bytes;
    let result = compile_project(example("graph-blur"), &options).unwrap();
    assert_eq!(result.report.resized_images, 1);
    assert_eq!(result.report.intermediate_bytes, targets);
    assert!(result.report.image_bytes <= 32 * 32 * 4);
    wisp_runtime::Wallpaper::from_package_with_limits(&result.package, &options.runtime_limits)
        .unwrap();
}

#[test]
fn wide_jpeg_and_webp_sources_export_with_correct_image_paths() {
    let g = graph(json!([
        {"id":"uv","type":"uv"},
        {"id":"image","type":"image","asset":"source","uv":"uv"},
        {"id":"out","type":"output","color":"image"}
    ]))
    .check()
    .unwrap();
    let pixels = image::RgbImage::from_pixel(7000, 14, image::Rgb([40, 100, 180]));
    for (format, extension) in [
        (image::ImageFormat::Jpeg, "jpg"),
        (image::ImageFormat::WebP, "webp"),
    ] {
        let mut bytes = std::io::Cursor::new(Vec::new());
        pixels.write_to(&mut bytes, format).unwrap();
        let bytes = bytes.into_inner();
        let assets = BTreeMap::from([(
            "source".into(),
            Asset {
                extension: extension.into(),
                bytes: bytes.clone(),
            },
        )]);
        let result = compile(&g, &assets, manifest(), &CompileOptions::default()).unwrap();
        assert_eq!(result.report.resized_images, 1);
        assert_eq!(
            result.package.manifest().preview.as_deref(),
            Some("assets/image0.png")
        );
        let wallpaper = wisp_runtime::Wallpaper::from_package(&result.package).unwrap();
        assert_eq!(wallpaper.images()[0].dimensions().0, 4096);
        assert_eq!(assets["source"].bytes, bytes);
    }
}
#[test]
fn fusable_node_catalog_generates_valid_wgsl() {
    let g = graph(json!([
        {"id":"uv","type":"uv"}, {"id":"mouse","type":"mouse"},
        {"id":"resolution","type":"resolution"},
        {"id":"time","type":"time"}, {"id":"dt","type":"delta_time"},
        {"id":"bass","type":"audio_bass"}, {"id":"mid","type":"audio_mid"},
        {"id":"treble","type":"audio_treble"}, {"id":"volume","type":"audio_volume"},
        {"id":"sum","type":"add","a":"time","b":"dt"},
        {"id":"wave","type":"sin","input":"sum"},
        {"id":"product","type":"multiply","a":"wave","b":"bass"},
        {"id":"gain","type":"add","a":"product","b":"mid"},
        {"id":"gain2","type":"add","a":"gain","b":"treble"},
        {"id":"gain3","type":"add","a":"gain2","b":"volume"},
        {"id":"offset","type":"multiply","a":"mouse","b":"resolution"},
        {"id":"shift","type":"translate","uv":"uv","offset":"offset"},
        {"id":"image","type":"image","asset":"image","uv":"shift"},
        {"id":"contrast","type":"contrast","color":"image","amount":"gain3"},
        {"id":"mix","type":"mix","a":"contrast","b":"image","factor":"wave"},
        {"id":"out","type":"output","color":"mix"}
    ]))
    .check()
    .unwrap();
    let assets = BTreeMap::from([(
        "image".into(),
        Asset {
            extension: "png".into(),
            bytes: wisp_runtime::EXAMPLE_IMAGE.to_vec(),
        },
    )]);
    let result = compile(&g, &assets, manifest(), &CompileOptions::default()).unwrap();
    assert_eq!(result.report.passes, 1);
    assert_eq!(result.report.eliminated_nodes, 0);
    assert_eq!(
        result
            .package
            .program()
            .resources
            .iter()
            .filter(|r| matches!(r, Resource::Image { .. }))
            .count(),
        1
    );
}

#[test]
fn simulation_compiles_existing_abi_and_enforces_work_and_memory_limits() {
    let result = compile_project(example("audio-reactive"), &Default::default()).unwrap();
    assert_eq!(result.package.program().version, 4);
    assert_eq!(result.package.program().shader_abi_version, 3);
    assert_eq!(result.report.passes, 5);
    assert_eq!(result.report.storage_bytes, 32 * 16 + 512 * 512 * 16);
    let bytes = result.package.to_bytes(&Default::default()).unwrap();
    assert_eq!(
        bytes,
        compile_project(example("audio-reactive"), &Default::default())
            .unwrap()
            .package
            .to_bytes(&Default::default())
            .unwrap()
    );
    let source: Graph = serde_json::from_slice(
        &std::fs::read(example("audio-reactive").join("graph.json")).unwrap(),
    )
    .unwrap();
    for (field, value, error) in [
        ("count", 0, "count"),
        ("count", 129, "count"),
        ("width", 0, "dimensions"),
        ("height", 1025, "dimensions"),
        ("count", 128, "workload"),
    ] {
        let mut g = serde_json::to_value(&source).unwrap();
        let op = g["nodes"]
            .as_array_mut()
            .unwrap()
            .iter_mut()
            .find(|n| n["id"] == "particles")
            .unwrap();
        op["op"][field] = value.into();
        let graph: Graph = serde_json::from_value(g).unwrap();
        assert!(graph.check().err().unwrap().to_string().contains(error));
    }
    for limit in [0, 1024] {
        let mut opts = CompileOptions::default();
        opts.runtime_limits.max_storage_bytes = limit;
        assert!(
            compile_project(example("audio-reactive"), &opts)
                .err()
                .unwrap()
                .to_string()
                .contains("storage")
        );
    }
    let mut opts = CompileOptions::default();
    opts.runtime_limits.max_resource_bytes = 1024;
    assert!(compile_project(example("audio-reactive"), &opts).is_err());
    // A temporal effect breaks frame dependencies in its own buffer, not through graph cycles.
    let mut g = source.clone();
    if let wisp_ir::Op::Feedback { color, .. } =
        &mut g.nodes.iter_mut().find(|n| n.id == "trails").unwrap().op
    {
        *color = "trails".into();
    }
    assert!(g.check().err().unwrap().to_string().contains("cycle"));
    // Removing the simulation branch removes all its state and compute work.
    let mut g = source;
    if let wisp_ir::Op::Output { color } =
        &mut g.nodes.iter_mut().find(|n| n.id == "out").unwrap().op
    {
        *color = "blue".into();
    }
    let compiled = compile(
        &g.check().unwrap(),
        &BTreeMap::new(),
        manifest(),
        &Default::default(),
    )
    .unwrap();
    assert_eq!(compiled.report.storage_bytes, 0);
    assert_eq!(compiled.report.passes, 1);
    assert_eq!(compiled.package.program().version, 2);
}
