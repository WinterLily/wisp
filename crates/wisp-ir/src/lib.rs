//! Versioned authoring graph and checked, topologically ordered compiler input.
//! This crate is never linked into the wallpaper daemon.
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use wisp_format::{Parameter, ParameterValue, validate_parameters};

pub const GRAPH_VERSION: u32 = 1;
fn unit_scale() -> f32 {
    1.0
}

pub const MAX_NODES: usize = 256;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ValueType {
    Float,
    Integer,
    Boolean,
    Vector2,
    Vector3,
    Vector4,
    Color,
}
impl ValueType {
    pub fn of(value: &ParameterValue) -> Self {
        match value {
            ParameterValue::Float(_) => Self::Float,
            ParameterValue::Integer(_) => Self::Integer,
            ParameterValue::Boolean(_) => Self::Boolean,
            ParameterValue::Vector2(_) => Self::Vector2,
            ParameterValue::Vector3(_) => Self::Vector3,
            ParameterValue::Vector4(_) => Self::Vector4,
            ParameterValue::Color(_) => Self::Color,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Graph {
    pub version: u32,
    pub nodes: Vec<Node>,
    pub output: String,
    #[serde(default)]
    pub parameters: Vec<Parameter>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Node {
    pub id: String,
    pub op: Op,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BlendMode {
    #[default]
    Alpha,
    Add,
    Multiply,
    Screen,
    Overlay,
}
impl BlendMode {
    pub const ALL: [Self; 5] = [
        Self::Alpha,
        Self::Add,
        Self::Multiply,
        Self::Screen,
        Self::Overlay,
    ];
}
impl std::fmt::Display for BlendMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{self:?}")
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WeatherKind {
    Droplets,
    Rain,
    Ripples,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum Op {
    /// Inverse image transform; position/scale/pivot are normalized image coordinates.
    Transform {
        uv: String,
        position: String,
        scale: String,
        rotation: String,
        pivot: String,
    },
    /// Grayscale coverage, feathered in normalized coordinates, then optionally inverted.
    Mask {
        color: String,
        feather: String,
        invert: String,
    },
    Layer {
        color: String,
        uv: String,
        opacity: String,
        mask: String,
    },
    Blend {
        a: String,
        b: String,
        mode: BlendMode,
    },
    Wave {
        uv: String,
        mask: String,
        direction: String,
        amplitude: String,
        frequency: String,
        speed: String,
    },
    Warp {
        uv: String,
        mask: String,
        direction: String,
        amplitude: String,
        frequency: String,
        speed: String,
    },
    Displacement {
        uv: String,
        map: String,
        mask: String,
        strength: String,
    },
    /// Analytic bounded weather; no history is required for these periodic presets.
    Weather {
        color: String,
        uv: String,
        mask: String,
        density: String,
        size: String,
        speed: String,
        strength: String,
        kind: WeatherKind,
        /// Clockwise degrees from downward; independent of layer transforms.
        #[serde(default)]
        rain_direction: f32,
        /// Width across travel and height along travel, relative to `size`.
        #[serde(default = "unit_scale")]
        size_x: f32,
        #[serde(default = "unit_scale")]
        size_y: f32,
    },

    Constant {
        value: ParameterValue,
    },
    Parameter {
        parameter: String,
    },
    Uv {},
    Time {},
    DeltaTime {},
    Resolution {},
    Mouse {},
    AudioBass {},
    AudioMid {},
    AudioTreble {},
    AudioVolume {},
    Add {
        a: String,
        b: String,
    },
    Multiply {
        a: String,
        b: String,
    },
    Sin {
        input: String,
    },
    Translate {
        uv: String,
        offset: String,
    },
    Parallax {
        uv: String,
        strength: String,
    },
    MouseDistance {
        uv: String,
    },
    /// Persistent emitter, advanced once per rendered frame.
    Particles {
        position: String,
        color: String,
        speed: String,
        size: String,
        count: u32,
        width: u32,
        height: u32,
    },
    /// Per-pixel temporal peak hold, with decay specified per 1/60 second.
    Feedback {
        color: String,
        decay: String,
        width: u32,
        height: u32,
    },
    Image {
        asset: String,
        uv: String,
    },
    Brightness {
        color: String,
        amount: String,
    },
    Contrast {
        color: String,
        amount: String,
    },
    Tint {
        color: String,
        tint: String,
    },
    Vignette {
        color: String,
        uv: String,
        strength: String,
    },
    Mix {
        a: String,
        b: String,
        factor: String,
    },
    /// Separable five-tap Gaussian. Radius is a spacing in intermediate pixels.
    Blur {
        color: String,
        radius: String,
        width: u32,
        height: u32,
    },
    /// Explicit render-to-texture boundary, useful for sharing an expensive branch.
    Materialize {
        color: String,
        width: u32,
        height: u32,
    },
    Output {
        color: String,
    },
}
impl Op {
    pub fn inputs(&self) -> Vec<&str> {
        match self {
            Self::Transform {
                uv,
                position,
                scale,
                rotation,
                pivot,
            } => vec![uv, position, scale, rotation, pivot],
            Self::Mask {
                color,
                feather,
                invert,
            } => vec![color, feather, invert],
            Self::Layer {
                color,
                uv,
                opacity,
                mask,
            } => vec![color, uv, opacity, mask],
            Self::Blend { a, b, .. } => vec![a, b],
            Self::Wave {
                uv,
                mask,
                direction,
                amplitude,
                frequency,
                speed,
            }
            | Self::Warp {
                uv,
                mask,
                direction,
                amplitude,
                frequency,
                speed,
            } => vec![uv, mask, direction, amplitude, frequency, speed],
            Self::Displacement {
                uv,
                map,
                mask,
                strength,
            } => vec![uv, map, mask, strength],
            Self::Weather {
                color,
                uv,
                mask,
                density,
                size,
                speed,
                strength,
                ..
            } => vec![color, uv, mask, density, size, speed, strength],
            Self::Constant { .. }
            | Self::Parameter { .. }
            | Self::Uv {}
            | Self::Time {}
            | Self::DeltaTime {}
            | Self::Resolution {}
            | Self::Mouse {}
            | Self::AudioBass {}
            | Self::AudioMid {}
            | Self::AudioTreble {}
            | Self::AudioVolume {} => vec![],
            Self::Add { a, b } | Self::Multiply { a, b } | Self::Tint { color: a, tint: b } => {
                vec![a, b]
            }
            Self::Sin { input }
            | Self::Output { color: input }
            | Self::Materialize { color: input, .. } => vec![input],
            Self::Translate { uv, offset } => vec![uv, offset],
            Self::Parallax { uv, strength } => vec![uv, strength],
            Self::Image { uv, .. } | Self::MouseDistance { uv } => vec![uv],
            Self::Particles {
                position,
                color,
                speed,
                size,
                ..
            } => vec![position, color, speed, size],
            Self::Feedback { color, decay, .. } => vec![color, decay],
            Self::Brightness { color, amount } | Self::Contrast { color, amount } => {
                vec![color, amount]
            }
            Self::Vignette {
                color,
                uv,
                strength,
            } => vec![color, uv, strength],
            Self::Mix { a, b, factor } => vec![a, b, factor],
            Self::Blur { color, radius, .. } => vec![color, radius],
        }
    }
}

/// Private construction guarantees every node and edge is valid, including dead nodes.
/// `live_order` contains only nodes reachable from the selected Output node.
pub struct TypedGraph {
    graph: Graph,
    types: BTreeMap<String, ValueType>,
    indices: BTreeMap<String, usize>,
    live_order: Vec<usize>,
}
impl TypedGraph {
    pub fn graph(&self) -> &Graph {
        &self.graph
    }
    pub fn live_order(&self) -> &[usize] {
        &self.live_order
    }
    pub fn node(&self, id: &str) -> &Node {
        &self.graph.nodes[self.indices[id]]
    }
    pub fn index(&self, id: &str) -> usize {
        self.indices[id]
    }
    pub fn value_type(&self, id: &str) -> ValueType {
        self.types[id]
    }
}

fn identifier(id: &str) -> Result<()> {
    ensure!(
        !id.is_empty()
            && id.len() <= 128
            && id
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"-_.".contains(&b)),
        "invalid node identifier: {id:?}"
    );
    Ok(())
}
impl Graph {
    pub fn check(self) -> Result<TypedGraph> {
        ensure!(
            self.version == GRAPH_VERSION,
            "unsupported graph version: {}",
            self.version
        );
        ensure!(
            (1..=MAX_NODES).contains(&self.nodes.len()),
            "graph requires 1..={MAX_NODES} nodes"
        );
        validate_parameters(&self.parameters)?;
        let mut indices = BTreeMap::new();
        for (i, node) in self.nodes.iter().enumerate() {
            identifier(&node.id)?;
            ensure!(
                indices.insert(node.id.clone(), i).is_none(),
                "duplicate node: {}",
                node.id
            );
        }
        let output = *indices.get(&self.output).context("missing graph output")?;
        ensure!(
            matches!(self.nodes[output].op, Op::Output { .. }),
            "graph output must select an Output node"
        );
        // Bounded DFS: at most MAX_NODES frames. Check all components before pruning.
        fn visit(
            i: usize,
            graph: &Graph,
            indices: &BTreeMap<String, usize>,
            state: &mut [u8],
            order: &mut Vec<usize>,
        ) -> Result<()> {
            ensure!(state[i] != 1, "cycle at node {}", graph.nodes[i].id);
            if state[i] == 2 {
                return Ok(());
            }
            state[i] = 1;
            for input in graph.nodes[i].op.inputs() {
                let j = *indices.get(input).with_context(|| {
                    format!("node {} references missing node {input}", graph.nodes[i].id)
                })?;
                ensure!(
                    !matches!(graph.nodes[j].op, Op::Output { .. }),
                    "Output node {input} cannot be used as an input"
                );
                visit(j, graph, indices, state, order)?;
            }
            state[i] = 2;
            order.push(i);
            Ok(())
        }
        let mut order = vec![];
        let mut state = vec![0; self.nodes.len()];
        for i in 0..self.nodes.len() {
            visit(i, &self, &indices, &mut state, &mut order)?;
        }
        let mut types = BTreeMap::new();
        for i in order {
            let node = &self.nodes[i];
            let expect = |id: &str, ty| -> Result<()> {
                ensure!(
                    types[id] == ty,
                    "node {} input {id}: expected {ty:?}, found {:?}",
                    node.id,
                    types[id]
                );
                Ok(())
            };
            use ValueType::*;
            let ty = match &node.op {
                Op::Transform {
                    uv,
                    position,
                    scale,
                    rotation,
                    pivot,
                } => {
                    for input in [uv, position, scale, pivot] {
                        expect(input, Vector2)?;
                    }
                    expect(rotation, Float)?;
                    Vector2
                }
                Op::Mask {
                    color,
                    feather,
                    invert,
                } => {
                    expect(color, Color)?;
                    expect(feather, Float)?;
                    expect(invert, Boolean)?;
                    Float
                }
                Op::Layer {
                    color,
                    uv,
                    opacity,
                    mask,
                } => {
                    expect(color, Color)?;
                    expect(uv, Vector2)?;
                    expect(opacity, Float)?;
                    expect(mask, Float)?;
                    Color
                }
                Op::Blend { a, b, .. } => {
                    expect(a, Color)?;
                    expect(b, Color)?;
                    Color
                }
                Op::Wave {
                    uv,
                    mask,
                    direction,
                    amplitude,
                    frequency,
                    speed,
                }
                | Op::Warp {
                    uv,
                    mask,
                    direction,
                    amplitude,
                    frequency,
                    speed,
                } => {
                    expect(uv, Vector2)?;
                    expect(direction, Vector2)?;
                    for input in [mask, amplitude, frequency, speed] {
                        expect(input, Float)?;
                    }
                    Vector2
                }
                Op::Displacement {
                    uv,
                    map,
                    mask,
                    strength,
                } => {
                    expect(uv, Vector2)?;
                    expect(map, Color)?;
                    expect(mask, Float)?;
                    expect(strength, Float)?;
                    Vector2
                }
                Op::Weather {
                    color,
                    uv,
                    mask,
                    density,
                    size,
                    speed,
                    strength,
                    rain_direction,
                    size_x,
                    size_y,
                    ..
                } => {
                    ensure!(
                        rain_direction.is_finite() && (-180.0..=180.0).contains(rain_direction),
                        "node {}: rain direction must be −180–180 degrees",
                        node.id
                    );
                    ensure!(
                        [size_x, size_y]
                            .iter()
                            .all(|v| v.is_finite() && (0.2..=2.0).contains(*v)),
                        "node {}: water axis scales must be 0.2–2",
                        node.id
                    );
                    expect(color, Color)?;
                    expect(uv, Vector2)?;
                    for input in [mask, density, size, speed, strength] {
                        expect(input, Float)?;
                    }
                    Color
                }

                Op::Constant { value } => {
                    Parameter {
                        id: node.id.clone(),
                        default: value.clone(),
                        min: None,
                        max: None,
                    }
                    .validate()?;
                    ValueType::of(value)
                }
                Op::Parameter { parameter } => ValueType::of(
                    &self
                        .parameters
                        .iter()
                        .find(|p| &p.id == parameter)
                        .with_context(|| {
                            format!("node {}: unknown parameter {parameter}", node.id)
                        })?
                        .default,
                ),
                Op::Uv {} | Op::Mouse {} | Op::Resolution {} => Vector2,
                Op::Time {}
                | Op::DeltaTime {}
                | Op::AudioBass {}
                | Op::AudioMid {}
                | Op::AudioTreble {}
                | Op::AudioVolume {} => Float,
                Op::Add { a, b } | Op::Multiply { a, b } => {
                    let ty = types[a.as_str()];
                    ensure!(
                        matches!(ty, Float | Vector2 | Vector3 | Vector4 | Color),
                        "node {}: arithmetic requires float or vector",
                        node.id
                    );
                    expect(b, ty)?;
                    ty
                }
                Op::Sin { input } => {
                    expect(input, Float)?;
                    Float
                }
                Op::Translate { uv, offset } => {
                    expect(uv, Vector2)?;
                    expect(offset, Vector2)?;
                    Vector2
                }
                Op::Parallax { uv, strength } => {
                    expect(uv, Vector2)?;
                    expect(strength, Float)?;
                    Vector2
                }
                Op::MouseDistance { uv } => {
                    expect(uv, Vector2)?;
                    Float
                }
                Op::Particles {
                    position,
                    color,
                    speed,
                    size,
                    count,
                    width,
                    height,
                } => {
                    expect(position, Vector2)?;
                    expect(color, Color)?;
                    expect(speed, Float)?;
                    expect(size, Float)?;
                    ensure!(
                        (1..=128).contains(count),
                        "node {}: particle count must be 1..=128",
                        node.id
                    );
                    ensure!(
                        (1..=1024).contains(width) && (1..=1024).contains(height),
                        "node {}: particle dimensions must be 1..=1024",
                        node.id
                    );
                    ensure!(
                        u64::from(*count) * u64::from(*width) * u64::from(*height)
                            <= 16 * 1024 * 1024,
                        "node {}: particle pixel workload exceeds 16M",
                        node.id
                    );
                    Color
                }
                Op::Feedback {
                    color,
                    decay,
                    width,
                    height,
                } => {
                    expect(color, Color)?;
                    expect(decay, Float)?;
                    ensure!(
                        (1..=1024).contains(width) && (1..=1024).contains(height),
                        "node {}: feedback dimensions must be 1..=1024",
                        node.id
                    );
                    Color
                }
                Op::Image { asset, uv } => {
                    identifier(asset)?;
                    expect(uv, Vector2)?;
                    Color
                }
                Op::Brightness { color, amount } | Op::Contrast { color, amount } => {
                    expect(color, Color)?;
                    expect(amount, Float)?;
                    Color
                }
                Op::Tint { color, tint } => {
                    expect(color, Color)?;
                    expect(tint, Color)?;
                    Color
                }
                Op::Vignette {
                    color,
                    uv,
                    strength,
                } => {
                    expect(color, Color)?;
                    expect(uv, Vector2)?;
                    expect(strength, Float)?;
                    Color
                }
                Op::Mix { a, b, factor } => {
                    expect(a, Color)?;
                    expect(b, Color)?;
                    expect(factor, Float)?;
                    Color
                }
                Op::Blur {
                    color,
                    radius,
                    width,
                    height,
                } => {
                    expect(color, Color)?;
                    expect(radius, Float)?;
                    ensure!(
                        (1..=4096).contains(width) && (1..=4096).contains(height),
                        "node {}: blur dimensions must be 1..=4096",
                        node.id
                    );
                    Color
                }
                Op::Output { color } => {
                    expect(color, Color)?;
                    Color
                }
                Op::Materialize {
                    color,
                    width,
                    height,
                } => {
                    expect(color, Color)?;
                    ensure!(
                        (1..=4096).contains(width) && (1..=4096).contains(height),
                        "node {}: materialize dimensions must be 1..=4096",
                        node.id
                    );
                    Color
                }
            };
            types.insert(node.id.clone(), ty);
        }
        let mut live_order = vec![];
        visit(
            output,
            &self,
            &indices,
            &mut vec![0; self.nodes.len()],
            &mut live_order,
        )?;
        // Keep deterministic graph order only for diagnostics; lowering uses dependency order.
        let live: BTreeSet<_> = live_order.iter().copied().collect();
        debug_assert_eq!(live.len(), live_order.len());
        Ok(TypedGraph {
            graph: self,
            types,
            indices,
            live_order,
        })
    }
}
