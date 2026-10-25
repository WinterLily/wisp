use std::path::Path;
use wisp_app::graph::{CATALOG, Document, ports};
use wisp_ir::Op;

fn fixture() -> Document {
    Document::open(&Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/graph-fused"))
        .unwrap()
}
#[test]
fn projects_round_trip_assets_layout_and_compile_identically() {
    let temp = tempfile::tempdir().unwrap();
    let mut d = fixture();
    let unused = d.add("Time", [777., 888.]).unwrap();
    let bytes = d
        .compile()
        .unwrap()
        .package
        .to_bytes(&Default::default())
        .unwrap();
    let path = temp.path().join("draft");
    d.save(&path).unwrap();
    let reopened = Document::open(&path).unwrap();
    assert_eq!(reopened.layout.positions[&unused], [777., 888.]);
    assert_eq!(
        serde_json::to_value(&d.graph).unwrap(),
        serde_json::to_value(&reopened.graph).unwrap()
    );
    assert_eq!(
        bytes,
        reopened
            .compile()
            .unwrap()
            .package
            .to_bytes(&Default::default())
            .unwrap()
    );
    assert_eq!(
        bytes,
        wisp_compiler::compile_project(&path, &Default::default())
            .unwrap()
            .package
            .to_bytes(&Default::default())
            .unwrap()
    );
    assert!(d.save(&path).is_err());
    let export = temp.path().join("wallpaper.wisp");
    d.export(&export).unwrap();
    assert!(d.export(&export).is_err());
    assert_eq!(bytes, std::fs::read(export).unwrap());
}
#[test]
fn connections_reject_cycles_and_types_without_mutation_and_recover_incomplete_drafts() {
    let mut d = Document::default();
    let time = d.add("Time", [0., 0.]).unwrap();
    let brightness = d.add("Brightness", [0., 0.]).unwrap();
    let before = serde_json::to_value(&d.graph).unwrap();
    assert!(
        d.connect(&time, &brightness, "color")
            .unwrap_err()
            .to_string()
            .contains("expected Color")
    );
    assert_eq!(before, serde_json::to_value(&d.graph).unwrap());
    d.connect("color", &brightness, "color").unwrap();
    d.connect(&time, &brightness, "amount").unwrap();
    d.connect(&brightness, "output", "color").unwrap();
    assert!(d.compile().is_ok());
    assert!(d.connect(&brightness, &brightness, "color").is_err());
    assert!(d.connect("output", &brightness, "color").is_err());
    d.remove(&time);
    assert!(d.compile().is_err());
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("unfinished");
    d.save(&path).unwrap();
    let mut d = Document::open(&path).unwrap();
    let constant = d.add("Constant", [0., 0.]).unwrap();
    d.connect(&constant, &brightness, "amount").unwrap();
    assert!(d.compile().is_ok());
}
#[test]
fn catalog_ports_and_typed_values_are_editable() {
    let mut d = Document::default();
    assert!(!CATALOG.contains(&"Parallax"));
    assert!(d.add("Parallax", [0., 0.]).is_err());
    for kind in CATALOG {
        let id = d.add(kind, [0., 0.]).unwrap();
        let n = d.node(&id).unwrap();
        assert_eq!(ports(&n.op).len(), n.op.inputs().len(), "{kind}");
        if let Op::Constant { .. } = n.op {
            d.set_field(
                &id,
                "value",
                serde_json::json!({"type":"vector2","value":[0.2,0.3]}),
            )
            .unwrap();
            assert_eq!(d.value_type(&id), Some(wisp_ir::ValueType::Vector2));
        }
    }
    // Arrangement remains bounded for cyclic drafts and shared subexpressions.
    let add = d.add("Add", [0., 0.]).unwrap();
    d.set_field(&add, "a", add.clone().into()).unwrap();
    d.set_field(&add, "b", add.clone().into()).unwrap();
    d.arrange();
    assert_eq!(d.value_type(&add), None);
    assert!(d.compile().is_err());
}
#[test]
fn easy_designer_and_multiple_images_remain_editable() {
    let temp = tempfile::tempdir().unwrap();
    let image = temp.path().join("source.png");
    std::fs::write(&image, wisp_runtime::EXAMPLE_IMAGE).unwrap();
    let easy = wisp_app::designer::Draft::from_image(&image).unwrap();
    let mut d = Document::from_easy(&easy).unwrap();
    assert_eq!(
        easy.compile()
            .unwrap()
            .package
            .to_bytes(&Default::default())
            .unwrap(),
        d.compile()
            .unwrap()
            .package
            .to_bytes(&Default::default())
            .unwrap()
    );
    let asset = d.import_image(&image).unwrap();
    let node = d.add("Image", [0., 0.]).unwrap();
    d.set_field(&node, "asset", asset.clone().into()).unwrap();
    d.connect("uv", &node, "uv").unwrap();
    let saved = temp.path().join("saved");
    d.save(&saved).unwrap();
    std::fs::remove_file(image).unwrap();
    let reopened = Document::open(&saved).unwrap();
    assert_eq!(reopened.assets.len(), 2);
    assert!(reopened.assets.contains_key(&asset));
    assert!(reopened.compile().is_ok());
}
#[test]
fn malformed_layout_paths_and_oversized_metadata_are_rejected() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("project");
    fixture().save(&path).unwrap();
    std::fs::write(path.join("layout.json"), r#"{"version":9,"positions":{}}"#).unwrap();
    assert!(Document::open(&path).is_err());
    std::fs::remove_file(path.join("layout.json")).unwrap();
    let mut project: serde_json::Value =
        serde_json::from_slice(&std::fs::read(path.join("project.json")).unwrap()).unwrap();
    project["assets"]
        .as_object_mut()
        .unwrap()
        .values_mut()
        .next()
        .unwrap()["path"] = "assets/../../outside.png".into();
    std::fs::write(
        path.join("project.json"),
        serde_json::to_vec(&project).unwrap(),
    )
    .unwrap();
    assert!(Document::open(&path).is_err());
    std::fs::write(path.join("project.json"), vec![b' '; 1024 * 1024 + 1]).unwrap();
    assert!(Document::open(&path).is_err());
}

#[test]
fn interactive_graph_roundtrips_sources_controls_and_typed_sockets() {
    let root =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/audio-reactive");
    let mut doc = wisp_app::graph::Document::open(&root).unwrap();
    let original = doc
        .compile()
        .unwrap()
        .package
        .to_bytes(&Default::default())
        .unwrap();
    let temp = tempfile::tempdir().unwrap();
    doc.save(&temp.path().join("draft")).unwrap();
    let loaded = wisp_app::graph::Document::open(&temp.path().join("draft")).unwrap();
    assert_eq!(
        original,
        loaded
            .compile()
            .unwrap()
            .package
            .to_bytes(&Default::default())
            .unwrap()
    );
    assert!(doc.connect("blue", "particles", "position").is_err());
    assert!(doc.connect("mouse", "trails", "decay").is_err());
    let uv = doc.add("UV", [0., 0.]).unwrap();
    let distance = doc.add("Mouse distance", [0., 0.]).unwrap();
    doc.connect(&uv, &distance, "uv").unwrap();
    doc.connect(&distance, "particles", "size").unwrap();
    assert!(doc.compile().unwrap().report.storage_bytes > 0);
    doc.set_field("particles", "count", 0.into()).unwrap();
    assert!(doc.compile().is_err());
}
