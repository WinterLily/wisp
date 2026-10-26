//! Ahead-of-time graph lowering to the existing portable Wisp program and shader ABI.
mod effects;
mod image;
mod project;
mod simulation;
use anyhow::{Context, Result, ensure};
pub use image::{decode_source_image, read_source_image};
pub use project::{Project, ProjectAsset, compile_project};
use std::collections::{BTreeMap, BTreeSet};
use wisp_format::{
    Manifest, Output, Package, PackageLimits, Parameter, ParameterValue, Pass, Program, Resource,
};
use wisp_ir::{Op, TypedGraph, ValueType};
use wisp_runtime::{RuntimeLimits, Shader, Wallpaper};

/// Encoded image assets keyed by authoring ID. Only reachable assets are exported.
#[derive(Clone)]
pub struct Asset {
    pub extension: String,
    pub bytes: Vec<u8>,
}
#[derive(Debug, Clone, Default)]
pub struct CompileReport {
    pub live_nodes: usize,
    pub eliminated_nodes: usize,
    pub passes: usize,
    pub intermediate_targets: usize,
    pub image_assets: usize,
    pub image_bytes: u64,
    /// Number of runtime image copies reduced in resolution; sources are intact.
    pub resized_images: usize,
    pub parameters: usize,
    pub intermediate_bytes: u64,
    pub storage_bytes: u64,
}
pub struct Compilation {
    pub package: Package,
    pub report: CompileReport,
}
#[derive(Default)]
pub struct CompileOptions {
    pub package_limits: PackageLimits,
    pub runtime_limits: RuntimeLimits,
}

