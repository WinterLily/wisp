use std::{
    path::Path,
    process::{Child, Command, Stdio},
    time::{Duration, Instant},
};
use wisp_app::{daemon, library::Library};
use wisp_host::Assignment;
struct Process(Child);
impl Drop for Process {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
#[test]
#[ignore = "requires a disposable Niri/Hyprland or Windows desktop and built wispd"]
fn saved_gui_assignments_reload_in_an_independent_daemon() {
    let temporary = tempfile::tempdir().unwrap();
    let library = Library::open(Some(temporary.path().to_owned())).unwrap();
    let item = library
        .import(&Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/graph-fused"))
        .unwrap();
    library.save_config(&Default::default()).unwrap();
    let binary = Path::new(env!("CARGO_BIN_EXE_wisp")).with_file_name(if cfg!(windows) {
        "wispd.exe"
    } else {
        "wispd"
    });
    let mut child = Process(
        Command::new(&binary)
            .arg("--config")
            .arg(library.config_path())
            .arg("--lock-file")
            .arg(library.root.join("daemon.lock"))
            .args(["--frames", "8"])
            .stdin(Stdio::null())
            .spawn()
            .unwrap(),
    );
    let deadline = Instant::now() + Duration::from_secs(15);
    while !daemon::running(&library).unwrap() {
        assert!(Instant::now() < deadline, "daemon did not acquire lock");
        assert!(child.0.try_wait().unwrap().is_none());
        std::thread::sleep(Duration::from_millis(20));
    }
    // Start is idempotent and discovery uses the exact IDs that assignment consumes.
    daemon::start(&library).unwrap();
    let output = Command::new(&binary)
        .args(["--list-outputs", "--json"])
        .output()
        .unwrap();
    assert!(output.status.success());
    let monitors: Vec<wisp_host::MonitorInfo> = serde_json::from_slice(&output.stdout).unwrap();
    assert!(!monitors.is_empty());
    library
        .assign(
            Some(&monitors[0].id),
            Some(Assignment::new(
                library.package_path(&item.record.key).unwrap(),
                30,
            )),
        )
        .unwrap();
    loop {
        if let Some(status) = child.0.try_wait().unwrap() {
            assert!(status.success());
            break;
        }
        assert!(
            Instant::now() < deadline,
            "daemon did not reload and render saved GUI assignment"
        );
        std::thread::sleep(Duration::from_millis(50));
    }
    assert!(!daemon::running(&library).unwrap());
}
