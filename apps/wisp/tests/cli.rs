use std::{path::Path, process::Command};
#[test]
fn headless_import_check_and_reopen_need_no_gpu_or_display() {
    let temporary = tempfile::tempdir().unwrap();
    let project = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/graph-fused");
    for import in [true, false] {
        let mut command = Command::new(env!("CARGO_BIN_EXE_wisp"));
        command
            .arg("--data-dir")
            .arg(temporary.path())
            .arg("--check")
            .env_remove("WAYLAND_DISPLAY")
            .env_remove("DISPLAY")
            .env("WGPU_BACKEND", "unavailable");
        if import {
            command.arg("--import").arg(&project);
        }
        let output = command.output().unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(String::from_utf8_lossy(&output.stdout).contains("1 wallpapers"));
    }
    assert!(
        !Command::new(env!("CARGO_BIN_EXE_wisp"))
            .args(["--check", "--frames", "1"])
            .status()
            .unwrap()
            .success()
    );
}

#[test]
fn image_creation_and_image_import_are_cpu_checked_without_installing_a_draft() {
    let temp = tempfile::tempdir().unwrap();
    let source =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/shader-basic/background.png");
    for flag in ["--create", "--import"] {
        let output = Command::new(env!("CARGO_BIN_EXE_wisp"))
            .arg("--data-dir")
            .arg(temp.path())
            .arg(flag)
            .arg(&source)
            .arg("--check")
            .env_remove("WAYLAND_DISPLAY")
            .env_remove("DISPLAY")
            .env("WGPU_BACKEND", "unavailable")
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(stdout.contains("Valid Easy Designer wallpaper: 1 passes"));
        assert!(stdout.contains("0 wallpapers"));
    }
}

#[test]
fn visual_graph_check_compiles_without_gpu_and_rejects_conflicting_modes() {
    let temp = tempfile::tempdir().unwrap();
    let project = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/graph-blur");
    let output = Command::new(env!("CARGO_BIN_EXE_wisp"))
        .arg("--data-dir")
        .arg(temp.path())
        .arg("--graph")
        .arg(&project)
        .arg("--check")
        .env_remove("WAYLAND_DISPLAY")
        .env_remove("DISPLAY")
        .env("WGPU_BACKEND", "unavailable")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(String::from_utf8_lossy(&output.stdout).contains("Valid graph wallpaper: 3 passes"));
    assert!(String::from_utf8_lossy(&output.stdout).contains("0 wallpapers"));
    for other in ["--create", "--import"] {
        assert!(
            !Command::new(env!("CARGO_BIN_EXE_wisp"))
                .arg("--graph")
                .arg(&project)
                .arg(other)
                .arg(&project)
                .arg("--check")
                .output()
                .unwrap()
                .status
                .success()
        );
    }
}

#[test]
#[ignore = "requires a disposable desktop with a working graphics backend"]
fn library_export_commands_preserve_existing_files_and_recover_to_a_new_destination() {
    use std::{
        process::Stdio,
        time::{Duration, Instant},
    };
    let temporary = tempfile::tempdir().unwrap();
    let library = wisp_app::library::Library::open(Some(temporary.path().join("library"))).unwrap();
    let project = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/graph-fused");
    let item = library.import(&project).unwrap();
    let existing = temporary.path().join("existing.wisp");
    std::fs::write(&existing, b"preserve the original destination").unwrap();
    let exported = temporary.path().join("exported.wisp");
    let missing_parent = temporary.path().join("missing/export.wisp");
    let script = temporary.path().join("commands.json");
    std::fs::write(&script, serde_json::to_vec(&serde_json::json!([
        {"command":"library_export", "path":existing},
        {"command":"assert", "state":{"busy":false, "status_contains":"export requires a new destination"}},
        {"command":"library_export", "path":missing_parent},
        {"command":"assert", "state":{"busy":false, "status_contains":"export requires a new destination"}},
        {"command":"library_export", "path":exported},
        {"command":"assert", "state":{"busy":false, "status_contains":"Wallpaper exported"}},
        {"command":"finish"}
    ])).unwrap()).unwrap();
    let mut child = Command::new(env!("CARGO_BIN_EXE_wisp"))
        .arg("--data-dir")
        .arg(&library.root)
        .arg("--audit-commands")
        .arg(&script)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(45);
    while child.try_wait().unwrap().is_none() {
        if Instant::now() >= deadline {
            let _ = child.kill();
            let output = child.wait_with_output().unwrap();
            panic!(
                "library export audit timed out: {}",
                String::from_utf8_lossy(&output.stderr)
            );
        }
        std::thread::sleep(Duration::from_millis(25));
    }
    let output = child.wait_with_output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert_eq!(stdout.matches("AUDIT_COMPLETE ").count(), 1, "{stdout}");
    assert!(!String::from_utf8_lossy(&output.stderr).contains("AUDIT_FAILED"));
    assert_eq!(
        std::fs::read(existing).unwrap(),
        b"preserve the original destination"
    );
    assert!(!missing_parent.exists());
    assert_eq!(
        std::fs::read(exported).unwrap(),
        std::fs::read(library.package_path(&item.record.key).unwrap()).unwrap()
    );
}
