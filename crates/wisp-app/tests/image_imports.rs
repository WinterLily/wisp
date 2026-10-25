use std::{fs, io::Cursor};
use wisp_app::{designer::Draft, graph::Document, layers::Effect};
use wisp_runtime::Wallpaper;

#[test]
fn large_images_import_save_reopen_and_export_without_changing_sources() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("large.png");
    let pixels = image::RgbaImage::from_pixel(5000, 20, image::Rgba([40, 100, 180, 170]));
    let mut encoded = Cursor::new(Vec::new());
    pixels
        .write_to(&mut encoded, image::ImageFormat::Png)
        .unwrap();
    let mut bytes = encoded.into_inner();
    // Padding after the PNG stream exercises file-byte limits independently of
    // decoded memory, keeping this regression inexpensive on CI.
    bytes.resize(33 * 1024 * 1024, 0);
    fs::write(&source, &bytes).unwrap();

    let mut draft = Draft::from_image(&source).unwrap();
    assert_eq!(draft.dimensions, (5000, 20));
    let saved = temp.path().join("easy");
    draft.save_project(&saved).unwrap();
    let reopened = Draft::open_project(&saved).unwrap();
    assert_eq!(reopened.dimensions, (5000, 20));
    assert_eq!(reopened.assets()["background"].bytes, bytes);
    let compiled = reopened.compile().unwrap();
    assert_eq!(compiled.report.resized_images, 1);
    let runtime = Wallpaper::from_package(&compiled.package).unwrap();
    assert_eq!(runtime.images()[0].dimensions(), (4096, 16));
    let copy = image::load_from_memory(compiled.package.image_bytes())
        .unwrap()
        .into_rgba8();
    assert_eq!(copy.get_pixel(100, 8).0, [40, 100, 180, 170]);
    assert_eq!(
        compiled.package.to_bytes(&Default::default()).unwrap(),
        wisp_compiler::compile_project(&saved, &Default::default())
            .unwrap()
            .package
            .to_bytes(&Default::default())
            .unwrap()
    );
    let exported = temp.path().join("large.wisp");
    reopened.export(&exported).unwrap();
    let package = wisp_format::Package::load(&exported, &Default::default()).unwrap();
    Wallpaper::from_package(&package).unwrap();

    let stack = draft.enable_layers();
    stack.import(&source, None).unwrap();
    stack.import(&source, Some((1, false))).unwrap();
    stack.import(&source, Some((1, true))).unwrap();
    stack.layers[1].preset(Effect::Displacement);
    let mask = &stack.layers[1].mask.as_ref().unwrap().asset;
    let decoded = wisp_compiler::decode_source_image(&stack.assets[mask].bytes).unwrap();
    assert_eq!((decoded.width(), decoded.height()), (5000, 20));
    let layered = temp.path().join("layers");
    draft.save_project(&layered).unwrap();
    let reopened = Draft::open_project(&layered).unwrap();
    assert_eq!(reopened.assets()["background"].bytes, bytes);
    Wallpaper::from_package(&reopened.compile().unwrap().package).unwrap();

    let mut graph = Document::from_easy(&draft).unwrap();
    let imported = graph.import_image(&source).unwrap();
    let saved = temp.path().join("graph");
    graph.save(&saved).unwrap();
    fs::remove_file(&source).unwrap();
    let reopened = Document::open(&saved).unwrap();
    assert_eq!(reopened.assets[&imported].bytes, bytes);
    Wallpaper::from_package(&reopened.compile().unwrap().package).unwrap();
}
