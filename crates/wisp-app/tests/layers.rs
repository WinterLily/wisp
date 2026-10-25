use std::path::Path;
use wisp_app::{
    designer::{Draft, Preset},
    graph::{CATALOG, Document, ports},
    layers::Effect,
};
use wisp_ir::{BlendMode, Op};

fn draft(root: &Path) -> Draft {
    let path = root.join("source.png");
    std::fs::write(&path, wisp_runtime::EXAMPLE_IMAGE).unwrap();
    let mut d = Draft::from_image(&path).unwrap();
    d.design.preset(Preset::Still);
    let s = d.enable_layers();
    s.import(&path, None).unwrap();
    s.layers[1].position = [0.1, -0.1];
    s.layers[1].scale = [0.8, 0.7];
    s.layers[1].pivot = [0.3, 0.6];
    s.layers[1].rotation = 15.;
    s.layers[1].opacity = 0.7;
    s.blank_mask(1, false).unwrap();
    s.paint(1, [0.3, 0.2], [0.7, 0.4], 0.1, 255).unwrap();
    d
}
#[test]
fn all_layer_effects_and_blends_compile_and_preserve_editable_sources() {
    let temp = tempfile::tempdir().unwrap();
    let mut d = draft(temp.path());
    for (i, effect) in Effect::ALL.into_iter().enumerate() {
        let s = d.layers.as_mut().unwrap();
        s.layers[1].preset(effect);
        s.layers[1].displacement = Some("background".into());
        s.layers[1].blend = BlendMode::ALL[i % 5];
        let compiled = d.compile().unwrap();
        assert_eq!(compiled.report.passes, 1);
        assert_eq!(compiled.report.storage_bytes, 0);
        let dir = temp.path().join(format!("project{i}"));
        d.save_project(&dir).unwrap();
        let reopened = Draft::open_project(&dir).unwrap();
        let expected = compiled.package.to_bytes(&Default::default()).unwrap();
        assert_eq!(
            expected,
            reopened
                .compile()
                .unwrap()
                .package
                .to_bytes(&Default::default())
                .unwrap()
        );
        assert_eq!(
            expected,
            wisp_compiler::compile_project(&dir, &Default::default())
                .unwrap()
                .package
                .to_bytes(&Default::default())
                .unwrap()
        );
        assert_eq!(
            serde_json::to_value(&d.layers).unwrap(),
            serde_json::to_value(&reopened.layers).unwrap()
        );
        assert!(d.save_project(&dir).is_err());
        let package = temp.path().join(format!("effect{i}.wisp"));
        d.export(&package).unwrap();
        assert!(d.export(&package).is_err());
    }
    // Easy conversion carries the complete graph and all mask/source assets.
    let mut document = Document::from_easy(&d).unwrap();
    assert_eq!(
        d.compile()
            .unwrap()
            .package
            .to_bytes(&Default::default())
            .unwrap(),
        document
            .compile()
            .unwrap()
            .package
            .to_bytes(&Default::default())
            .unwrap()
    );
    // Layer effects also feed existing persistent feedback passes without a new ABI.
    let decay = document.add("Constant", [0., 0.]).unwrap();
    let feedback = document.add("Feedback", [0., 0.]).unwrap();
    let source = match &document.node("output").unwrap().op {
        Op::Output { color } => color.clone(),
        _ => unreachable!(),
    };
    document.connect(&source, &feedback, "color").unwrap();
    document.connect(&decay, &feedback, "decay").unwrap();
    document.connect(&feedback, "output", "color").unwrap();
    let persistent = document.compile().unwrap();
    assert_eq!(persistent.package.program().version, 4);
    assert!(persistent.report.storage_bytes > 0 && persistent.report.passes > 1);
    std::fs::remove_file(temp.path().join("source.png")).unwrap();
    assert!(Draft::open_project(&temp.path().join("project1")).is_ok());
}
#[test]
fn masks_paint_erase_import_and_limits_fail_before_export() {
    let temp = tempfile::tempdir().unwrap();
    let mut d = draft(temp.path());
    let s = d.layers.as_mut().unwrap();
    let id = s.layers[1].mask.as_ref().unwrap().asset.clone();
    let pixels = image::load_from_memory(&s.assets[&id].bytes)
        .unwrap()
        .into_luma8();
    assert_eq!(pixels.get_pixel(0, 0).0[0], 0);
    assert_eq!(pixels.get_pixel(128, 77).0[0], 255);
    s.paint(1, [0.5, 0.3], [0.5, 0.3], 0.05, 0).unwrap();
    assert_eq!(
        image::load_from_memory(&s.assets[&id].bytes)
            .unwrap()
            .into_luma8()
            .get_pixel(128, 77)
            .0[0],
        0
    );
    assert!(s.paint(1, [f32::NAN, 0.], [0., 0.], 0.1, 255).is_err());
    assert!(s.layers[1].set_value("sx", 0.).is_err());
    s.import(&temp.path().join("source.png"), Some((1, false)))
        .unwrap();
    let m = s.layers[1].mask.as_ref().unwrap();
    let pixels = image::load_from_memory(&s.assets[&m.asset].bytes)
        .unwrap()
        .into_rgb8();
    assert!(
        pixels
            .pixels()
            .all(|p| p.0[0] == p.0[1] && p.0[1] == p.0[2])
    );
    s.layers[1].visible = false;
    assert_eq!(d.compile().unwrap().report.image_assets, 1);
    let s = d.layers.as_mut().unwrap();
    s.layers[1].visible = true;
    s.layers[1].rotation = f32::NAN;
    assert!(d.compile().is_err());
    d.layers.as_mut().unwrap().layers[1].rotation = 0.;
    let dir = temp.path().join("draft");
    d.save_project(&dir).unwrap();
    let mut graph: serde_json::Value =
        serde_json::from_slice(&std::fs::read(dir.join("graph.json")).unwrap()).unwrap();
    graph["nodes"][0]["id"] = "external_edit".into();
    std::fs::write(dir.join("graph.json"), serde_json::to_vec(&graph).unwrap()).unwrap();
    assert!(Draft::open_project(&dir).is_err());
}
#[test]
fn new_graph_catalog_sockets_and_types_match_canonical_ir() {
    let mut document = Document::default();
    for kind in CATALOG {
        let id = document.add(kind, [0., 0.]).unwrap();
        let op = &document.node(&id).unwrap().op;
        assert_eq!(op.inputs().len(), ports(op).len(), "{kind}");
    }
    let uv = document.add("UV", [0., 0.]).unwrap();
    let mask = document.add("Mask", [0., 0.]).unwrap();
    let layer = document.add("Layer", [0., 0.]).unwrap();
    assert!(document.connect(&uv, &layer, "mask").is_err());
    document.connect(&mask, &layer, "mask").unwrap();
    assert!(matches!(
        document.node(&layer).unwrap().op,
        Op::Layer { .. }
    ));
}

#[test]
fn legacy_layer_projects_upgrade_after_validating_their_original_graph() {
    let temp = tempfile::tempdir().unwrap();
    let mut draft = draft(temp.path());
    draft.layers.as_mut().unwrap().version = 1;
    draft.layers.as_mut().unwrap().layers[1].effect = Effect::Rain;
    let old = temp.path().join("legacy");
    draft.save_project(&old).unwrap();
    let reopened = Draft::open_project(&old).unwrap();
    assert_eq!(reopened.layers.as_ref().unwrap().version, 2);
    assert_eq!(
        reopened.layers.as_ref().unwrap().layers[1].position,
        draft.layers.as_ref().unwrap().layers[1].position
    );
    let new = temp.path().join("upgraded");
    reopened.save_project(&new).unwrap();
    assert_eq!(
        reopened
            .compile()
            .unwrap()
            .package
            .to_bytes(&Default::default())
            .unwrap(),
        wisp_compiler::compile_project(&new, &Default::default())
            .unwrap()
            .package
            .to_bytes(&Default::default())
            .unwrap()
    );
}
