//! Editable graph documents. Canvas layout is authoring-only and never exported.
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    fs,
    path::{Component, Path},
};
use wisp_compiler::{Asset, Compilation, Project, ProjectAsset};
use wisp_format::{Manifest, Parameter, ParameterValue};
use wisp_ir::{Graph, Node, Op, ValueType};

#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Layout {
    pub version: u32,
    pub positions: BTreeMap<String, [f32; 2]>,
}
#[derive(Clone)]
pub struct Document {
    pub graph: Graph,
    pub project: Project,
    pub assets: std::sync::Arc<BTreeMap<String, Asset>>,
    pub layout: Layout,
}
pub const CATALOG: &[&str] = &[
    "Transform",
    "Mask",
    "Layer",
    "Blend",
    "Wave",
    "Warp",
    "Displacement",
    "Droplets",
    "Rain",
    "Ripples",
    "Constant",
    "Parameter",
    "UV",
    "Time",
    "Delta time",
    "Resolution",
    "Mouse",
    "Mouse distance",
    "Particles",
    "Feedback",
    "Audio bass",
    "Audio mid",
    "Audio treble",
    "Audio volume",
    "Add",
    "Multiply",
    "Sin",
    "Translate",
    "Parallax",
    "Image",
    "Brightness",
    "Contrast",
    "Tint",
    "Vignette",
    "Mix",
    "Blur",
    "Materialize",
    "Output",
];
pub fn ports(op: &Op) -> &'static [&'static str] {
    match op {
        Op::Transform { .. } => &["uv", "position", "scale", "rotation", "pivot"],
        Op::Mask { .. } => &["color", "feather", "invert"],
        Op::Layer { .. } => &["color", "uv", "opacity", "mask"],
        Op::Blend { .. } => &["a", "b"],
        Op::Wave { .. } | Op::Warp { .. } => {
            &["uv", "mask", "direction", "amplitude", "frequency", "speed"]
        }
        Op::Displacement { .. } => &["uv", "map", "mask", "strength"],
        Op::Weather { .. } => &[
            "color", "uv", "mask", "density", "size", "speed", "strength",
        ],
        Op::Add { .. } | Op::Multiply { .. } => &["a", "b"],
        Op::Sin { .. } => &["input"],
        Op::Translate { .. } => &["uv", "offset"],
        Op::Parallax { .. } => &["uv", "strength"],
        Op::Image { .. } | Op::MouseDistance { .. } => &["uv"],
        Op::Particles { .. } => &["position", "color", "speed", "size"],
        Op::Feedback { .. } => &["color", "decay"],
        Op::Brightness { .. } | Op::Contrast { .. } => &["color", "amount"],
        Op::Tint { .. } => &["color", "tint"],
        Op::Vignette { .. } => &["color", "uv", "strength"],
        Op::Mix { .. } => &["a", "b", "factor"],
        Op::Blur { .. } => &["color", "radius"],
        Op::Materialize { .. } | Op::Output { .. } => &["color"],
        _ => &[],
    }
}
pub fn label(op: &Op) -> String {
    serde_json::to_value(op).unwrap()["type"]
        .as_str()
        .unwrap()
        .replace('_', " ")
}
fn read(path: &Path, limit: u64) -> Result<Vec<u8>> {
    wisp_io::read_bounded(path, limit)
}
fn contained(root: &Path, name: &str) -> Result<std::path::PathBuf> {
    ensure!(
        !name.is_empty()
            && !name.contains(['\\', ':'])
            && Path::new(name)
                .components()
                .all(|p| matches!(p, Component::Normal(_))),
        "unsafe project path"
    );
    let path = root.join(name).canonicalize()?;
    ensure!(path.starts_with(root), "project path escapes folder");
    Ok(path)
}
impl Default for Document {
    fn default() -> Self {
        let mut doc = Self {
            graph: Graph {
                version: 1,
                nodes: vec![
                    Node {
                        id: "color".into(),
                        op: Op::Constant {
                            value: ParameterValue::Color([0.15, 0.3, 0.6, 1.]),
                        },
                    },
                    Node {
                        id: "output".into(),
                        op: Op::Output {
                            color: "color".into(),
                        },
                    },
                ],
                output: "output".into(),
                parameters: vec![],
            },
            project: Project {
                version: 1,
                id: "local.graph.wallpaper".into(),
                name: "My graph wallpaper".into(),
                author: "Me".into(),
                description: String::new(),
                tags: vec![],
                assets: BTreeMap::new(),
            },
            assets: Default::default(),
            layout: Layout {
                version: 1,
                positions: BTreeMap::new(),
            },
        };
        doc.arrange();
        doc
    }
}
impl Document {
    pub fn open(directory: &Path) -> Result<Self> {
        let root = directory.canonicalize()?;
        let project: Project =
            serde_json::from_slice(&read(&contained(&root, "project.json")?, 1024 * 1024)?)?;
        let graph: Graph =
            serde_json::from_slice(&read(&contained(&root, "graph.json")?, 1024 * 1024)?)?;
        ensure!(
            project.version == 1 && graph.version == 1,
            "unsupported project or graph version"
        );
        ensure!(
            graph.nodes.len() <= wisp_ir::MAX_NODES && project.assets.len() <= wisp_ir::MAX_NODES,
            "project exceeds editor limits"
        );
        let mut ids = std::collections::BTreeSet::new();
        for node in &graph.nodes {
            ensure!(
                !node.id.is_empty() && ids.insert(&node.id),
                "empty or duplicate node ID"
            );
        }
        let mut assets = BTreeMap::new();
        // Preserve disconnected assets too: a draft must remain editable after reopening.
        for (id, asset) in &project.assets {
            ensure!(
                asset.path.starts_with("assets/"),
                "image must be under assets/"
            );
            ensure!(
                !asset.path.contains(['\\', ':'])
                    && Path::new(&asset.path)
                        .components()
                        .all(|p| matches!(p, Component::Normal(_))),
                "unsafe project asset path"
            );
            // Missing sources are repairable authoring diagnostics. Existing paths
            // still undergo containment checks, including dangling/escaping symlinks.
            if fs::symlink_metadata(root.join(&asset.path))
                .is_err_and(|e| e.kind() == std::io::ErrorKind::NotFound)
            {
                continue;
            }
            let path = contained(&root, &asset.path)?;
            let bytes = wisp_compiler::read_source_image(&path)?;
            let extension = Path::new(&asset.path)
                .extension()
                .and_then(|x| x.to_str())
                .context("image requires extension")?
                .to_owned();
            ensure!(
                ["png", "jpg", "jpeg", "webp"].contains(&extension.as_str()),
                "unsupported image extension"
            );
            wisp_compiler::decode_source_image(&bytes)?;
            assets.insert(id.clone(), Asset { extension, bytes });
        }
        let mut doc = Self {
            graph,
            project,
            assets: std::sync::Arc::new(assets),
            layout: Layout {
                version: 1,
                positions: BTreeMap::new(),
            },
        };
        doc.arrange();
        if root.join("layout.json").exists() {
            let layout: Layout =
                serde_json::from_slice(&read(&contained(&root, "layout.json")?, 1024 * 1024)?)?;
            ensure!(
                layout.version == 1 && layout.positions.len() <= wisp_ir::MAX_NODES,
                "invalid layout version or size"
            );
            for (id, position) in layout.positions {
                ensure!(
                    position.iter().all(|v| v.is_finite() && v.abs() <= 100000.),
                    "invalid node position"
                );
                if doc.node(&id).is_some() {
                    doc.layout.positions.insert(id, position);
                }
            }
        }
        Ok(doc)
    }
    pub fn from_easy(draft: &crate::designer::Draft) -> Result<Self> {
        let dir = tempfile::tempdir()?;
        let path = dir.path().join("project");
        draft.save_project(&path)?;
        Self::open(&path)
    }
    pub fn node(&self, id: &str) -> Option<&Node> {
        self.graph.nodes.iter().find(|n| n.id == id)
    }
    pub fn compile(&self) -> Result<Compilation> {
        wisp_compiler::compile(
            &self.graph.clone().check()?,
            &self.assets,
            Manifest {
                format_version: 1,
                id: self.project.id.clone(),
                name: self.project.name.clone(),
                author: self.project.author.clone(),
                description: self.project.description.clone(),
                tags: self.project.tags.clone(),
                preview: None,
                program: "program.json".into(),
                minimum_wisp_version: env!("CARGO_PKG_VERSION").into(),
                gpu_profile: 1,
            },
            &Default::default(),
        )
    }
    pub fn arrange(&mut self) {
        // Memoized dependency depth; cycles in incomplete drafts fall back to layer zero.
        fn depth(
            doc: &Document,
            id: &str,
            seen: &mut Vec<String>,
            memo: &mut BTreeMap<String, usize>,
        ) -> usize {
            if let Some(d) = memo.get(id) {
                return *d;
            }
            if seen.iter().any(|s| s == id) {
                return 0;
            }
            seen.push(id.into());
            let d = doc
                .node(id)
                .map(|n| {
                    n.op.inputs()
                        .iter()
                        .filter(|s| !s.is_empty())
                        .map(|s| 1 + depth(doc, s, seen, memo))
                        .max()
                        .unwrap_or(0)
                })
                .unwrap_or(0);
            seen.pop();
            memo.insert(id.into(), d);
            d
        }
        let mut memo = BTreeMap::new();
        let mut rows = BTreeMap::new();
        let positions = self
            .graph
            .nodes
            .iter()
            .map(|n| {
                let d = depth(self, &n.id, &mut vec![], &mut memo);
                let r = rows.entry(d).or_insert(0);
                let p = [30. + d as f32 * 250., 30. + *r as f32 * 160.];
                *r += 1;
                (n.id.clone(), p)
            })
            .collect();
        self.layout.positions = positions;
    }
    pub fn add(&mut self, kind: &str, position: [f32; 2]) -> Result<String> {
        ensure!(
            self.graph.nodes.len() < wisp_ir::MAX_NODES,
            "maximum 256 nodes"
        );
        let mut number = 1;
        let id = loop {
            let id = format!("node{number}");
            if self.node(&id).is_none() {
                break id;
            }
            number += 1;
        };
        let empty = String::new;
        let op = match kind {
            "Transform" => Op::Transform {
                uv: empty(),
                position: empty(),
                scale: empty(),
                rotation: empty(),
                pivot: empty(),
            },
            "Mask" => Op::Mask {
                color: empty(),
                feather: empty(),
                invert: empty(),
            },
            "Layer" => Op::Layer {
                color: empty(),
                uv: empty(),
                opacity: empty(),
                mask: empty(),
            },
            "Blend" => Op::Blend {
                a: empty(),
                b: empty(),
                mode: wisp_ir::BlendMode::Alpha,
            },
            "Wave" => Op::Wave {
                uv: empty(),
                mask: empty(),
                direction: empty(),
                amplitude: empty(),
                frequency: empty(),
                speed: empty(),
            },
            "Warp" => Op::Warp {
                uv: empty(),
                mask: empty(),
                direction: empty(),
                amplitude: empty(),
                frequency: empty(),
                speed: empty(),
            },
            "Displacement" => Op::Displacement {
                uv: empty(),
                map: empty(),
                mask: empty(),
                strength: empty(),
            },
            "Droplets" | "Rain" | "Ripples" => Op::Weather {
                rain_direction: 0.,
                size_x: 1.,
                size_y: 1.,
                color: empty(),
                uv: empty(),
                mask: empty(),
                density: empty(),
                size: empty(),
                speed: empty(),
                strength: empty(),
                kind: match kind {
                    "Rain" => wisp_ir::WeatherKind::Rain,
                    "Ripples" => wisp_ir::WeatherKind::Ripples,
                    _ => wisp_ir::WeatherKind::Droplets,
                },
            },
            "Constant" => Op::Constant {
                value: ParameterValue::Float(1.),
            },
            "Parameter" => {
                let mut parameter = format!("param{number}");
                while self.graph.parameters.iter().any(|p| p.id == parameter) {
                    number += 1;
                    parameter = format!("param{number}");
                }
                ensure!(self.graph.parameters.len() < 64, "maximum 64 parameters");
                self.graph.parameters.push(Parameter {
                    id: parameter.clone(),
                    default: ParameterValue::Float(1.),
                    min: None,
                    max: None,
                });
                Op::Parameter { parameter }
            }
            "UV" => Op::Uv {},
            "Time" => Op::Time {},
            "Delta time" => Op::DeltaTime {},
            "Resolution" => Op::Resolution {},
            "Mouse" => Op::Mouse {},
            "Mouse distance" => Op::MouseDistance { uv: empty() },
            "Particles" => Op::Particles {
                position: empty(),
                color: empty(),
                speed: empty(),
                size: empty(),
                count: 32,
                width: 512,
                height: 512,
            },
            "Feedback" => Op::Feedback {
                color: empty(),
                decay: empty(),
                width: 512,
                height: 512,
            },
            "Audio bass" => Op::AudioBass {},
            "Audio mid" => Op::AudioMid {},
            "Audio treble" => Op::AudioTreble {},
            "Audio volume" => Op::AudioVolume {},
            "Add" => Op::Add {
                a: empty(),
                b: empty(),
            },
            "Multiply" => Op::Multiply {
                a: empty(),
                b: empty(),
            },
            "Sin" => Op::Sin { input: empty() },
            "Translate" => Op::Translate {
                uv: empty(),
                offset: empty(),
            },
            "Parallax" => Op::Parallax {
                uv: empty(),
                strength: empty(),
            },
            "Image" => Op::Image {
                asset: self.assets.keys().next().cloned().unwrap_or_default(),
                uv: empty(),
            },
            "Brightness" => Op::Brightness {
                color: empty(),
                amount: empty(),
            },
            "Contrast" => Op::Contrast {
                color: empty(),
                amount: empty(),
            },
            "Tint" => Op::Tint {
                color: empty(),
                tint: empty(),
            },
            "Vignette" => Op::Vignette {
                color: empty(),
                uv: empty(),
                strength: empty(),
            },
            "Mix" => Op::Mix {
                a: empty(),
                b: empty(),
                factor: empty(),
            },
            "Blur" => Op::Blur {
                color: empty(),
                radius: empty(),
                width: 512,
                height: 512,
            },
            "Materialize" => Op::Materialize {
                color: empty(),
                width: 512,
                height: 512,
            },
            "Output" => Op::Output { color: empty() },
            _ => anyhow::bail!("unknown node kind"),
        };
        self.graph.nodes.push(Node { id: id.clone(), op });
        self.layout.positions.insert(id.clone(), position);
        Ok(id)
    }
    pub fn set_field(&mut self, id: &str, field: &str, value: serde_json::Value) -> Result<()> {
        let n = self
            .graph
            .nodes
            .iter_mut()
            .find(|n| n.id == id)
            .context("missing node")?;
        let mut op = serde_json::to_value(&n.op)?;
        ensure!(
            field != "type" && op.get(field).is_some(),
            "unknown property"
        );
        op[field] = value;
        n.op = serde_json::from_value(op)?;
        Ok(())
    }
    pub fn connect(&mut self, source: &str, target: &str, port: &str) -> Result<()> {
        let node = self.node(source).context("missing source")?;
        ensure!(
            !matches!(node.op, Op::Output { .. }),
            "Output has no outgoing socket"
        );
        ensure!(
            ports(&self.node(target).context("missing target")?.op).contains(&port),
            "missing input socket"
        );
        let mut candidate = self.clone();
        candidate.set_field(target, port, source.into())?;
        // Run the canonical checker on the connected component. Ignore unrelated
        // incomplete nodes; missing inputs in this component remain visible diagnostics.
        fn visit(
            doc: &Document,
            id: &str,
            stack: &mut Vec<String>,
            done: &mut Vec<String>,
        ) -> Result<()> {
            ensure!(!stack.iter().any(|s| s == id), "cycle at node {id}");
            if done.iter().any(|s| s == id) {
                return Ok(());
            }
            if let Some(n) = doc.node(id) {
                stack.push(id.into());
                for input in n.op.inputs() {
                    if !input.is_empty() {
                        visit(doc, input, stack, done)?;
                    }
                }
                stack.pop();
                done.push(id.into());
            }
            Ok(())
        }
        visit(&candidate, target, &mut vec![], &mut vec![])?;
        // Known types reject incompatible sockets even in an unfinished graph.
        if let (Some(actual), Some(expected)) = (
            candidate.value_type(source),
            candidate.input_type(target, port),
        ) {
            ensure!(
                actual == expected,
                "node {target} input {port}: expected {expected:?}, found {actual:?}"
            );
        }
        if matches!(
            candidate.node(target).unwrap().op,
            Op::Add { .. } | Op::Multiply { .. }
        ) {
            ensure!(
                !matches!(
                    candidate.value_type(source),
                    Some(ValueType::Boolean | ValueType::Integer)
                ),
                "arithmetic requires float or vector"
            );
        }
        self.graph = candidate.graph;
        Ok(())
    }
    pub fn value_type(&self, id: &str) -> Option<ValueType> {
        fn infer(
            doc: &Document,
            id: &str,
            seen: &mut std::collections::BTreeSet<String>,
        ) -> Option<ValueType> {
            if !seen.insert(id.into()) {
                return None;
            }
            use ValueType::*;
            Some(match &doc.node(id)?.op {
                Op::Constant { value } => ValueType::of(value),
                Op::Parameter { parameter } => ValueType::of(
                    &doc.graph
                        .parameters
                        .iter()
                        .find(|p| p.id == *parameter)?
                        .default,
                ),
                Op::Uv {}
                | Op::Mouse {}
                | Op::Resolution {}
                | Op::Transform { .. }
                | Op::Wave { .. }
                | Op::Warp { .. }
                | Op::Displacement { .. }
                | Op::Translate { .. }
                | Op::Parallax { .. } => Vector2,
                Op::Time {}
                | Op::DeltaTime {}
                | Op::AudioBass {}
                | Op::AudioMid {}
                | Op::AudioTreble {}
                | Op::AudioVolume {}
                | Op::Sin { .. }
                | Op::MouseDistance { .. }
                | Op::Mask { .. } => Float,
                Op::Add { a, b } | Op::Multiply { a, b } => {
                    return infer(doc, a, seen).or_else(|| infer(doc, b, seen));
                }
                _ => Color,
            })
        }
        infer(self, id, &mut Default::default())
    }
    pub fn input_type(&self, id: &str, port: &str) -> Option<ValueType> {
        use ValueType::*;
        match &self.node(id)?.op {
            Op::Add { a, b } | Op::Multiply { a, b } => {
                self.value_type(if port == "a" { b } else { a })
            }
            _ => Some(match port {
                "uv" | "offset" | "position" | "scale" | "pivot" | "direction" => Vector2,
                "invert" => Boolean,
                "rotation" | "feather" | "opacity" | "mask" | "amplitude" | "frequency"
                | "density" => Float,
                "input" | "strength" | "amount" | "factor" | "radius" | "speed" | "size"
                | "decay" => Float,
                _ => Color,
            }),
        }
    }
    pub fn remove(&mut self, id: &str) {
        self.graph.nodes.retain(|n| n.id != id);
        for n in &mut self.graph.nodes {
            let mut op = serde_json::to_value(&n.op).unwrap();
            for port in ports(&n.op) {
                if op[*port].as_str() == Some(id) {
                    op[*port] = "".into();
                }
            }
            n.op = serde_json::from_value(op).unwrap();
        }
        self.layout.positions.remove(id);
    }
    pub fn import_image(&mut self, path: &Path) -> Result<String> {
        let bytes = wisp_compiler::read_source_image(path)?;
        wisp_compiler::decode_source_image(&bytes)?;
        let extension = match image::guess_format(&bytes)? {
            image::ImageFormat::Png => "png",
            image::ImageFormat::Jpeg => "jpg",
            image::ImageFormat::WebP => "webp",
            _ => anyhow::bail!("choose PNG, JPEG or WebP"),
        };
        ensure!(
            self.assets.len() < wisp_ir::MAX_NODES,
            "asset limit exceeded"
        );
        let mut i = 1;
        let id = loop {
            let id = format!("image{i}");
            if !self.assets.contains_key(&id) {
                break id;
            }
            i += 1;
        };
        std::sync::Arc::make_mut(&mut self.assets).insert(
            id.clone(),
            Asset {
                extension: extension.into(),
                bytes,
            },
        );
        Ok(id)
    }
    /// Save incomplete drafts as well as compiled graphs, preserving all source assets.
    pub fn save(&self, destination: &Path) -> Result<()> {
        fs::create_dir(destination).context("choose a new project folder name")?;
        let result = (|| -> Result<()> {
            fs::create_dir(destination.join("assets"))?;
            let mut project = self.project.clone();
            project.assets.clear();
            for (i, id) in self
                .project
                .assets
                .keys()
                .filter(|id| !self.assets.contains_key(*id))
                .enumerate()
            {
                project.assets.insert(
                    id.clone(),
                    ProjectAsset {
                        path: format!("assets/missing{i}.png"),
                    },
                );
            }
            for (i, (id, asset)) in self.assets.iter().enumerate() {
                ensure!(
                    ["png", "jpg", "jpeg", "webp"].contains(&asset.extension.as_str()),
                    "unsupported asset extension"
                );
                let path = format!("assets/image{i}.{}", asset.extension);
                crate::library::atomic_write(&destination.join(&path), &asset.bytes)?;
                project.assets.insert(id.clone(), ProjectAsset { path });
            }
            crate::library::atomic_write(
                &destination.join("project.json"),
                &serde_json::to_vec_pretty(&project)?,
            )?;
            crate::library::atomic_write(
                &destination.join("graph.json"),
                &serde_json::to_vec_pretty(&self.graph)?,
            )?;
            crate::library::atomic_write(
                &destination.join("layout.json"),
                &serde_json::to_vec_pretty(&self.layout)?,
            )?;
            Ok(())
        })();
        if result.is_err() {
            let _ = fs::remove_dir_all(destination);
        }
        result
    }
    pub fn export(&self, path: &Path) -> Result<()> {
        let bytes = self.compile()?.package.to_bytes(&Default::default())?;
        crate::library::write_new(path, &bytes).context("choose a new export filename")?;
        Ok(())
    }
}
