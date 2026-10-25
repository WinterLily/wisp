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
