use wisp_format::{Manifest, Output, Package, PackageLimits, Pass, Program, Resource};

pub fn package(shader: &str, image: &[u8], preview: Option<&[u8]>) -> Package {
    let mut assets = [
        ("shaders/pass.wgsl".into(), shader.as_bytes().to_vec()),
        ("assets/image.png".into(), image.to_vec()),
    ]
    .into_iter()
    .collect::<std::collections::BTreeMap<_, _>>();
    if let Some(bytes) = preview {
        assets.insert("preview.png".into(), bytes.to_vec());
    }
    Package::build(
        Manifest {
            format_version: 1,
            id: "test.wallpaper".into(),
            name: "Test".into(),
            author: "Test".into(),
            description: String::new(),
            preview: preview.map(|_| "preview.png".into()),
            program: "program.json".into(),
            minimum_wisp_version: "0.1.0".into(),
            gpu_profile: 1,
            tags: vec![],
        },
        Program {
            version: 1,
            shader_abi_version: 1,
            parameters: vec![],
            resources: vec![Resource::Image {
                id: "background".into(),
                path: "assets/image.png".into(),
            }],
            passes: vec![Pass::Fragment {
                shader: "shaders/pass.wgsl".into(),
                background: "background".into(),
                images: vec![],
                output: Output::Presentation,
            }],
        },
        assets,
        &PackageLimits::default(),
    )
    .unwrap()
}

/// Every parameter type, in a deliberately mixed alignment order.
pub fn parameters() -> Vec<wisp_format::Parameter> {
    use wisp_format::ParameterValue::*;
    [
        Float(0.0),
        Integer(-7),
        Boolean(true),
        Vector2([0.25, 0.5]),
        Vector3([0.125, 0.25, 0.5]),
        Color([1.0; 4]),
        Vector4([0.0, 0.25, 0.5, 1.0]),
    ]
    .into_iter()
    .enumerate()
    .map(|(i, default)| wisp_format::Parameter {
        id: format!("value{i}"),
        default,
        min: (i == 0).then_some(0.0),
        max: (i == 0).then_some(1.0),
    })
    .collect()
}

pub const MULTI_SHADER: &str = r#"
fn wallpaper(uv: vec2<f32>) -> vec4<f32> {
    let first = textureSample(background, background_sampler, uv);
    let second = textureSample(image_1, image_1_sampler, uv);
    // A wrong type, offset, sign, or vector lane turns the expected color black.
    let valid = params.p1 == -7 && params.p2 == 1u
        && all(params.p3 == vec2<f32>(0.25, 0.5))
        && all(params.p4 == vec3<f32>(0.125, 0.25, 0.5))
        && all(params.p6 == vec4<f32>(0.0, 0.25, 0.5, 1.0));
    return select(vec4<f32>(0.0, 0.0, 0.0, 1.0), mix(first, second, params.p0) * params.p5, valid);
}
"#;

pub fn multi_package(first: &[u8], second: &[u8]) -> Package {
    let base = package(MULTI_SHADER, first, None);
    let mut program = base.program().clone();
    program.version = 2;
    program.shader_abi_version = 2;
    program.parameters = parameters();
    // Resource order intentionally differs from binding order.
    program.resources.insert(
        0,
        Resource::Image {
            id: "second".into(),
            path: "assets/second.png".into(),
        },
    );
    let Pass::Fragment { images, .. } = &mut program.passes[0] else {
        unreachable!()
    };
    images.push("second".into());
    Package::build(
        base.manifest().clone(),
        program,
        [
            ("shaders/pass.wgsl".into(), MULTI_SHADER.as_bytes().to_vec()),
            ("assets/image.png".into(), first.to_vec()),
            ("assets/second.png".into(), second.to_vec()),
        ]
        .into(),
        &PackageLimits::default(),
    )
    .unwrap()
}

