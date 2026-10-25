use std::{
    path::PathBuf,
    process::{Command, Output},
};

struct Directory(PathBuf);
impl Drop for Directory {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
fn run(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_wisp-preview"))
        .args(args)
        .output()
        .unwrap()
}
#[test]
fn exports_checks_and_preserves_existing_packages_without_gpu() {
    let dir = directory();
    let package = dir.0.join("test.wisp");
    let path = package.to_str().unwrap();
    let exported = run(&[
        "--export",
        path,
        "--name",
        "CLI Test",
        "--author",
        "Test Author",
    ]);
    assert!(
        exported.status.success(),
        "{}",
        String::from_utf8_lossy(&exported.stderr)
    );
    let checked = run(&["--package", path, "--check"]);
    assert!(
        checked.status.success(),
        "{}",
        String::from_utf8_lossy(&checked.stderr)
    );
    assert!(String::from_utf8_lossy(&checked.stderr).contains("CLI Test by Test Author"));
    let original = std::fs::read(&package).unwrap();
    assert!(!run(&["--export", path]).status.success());
    assert_eq!(original, std::fs::read(&package).unwrap());
    assert!(
        !run(&["--package", path, "--shader", "missing.wgsl", "--check"])
            .status
            .success()
    );
    assert!(!run(&["--export", path, "--check"]).status.success());
    let invalid = dir.0.join("invalid.wgsl");
    std::fs::write(&invalid, "invalid shader").unwrap();
    let failed = dir.0.join("failed.wisp");
    assert!(
        !run(&[
            "--shader",
            invalid.to_str().unwrap(),
            "--export",
            failed.to_str().unwrap()
        ])
        .status
        .success()
    );
    assert!(!failed.exists());
    std::fs::write(&package, b"not a zip").unwrap();
    assert!(!run(&["--package", path, "--check"]).status.success());
}

fn directory() -> Directory {
    let path = std::env::temp_dir().join(format!(
        "wisp-package-test-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir(&path).unwrap();
    Directory(path)
}

#[test]
fn package_parameter_overrides_validate_without_gpu_and_do_not_modify_archive() {
    use wisp_format::{Package, PackageLimits, Parameter, ParameterValue, Pass, Resource};
    let dir = directory();
    let path = dir.0.join("parameters.wisp");
    let filename = path.to_str().unwrap();
    assert!(run(&["--export", filename]).status.success());
    let limits = PackageLimits::default();
    let original = Package::load(&path, &limits).unwrap();
    let mut program = original.program().clone();
    program.version = 2;
    program.shader_abi_version = 2;
    let Resource::Image {
        path: image_path, ..
    } = &program.resources[0]
    else {
        panic!("expected image")
    };
    let image_path = image_path.clone();
    program.resources.push(Resource::Image {
        id: "second".into(),
        path: image_path.clone(),
    });
    let Pass::Fragment { images, shader, .. } = &mut program.passes[0] else {
        unreachable!()
    };
    let shader_path = shader.clone();
    images.push("second".into());
    program.parameters = [
        ("gain", ParameterValue::Float(0.5)),
        ("count", ParameterValue::Integer(-2)),
        ("enabled", ParameterValue::Boolean(true)),
        ("tint", ParameterValue::Color([1.0; 4])),
        ("offset", ParameterValue::Vector2([0.0; 2])),
    ]
    .into_iter()
    .map(|(id, default)| Parameter {
        id: id.into(),
        default,
        min: (id == "gain").then_some(0.0),
        max: (id == "gain").then_some(1.0),
    })
    .collect();
    let package = Package::build(
        original.manifest().clone(),
        program,
        [
            (image_path, original.image_bytes().to_vec()),
            (shader_path, original.shader_source().as_bytes().to_vec()),
        ]
        .into(),
        &limits,
    )
    .unwrap();
    let bytes = package.to_bytes(&limits).unwrap();
    std::fs::write(&path, &bytes).unwrap();
    let checked = run(&[
        "--package",
        filename,
        "--check",
        "--param",
        "gain=0.1",
        "--param",
        "count=-2147483648",
        "--param",
        "enabled=false",
        "--param",
        "tint=[0.2,0.4,0.6,1]",
        "--param",
        "offset=[-1,2]",
    ]);
    assert!(
        checked.status.success(),
        "{}",
        String::from_utf8_lossy(&checked.stderr)
    );
    assert!(String::from_utf8_lossy(&checked.stdout).contains("2 image(s), 5 parameter(s)"));
    for value in [
        "gain=2",
        "gain=NaN",
        "gain=1e100",
        "count=0.5",
        "count=2147483648",
        "enabled=1",
        "tint=[2,0,0,1]",
        "offset=[0]",
        "missing=0",
        "gain",
    ] {
        assert!(
            !run(&["--package", filename, "--check", "--param", value])
                .status
                .success(),
            "{value}"
        );
    }
    assert_eq!(std::fs::read(&path).unwrap(), bytes);
    assert!(!run(&["--check", "--param", "gain=0.5"]).status.success());
}

#[test]
fn checks_multipass_packages_and_reports_later_shader_errors() {
    use wisp_format::{Output as PassOutput, Package, PackageLimits, Pass, Resource};
    let dir = directory();
    let path = dir.0.join("multipass.wisp");
    let filename = path.to_str().unwrap();
    assert!(run(&["--export", filename]).status.success());
    let limits = PackageLimits::default();
    let original = Package::load(&path, &limits).unwrap();
    let mut program = original.program().clone();
    program.version = 3;
    program.shader_abi_version = 2;
    program.resources.push(Resource::RenderTarget {
        id: "blurred".into(),
        width: 16,
        height: 8,
    });
    let Pass::Fragment { output, shader, .. } = &mut program.passes[0] else {
        unreachable!()
    };
    *output = PassOutput::Target("blurred".into());
    let first_shader = shader.clone();
    program.passes.push(Pass::Fragment {
        shader: "shaders/final.wgsl".into(),
        background: "blurred".into(),
        images: vec![],
        output: PassOutput::Presentation,
    });
    let Resource::Image {
        path: image_path, ..
    } = &program.resources[0]
    else {
        unreachable!()
    };
    let image_path = image_path.clone();
    for valid in [true, false] {
        let source = if valid {
            "fn wallpaper(uv: vec2<f32>) -> vec4<f32> { return textureSample(background, background_sampler, uv); }"
        } else {
            "invalid shader"
        };
        let package = Package::build(
            original.manifest().clone(),
            program.clone(),
            [
                (image_path.clone(), original.image_bytes().to_vec()),
                (
                    first_shader.clone(),
                    original.shader_source().as_bytes().to_vec(),
                ),
                ("shaders/final.wgsl".into(), source.as_bytes().to_vec()),
            ]
            .into(),
            &limits,
        )
        .unwrap();
        std::fs::write(&path, package.to_bytes(&limits).unwrap()).unwrap();
        let checked = run(&["--package", filename, "--check"]);
        assert_eq!(
            checked.status.success(),
            valid,
            "{}",
            String::from_utf8_lossy(&checked.stderr)
        );
        if valid {
            assert!(String::from_utf8_lossy(&checked.stdout).contains("2 pass(es)"));
        } else {
            assert!(String::from_utf8_lossy(&checked.stderr).contains("pass 1"));
        }
    }
}
