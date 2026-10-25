//! Easy Mode produces the same typed graph consumed by wispc; no authoring data
//! is required by the runtime package.
use anyhow::{Context, Result, ensure};
use image::GenericImageView;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{collections::BTreeMap, fs, path::Path};
use wisp_compiler::{Asset, Compilation, Project, ProjectAsset};
use wisp_compiler::{decode_source_image, read_source_image};
use wisp_format::{Manifest, PackageLimits, Parameter, ParameterValue};
use wisp_ir::{Graph, Node, Op};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Preset {
    Still,
    Gentle,
    Dreamy,
    Pulse,
}
impl Preset {
    pub const ALL: [Self; 4] = [Self::Still, Self::Gentle, Self::Dreamy, Self::Pulse];
}
impl std::fmt::Display for Preset {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Still => "Clean image",
            Self::Gentle => "Gentle motion",
            Self::Dreamy => "Dreamy focus",
            Self::Pulse => "Breathing light",
        })
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Design {
    pub version: u32,
    pub name: String,
    pub author: String,
    pub parallax: bool,
    pub pulse: bool,
    pub blur: bool,
    pub vignette: bool,
    pub values: BTreeMap<String, f32>,
}
impl Default for Design {
    fn default() -> Self {
        Self {
            version: 1,
            name: "My wallpaper".into(),
            author: "Me".into(),
            parallax: true,
            pulse: true,
            blur: false,
            vignette: true,
            values: CONTROLS.iter().map(|c| (c.id.into(), c.default)).collect(),
        }
    }
}
pub struct Control {
    pub id: &'static str,
    pub label: &'static str,
    pub min: f32,
    pub max: f32,
    pub default: f32,
}
pub const CONTROLS: &[Control] = &[
    Control {
        id: "parallax",
        label: "Mouse movement",
        min: 0.,
        max: 0.15,
        default: 0.03,
    },
    Control {
        id: "speed",
        label: "Animation speed",
        min: 0.1,
        max: 4.,
        default: 0.6,
    },
    Control {
        id: "pulse",
        label: "Light variation",
        min: 0.,
        max: 0.5,
        default: 0.12,
    },
    Control {
        id: "brightness",
        label: "Brightness",
        min: 0.,
        max: 2.,
        default: 1.,
    },
    Control {
        id: "contrast",
        label: "Contrast",
        min: 0.,
        max: 2.,
        default: 1.,
    },
    Control {
        id: "vignette",
        label: "Darken edges",
        min: 0.,
        max: 1.,
        default: 0.3,
    },
    Control {
        id: "radius",
        label: "Softness",
        min: 0.,
        max: 8.,
        default: 2.,
    },
];
impl Design {
    pub fn preset(&mut self, preset: Preset) {
        let name = self.name.clone();
        let author = self.author.clone();
        *self = Self::default();
        self.name = name;
        self.author = author;
        self.parallax = matches!(preset, Preset::Gentle | Preset::Dreamy);
        self.pulse = matches!(preset, Preset::Gentle | Preset::Pulse);
        self.blur = preset == Preset::Dreamy;
        self.vignette = preset != Preset::Still;
    }
    pub fn validate(&self) -> Result<()> {
        ensure!(self.version == 1, "unsupported Easy Designer version");
        ensure!(
            !self.name.trim().is_empty() && self.name.len() <= 256,
            "name must contain 1–256 bytes"
        );
        ensure!(
            !self.author.trim().is_empty() && self.author.len() <= 256,
            "author must contain 1–256 bytes"
        );
        ensure!(
            self.values.len() == CONTROLS.len(),
            "invalid designer controls"
        );
        for c in CONTROLS {
            let v = self.values.get(c.id).context("missing designer control")?;
            ensure!(
                v.is_finite() && (c.min..=c.max).contains(v),
                "{} must be {}–{}",
                c.label,
                c.min,
                c.max
            );
        }
        Ok(())
    }
}
#[derive(Clone)]
pub struct Draft {
    pub design: Design,
    image: Vec<u8>,
    pub layers: Option<crate::layers::Stack>,
    extension: String,
    pub dimensions: (u32, u32),
}
pub(crate) fn read(path: &Path, limit: u64) -> Result<Vec<u8>> {
    wisp_io::read_bounded(path, limit)
}
impl Draft {
    pub fn from_image(path: &Path) -> Result<Self> {
        let bytes = read_source_image(path)?;
        Self::from_bytes(
            bytes,
            Design {
                name: path
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or("My wallpaper")
                    .chars()
                    .take(128)
                    .collect(),
                ..Design::default()
            },
        )
    }
    fn from_bytes(image: Vec<u8>, design: Design) -> Result<Self> {
        design.validate()?;
        let dimensions = decode_source_image(&image)?.dimensions();
        let extension = match image::guess_format(&image)? {
            image::ImageFormat::Png => "png",
            image::ImageFormat::Jpeg => "jpg",
            image::ImageFormat::WebP => "webp",
            _ => anyhow::bail!("choose a PNG, JPEG or WebP image"),
        }
        .into();
        Ok(Self {
            design,
            layers: None,
            image,
            extension,
            dimensions,
        })
    }
    pub fn graph(&self) -> Result<Graph> {
        self.design.validate()?;
        let d = &self.design;
        let mut nodes = vec![];
        let mut add = |id: &str, op| {
            nodes.push(Node { id: id.into(), op });
            id.to_owned()
        };
        let uv = add("uv", Op::Uv {});
        for c in CONTROLS {
            add(
                c.id,
                Op::Parameter {
                    parameter: c.id.into(),
                },
            );
        }
        let sample_uv = if d.parallax {
            add(
                "moving_uv",
                Op::Parallax {
                    uv: uv.clone(),
                    strength: "parallax".into(),
                },
            )
        } else {
            uv.clone()
        };
        let mut layer_nodes = vec![];
        let mut color = if let Some(stack) = &self.layers {
            stack.lower(&mut layer_nodes, &sample_uv)?
        } else {
            add(
                "image",
                Op::Image {
                    asset: "background".into(),
                    uv: sample_uv,
                },
            )
        };
        let brightness = if d.pulse {
            let time = add("time", Op::Time {});
            let phase = add(
                "phase",
                Op::Multiply {
                    a: time,
                    b: "speed".into(),
                },
            );
            let wave = add("wave", Op::Sin { input: phase });
            let amount = add(
                "variation",
                Op::Multiply {
                    a: wave,
                    b: "pulse".into(),
                },
            );
            add(
                "animated_brightness",
                Op::Add {
                    a: "brightness".into(),
                    b: amount,
                },
            )
        } else {
            "brightness".into()
        };
        color = add(
            "lit",
            Op::Brightness {
                color,
                amount: brightness,
            },
        );
        color = add(
            "graded",
            Op::Contrast {
                color,
                amount: "contrast".into(),
            },
        );
        if d.blur {
            // Keep intermediate work bounded even for a 4K source; retain aspect ratio.
            let scale = (1024. / self.dimensions.0.max(self.dimensions.1) as f32).min(1.);
            color = add(
                "soft",
                Op::Blur {
                    color,
                    radius: "radius".into(),
                    width: (self.dimensions.0 as f32 * scale).round().max(1.) as u32,
                    height: (self.dimensions.1 as f32 * scale).round().max(1.) as u32,
                },
            );
        }
        if d.vignette {
            color = add(
                "edges",
                Op::Vignette {
                    color,
                    uv,
                    strength: "vignette".into(),
                },
            );
        }
        let output = add("output", Op::Output { color });
        nodes.extend(layer_nodes);
        Ok(Graph {
            version: 1,
            nodes,
            output,
            parameters: CONTROLS
                .iter()
                .map(|c| Parameter {
                    id: c.id.into(),
                    default: ParameterValue::Float(d.values[c.id]),
                    min: Some(c.min as f64),
                    max: Some(c.max as f64),
                })
                .collect(),
        })
    }
    pub fn enable_layers(&mut self) -> &mut crate::layers::Stack {
        self.layers.get_or_insert_with(|| {
            crate::layers::Stack::new(Asset {
                extension: self.extension.clone(),
                bytes: self.image.clone(),
            })
        })
    }
    pub fn assets(&self) -> BTreeMap<String, Asset> {
        self.layers
            .as_ref()
            .map(|s| (*s.assets).clone())
            .unwrap_or_else(|| {
                BTreeMap::from([(
                    "background".into(),
                    Asset {
                        extension: self.extension.clone(),
                        bytes: self.image.clone(),
                    },
                )])
            })
    }
    pub fn compile(&self) -> Result<Compilation> {
        wisp_compiler::compile(
            &self.graph()?.check()?,
            &self.assets(),
            Manifest {
                format_version: 1,
                id: format!("local.easy.{:x}", Sha256::digest(&self.image)),
                name: self.design.name.trim().into(),
                author: self.design.author.trim().into(),
                description: "Created with Wisp Easy Designer".into(),
                tags: vec!["easy-designer".into()],
                preview: None,
                program: "program.json".into(),
                minimum_wisp_version: env!("CARGO_PKG_VERSION").into(),
                gpu_profile: 1,
            },
            &Default::default(),
        )
    }
    pub fn export(&self, destination: &Path) -> Result<()> {
        let bytes = self
            .compile()?
            .package
            .to_bytes(&PackageLimits::default())?;
        crate::library::write_new(destination, &bytes).context("choose a new export filename")?;
        Ok(())
    }
    /// A new directory keeps existing projects safe; contains both standard compiler
    /// inputs and Easy Mode settings. Assets are copied, so originals may be moved.
    pub fn save_project(&self, destination: &Path) -> Result<()> {
        self.compile()?;
        if let Some(stack) = &self.layers {
            let document = crate::graph::Document {
                graph: self.graph()?,
                project: Project {
                    version: 1,
                    id: format!("local.easy.{:x}", Sha256::digest(&self.image)),
                    name: self.design.name.trim().into(),
                    author: self.design.author.trim().into(),
                    description: "Created with Wisp Easy Designer".into(),
                    tags: vec!["easy-designer".into()],
                    assets: BTreeMap::new(),
                },
                assets: std::sync::Arc::new(self.assets()),
                layout: crate::graph::Layout {
                    version: 1,
                    positions: BTreeMap::new(),
                },
            };
            document.save(destination)?;
            let result = (|| -> Result<()> {
                crate::library::atomic_write(
                    &destination.join("designer.json"),
                    &serde_json::to_vec_pretty(&self.design)?,
                )?;
                crate::library::atomic_write(
                    &destination.join("layers.json"),
                    &serde_json::to_vec_pretty(stack)?,
                )?;
                Ok(())
            })();
            if result.is_err() {
                let _ = fs::remove_dir_all(destination);
            }
            return result;
        }
        fs::create_dir(destination).context("choose a new project folder name")?;
        let result = (|| -> Result<()> {
            fs::create_dir(destination.join("assets"))?;
            let path = format!("assets/background.{}", self.extension);
            crate::library::atomic_write(&destination.join(&path), &self.image)?;
            let project = Project {
                version: 1,
                id: format!("local.easy.{:x}", Sha256::digest(&self.image)),
                name: self.design.name.trim().into(),
                author: self.design.author.trim().into(),
                description: "Created with Wisp Easy Designer".into(),
                tags: vec!["easy-designer".into()],
                assets: BTreeMap::from([("background".into(), ProjectAsset { path })]),
            };
            crate::library::atomic_write(
                &destination.join("project.json"),
                &serde_json::to_vec_pretty(&project)?,
            )?;
            crate::library::atomic_write(
                &destination.join("graph.json"),
                &serde_json::to_vec_pretty(&self.graph()?)?,
            )?;
            crate::library::atomic_write(
                &destination.join("designer.json"),
                &serde_json::to_vec_pretty(&self.design)?,
            )?;
            Ok(())
        })();
        if result.is_err() {
            let _ = fs::remove_dir_all(destination);
        }
        result
    }
    pub fn open_project(directory: &Path) -> Result<Self> {
        let root = directory.canonicalize()?;
        let contained = |relative: &str| -> Result<std::path::PathBuf> {
            let path = root.join(relative).canonicalize()?;
            ensure!(path.starts_with(&root), "project file escapes folder");
            Ok(path)
        };
        let design: Design = serde_json::from_slice(&read(&contained("designer.json")?, 65536)?)?;
        let project: Project = serde_json::from_slice(&read(&contained("project.json")?, 65536)?)?;
        if root.join("layers.json").exists() {
            let document = crate::graph::Document::open(&root)?;
            let mut stack: crate::layers::Stack =
                serde_json::from_slice(&read(&contained("layers.json")?, 65536)?)?;
            stack.assets = document.assets;
            stack.validate()?;
            let background = stack
                .assets
                .get("background")
                .context("missing original background")?;
            let mut draft = Self::from_bytes(background.bytes.clone(), design)?;
            draft.layers = Some(stack);
            ensure!(
                serde_json::to_value(document.graph)? == serde_json::to_value(draft.graph()?)?,
                "graph was edited outside Easy Designer; open it in Graph instead"
            );
            ensure!(
                project.name == draft.design.name.trim()
                    && project.author == draft.design.author.trim(),
                "project metadata differs from designer settings"
            );
            // Verify the old generated graph before upgrading its layer composition semantics.
            let stack = draft.layers.as_mut().unwrap();
            stack.version = stack.version.max(2);
            draft.compile()?;
            return Ok(draft);
        }
        let asset = project
            .assets
            .get("background")
            .context("missing background")?;
        ensure!(
            [
                "assets/background.png",
                "assets/background.jpg",
                "assets/background.webp"
            ]
            .contains(&asset.path.as_str()),
            "invalid designer image path"
        );
        let draft = Self::from_bytes(read_source_image(&contained(&asset.path)?)?, design)?;
        // Refuse to silently discard externally edited authoring data.
        let graph: Graph = serde_json::from_slice(&read(&contained("graph.json")?, 1024 * 1024)?)?;
        ensure!(
            serde_json::to_value(graph)? == serde_json::to_value(draft.graph()?)?,
            "graph was edited outside Easy Designer; import it as a graph project instead"
        );
        ensure!(
            project.version == 1
                && project.name == draft.design.name.trim()
                && project.author == draft.design.author.trim(),
            "project metadata differs from designer settings"
        );
        draft.compile()?;
        Ok(draft)
    }
}
