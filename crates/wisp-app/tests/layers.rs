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

#[test]
fn attached_effects_keep_independent_masks_and_round_trip_through_projects_and_graphs() {
    use wisp_app::layers::{ImportTarget, LayerTarget};
    let temp = tempfile::tempdir().unwrap();
    let mut draft = draft(temp.path());
    let stack = draft.layers.as_mut().unwrap();
    for effect in [
        Effect::Hair,
        Effect::Hair,
        Effect::Rain,
        Effect::Displacement,
    ] {
        stack.add_effect(1, effect).unwrap();
    }
    let targets = [
        LayerTarget::Image(1),
        LayerTarget::Effect(1, 0),
        LayerTarget::Effect(1, 1),
    ];
    let ids: Vec<_> = targets
        .iter()
        .map(|target| {
            stack
                .target_mask(*target)
                .unwrap()
                .as_ref()
                .unwrap()
                .asset
                .clone()
        })
        .collect();
    assert_eq!(
        ids.iter().collect::<std::collections::BTreeSet<_>>().len(),
        3
    );
    let original_image_mask = stack.assets[&ids[0]].bytes.clone();
    stack
        .paint_target(targets[1], [0.2, 0.2], [0.2, 0.2], 0.1, 255)
        .unwrap();
    stack
        .paint_target(targets[2], [0.8, 0.8], [0.8, 0.8], 0.1, 255)
        .unwrap();
    for (index, point) in [(1, (51, 51)), (2, (205, 205))] {
        let pixels = image::load_from_memory(&stack.assets[&ids[index]].bytes)
            .unwrap()
            .into_luma8();
        assert_eq!(pixels.get_pixel(point.0, point.1).0[0], 255);
        let other = if index == 1 { (205, 205) } else { (51, 51) };
        assert_eq!(pixels.get_pixel(other.0, other.1).0[0], 0);
    }
    assert_eq!(stack.assets[&ids[0]].bytes, original_image_mask);
    stack
        .import(
            &temp.path().join("source.png"),
            ImportTarget::EffectDisplacement(1, 3),
        )
        .unwrap();
    stack.layers[1].effects[0]
        .set_value("amplitude".try_into().unwrap(), 0.08)
        .unwrap();
    stack.layers[1].effects[1]
        .set_value("speed".try_into().unwrap(), 2.)
        .unwrap();
    stack.prune();
    assert!(ids.iter().all(|id| stack.assets.contains_key(id)));
    let graph = draft.graph().unwrap();
    assert_eq!(
        graph
            .nodes
            .iter()
            .filter(|node| matches!(node.op, Op::Wave { .. }))
            .count(),
        2
    );
    assert_eq!(
        graph
            .nodes
            .iter()
            .filter(|node| matches!(node.op, Op::Displacement { .. }))
            .count(),
        1
    );
    // Weather on an attached image must run before that image is blended into the scene.
    for node in &graph.nodes {
        if let Op::Weather { color, .. } = &node.op {
            assert!(!matches!(
                graph
                    .nodes
                    .iter()
                    .find(|node| &node.id == color)
                    .unwrap()
                    .op,
                Op::Blend { .. }
            ));
        }
    }
    let expected = draft
        .compile()
        .unwrap()
        .package
        .to_bytes(&Default::default())
        .unwrap();
    let project = temp.path().join("attached");
    draft.save_project(&project).unwrap();
    let reopened = Draft::open_project(&project).unwrap();
    assert_eq!(
        serde_json::to_value(&draft.layers).unwrap(),
        serde_json::to_value(&reopened.layers).unwrap()
    );
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
        Document::from_easy(&reopened)
            .unwrap()
            .compile()
            .unwrap()
            .package
            .to_bytes(&Default::default())
            .unwrap()
    );
    assert_eq!(
        expected,
        wisp_compiler::compile_project(&project, &Default::default())
            .unwrap()
            .package
            .to_bytes(&Default::default())
            .unwrap()
    );
    let stack = draft.layers.as_mut().unwrap();
    stack.layers[1].effects[0].visible = false;
    let graph = draft.graph().unwrap();
    assert_eq!(
        graph
            .nodes
            .iter()
            .filter(|node| matches!(node.op, Op::Wave { .. }))
            .count(),
        1
    );
    draft.layers.as_mut().unwrap().layers[1].visible = false;
    assert!(!draft.graph().unwrap().nodes.iter().any(|node| matches!(
        node.op,
        Op::Wave { .. } | Op::Weather { .. } | Op::Displacement { .. }
    )));
}

