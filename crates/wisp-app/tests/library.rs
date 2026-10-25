use std::{collections::BTreeMap, fs, path::Path};
use wisp_app::library::{Library, Settings};
use wisp_format::ParameterValue;
use wisp_host::Assignment;

fn project() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/graph-fused")
}
#[test]
fn imports_are_deduplicated_portable_and_persistent() {
    let temporary = tempfile::tempdir().unwrap();
    let library = Library::open(Some(temporary.path().join("library"))).unwrap();
    let item = library.import(&project()).unwrap();
    assert_eq!(item.passes, 1);
    assert!(item.thumbnail.is_some());
    let exported = temporary.path().join("exported.wisp");
    library.export(&item.record.key, &exported).unwrap();
    let duplicate = library.import(&exported).unwrap();
    assert_eq!(item.record.key, duplicate.record.key);
    assert!(library.export(&item.record.key, &exported).is_err());
    fs::remove_file(exported).unwrap();
    let reopened = Library::open(Some(library.root.clone())).unwrap();
    let (items, errors) = reopened.scan().unwrap();
    assert!(errors.is_empty());
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].record.key, item.record.key);
    assert_eq!(items[0].record.source, "Local project");
    assert!(reopened.package_path("../../escape").is_err());
}
#[test]
fn invalid_imports_and_failed_assignments_preserve_previous_state() {
    let temporary = tempfile::tempdir().unwrap();
    let library = Library::open(Some(temporary.path().join("library"))).unwrap();
    let item = library.import(&project()).unwrap();
    let assignment = Assignment::new(library.package_path(&item.record.key).unwrap(), 30);
    library.assign(None, Some(assignment.clone())).unwrap();
    let before = fs::read(library.config_path()).unwrap();
    let catalog = fs::read(library.root.join("library.json")).unwrap();
    let bad = temporary.path().join("bad.wisp");
    fs::write(&bad, b"invalid ZIP").unwrap();
    assert!(library.import(&bad).is_err());
    assert_eq!(
        catalog,
        fs::read(library.root.join("library.json")).unwrap()
    );
    let mut invalid = assignment;
    invalid
        .parameters
        .insert("brightness".into(), ParameterValue::Float(-100.0));
    assert!(library.assign(Some("DP-1"), Some(invalid)).is_err());
    assert_eq!(before, fs::read(library.config_path()).unwrap());
    // A changed installed object is reported and never silently trusted.
    fs::write(library.package_path(&item.record.key).unwrap(), b"changed").unwrap();
    let (items, errors) = library.scan().unwrap();
    assert!(items.is_empty());
    assert_eq!(errors.len(), 1);
    library.assign(None, None).unwrap();
    assert!(library.config().unwrap().default.is_none());
}
#[test]
fn per_monitor_parameters_clear_and_reassign_round_trip() {
    let temporary = tempfile::tempdir().unwrap();
    let library = Library::open(Some(temporary.path().to_owned())).unwrap();
    let item = library.import(&project()).unwrap();
    let path = library.package_path(&item.record.key).unwrap();
    library
        .assign(None, Some(Assignment::new(path.clone(), 60)))
        .unwrap();
    library.assign(Some("DP-1"), None).unwrap();
    let config = library.config().unwrap();
    assert!(config.assignment("DP-1").is_none());
    assert_eq!(config.assignment("DP-2").unwrap().fps, 60);
    let parameters = BTreeMap::from([("brightness".into(), ParameterValue::Float(1.3))]);
    library
        .assign(
            Some("DP-1"),
            Some(Assignment {
                package: path,
                parameters: parameters.clone(),
                fps: 24,
            }),
        )
        .unwrap();
    let config = library.config().unwrap();
    assert_eq!(config.assignment("DP-1").unwrap().parameters, parameters);
    assert_eq!(config.assignment("DP-1").unwrap().fps, 24);
    assert!(config.disabled_outputs.is_empty());
    library.assign(None, None).unwrap();
    let config = library.config().unwrap();
    assert!(
        config.default.is_none() && config.outputs.is_empty() && config.disabled_outputs.is_empty()
    );
}
#[test]
fn settings_are_bounded_and_invalid_values_do_not_replace_saved_settings() {
    let temporary = tempfile::tempdir().unwrap();
    let library = Library::open(Some(temporary.path().to_owned())).unwrap();
    let settings = Settings {
        desktop_fps: 24,
        preview_fps: 15,
        dark_theme: false,
        ..Default::default()
    };
    library.save_settings(&settings).unwrap();
    assert_eq!(library.settings().unwrap().desktop_fps, 24);
    assert!(
        library
            .save_settings(&Settings {
                preview_fps: 0,
                ..settings.clone()
            })
            .is_err()
    );
    assert!(
        library
            .save_settings(&Settings {
                version: 2,
                ..settings
            })
            .is_err()
    );
    assert_eq!(library.settings().unwrap().preview_fps, 15);
    fs::write(library.root.join("settings.json"), vec![b' '; 65537]).unwrap();
    assert!(library.settings().is_err());
}
#[test]
fn application_and_daemon_locks_release_with_the_owner() {
    use fs2::FileExt;
    let temporary = tempfile::tempdir().unwrap();
    let library = Library::open(Some(temporary.path().to_owned())).unwrap();
    let lock = library.lock().unwrap();
    assert!(library.lock().is_err());
    drop(lock);
    assert!(library.lock().is_ok());
    assert!(!wisp_app::daemon::running(&library).unwrap());
    let daemon = wisp_app::library::lock_file(&library.root.join("daemon.lock")).unwrap();
    daemon.try_lock_exclusive().unwrap();
    assert!(wisp_app::daemon::running(&library).unwrap());
    drop(daemon);
    assert!(!wisp_app::daemon::running(&library).unwrap());
}