/// Compile a type-checked graph. Generated source and all packaged images are
/// validated through the shared Naga/ABI/runtime validation path, without a GPU.
/// The manifest's preview is chosen by the compiler from the first used image.
pub fn compile(
    graph: &TypedGraph,
    assets: &BTreeMap<String, Asset>,
    mut manifest: Manifest,
    options: &CompileOptions,
) -> Result<Compilation> {
    let mut used_assets = BTreeSet::new();
    let mut used_params = BTreeSet::new();
    for &i in graph.live_order() {
        match &graph.graph().nodes[i].op {
            Op::Image { asset, .. } => {
                used_assets.insert(asset.clone());
            }
            Op::Parameter { parameter } => {
                used_params.insert(parameter.clone());
            }
            _ => {}
        }
    }
    ensure!(
        used_assets.len() <= wisp_format::MAX_IMAGES,
        "graph exceeds image resource limit"
    );
    let parameters: Vec<_> = graph
        .graph()
        .parameters
        .iter()
        .filter(|p| used_params.contains(&p.id))
        .cloned()
        .collect();
    let mut compiler = Compiler {
        graph,
        parameters,
        asset_ids: BTreeMap::new(),
        targets: BTreeMap::new(),
        resources: vec![],
        passes: vec![],
        files: BTreeMap::new(),
        intermediate_bytes: 0,
        storage_bytes: 0,
        options,
    };
    for (index, asset) in used_assets.iter().enumerate() {
        let data = assets
            .get(asset)
            .with_context(|| format!("missing image asset: {asset}"))?;
        ensure!(
            ["png", "jpg", "jpeg", "webp"].contains(&data.extension.as_str()),
            "unsupported image extension for {asset}"
        );
        let id = format!("image{index}");
        let path = format!("assets/{id}.{}", data.extension);
        compiler.resources.push(Resource::Image {
            id: id.clone(),
            path,
        });
        compiler.asset_ids.insert(asset.clone(), id);
    }
    // The current fragment ABI requires a background binding. Procedural graphs
    // use a shared one-pixel image; this does not require a runtime format change.
    if compiler.resources.is_empty() {
        compiler.files.insert(
            "assets/fallback.png".into(),
            include_bytes!("fallback.png").to_vec(),
        );
        compiler.resources.push(Resource::Image {
            id: "fallback".into(),
            path: "assets/fallback.png".into(),
        });
    }
    manifest.preview = compiler.resources.iter().find_map(|r| match r {
        Resource::Image { path, .. } => Some(path.clone()),
        _ => None,
    });
    for &i in graph.live_order() {
        compiler.simulation(&graph.graph().nodes[i])?;
        if let Op::Blur {
            color,
            radius,
            width,
            height,
        } = &graph.graph().nodes[i].op
        {
            let source = compiler.target(*width, *height)?;
            compiler.emit(color, Output::Target(source.clone()), None)?;
            let horizontal = compiler.target(*width, *height)?;
            compiler.emit(
                radius,
                Output::Target(horizontal.clone()),
                Some((&source, *width)),
            )?;
            compiler
                .targets
                .insert(graph.graph().nodes[i].id.clone(), horizontal);
        }
        if let Op::Materialize {
            color,
            width,
            height,
        } = &graph.graph().nodes[i].op
        {
            let target = compiler.target(*width, *height)?;
            compiler.emit(color, Output::Target(target.clone()), None)?;
            compiler
                .targets
                .insert(graph.graph().nodes[i].id.clone(), target);
        }
    }
    compiler.emit(&graph.graph().output, Output::Presentation, None)?;
    let mut report = CompileReport {
        live_nodes: graph.live_order().len(),
        eliminated_nodes: graph.graph().nodes.len() - graph.live_order().len(),
        passes: compiler.passes.len(),
        intermediate_targets: compiler
            .resources
            .iter()
            .filter(|r| matches!(r, Resource::RenderTarget { .. }))
            .count(),
        image_assets: used_assets.len(),
        image_bytes: 0,
        resized_images: 0,
        parameters: compiler.parameters.len(),
        intermediate_bytes: compiler.intermediate_bytes,
        storage_bytes: compiler.storage_bytes,
    };
    let simulated = compiler.storage_bytes > 0;
    let mut program = Program {
        version: if simulated {
            4
        } else if report.intermediate_targets == 0 {
            2
        } else {
            3
        },
        shader_abi_version: if simulated { 3 } else { 2 },
        parameters: compiler.parameters,
        resources: compiler.resources,
        passes: compiler.passes,
    };
    if !used_assets.is_empty() {
        // Reserve actual shader/metadata costs and conservative ZIP framing space
        // before sharing the remaining portable budgets between live images.
        let overhead = compiler.files.values().map(|b| b.len() as u64).sum::<u64>()
            + serde_json::to_vec_pretty(&manifest)?.len() as u64
            + serde_json::to_vec_pretty(&program)?.len() as u64
            + (compiler.files.len() + used_assets.len() + 2) as u64 * 512;
        let encoded_budget = options
            .package_limits
            .max_uncompressed_bytes
            .min(options.package_limits.max_archive_bytes)
            .saturating_sub(overhead);
        let texture_budget = options
            .runtime_limits
            .max_texture_bytes
            .saturating_sub(report.intermediate_bytes)
            .min(
                options
                    .runtime_limits
                    .max_resource_bytes
                    .saturating_sub(report.intermediate_bytes)
                    .saturating_sub(report.storage_bytes),
            );
        let sources: Vec<_> = used_assets.iter().map(|id| &assets[id]).collect();
        let prepared = image::prepare_all(
            &sources,
            options.runtime_limits.max_texture_dimension,
            texture_budget / 4,
            options.package_limits.max_file_bytes,
            encoded_budget,
        )
        .context("could not prepare source images for playback")?;
        for (index, (data, resized)) in prepared.into_iter().enumerate() {
            report.resized_images += usize::from(resized);
            let Resource::Image { path, .. } = &mut program.resources[index] else {
                unreachable!("image resources precede generated targets");
            };
            *path = format!("assets/image{index}.{}", data.extension);
            compiler.files.insert(path.clone(), data.bytes);
        }
        manifest.preview = program.resources.iter().find_map(|r| match r {
            Resource::Image { path, .. } => Some(path.clone()),
            _ => None,
        });
    }
    let package = Package::build(manifest, program, compiler.files, &options.package_limits)?;
    let wallpaper = Wallpaper::from_package_with_limits(&package, &options.runtime_limits)
        .context("generated wallpaper failed runtime validation")?;
    report.image_bytes = wallpaper
        .images()
        .iter()
        .map(|image| {
            let (w, h) = image.dimensions();
            u64::from(w) * u64::from(h) * 4
        })
        .sum();
    // Apply encoded archive limits too, before reporting a successful compilation.
    package.to_bytes(&options.package_limits)?;
    Ok(Compilation { package, report })
}

