use std::fs;
use wisp_app::{
    designer::{Draft, Preset},
    library::Library,
};
use wisp_format::{Package, PackageLimits, ParameterValue};
use wisp_runtime::{EXAMPLE_IMAGE, Wallpaper};

fn draft(root: &std::path::Path) -> Draft {
    let path = root.join("source.PNG");
    fs::write(&path, EXAMPLE_IMAGE).unwrap();
    Draft::from_image(&path).unwrap()
}
#[test]
fn presets_compile_to_portable_graphs_with_only_relevant_controls() {
    let temp = tempfile::tempdir().unwrap();
    let mut draft = draft(temp.path());
    for preset in Preset::ALL {
        draft.design.preset(preset);
        let compiled = draft.compile().unwrap();
        assert_eq!(
            compiled.report.passes,
            if preset == Preset::Dreamy { 3 } else { 1 }
        );
        assert_eq!(compiled.report.image_assets, 1);
        let parameters = &compiled.package.program().parameters;
        assert_eq!(
            parameters.iter().any(|p| p.id == "speed"),
            draft.design.pulse
        );
        assert_eq!(
            parameters.iter().any(|p| p.id == "radius"),
            draft.design.blur
        );
        assert!(!parameters.iter().any(|p| p.id == "parallax"));
        assert!(
            !draft
                .graph()
                .unwrap()
                .nodes
                .iter()
                .any(|node| { matches!(node.op, wisp_ir::Op::Parallax { .. }) })
        );
        let bytes = compiled
            .package
            .to_bytes(&PackageLimits::default())
            .unwrap();
        let package = Package::from_bytes(&bytes, &PackageLimits::default()).unwrap();
        Wallpaper::from_package(&package).unwrap();
    }
}
#[test]
fn project_reopens_without_original_and_compiles_identically_with_wispc() {
    let temp = tempfile::tempdir().unwrap();
    let mut draft = draft(temp.path());
    draft.design.preset(Preset::Dreamy);
    draft.design.name = "Saved dream".into();
    draft.design.values.insert("radius".into(), 4.);
    let destination = temp.path().join("project");
    draft.save_project(&destination).unwrap();
    fs::remove_file(temp.path().join("source.PNG")).unwrap();
    let reopened = Draft::open_project(&destination).unwrap();
    let compiled = reopened
        .compile()
        .unwrap()
        .package
        .to_bytes(&PackageLimits::default())
        .unwrap();
    let standard = wisp_compiler::compile_project(&destination, &Default::default())
        .unwrap()
        .package
        .to_bytes(&PackageLimits::default())
        .unwrap();
    assert_eq!(compiled, standard);
    assert_eq!(reopened.design.values["radius"], 4.);
    assert!(draft.save_project(&destination).is_err());
    assert_eq!(
        Draft::open_project(&destination).unwrap().design.name,
        "Saved dream"
    );
    let mut graph: serde_json::Value =
        serde_json::from_slice(&fs::read(destination.join("graph.json")).unwrap()).unwrap();
    graph["parameters"][0]["default"]["value"] = 0.1.into();
    fs::write(
        destination.join("graph.json"),
        serde_json::to_vec(&graph).unwrap(),
    )
    .unwrap();
    assert!(Draft::open_project(&destination).is_err());
}
#[test]
fn export_and_library_install_keep_author_defaults_and_protect_existing_files() {
    let temp = tempfile::tempdir().unwrap();
    let mut draft = draft(temp.path());
    draft.design.values.insert("brightness".into(), 0.65);
    let package = draft.compile().unwrap().package;
    let library = Library::open(Some(temp.path().join("library"))).unwrap();
    let item = library.install(&package, "Easy Designer").unwrap();
    assert_eq!(item.record.source, "Easy Designer");
    assert_eq!(
        item.parameters
            .iter()
            .find(|p| p.id == "brightness")
            .unwrap()
            .default,
        ParameterValue::Float(0.65)
    );
    assert_eq!(
        library
            .install(&package, "Easy Designer")
            .unwrap()
            .record
            .key,
        item.record.key
    );
    let output = temp.path().join("created.wisp");
    draft.export(&output).unwrap();
    let original = fs::read(&output).unwrap();
    assert_eq!(library.import(&output).unwrap().record.key, item.record.key);
    draft.design.values.insert("brightness".into(), 0.2);
    assert!(draft.export(&output).is_err());
    assert_eq!(fs::read(output).unwrap(), original);
}
#[test]
fn rejects_bad_images_settings_and_escaping_project_assets() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("broken.png");
    fs::write(&path, b"bad image").unwrap();
    assert!(Draft::from_image(&path).is_err());
    assert!(Draft::from_image(temp.path()).is_err());
    let mut draft = draft(temp.path());
    let destination = temp.path().join("project");
    draft.save_project(&destination).unwrap();
    let mut metadata: serde_json::Value =
        serde_json::from_slice(&fs::read(destination.join("project.json")).unwrap()).unwrap();
    metadata["assets"]["background"]["path"] = "../source.PNG".into();
    fs::write(
        destination.join("project.json"),
        serde_json::to_vec(&metadata).unwrap(),
    )
    .unwrap();
    assert!(Draft::open_project(&destination).is_err());
    for bad in [-1., 5., f32::NAN, f32::INFINITY] {
        draft.design.values.insert("brightness".into(), bad);
        assert!(draft.compile().is_err());
    }
    draft.design.values.insert("brightness".into(), 1.);
    draft.design.version = 2;
    assert!(draft.compile().is_err());
    draft.design.version = 1;
    draft.design.name.clear();
    assert!(draft.compile().is_err());
    assert!(!temp.path().join("invalid-project").exists());
}

