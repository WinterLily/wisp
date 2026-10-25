use std::{path::PathBuf, process::Command};

struct Directory(PathBuf);
impl Drop for Directory {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn checks_configuration_without_desktop_or_gpu() {
    let dir =
        Directory(std::env::temp_dir().join(format!("wispd-config-test-{}", std::process::id())));
    std::fs::create_dir_all(&dir.0).unwrap();
    let path = dir.0.join("assignments.json");
    let check = || {
        Command::new(env!("CARGO_BIN_EXE_wispd"))
            .args(["--config", path.to_str().unwrap(), "--check"])
            .env_remove("WAYLAND_DISPLAY")
            .env_remove("DISPLAY")
            .output()
            .unwrap()
    };
    std::fs::write(&path, "{}").unwrap();
    let result = check();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    std::fs::write(
        &path,
        r#"{"audio":{"enabled":true,"device":"nonexistent","gain":2}}"#,
    )
    .unwrap();
    assert!(check().status.success()); // CPU checks must never open a capture device.
    for invalid in [
        r#"{"audio":{"gain":-1}}"#,
        r#"{"audio":{"gain":11}}"#,
        r#"{"audio":{"device":""}}"#,
        r#"{"audio":{"unknown":true}}"#,
        r#"{"typo":true}"#,
        r#"{"disabled_outputs":[""]}"#,
        r#"{"outputs":{"DP-1":{"package":"a.wisp"}},"disabled_outputs":["DP-1"]}"#,
        r#"{"outputs":{"":{"package":"missing.wisp"}}}"#,
        r#"{"default":{"package":"missing.wisp","fps":0}}"#,
        r#"{"default":{"package":"missing.wisp"}}"#,
    ] {
        std::fs::write(&path, invalid).unwrap();
        assert!(!check().status.success(), "accepted {invalid}");
    }
    std::fs::write(&path, vec![b' '; 1024 * 1024 + 1]).unwrap();
    assert!(!check().status.success());
}

#[test]
fn rejects_ambiguous_assignments_before_opening_a_desktop() {
    for args in [
        vec![
            "--assign",
            "DISPLAY1=a.wisp",
            "--assign",
            "DISPLAY1=b.wisp",
            "--check",
        ],
        vec!["--assign", "=a.wisp", "--check"],
        vec!["--assign", "DISPLAY1=", "--check"],
        vec!["--package", "a.wisp", "--fps", "0", "--check"],
        vec!["--list-outputs", "--package", "a.wisp"],
        vec!["--json", "--check"],
    ] {
        let result = Command::new(env!("CARGO_BIN_EXE_wispd"))
            .args(args)
            .output()
            .unwrap();
        assert!(!result.status.success());
    }
}