struct Compiler<'a> {
    graph: &'a TypedGraph,
    parameters: Vec<Parameter>,
    asset_ids: BTreeMap<String, String>,
    /// A blur is lowered to a vertical sample of an already produced horizontal target.
    targets: BTreeMap<String, String>,
    resources: Vec<Resource>,
    passes: Vec<Pass>,
    files: BTreeMap<String, Vec<u8>>,
    intermediate_bytes: u64,
    storage_bytes: u64,
    options: &'a CompileOptions,
}
impl Compiler<'_> {
    fn target(&mut self, width: u32, height: u32) -> Result<String> {
        let count = self
            .resources
            .iter()
            .filter(|r| matches!(r, Resource::RenderTarget { .. }))
            .count();
        ensure!(
            count
                < self
                    .options
                    .runtime_limits
                    .max_intermediate_targets
                    .min(wisp_format::MAX_RENDER_TARGETS),
            "graph exceeds intermediate target limit"
        );
        ensure!(
            width <= self.options.runtime_limits.max_texture_dimension
                && height <= self.options.runtime_limits.max_texture_dimension,
            "graph exceeds target dimension limit"
        );
        self.intermediate_bytes += u64::from(width) * u64::from(height) * 4;
        ensure!(
            self.intermediate_bytes
                <= self
                    .options
                    .runtime_limits
                    .max_texture_bytes
                    .min(self.options.runtime_limits.max_resource_bytes),
            "graph exceeds intermediate memory limit"
        );
        let id = format!("target{count}");
        self.resources.push(Resource::RenderTarget {
            id: id.clone(),
            width,
            height,
        });
        Ok(id)
    }
    fn collect(
        &self,
        id: &str,
        seen: &mut BTreeSet<String>,
        order: &mut Vec<String>,
        bindings: &mut BTreeSet<String>,
    ) {
        if !seen.insert(id.to_owned()) {
            return;
        }
        let node = self.graph.node(id);
        if let Op::Blur { radius, .. } = &node.op {
            bindings.insert(self.targets[id].clone());
            self.collect(radius, seen, order, bindings);
        } else if matches!(
            node.op,
            Op::Materialize { .. } | Op::Particles { .. } | Op::Feedback { .. }
        ) {
            bindings.insert(self.targets[id].clone());
        } else {
            if let Op::Image { asset, .. } = &node.op {
                bindings.insert(self.asset_ids[asset].clone());
            }
            for input in node.op.inputs() {
                self.collect(input, seen, order, bindings);
            }
        }
        order.push(id.to_owned());
    }
    fn source(
        &self,
        roots: &[&str],
        horizontal: Option<(&str, u32)>,
        compute: bool,
    ) -> Result<(String, Vec<String>)> {
        ensure!(
            self.passes.len()
                < self
                    .options
                    .runtime_limits
                    .max_passes
                    .min(wisp_format::MAX_PASSES),
            "graph exceeds pass limit"
        );
        let mut bindings = BTreeSet::new();
        let mut order = vec![];
        let mut seen = BTreeSet::new();
        for root in roots {
            self.collect(root, &mut seen, &mut order, &mut bindings);
        }
        if let Some((source, _)) = horizontal {
            bindings.insert(source.into());
        }
        if bindings.is_empty() {
            bindings.insert(
                self.resources
                    .iter()
                    .find(|r| matches!(r, Resource::Image { .. }))
                    .unwrap()
                    .id()
                    .to_owned(),
            );
        }
        ensure!(
            bindings.len() <= wisp_format::MAX_IMAGES,
            "generated pass exceeds texture binding limit"
        );
        let bindings: Vec<_> = bindings.into_iter().collect();
        let sample = |resource: &str, uv: &str| {
            let index = bindings.iter().position(|id| id == resource).unwrap();
            let name = if index == 0 {
                "background".into()
            } else {
                format!("image_{index}")
            };
            if compute {
                format!("textureSampleLevel({name}, {name}_sampler, {uv}, 0.0)")
            } else {
                format!("textureSample({name}, {name}_sampler, {uv})")
            }
        };
        let mut source = include_str!("effects.wgsl").to_owned();
        source.push_str(include_str!("droplets.wgsl"));
        for id in order {
            let node = self.graph.node(&id);
            let index = self.graph.index(&id);
            let call = |id: &str| format!("n{}(uv)", self.graph.index(id));
            let v = |id: &str| format!("v{}", self.graph.index(id));
            let inputs = match &node.op {
                Op::Blur { radius, .. } => vec![radius.as_str()],
                Op::Materialize { .. } | Op::Particles { .. } | Op::Feedback { .. } => vec![],
                _ => node.op.inputs(),
            };
            source.push_str(&format!(
                "// node {}\nfn n{index}(uv: vec2<f32>) -> {} {{\n",
                node.id,
                wgsl_type(self.graph.value_type(&id))
            ));
            for input in inputs.into_iter().collect::<BTreeSet<_>>() {
                source.push_str(&format!("let {} = {};\n", v(input), call(input)));
            }
            let expression = if let Some(expression) = self.effect(&node.op, &mut source) {
                expression
            } else {
                match &node.op {
                    Op::Constant { value } => literal(value),
                    Op::Parameter { parameter } => {
                        let i = self
                            .parameters
                            .iter()
                            .position(|p| &p.id == parameter)
                            .unwrap();
                        if self.graph.value_type(&id) == ValueType::Boolean {
                            format!("(params.p{i} != 0u)")
                        } else {
                            format!("params.p{i}")
                        }
                    }
                    Op::Uv {} => "uv".into(),
                    Op::Time {} => "globals.time".into(),
                    Op::DeltaTime {} => "globals.delta_time".into(),
                    Op::Mouse {} => "globals.mouse".into(),
                    Op::MouseDistance { uv } => format!("distance({}, globals.mouse)", v(uv)),
                    Op::Resolution {} => "globals.resolution".into(),
                    Op::AudioBass {} => "globals.audio_bass".into(),
                    Op::AudioMid {} => "globals.audio_mid".into(),
                    Op::AudioTreble {} => "globals.audio_treble".into(),
                    Op::AudioVolume {} => "globals.audio_volume".into(),
                    Op::Add { a, b } | Op::Translate { uv: a, offset: b } => {
                        format!("{} + {}", v(a), v(b))
                    }
                    Op::Multiply { a, b } | Op::Tint { color: a, tint: b } => {
                        format!("{} * {}", v(a), v(b))
                    }
                    Op::Sin { input } => format!("sin({})", v(input)),
                    Op::Parallax { uv, strength } => {
                        // Reserve enough source area for the full pointer range without stretching edge texels.
                        source.push_str(&format!(
                            "let parallax_strength = {};\nlet parallax_margin = abs(parallax_strength) * 0.5;\n",
                            v(strength)
                        ));
                        format!(
                            "({} + vec2<f32>(parallax_margin) + (clamp(globals.mouse, vec2<f32>(0.0), vec2<f32>(1.0)) - vec2<f32>(0.5)) * parallax_strength) / (1.0 + abs(parallax_strength))",
                            v(uv)
                        )
                    }
                    Op::Image { asset, uv } => sample(&self.asset_ids[asset], &v(uv)),
                    Op::Brightness { color, amount } => format!(
                        "vec4<f32>({}.rgb * {}, {}.a)",
                        v(color),
                        v(amount),
                        v(color)
                    ),
                    Op::Contrast { color, amount } => format!(
                        "vec4<f32>(({}.rgb - vec3<f32>(0.5)) * {} + vec3<f32>(0.5), {}.a)",
                        v(color),
                        v(amount),
                        v(color)
                    ),
                    Op::Vignette {
                        color,
                        uv,
                        strength,
                    } => format!(
                        "vec4<f32>({}.rgb * clamp(1.0 - dot({} - vec2<f32>(0.5), {} - vec2<f32>(0.5)) * {}, 0.0, 1.0), {}.a)",
                        v(color),
                        v(uv),
                        v(uv),
                        v(strength),
                        v(color)
                    ),
                    Op::Mix { a, b, factor } => format!("mix({}, {}, {})", v(a), v(b), v(factor)),
                    Op::Blur { radius, height, .. } => {
                        source.push_str(&format!(
                            "let step_uv = vec2<f32>(0.0, clamp({}, 0.0, 64.0) / {}.0);\n",
                            v(radius),
                            height
                        ));
                        gaussian(&self.targets[&id], &sample)
                    }
                    Op::Output { color } => v(color),
                    Op::Materialize { .. } | Op::Particles { .. } | Op::Feedback { .. } => {
                        sample(&self.targets[&id], "uv")
                    }
                    _ => unreachable!("effect handled above"),
                }
            };
            source.push_str(&format!("return {expression};\n}}\n"));
        }
        Ok((source, bindings))
    }
    fn emit(&mut self, root: &str, output: Output, horizontal: Option<(&str, u32)>) -> Result<()> {
        let (mut source, bindings) = self.source(&[root], horizontal, false)?;
        let sample = |resource: &str, uv: &str| sample_resource(&bindings, resource, uv);
        source.push_str("fn wallpaper(uv: vec2<f32>) -> vec4<f32> {\n");
        if let Some((resource, width)) = horizontal {
            source.push_str(&format!("let step_uv = vec2<f32>(clamp(n{}(uv), 0.0, 64.0) / {width}.0, 0.0);\nreturn {};\n",self.graph.index(root),gaussian(resource,&sample)));
        } else {
            source.push_str(&format!("return n{}(uv);\n", self.graph.index(root)));
        }
        source.push_str("}\n");
        Shader::with_bindings(&source, bindings.len(), &self.parameters)
            .with_context(|| format!("generated pass {} for node {root}", self.passes.len()))?;
        let path = format!("shaders/pass_{}.wgsl", self.passes.len());
        self.files.insert(path.clone(), source.into_bytes());
        self.passes.push(Pass::Fragment {
            shader: path,
            background: bindings[0].clone(),
            images: bindings[1..].to_vec(),
            output,
        });
        Ok(())
    }
}
fn gaussian(resource: &str, sample: &impl Fn(&str, &str) -> String) -> String {
    format!(
        "{} * 0.375 + ({} + {}) * 0.25 + ({} + {}) * 0.0625",
        sample(resource, "uv"),
        sample(resource, "uv - step_uv"),
        sample(resource, "uv + step_uv"),
        sample(resource, "uv - step_uv * 2.0"),
        sample(resource, "uv + step_uv * 2.0")
    )
}
fn wgsl_type(ty: ValueType) -> &'static str {
    match ty {
        ValueType::Float => "f32",
        ValueType::Integer => "i32",
        ValueType::Boolean => "bool",
        ValueType::Vector2 => "vec2<f32>",
        ValueType::Vector3 => "vec3<f32>",
        ValueType::Vector4 | ValueType::Color => "vec4<f32>",
    }
}
fn literal(value: &ParameterValue) -> String {
    let vector = |values: &[f32]| {
        format!(
            "vec{}<f32>({})",
            values.len(),
            values
                .iter()
                .map(|v| format!("{v:?}"))
                .collect::<Vec<_>>()
                .join(", ")
        )
    };
    match value {
        ParameterValue::Float(v) => format!("{v:?}"),
        ParameterValue::Integer(v) => format!("i32({v})"),
        ParameterValue::Boolean(v) => v.to_string(),
        ParameterValue::Vector2(v) => vector(v),
        ParameterValue::Vector3(v) => vector(v),
        ParameterValue::Vector4(v) | ParameterValue::Color(v) => vector(v),
    }
}

fn sample_resource(bindings: &[String], resource: &str, uv: &str) -> String {
    let i = bindings.iter().position(|r| r == resource).unwrap();
    let name = if i == 0 {
        "background".into()
    } else {
        format!("image_{i}")
    };
    format!("textureSample({name}, {name}_sampler, {uv})")
}
