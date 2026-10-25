use std::fs;
use wisp_app::designer::Draft;
#[cfg(unix)]
use wisp_app::library::Library;
use wisp_format::{Package, PackageLimits};
use wisp_host::Config;

#[test]
fn source_image_reads_preserve_bytes_without_the_runtime_size_limit() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("source.png");
    let bytes = vec![42; 32 * 1024 * 1024 + 1];
    fs::write(&path, &bytes).unwrap();
    assert_eq!(wisp_compiler::read_source_image(&path).unwrap(), bytes);
    fs::write(&path, []).unwrap();
    assert!(wisp_compiler::read_source_image(&path).unwrap().is_empty());
    assert!(wisp_compiler::read_source_image(directory.path()).is_err());
    assert!(wisp_compiler::read_source_image(&path.with_extension("missing")).is_err());
}

#[test]
fn package_and_assignment_loaders_preserve_file_limits_and_context() {
    let directory = tempfile::tempdir().unwrap();
    let source = directory.path().join("source.png");
    fs::write(&source, wisp_runtime::EXAMPLE_IMAGE).unwrap();
    let package = Draft::from_image(&source)
        .unwrap()
        .compile()
        .unwrap()
        .package;
    let bytes = package.to_bytes(&Default::default()).unwrap();
    let path = directory.path().join("wallpaper.wisp");
    fs::write(&path, &bytes).unwrap();
    let mut limits = PackageLimits {
        max_archive_bytes: bytes.len() as u64,
        ..Default::default()
    };
    assert_eq!(
        Package::load(&path, &limits).unwrap().manifest().id,
        package.manifest().id
    );
    limits.max_archive_bytes -= 1;
    let error = format!("{:#}", Package::load(&path, &limits).err().unwrap());
    assert!(error.contains("byte input limit"), "{error}");
    assert!(error.contains("wallpaper.wisp"), "{error}");
    assert!(Package::load(directory.path(), &limits).is_err());

    let config = directory.path().join("assignments.json");
    fs::write(&config, br#"{"default":{"package":"wallpaper.wisp"}}"#).unwrap();
    let loaded = Config::load(&config).unwrap();
    assert_eq!(loaded.default.unwrap().package, path);
    let mut exact_limit = b"{}".to_vec();
    exact_limit.resize(1024 * 1024, b' ');
    fs::write(&config, &exact_limit).unwrap();
    Config::load(&config).unwrap();
    exact_limit.push(b' ');
    fs::write(&config, &exact_limit).unwrap();
    let error = format!("{:#}", Config::load(&config).unwrap_err());
    assert!(error.contains("maximum 1 MiB"), "{error}");
    assert!(error.contains("byte input limit"), "{error}");
    assert!(Config::load(directory.path()).is_err());
}

#[cfg(unix)]
#[test]
fn special_inputs_are_rejected_by_every_loader_before_deadline() {
    use std::{
        process::Command,
        thread,
        time::{Duration, Instant},
    };
    let directory = tempfile::tempdir().unwrap();
    for (scenario, filename) in [
        ("package", "input.wisp"),
        ("config", "assignments.json"),
        ("library", "library.json"),
        ("settings", "settings.json"),
        ("graph", "project.json"),
        ("designer", "designer.json"),
        ("compiler", "project.json"),
        ("source_image", "source.png"),
        ("designer_image", "source.png"),
    ] {
        let root = directory.path().join(scenario);
        fs::create_dir(&root).unwrap();
        let input = root.join(filename);
        assert!(
            Command::new("mkfifo")
                .arg(&input)
                .status()
                .unwrap()
                .success()
        );
        let mut child = Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "fifo_loader_child", "--nocapture"])
            .env("WISP_LOADER_FIFO_ROOT", &root)
            .env("WISP_LOADER_FIFO_SCENARIO", scenario)
            .spawn()
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            if let Some(status) = child.try_wait().unwrap() {
                assert!(status.success(), "{scenario} loader child failed");
                break;
            }
            if Instant::now() >= deadline {
                child.kill().unwrap();
                child.wait().unwrap();
                panic!("{scenario} loader blocked on FIFO");
            }
            thread::sleep(Duration::from_millis(10));
        }
        fs::remove_file(&input).unwrap();
    }
}

#[cfg(unix)]
#[test]
fn fifo_loader_child() {
    let Some(root) = std::env::var_os("WISP_LOADER_FIFO_ROOT") else {
        return;
    };
    let root = std::path::Path::new(&root);
    let scenario = std::env::var("WISP_LOADER_FIFO_SCENARIO").unwrap();
    let result: anyhow::Result<()> = match scenario.as_str() {
        "package" => Package::load(root.join("input.wisp"), &Default::default()).map(|_| ()),
        "config" => Config::load(&root.join("assignments.json")).map(|_| ()),
        "library" => Library::open(Some(root.to_owned()))
            .unwrap()
            .scan()
            .map(|_| ()),
        "settings" => Library::open(Some(root.to_owned()))
            .unwrap()
            .settings()
            .map(|_| ()),
        "graph" => wisp_app::graph::Document::open(root).map(|_| ()),
        "designer" => Draft::open_project(root).map(|_| ()),
        "compiler" => wisp_compiler::compile_project(root, &Default::default()).map(|_| ()),
        "source_image" => wisp_compiler::read_source_image(&root.join("source.png")).map(|_| ()),
        "designer_image" => Draft::from_image(&root.join("source.png")).map(|_| ()),
        _ => panic!("unknown FIFO scenario"),
    };
    let error = format!("{:#}", result.unwrap_err());
    assert!(error.contains("regular file"), "{scenario}: {error}");
}
