use std::{
    path::Path,
    time::{Duration, Instant},
};
use wisp_app::{daemon, library::Library};
use wisp_host::{Assignment, status::DaemonStatus};

fn wait_status(library: &Library, predicate: impl Fn(&DaemonStatus) -> bool) -> DaemonStatus {
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        let status = daemon::status(library).unwrap();
        if let Some(status) = status.as_ref().filter(|status| predicate(status)) {
            return status.clone();
        }
        assert!(
            Instant::now() < deadline,
            "daemon status did not converge: {status:?}"
        );
        std::thread::sleep(Duration::from_millis(30));
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
    let mut child = daemon::OwnedDaemon::start_with_executable(&library, &binary).unwrap();
    let initial = wait_status(&library, |status| status.ready);
    assert!(
        initial.outputs.is_empty(),
        "unassigned daemon must initialize without surfaces"
    );
    daemon::start(&library).unwrap();
    let output = std::process::Command::new(&binary)
        .args(["--list-outputs", "--json"])
        .output()
        .unwrap();
    assert!(output.status.success());
    let monitors: Vec<wisp_host::MonitorInfo> = serde_json::from_slice(&output.stdout).unwrap();
    assert!(!monitors.is_empty());
    let assignment = Assignment::new(library.package_path(&item.record.key).unwrap(), 30);
    library
        .assign(Some(&monitors[0].id), Some(assignment.clone()))
        .unwrap();
    let assigned = wait_status(&library, |status| {
        status.accepted_generation > initial.accepted_generation
            && status.ready
            && status
                .outputs
                .iter()
                .any(|output| output.id == monitors[0].id && output.presented >= 2)
    });
    let output = assigned
        .outputs
        .iter()
        .find(|output| output.id == monitors[0].id)
        .unwrap();
    assert_eq!(output.assignment, assignment);
    let output_identity = output.instance_id;
    let mut audio_config = library.config().unwrap();
    audio_config.audio.gain = 0.5;
    library.save_config(&audio_config).unwrap();
    let audio = wait_status(&library, |status| {
        status.accepted_generation > assigned.accepted_generation && status.ready
    });
    assert_eq!(audio.config.audio.gain, 0.5);
    assert_eq!(audio.outputs[0].instance_id, output_identity);
    assert!(audio.outputs[0].presented >= output.presented);

    std::fs::write(library.config_path(), b"{invalid").unwrap();
    let rejected = wait_status(&library, |status| {
        status.attempted_generation > audio.attempted_generation && status.rejection.is_some()
    });
    assert_eq!(rejected.accepted_generation, audio.accepted_generation);
    assert_eq!(rejected.config.outputs, audio.config.outputs);
    assert_eq!(rejected.outputs[0].instance_id, output_identity);
    assert!(rejected.outputs[0].presented >= audio.outputs[0].presented);
    library.save_config(&audio_config).unwrap();
    let recovered = wait_status(&library, |status| {
        status.accepted_generation > rejected.attempted_generation && status.rejection.is_none()
    });
    assert_eq!(recovered.outputs[0].instance_id, output_identity);
    library.assign(Some(&monitors[0].id), None).unwrap();
    wait_status(&library, |status| {
        status.accepted_generation > recovered.accepted_generation
            && status.ready
            && status.outputs.is_empty()
    });
    let stop_path = child.stop_path().to_owned();
    assert!(child.stop().unwrap().success());
    assert!(!daemon::running(&library).unwrap());
    assert!(!daemon::ready(&library).unwrap());
    assert!(!stop_path.exists());
    let stopped = DaemonStatus::read(&library.root.join("daemon.status.json")).unwrap();
    assert!(!stopped.running);
    assert!(!stopped.ready);
}
