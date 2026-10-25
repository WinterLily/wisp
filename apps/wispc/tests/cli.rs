use serde_json::{Value, json};
use std::{
    path::{Path, PathBuf},
    process::{Command, Output},
    sync::atomic::{AtomicUsize, Ordering},
};
static NEXT: AtomicUsize = AtomicUsize::new(0);
struct Temp(PathBuf);
impl Temp {
    fn new() -> Self {
        let dir = std::env::temp_dir().join(format!(
            "wispc-test-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&dir).unwrap();
        Self(dir)
    }
}
impl Drop for Temp {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
fn example() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/graph-fused")
}
fn run(project: &Path, args: &[&std::ffi::OsStr]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_wispc"))
        .arg(project)
        .args(args)
        .output()
        .unwrap()
}
fn copy_project(temp: &Temp) {
    for file in ["project.json", "graph.json"] {
        std::fs::copy(example().join(file), temp.0.join(file)).unwrap();
    }
    std::fs::create_dir(temp.0.join("assets")).unwrap();
    std::fs::copy(
        example().join("assets/background.png"),
        temp.0.join("assets/background.png"),
    )
    .unwrap();
}
fn edit_asset(temp: &Temp, path: &str) {
    let mut project: Value =
        serde_json::from_slice(&std::fs::read(temp.0.join("project.json")).unwrap()).unwrap();
    project["assets"]["background"]["path"] = json!(path);
    std::fs::write(
        temp.0.join("project.json"),
        serde_json::to_vec(&project).unwrap(),
    )
    .unwrap();
}
#[test]
fn cli_compiles_checks_and_preserves_existing_files() {
    let temp = Temp::new();
    let output = temp.0.join("wallpaper.wisp");
    let result = run(&example(), &["--check".as_ref()]);
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert!(String::from_utf8_lossy(&result.stdout).contains("1 passes"));
    assert!(!run(&example(), &[]).status.success());
    assert!(
        !run(
            &example(),
            &["--check".as_ref(), "--output".as_ref(), output.as_os_str()]
        )
        .status
        .success()
    );
    let result = run(&example(), &["--output".as_ref(), output.as_os_str()]);
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let bytes = std::fs::read(&output).unwrap();
    let package =
        wisp_format::Package::from_bytes(&bytes, &wisp_format::PackageLimits::default()).unwrap();
    assert_eq!(package.program().passes.len(), 1);
    assert!(
        !run(&example(), &["--output".as_ref(), output.as_os_str()])
            .status
            .success()
    );
    assert_eq!(bytes, std::fs::read(&output).unwrap());
    assert!(!String::from_utf8_lossy(&bytes).contains("graph.json"));
}
#[test]
fn invalid_projects_never_create_output_and_inputs_are_bounded() {
    let temp = Temp::new();
    copy_project(&temp);
    let output = temp.0.join("out.wisp");
    let valid = run(&temp.0, &["--check".as_ref()]);
    assert!(valid.status.success(), "{:?}", valid);
    for (path, diagnostic) in [
        ("assets/../../outside.png", "unsafe project path"),
        ("assets/..\\outside.png", "unsafe project path"),
        ("assets/C:/outside.png", "unsafe project path"),
        ("/outside.png", "image must be under project assets"),
    ] {
        edit_asset(&temp, path);
        let result = run(&temp.0, &["--output".as_ref(), output.as_os_str()]);
        assert!(!result.status.success());
        assert!(
            String::from_utf8_lossy(&result.stderr).contains(diagnostic),
            "{path}: {:?}",
            result
        );
        assert!(!output.exists());
    }
    edit_asset(&temp, "assets/background.png");
    std::fs::write(temp.0.join("graph.json"), vec![b' '; 1024 * 1024 + 1]).unwrap();
    let result = run(&temp.0, &["--check".as_ref()]);
    assert!(!result.status.success());
    assert!(String::from_utf8_lossy(&result.stderr).contains("byte limit"));
}
#[cfg(unix)]
#[test]
fn escaping_symlink_is_rejected() {
    let temp = Temp::new();
    copy_project(&temp);
    std::fs::remove_file(temp.0.join("assets/background.png")).unwrap();
    std::os::unix::fs::symlink(
        example().join("assets/background.png"),
        temp.0.join("assets/background.png"),
    )
    .unwrap();
    let result = run(&temp.0, &["--check".as_ref()]);
    assert!(!result.status.success());
    assert!(String::from_utf8_lossy(&result.stderr).contains("escapes project directory"));
}