pub fn multipass_package(image: &[u8], final_shader: Option<&str>) -> Package {
    let first = r#"
fn wallpaper(uv: vec2<f32>) -> vec4<f32> {
    let valid = all(globals.resolution == vec2<f32>(7.0, 5.0));
    return vec4<f32>(select(0.0, params.p0 * 0.5, valid), 0.0, 0.0, 1.0);
}"#;
    let second = r#"
fn wallpaper(uv: vec2<f32>) -> vec4<f32> {
    let valid = all(globals.resolution == vec2<f32>(11.0, 3.0));
    let red = textureSample(background, background_sampler, uv).r;
    return vec4<f32>(select(0.0, red, valid), select(0.0, 0.25, globals.time == 3.0), 0.0, 1.0);
}"#;
    let final_shader = final_shader.unwrap_or(r#"
fn wallpaper(uv: vec2<f32>) -> vec4<f32> {
    let first = textureSample(background, background_sampler, uv);
    let branch = textureSample(image_1, image_1_sampler, uv);
    let original = textureSample(image_2, image_2_sampler, uv);
    let valid = all(globals.resolution == vec2<f32>(65.0, 33.0)) || all(globals.resolution == vec2<f32>(19.0, 9.0));
    return vec4<f32>(select(0.0, (first.r + branch.r) * params.p0, valid), first.g, original.b, 1.0);
}"#);
    let base = package(first, image, None);
    let program = Program {
        version: 3,
        shader_abi_version: 2,
        parameters: vec![wisp_format::Parameter {
            id: "gain".into(),
            default: wisp_format::ParameterValue::Float(1.0),
            min: Some(0.0),
            max: Some(1.0),
        }],
        resources: vec![
            Resource::RenderTarget {
                id: "second".into(),
                width: 11,
                height: 3,
            },
            Resource::Image {
                id: "source".into(),
                path: "assets/image.png".into(),
            },
            Resource::RenderTarget {
                id: "first".into(),
                width: 7,
                height: 5,
            },
        ],
        passes: vec![
            Pass::Fragment {
                shader: "shaders/first.wgsl".into(),
                background: "source".into(),
                images: vec![],
                output: Output::Target("first".into()),
            },
            Pass::Fragment {
                shader: "shaders/second.wgsl".into(),
                background: "first".into(),
                images: vec![],
                output: Output::Target("second".into()),
            },
            Pass::Fragment {
                shader: "shaders/final.wgsl".into(),
                background: "second".into(),
                images: vec!["first".into(), "source".into()],
                output: Output::Presentation,
            },
        ],
    };
    Package::build(
        base.manifest().clone(),
        program,
        [
            ("assets/image.png".into(), image.to_vec()),
            ("shaders/first.wgsl".into(), first.as_bytes().to_vec()),
            ("shaders/second.wgsl".into(), second.as_bytes().to_vec()),
            (
                "shaders/final.wgsl".into(),
                final_shader.as_bytes().to_vec(),
            ),
        ]
        .into(),
        &PackageLimits::default(),
    )
    .unwrap()
}

pub fn compute_parts() -> (
    Manifest,
    Program,
    std::collections::BTreeMap<String, Vec<u8>>,
) {
    use wisp_format::{Parameter, ParameterValue, StorageBinding};
    let base = package(
        wisp_runtime::EXAMPLE_SHADER,
        wisp_runtime::EXAMPLE_IMAGE,
        None,
    );
    let first = r#"
fn compute(id: vec3<u32>) {
    let size = vec2<u32>(globals.resolution);
    if id.x >= size.x || id.y >= size.y || id.z != 0u { return; }
    buffer_0[id.y * size.x + id.x] = vec4<f32>(params.p0, globals.time, 0.0, 1.0);
    textureStore(output_image, vec2<i32>(id.xy), vec4<f32>(1.0));
}"#;
    let second = r#"
fn compute(id: vec3<u32>) {
    let size = vec2<u32>(globals.resolution);
    if id.x >= size.x || id.y >= size.y || id.z != 0u { return; }
    let color = buffer_0[id.y * size.x + id.x] * textureLoad(background, vec2<i32>(id.xy), 0);
    textureStore(output_image, vec2<i32>(id.xy), color);
}"#;
    let final_shader = "fn wallpaper(uv: vec2<f32>) -> vec4<f32> { return textureSample(background, background_sampler, uv); }";
    (
        base.manifest().clone(),
        Program {
            version: 4,
            shader_abi_version: 3,
            parameters: vec![Parameter {
                id: "gain".into(),
                default: ParameterValue::Float(1.0),
                min: Some(0.0),
                max: Some(1.0),
            }],
            resources: vec![
                Resource::StorageBuffer {
                    id: "data".into(),
                    elements: 63,
                },
                Resource::RenderTarget {
                    id: "first".into(),
                    width: 9,
                    height: 7,
                },
                Resource::RenderTarget {
                    id: "second".into(),
                    width: 9,
                    height: 7,
                },
            ],
            passes: vec![
                Pass::Compute {
                    shader: "shaders/first.wgsl".into(),
                    images: vec![],
                    buffers: vec![StorageBinding {
                        resource: "data".into(),
                        writable: true,
                    }],
                    output: "first".into(),
                    dispatch: [2, 1, 1],
                },
                Pass::Compute {
                    shader: "shaders/second.wgsl".into(),
                    images: vec!["first".into()],
                    buffers: vec![StorageBinding {
                        resource: "data".into(),
                        writable: false,
                    }],
                    output: "second".into(),
                    dispatch: [2, 1, 1],
                },
                Pass::Fragment {
                    shader: "shaders/final.wgsl".into(),
                    background: "second".into(),
                    images: vec![],
                    output: Output::Presentation,
                },
            ],
        },
        [
            ("shaders/first.wgsl".into(), first.as_bytes().to_vec()),
            ("shaders/second.wgsl".into(), second.as_bytes().to_vec()),
            (
                "shaders/final.wgsl".into(),
                final_shader.as_bytes().to_vec(),
            ),
        ]
        .into(),
    )
}
