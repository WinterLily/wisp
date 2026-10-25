//! Layer authoring lowers to ordinary typed nodes; assets are portable project data.
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, io::Cursor, path::Path, sync::Arc};
use wisp_compiler::Asset;
use wisp_format::ParameterValue as V;
use wisp_ir::{BlendMode, Node, Op, WeatherKind};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Effect {
    #[default]
    None,
    Hair,
    Clothing,
    Wave,
    Warp,
    Displacement,
    Droplets,
    Rain,
    Ripples,
}
impl Effect {
    pub const ALL: [Self; 9] = [
        Self::None,
        Self::Hair,
        Self::Clothing,
        Self::Wave,
        Self::Warp,
        Self::Displacement,
        Self::Droplets,
        Self::Rain,
        Self::Ripples,
    ];
}
impl std::fmt::Display for Effect {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{self:?}")
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Mask {
    pub asset: String,
    pub feather: f32,
    pub invert: bool,
    pub composition: bool,
    pub effect: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Layer {
    pub name: String,
    pub asset: String,
    pub visible: bool,
    pub opacity: f32,
    pub position: [f32; 2],
    pub scale: [f32; 2],
    pub rotation: f32,
    pub pivot: [f32; 2],
    pub blend: BlendMode,
    pub mask: Option<Mask>,
    pub effect: Effect,
    pub direction: f32,
    pub amplitude: f32,
    pub frequency: f32,
    pub speed: f32,
    pub density: f32,
    pub size: f32,
    #[serde(default)]
    pub rain_direction: f32,
    #[serde(default = "unit_scale")]
    pub size_x: f32,
    #[serde(default = "unit_scale")]
    pub size_y: f32,
    pub strength: f32,
    pub displacement: Option<String>,
}
fn unit_scale() -> f32 {
    1.0
}
impl Layer {
    pub fn new(name: String, asset: String) -> Self {
        Self {
            name,
            asset,
            visible: true,
            opacity: 1.,
            position: [0.; 2],
            scale: [1.; 2],
            rotation: 0.,
            pivot: [0.5; 2],
            blend: BlendMode::Alpha,
            mask: None,
            effect: Effect::None,
            direction: 0.,
            amplitude: 0.02,
            frequency: 1.5,
            speed: 1.,
            density: 10.,
            size: 0.25,
            rain_direction: 0.,
            size_x: 1.,
            size_y: 1.,
            strength: 0.5,
            displacement: None,
        }
    }
    pub fn preset(&mut self, effect: Effect) {
        self.effect = effect;
        self.amplitude = if effect == Effect::Clothing {
            0.012
        } else {
            0.025
        };
        self.frequency = if effect == Effect::Clothing { 2.5 } else { 1.2 };
        self.speed = if effect == Effect::Hair { 1.5 } else { 1. };
    }
    pub fn value(&self, id: &str) -> f32 {
        match id {
            "x" => self.position[0],
            "y" => self.position[1],
            "sx" => self.scale[0],
            "sy" => self.scale[1],
            "px" => self.pivot[0],
            "py" => self.pivot[1],
            "rotation" => self.rotation,
            "opacity" => self.opacity,
            "direction" => self.direction,
            "amplitude" => self.amplitude,
            "frequency" => self.frequency,
            "speed" => self.speed,
            "density" => self.density,
            "size" => self.size,
            "rain_direction" => self.rain_direction,
            "size_x" => self.size_x,
            "size_y" => self.size_y,
            "strength" => self.strength,
            "feather" => self.mask.as_ref().map_or(0., |m| m.feather),
            _ => 0.,
        }
    }
    pub fn set_value(&mut self, id: &str, value: f32) -> Result<()> {
        let c = CONTROLS
            .iter()
            .find(|c| c.0 == id)
            .context("unknown layer control")?;
        ensure!(
            value.is_finite() && (c.2..=c.3).contains(&value),
            "layer control out of range"
        );
        match id {
            "x" => self.position[0] = value,
            "y" => self.position[1] = value,
            "sx" => self.scale[0] = value,
            "sy" => self.scale[1] = value,
            "px" => self.pivot[0] = value,
            "py" => self.pivot[1] = value,
            "rotation" => self.rotation = value,
            "opacity" => self.opacity = value,
            "direction" => self.direction = value,
            "amplitude" => self.amplitude = value,
            "frequency" => self.frequency = value,
            "speed" => self.speed = value,
            "density" => self.density = value,
            "size" => self.size = value,
            "rain_direction" => self.rain_direction = value,
            "size_x" => self.size_x = value,
            "size_y" => self.size_y = value,
            "strength" => self.strength = value,
            "feather" => {
                if let Some(m) = &mut self.mask {
                    m.feather = value
                }
            }
            _ => {}
        }
        Ok(())
    }
}
/// (key, label, minimum, maximum); angles are degrees in authoring UI.
pub const CONTROLS: &[(&str, &str, f32, f32)] = &[
    ("opacity", "Opacity", 0., 1.),
    ("x", "Position X", -1., 1.),
    ("y", "Position Y", -1., 1.),
    ("sx", "Scale X", 0.05, 3.),
    ("sy", "Scale Y", 0.05, 3.),
    ("rotation", "Rotation", -180., 180.),
    ("px", "Pivot X", 0., 1.),
    ("py", "Pivot Y", 0., 1.),
    ("feather", "Mask feather", 0., 0.05),
    ("direction", "Motion direction", -180., 180.),
    ("amplitude", "Motion amplitude", 0., 0.15),
    ("frequency", "Motion frequency", 0., 16.),
    ("speed", "Effect speed", 0., 5.),
    ("density", "Weather density", 1., 32.),
    ("size", "Weather size", 0.03, 0.45),
    (
        "rain_direction",
        "Rain direction (0° down, 90° right)",
        -180.,
        180.,
    ),
    ("size_x", "Water width (X)", 0.2, 2.),
    ("size_y", "Water height (Y)", 0.2, 2.),
    ("strength", "Distortion / effect strength", 0., 1.),
];
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Stack {
    pub version: u32,
    /// Back to front.
    pub layers: Vec<Layer>,
    #[serde(skip)]
    pub assets: Arc<BTreeMap<String, Asset>>,
}
impl Stack {
    pub fn new(asset: Asset) -> Self {
        Self {
            version: 2,
            layers: vec![Layer::new("Background".into(), "background".into())],
            assets: Arc::new(BTreeMap::from([("background".into(), asset)])),
        }
    }
    pub fn validate(&self) -> Result<()> {
        ensure!(
            (1..=2).contains(&self.version) && (1..=6).contains(&self.layers.len()),
            "layers require version 1 or 2 and 1–6 layers"
        );
        ensure!(self.assets.len() <= 20, "layer source assets exceed limit");
        for l in &self.layers {
            ensure!(
                !l.name.is_empty() && l.name.len() <= 256,
                "invalid layer name"
            );
            ensure!(self.assets.contains_key(&l.asset), "missing layer image");
            for c in CONTROLS {
                let v = l.value(c.0);
                ensure!(v.is_finite() && (c.2..=c.3).contains(&v), "invalid {}", c.1);
            }
            if let Some(m) = &l.mask {
                ensure!(self.assets.contains_key(&m.asset), "missing mask image");
            }
            if let Some(id) = &l.displacement {
                ensure!(self.assets.contains_key(id), "missing displacement image");
            }
        }
        Ok(())
    }
    fn insert(&mut self, asset: Asset) -> Result<String> {
        ensure!(self.assets.len() < 20, "layer source asset budget exceeded");
        let id = (0..)
            .map(|i| format!("layer_asset{i}"))
            .find(|id| !self.assets.contains_key(id))
            .unwrap();
        Arc::make_mut(&mut self.assets).insert(id.clone(), asset);
        Ok(id)
    }
    pub fn import(&mut self, path: &Path, target: Option<(usize, bool)>) -> Result<()> {
        let bytes = wisp_compiler::read_source_image(path)?;
        let decoded = wisp_compiler::decode_source_image(&bytes)?;
        if let Some((i, _)) = target {
            ensure!(i < self.layers.len(), "missing layer");
        } else {
            ensure!(self.layers.len() < 6, "maximum six layers");
        }
        let asset = if matches!(target, Some((_, false))) {
            let image = decoded.into_rgba8();
            let gray = image::GrayImage::from_fn(image.width(), image.height(), |x, y| {
                let p = image.get_pixel(x, y).0;
                image::Luma([
                    ((0.2126 * p[0] as f32 + 0.7152 * p[1] as f32 + 0.0722 * p[2] as f32)
                        * p[3] as f32
                        / 255.)
                        .round() as u8,
                ])
            });
            encode(image::DynamicImage::ImageLuma8(gray))?
        } else {
            let extension = match image::guess_format(&bytes)? {
                image::ImageFormat::Png => "png",
                image::ImageFormat::Jpeg => "jpg",
                image::ImageFormat::WebP => "webp",
                _ => anyhow::bail!("choose PNG, JPEG or WebP"),
            };
            Asset {
                extension: extension.into(),
                bytes,
            }
        };
        let id = self.insert(asset)?;
        if let Some((i, displacement)) = target {
            if displacement {
                self.layers[i].displacement = Some(id);
            } else {
                self.layers[i].mask = Some(Mask {
                    asset: id,
                    feather: 0.,
                    invert: false,
                    composition: false,
                    effect: true,
                });
            }
        } else {
            self.layers.push(Layer::new(
                path.file_stem()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .chars()
                    .take(128)
                    .collect(),
                id,
            ));
        }
        self.prune();
        Ok(())
    }
    pub fn blank_mask(&mut self, i: usize, white: bool) -> Result<()> {
        ensure!(i < self.layers.len(), "missing layer");
        let image =
            image::GrayImage::from_pixel(256, 256, image::Luma([if white { 255 } else { 0 }]));
        let id = self.insert(encode(image::DynamicImage::ImageLuma8(image))?)?;
        self.layers[i].mask = Some(Mask {
            asset: id,
            feather: 0.005,
            invert: false,
            composition: false,
            effect: true,
        });
        self.prune();
        Ok(())
    }
    /// Paint a continuous capsule in layer-local image space. Values are grayscale.
    pub fn paint(
        &mut self,
        i: usize,
        from: [f32; 2],
        to: [f32; 2],
        radius: f32,
        value: u8,
    ) -> Result<()> {
        let id = self
            .layers
            .get(i)
            .and_then(|l| l.mask.as_ref())
            .context("create a mask first")?
            .asset
            .clone();
        let mut image = wisp_compiler::decode_source_image(&self.assets[&id].bytes)?
            .resize_exact(256, 256, image::imageops::FilterType::Triangle)
            .into_luma8();
        crate::painting::paint_brush(&mut image, from, to, radius, value, 1., None)?;
        Arc::make_mut(&mut self.assets).insert(id, encode(image::DynamicImage::ImageLuma8(image))?);
        Ok(())
    }
    pub fn prune(&mut self) {
        let ids: std::collections::BTreeSet<_> = self
            .layers
            .iter()
            .flat_map(|l| {
                std::iter::once(l.asset.clone())
                    .chain(l.mask.iter().map(|m| m.asset.clone()))
                    .chain(l.displacement.clone())
            })
            .chain(std::iter::once("background".into()))
            .collect();
        Arc::make_mut(&mut self.assets).retain(|id, _| ids.contains(id));
    }
    pub fn lower(&self, nodes: &mut Vec<Node>, uv: &str) -> Result<String> {
        self.validate()?;
        let mut b = Builder { nodes, serial: 0 };
        let one = b.constant(V::Float(1.));
        let mut composed = b.constant(V::Color([0., 0., 0., 0.]));
        for l in &self.layers {
            if !l.visible {
                continue;
            }
            let position = b.constant(V::Vector2(l.position));
            let scale = b.constant(V::Vector2(l.scale));
            let rotation = b.constant(V::Float(l.rotation.to_radians()));
            let pivot = b.constant(V::Vector2(l.pivot));
            let local = b.add(Op::Transform {
                uv: uv.into(),
                position,
                scale,
                rotation,
                pivot,
            });
            let mut mask = one.clone();
            if let Some(m) = &l.mask {
                let color = b.add(Op::Image {
                    asset: m.asset.clone(),
                    uv: local.clone(),
                });
                let feather = b.constant(V::Float(m.feather));
                let invert = b.constant(V::Boolean(m.invert));
                mask = b.add(Op::Mask {
                    color,
                    feather,
                    invert,
                });
            }
            let effect_mask = if l.mask.as_ref().is_some_and(|m| m.effect) {
                mask.clone()
            } else {
                one.clone()
            };
            let composition_mask = if l.mask.as_ref().is_some_and(|m| m.composition) {
                mask
            } else {
                one.clone()
            };
            let mut sample_uv = local.clone();
            let speed = b.constant(V::Float(l.speed));
            if matches!(
                l.effect,
                Effect::Hair | Effect::Clothing | Effect::Wave | Effect::Warp
            ) {
                let direction = b.constant(V::Vector2([
                    l.direction.to_radians().cos(),
                    l.direction.to_radians().sin(),
                ]));
                let amplitude = b.constant(V::Float(l.amplitude));
                let frequency = b.constant(V::Float(l.frequency));
                // Hair/clothing never move without an explicit selected region.
                let mask =
                    if matches!(l.effect, Effect::Hair | Effect::Clothing) && l.mask.is_none() {
                        b.constant(V::Float(0.))
                    } else {
                        effect_mask.clone()
                    };
                sample_uv = b.add(if matches!(l.effect, Effect::Clothing | Effect::Warp) {
                    Op::Warp {
                        uv: local.clone(),
                        mask,
                        direction,
                        amplitude,
                        frequency,
                        speed: speed.clone(),
                    }
                } else {
                    Op::Wave {
                        uv: local.clone(),
                        mask,
                        direction,
                        amplitude,
                        frequency,
                        speed: speed.clone(),
                    }
                });
            }
            if l.effect == Effect::Displacement
                && let Some(map) = &l.displacement
            {
                let map = b.add(Op::Image {
                    asset: map.clone(),
                    uv: local.clone(),
                });
                let strength = b.constant(V::Float(l.strength * 0.15));
                sample_uv = b.add(Op::Displacement {
                    uv: local.clone(),
                    map,
                    mask: effect_mask.clone(),
                    strength,
                });
            }
            let mut color = b.add(Op::Image {
                asset: l.asset.clone(),
                uv: sample_uv.clone(),
            });
            if self.version == 1
                && matches!(l.effect, Effect::Droplets | Effect::Rain | Effect::Ripples)
            {
                let density = b.constant(V::Float(l.density));
                let size = b.constant(V::Float(l.size));
                let strength = b.constant(V::Float(l.strength));
                color = b.add(Op::Weather {
                    rain_direction: l.rain_direction,
                    size_x: l.size_x,
                    size_y: l.size_y,
                    color,
                    uv: local.clone(),
                    mask: effect_mask.clone(),
                    density,
                    size,
                    speed: speed.clone(),
                    strength,
                    kind: match l.effect {
                        Effect::Rain => WeatherKind::Rain,
                        Effect::Ripples => WeatherKind::Ripples,
                        _ => WeatherKind::Droplets,
                    },
                });
            }
            let opacity = b.constant(V::Float(l.opacity));
            color = b.add(Op::Layer {
                color,
                uv: sample_uv,
                opacity: opacity.clone(),
                mask: composition_mask,
            });
            composed = b.add(Op::Blend {
                a: composed,
                b: color,
                mode: l.blend,
            });
            if self.version >= 2
                && matches!(l.effect, Effect::Droplets | Effect::Rain | Effect::Ripples)
            {
                let density = b.constant(V::Float(l.density));
                let size = b.constant(V::Float(l.size));
                let strength = b.constant(V::Float(l.strength));
                let mask = b.add(Op::Multiply {
                    a: effect_mask,
                    b: opacity,
                });
                composed = b.add(Op::Weather {
                    rain_direction: l.rain_direction,
                    size_x: l.size_x,
                    size_y: l.size_y,
                    color: composed,
                    uv: local,
                    mask,
                    density,
                    size,
                    speed,
                    strength,
                    kind: match l.effect {
                        Effect::Rain => WeatherKind::Rain,
                        Effect::Ripples => WeatherKind::Ripples,
                        _ => WeatherKind::Droplets,
                    },
                });
            }
        }
        Ok(composed)
    }
}
pub(crate) fn encode(image: image::DynamicImage) -> Result<Asset> {
    let mut bytes = Cursor::new(vec![]);
    image.write_to(&mut bytes, image::ImageFormat::Png)?;
    Ok(Asset {
        extension: "png".into(),
        bytes: bytes.into_inner(),
    })
}
struct Builder<'a> {
    nodes: &'a mut Vec<Node>,
    serial: usize,
}
impl Builder<'_> {
    fn add(&mut self, op: Op) -> String {
        let id = format!("layer_n{}", self.serial);
        self.serial += 1;
        self.nodes.push(Node { id: id.clone(), op });
        id
    }
    fn constant(&mut self, value: V) -> String {
        self.add(Op::Constant { value })
    }
}