#[test]
fn legacy_projects_remove_parallax_without_discarding_external_graph_edits() {
    use serde_json::json;
    use wisp_format::Parameter;
    use wisp_ir::{Node, Op};

    let temp = tempfile::tempdir().unwrap();
    for layered in [false, true] {
        for enabled in [false, true] {
            let mut draft = draft(temp.path());
            if layered {
                draft.enable_layers();
            }
            let destination = temp.path().join(format!("legacy-{layered}-{enabled}"));
            draft.save_project(&destination).unwrap();
            let mut settings = serde_json::to_value(&draft.design).unwrap();
            settings["parallax"] = enabled.into();
            settings["values"]["parallax"] = 0.12.into();
            fs::write(
                destination.join("designer.json"),
                serde_json::to_vec(&settings).unwrap(),
            )
            .unwrap();

            let mut graph = draft.graph().unwrap();
            let control_count = graph.parameters.len();
            graph.parameters.insert(
                0,
                Parameter {
                    id: "parallax".into(),
                    default: ParameterValue::Float(0.12),
                    min: Some(0.),
                    max: Some(0.15_f32 as f64),
                },
            );
            if enabled {
                for node in &mut graph.nodes {
                    match &mut node.op {
                        Op::Image { uv, .. } | Op::Transform { uv, .. } if uv == "uv" => {
                            *uv = "moving_uv".into();
                        }
                        _ => {}
                    }
                }
                graph.nodes.insert(
                    1 + control_count,
                    Node {
                        id: "moving_uv".into(),
                        op: Op::Parallax {
                            uv: "uv".into(),
                            strength: "parallax".into(),
                        },
                    },
                );
            }
            graph.nodes.insert(
                1,
                Node {
                    id: "parallax".into(),
                    op: Op::Parameter {
                        parameter: "parallax".into(),
                    },
                },
            );
            fs::write(
                destination.join("graph.json"),
                serde_json::to_vec(&graph).unwrap(),
            )
            .unwrap();

            let reopened = Draft::open_project(&destination).unwrap();
            assert_eq!(
                serde_json::to_value(reopened.graph().unwrap()).unwrap(),
                serde_json::to_value(draft.graph().unwrap()).unwrap()
            );
            assert!(!reopened.design.values.contains_key("parallax"));
            let migrated = temp.path().join(format!("migrated-{layered}-{enabled}"));
            reopened.save_project(&migrated).unwrap();
            let saved: serde_json::Value =
                serde_json::from_slice(&fs::read(migrated.join("designer.json")).unwrap()).unwrap();
            assert!(saved.get("parallax").is_none());
            Draft::open_project(&migrated).unwrap();

            for strength in [json!(-0.1), json!(0.2), json!("invalid"), json!(null)] {
                settings["values"]["parallax"] = strength;
                fs::write(
                    destination.join("designer.json"),
                    serde_json::to_vec(&settings).unwrap(),
                )
                .unwrap();
                assert!(Draft::open_project(&destination).is_err());
            }
            settings["values"]["parallax"] = 0.12.into();
            fs::write(
                destination.join("designer.json"),
                serde_json::to_vec(&settings).unwrap(),
            )
            .unwrap();
            graph
                .nodes
                .iter_mut()
                .find(|node| node.id == "output")
                .unwrap()
                .op = Op::Output {
                color: "image".into(),
            };
            fs::write(
                destination.join("graph.json"),
                serde_json::to_vec(&graph).unwrap(),
            )
            .unwrap();
            assert!(Draft::open_project(&destination).is_err());
        }
    }
}