#[test]
fn attached_effect_imports_validation_limits_and_pruning_preserve_other_assets() {
    use wisp_app::layers::{ImportTarget, LayerEffect, LayerTarget, MAX_EFFECTS_PER_LAYER};
    let temp = tempfile::tempdir().unwrap();
    let mut draft = draft(temp.path());
    let stack = draft.layers.as_mut().unwrap();
    let original = serde_json::to_value(&stack.layers).unwrap();
    assert!(stack.add_effect(8, Effect::Hair).is_err());
    assert!(stack.add_effect(1, Effect::None).is_err());
    assert_eq!(original, serde_json::to_value(&stack.layers).unwrap());
    for _ in 0..MAX_EFFECTS_PER_LAYER {
        stack.add_effect(1, Effect::Hair).unwrap();
    }
    assert!(stack.add_effect(1, Effect::Wave).is_err());
    stack
        .import(
            &temp.path().join("source.png"),
            ImportTarget::EffectMask(1, 0),
        )
        .unwrap();
    let mask = stack.layers[1].effects[0]
        .mask
        .as_ref()
        .unwrap()
        .asset
        .clone();
    let other = stack.layers[1].effects[1]
        .mask
        .as_ref()
        .unwrap()
        .asset
        .clone();
    stack.layers[1].effects.remove(0);
    stack.prune();
    assert!(!stack.assets.contains_key(&mask));
    assert!(stack.assets.contains_key(&other));
    assert!(
        stack
            .blank_mask_for(LayerTarget::Effect(1, 20), true)
            .is_err()
    );
    assert!(
        stack
            .import(
                &temp.path().join("source.png"),
                ImportTarget::EffectDisplacement(1, 20)
            )
            .is_err()
    );
    stack.layers[1].effects[0].settings.speed = f32::NAN;
    assert!(stack.validate().is_err());
    stack.layers[1].effects[0].settings.speed = 1.;
    stack.layers[1].effects[0].mask.as_mut().unwrap().asset = "missing".into();
    assert!(stack.validate().is_err());
    stack.layers[1].effects[0].mask.as_mut().unwrap().asset = other;
    stack.layers[1]
        .effects
        .resize(MAX_EFFECTS_PER_LAYER + 1, LayerEffect::new(Effect::Wave));
    assert!(stack.validate().is_err());
}

#[test]
fn upgrading_legacy_effects_preserves_settings_and_separates_shared_alpha_masks() {
    use wisp_app::layers::LayerTarget;
    let temp = tempfile::tempdir().unwrap();
    let mut draft = draft(temp.path());
    let stack = draft.layers.as_mut().unwrap();
    stack.version = 2;
    stack.layers[1].preset(Effect::Hair);
    stack.layers[1].amplitude = 0.07;
    stack.layers[1].mask.as_mut().unwrap().composition = true;
    let image_mask = stack.layers[1].mask.as_ref().unwrap().asset.clone();
    let pixels = stack.assets[&image_mask].bytes.clone();
    stack.upgrade_effects().unwrap();
    assert_eq!(stack.layers[1].effect, Effect::None);
    assert_eq!(stack.layers[1].effects[0].settings.effect, Effect::Hair);
    assert_eq!(stack.layers[1].effects[0].settings.amplitude, 0.07);
    let effect_mask = stack.layers[1].effects[0]
        .mask
        .as_ref()
        .unwrap()
        .asset
        .clone();
    assert_ne!(image_mask, effect_mask);
    assert_eq!(stack.assets[&effect_mask].bytes, pixels);
    stack
        .paint_target(LayerTarget::Effect(1, 0), [0.5, 0.3], [0.5, 0.3], 0.1, 0)
        .unwrap();
    assert_eq!(stack.assets[&image_mask].bytes, pixels);
    stack.upgrade_effects().unwrap();
    assert_eq!(stack.layers[1].effects.len(), 1);
    draft.compile().unwrap();
}
