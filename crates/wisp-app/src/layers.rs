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
    pub fn controls(self) -> &'static [ControlId] {
        use ControlId::*;
        match self {
            Self::None => &[],
            Self::Hair | Self::Clothing | Self::Wave | Self::Warp => {
                &[Direction, Amplitude, Frequency, Speed]
            }
            Self::Displacement => &[Strength],
            Self::Droplets | Self::Ripples => &[Speed, Density, Size, SizeX, SizeY, Strength],
            Self::Rain => &[Speed, Density, Size, RainDirection, SizeX, SizeY, Strength],
        }
    }

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
impl Mask {
    fn new(asset: String, composition: bool) -> Self {
        Self {
            asset,
            feather: 0.,
            invert: false,
            composition,
            effect: true,
        }
    }
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
    // Preserve the legacy JSON shape until its generated graph has been verified on open.
    #[serde(flatten)]
    pub motion: EffectSettings,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub effects: Vec<LayerEffect>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct EffectSettings {
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

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LayerEffect {
    pub visible: bool,
    pub mask: Option<Mask>,
    pub settings: EffectSettings,
}

impl LayerEffect {
    pub fn new(effect: Effect) -> Self {
        let mut settings = EffectSettings::default();
        settings.preset(effect);
        Self {
            visible: true,
            mask: None,
            settings,
        }
    }

    pub fn value(&self, id: ControlId) -> f32 {
        if id == ControlId::Feather {
            self.mask.as_ref().map_or(0., |mask| mask.feather)
        } else {
            self.settings.value(id)
        }
    }

    pub fn set_value(&mut self, id: ControlId, value: f32) -> Result<()> {
        validate_control(id, value)?;
        if id == ControlId::Feather {
            self.mask.as_mut().context("effect has no mask")?.feather = value;
            Ok(())
        } else {
            self.settings.set_value(id, value)
        }
    }
}

impl std::ops::Deref for Layer {
    type Target = EffectSettings;
    fn deref(&self) -> &Self::Target {
        &self.motion
    }
}
impl std::ops::DerefMut for Layer {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.motion
    }
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
            motion: EffectSettings::default(),
            effects: vec![],
        }
    }
    pub fn preset(&mut self, effect: Effect) {
        self.motion.preset(effect);
    }
    pub fn value(&self, id: ControlId) -> f32 {
        match id {
            ControlId::PositionX => self.position[0],
            ControlId::PositionY => self.position[1],
            ControlId::ScaleX => self.scale[0],
            ControlId::ScaleY => self.scale[1],
            ControlId::PivotX => self.pivot[0],
            ControlId::PivotY => self.pivot[1],
            ControlId::Rotation => self.rotation,
            ControlId::Opacity => self.opacity,
            id if id.is_effect() => self.motion.value(id),
            ControlId::Feather => self.mask.as_ref().map_or(0., |m| m.feather),
            _ => unreachable!(),
        }
    }
    pub fn set_value(
        &mut self,
        id: impl TryInto<ControlId, Error: std::fmt::Display>,
        value: f32,
    ) -> Result<()> {
        let id = id.try_into().map_err(|e| anyhow::anyhow!("{e}"))?;
        validate_control(id, value)?;
        match id {
            ControlId::PositionX => self.position[0] = value,
            ControlId::PositionY => self.position[1] = value,
            ControlId::ScaleX => self.scale[0] = value,
            ControlId::ScaleY => self.scale[1] = value,
            ControlId::PivotX => self.pivot[0] = value,
            ControlId::PivotY => self.pivot[1] = value,
            ControlId::Rotation => self.rotation = value,
            ControlId::Opacity => self.opacity = value,
            id if id.is_effect() => self.motion.set_value(id, value)?,
            ControlId::Feather => {
                self.mask.as_mut().context("layer has no mask")?.feather = value;
            }
            _ => unreachable!(),
        }
        Ok(())
    }
}
impl Default for EffectSettings {
    fn default() -> Self {
        Self {
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
}
impl EffectSettings {
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

    pub fn value(&self, id: ControlId) -> f32 {
        match id {
            ControlId::Direction => self.direction,
            ControlId::Amplitude => self.amplitude,
            ControlId::Frequency => self.frequency,
            ControlId::Speed => self.speed,
            ControlId::Density => self.density,
            ControlId::Size => self.size,
            ControlId::RainDirection => self.rain_direction,
            ControlId::SizeX => self.size_x,
            ControlId::SizeY => self.size_y,
            ControlId::Strength => self.strength,
            _ => 0.,
        }
    }
    pub fn set_value(&mut self, id: ControlId, value: f32) -> Result<()> {
        validate_control(id, value)?;
        match id {
            ControlId::Direction => self.direction = value,
            ControlId::Amplitude => self.amplitude = value,
            ControlId::Frequency => self.frequency = value,
            ControlId::Speed => self.speed = value,
            ControlId::Density => self.density = value,
            ControlId::Size => self.size = value,
            ControlId::RainDirection => self.rain_direction = value,
            ControlId::SizeX => self.size_x = value,
            ControlId::SizeY => self.size_y = value,
            ControlId::Strength => self.strength = value,
            _ => anyhow::bail!("not an effect control"),
        }
        Ok(())
    }
}
fn validate_control(id: ControlId, value: f32) -> Result<()> {
    let control = id.metadata();
    ensure!(
        value.is_finite() && (control.min..=control.max).contains(&value),
        "layer control out of range"
    );
    Ok(())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ControlId {
    Opacity,
    PositionX,
    PositionY,
    ScaleX,
    ScaleY,
    Rotation,
    PivotX,
    PivotY,
    Feather,
    Direction,
    Amplitude,
    Frequency,
    Speed,
    Density,
    Size,
    RainDirection,
    SizeX,
    SizeY,
    Strength,
}
pub struct Control {
    pub id: ControlId,
    pub key: &'static str,
    pub label: &'static str,
    pub min: f32,
    pub max: f32,
}
impl ControlId {
    fn is_effect(self) -> bool {
        matches!(
            self,
            Self::Direction
                | Self::Amplitude
                | Self::Frequency
                | Self::Speed
                | Self::Density
                | Self::Size
                | Self::RainDirection
                | Self::SizeX
                | Self::SizeY
                | Self::Strength
        )
    }

    pub fn metadata(self) -> &'static Control {
        CONTROLS
            .iter()
            .find(|control| control.id == self)
            .expect("every control has metadata")
    }
}
impl TryFrom<&str> for ControlId {
    type Error = anyhow::Error;
    fn try_from(key: &str) -> Result<Self> {
        CONTROLS
            .iter()
            .find(|control| control.key == key)
            .map(|control| control.id)
            .context("unknown layer control")
    }
}
pub const CONTROLS: &[Control] = &[
    Control {
        id: ControlId::Opacity,
        key: "opacity",
        label: "Opacity",
        min: 0.,
        max: 1.,
    },
    Control {
        id: ControlId::PositionX,
        key: "x",
        label: "Position X",
        min: -1.,
        max: 1.,
    },
    Control {
        id: ControlId::PositionY,
        key: "y",
        label: "Position Y",
        min: -1.,
        max: 1.,
    },
    Control {
        id: ControlId::ScaleX,
        key: "sx",
        label: "Scale X",
        min: 0.05,
        max: 3.,
    },
    Control {
        id: ControlId::ScaleY,
        key: "sy",
        label: "Scale Y",
        min: 0.05,
        max: 3.,
    },
    Control {
        id: ControlId::Rotation,
        key: "rotation",
        label: "Rotation",
        min: -180.,
        max: 180.,
    },
    Control {
        id: ControlId::PivotX,
        key: "px",
        label: "Pivot X",
        min: 0.,
        max: 1.,
    },
    Control {
        id: ControlId::PivotY,
        key: "py",
        label: "Pivot Y",
        min: 0.,
        max: 1.,
    },
    Control {
        id: ControlId::Feather,
        key: "feather",
        label: "Mask feather",
        min: 0.,
        max: 0.05,
    },
    Control {
        id: ControlId::Direction,
        key: "direction",
        label: "Motion direction",
        min: -180.,
        max: 180.,
    },
    Control {
        id: ControlId::Amplitude,
        key: "amplitude",
        label: "Motion amplitude",
        min: 0.,
        max: 0.15,
    },
    Control {
        id: ControlId::Frequency,
        key: "frequency",
        label: "Motion frequency",
        min: 0.,
        max: 16.,
    },
    Control {
        id: ControlId::Speed,
        key: "speed",
        label: "Effect speed",
        min: 0.,
        max: 5.,
    },
    Control {
        id: ControlId::Density,
        key: "density",
        label: "Weather density",
        min: 1.,
        max: 32.,
    },
    Control {
        id: ControlId::Size,
        key: "size",
        label: "Weather size",
        min: 0.03,
        max: 0.45,
    },
    Control {
        id: ControlId::RainDirection,
        key: "rain_direction",
        label: "Rain direction (0° down, 90° right)",
        min: -180.,
        max: 180.,
    },
    Control {
        id: ControlId::SizeX,
        key: "size_x",
        label: "Water width (X)",
        min: 0.2,
        max: 2.,
    },
    Control {
        id: ControlId::SizeY,
        key: "size_y",
        label: "Water height (Y)",
        min: 0.2,
        max: 2.,
    },
    Control {
        id: ControlId::Strength,
        key: "strength",
        label: "Distortion / effect strength",
        min: 0.,
        max: 1.,
    },
];
pub const MAX_EFFECTS_PER_LAYER: usize = 8;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LayerTarget {
    Image(usize),
    Effect(usize, usize),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ImportTarget {
    Layer,
    Mask(usize),
    Displacement(usize),
    EffectMask(usize, usize),
    EffectDisplacement(usize, usize),
}
impl From<Option<(usize, bool)>> for ImportTarget {
    fn from(target: Option<(usize, bool)>) -> Self {
        match target {
            None => Self::Layer,
            Some((index, false)) => Self::Mask(index),
            Some((index, true)) => Self::Displacement(index),
        }
    }
}
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
            version: 3,
            layers: vec![Layer::new("Background".into(), "background".into())],
            assets: Arc::new(BTreeMap::from([("background".into(), asset)])),
        }
    }
    pub fn validate(&self) -> Result<()> {
        ensure!(
            (1..=3).contains(&self.version) && (1..=6).contains(&self.layers.len()),
            "layers require version 1–3 and 1–6 layers"
        );
        ensure!(self.assets.len() <= 20, "layer source assets exceed limit");
        for l in &self.layers {
            ensure!(
                !l.name.is_empty() && l.name.len() <= 256,
                "invalid layer name"
            );
            ensure!(self.assets.contains_key(&l.asset), "missing layer image");
            for c in CONTROLS {
                let v = l.value(c.id);
                ensure!(
                    v.is_finite() && (c.min..=c.max).contains(&v),
                    "invalid {}",
                    c.label
                );
            }
            ensure!(
                l.effects.len() <= MAX_EFFECTS_PER_LAYER,
                "maximum eight effects per image layer"
            );
            for effect in &l.effects {
                ensure!(
                    effect.settings.effect != Effect::None,
                    "attached effect needs a type"
                );
                for control in CONTROLS
                    .iter()
                    .filter(|control| control.id.is_effect() || control.id == ControlId::Feather)
                {
                    validate_control(control.id, effect.value(control.id))?;
                }
                if let Some(mask) = &effect.mask {
                    ensure!(
                        self.assets.contains_key(&mask.asset),
                        "missing effect mask image"
                    );
                }
                if let Some(asset) = &effect.settings.displacement {
                    ensure!(
                        self.assets.contains_key(asset),
                        "missing effect displacement image"
                    );
                }
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
    pub fn import(&mut self, path: &Path, target: impl Into<ImportTarget>) -> Result<()> {
        let target = target.into();
        let bytes = wisp_compiler::read_source_image(path)?;
        let decoded = wisp_compiler::decode_source_image(&bytes)?;
        match target {
            ImportTarget::Mask(i) | ImportTarget::Displacement(i) => {
                self.target_mask(LayerTarget::Image(i))?;
            }
            ImportTarget::EffectMask(i, effect) | ImportTarget::EffectDisplacement(i, effect) => {
                self.target_mask(LayerTarget::Effect(i, effect))?;
            }
            ImportTarget::Layer => ensure!(self.layers.len() < 6, "maximum six layers"),
        }
        let asset = if matches!(target, ImportTarget::Mask(_) | ImportTarget::EffectMask(..)) {
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
        match target {
            ImportTarget::Mask(i) => {
                *self.target_mask_mut(LayerTarget::Image(i))? = Some(Mask::new(id, false))
            }
            ImportTarget::EffectMask(i, effect) => {
                *self.target_mask_mut(LayerTarget::Effect(i, effect))? = Some(Mask::new(id, false))
            }
            ImportTarget::Displacement(i) => self.layers[i].displacement = Some(id),
            ImportTarget::EffectDisplacement(i, effect) => {
                self.layers[i].effects[effect].settings.displacement = Some(id)
            }
            ImportTarget::Layer => self.layers.push(Layer::new(
                path.file_stem()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .chars()
                    .take(128)
                    .collect(),
                id,
            )),
        }
        self.prune();
        Ok(())
    }
    pub fn target_mask(&self, target: LayerTarget) -> Result<&Option<Mask>> {
        match target {
            LayerTarget::Image(layer) => Ok(&self.layers.get(layer).context("missing layer")?.mask),
            LayerTarget::Effect(layer, effect) => Ok(&self
                .layers
                .get(layer)
                .context("missing layer")?
                .effects
                .get(effect)
                .context("missing effect")?
                .mask),
        }
    }
    pub fn target_mask_mut(&mut self, target: LayerTarget) -> Result<&mut Option<Mask>> {
        match target {
            LayerTarget::Image(layer) => {
                Ok(&mut self.layers.get_mut(layer).context("missing layer")?.mask)
            }
            LayerTarget::Effect(layer, effect) => Ok(&mut self
                .layers
                .get_mut(layer)
                .context("missing layer")?
                .effects
                .get_mut(effect)
                .context("missing effect")?
                .mask),
        }
    }
    pub fn blank_mask(&mut self, i: usize, white: bool) -> Result<()> {
        self.blank_mask_for(LayerTarget::Image(i), white)?;
        self.layers[i].mask.as_mut().unwrap().composition = false;
        Ok(())
    }
    pub fn blank_mask_for(&mut self, target: LayerTarget, white: bool) -> Result<()> {
        self.target_mask(target)?;
        let image =
            image::GrayImage::from_pixel(256, 256, image::Luma([if white { 255 } else { 0 }]));
        let asset = encode(image::DynamicImage::ImageLuma8(image))?;
        // Replacing a mask must still work when all asset slots are occupied.
        let existing = self
            .target_mask(target)?
            .as_ref()
            .map(|mask| mask.asset.clone());
        let id = if let Some(id) = existing {
            Arc::make_mut(&mut self.assets).insert(id.clone(), asset);
            id
        } else {
            self.insert(asset)?
        };
        let mut mask = Mask::new(id, matches!(target, LayerTarget::Image(_)));
        mask.feather = 0.005;
        *self.target_mask_mut(target)? = Some(mask);
        self.prune();
        Ok(())
    }
    pub fn add_effect(&mut self, layer: usize, effect: Effect) -> Result<usize> {
        let image = self.layers.get(layer).context("missing layer")?;
        ensure!(effect != Effect::None, "choose an effect");
        ensure!(
            image.effects.len() < MAX_EFFECTS_PER_LAYER,
            "maximum eight effects per image layer"
        );
        let index = image.effects.len();
        self.layers[layer].effects.push(LayerEffect::new(effect));
        if let Err(error) = self.blank_mask_for(
            LayerTarget::Effect(layer, index),
            !matches!(effect, Effect::Hair | Effect::Clothing),
        ) {
            self.layers[layer].effects.pop();
            return Err(error);
        }
        self.version = 3;
        Ok(index)
    }
    pub fn upgrade_effects(&mut self) -> Result<()> {
        let mut upgraded = self.clone();
        upgraded.upgrade_legacy_effects()?;
        *self = upgraded;
        Ok(())
    }
    fn upgrade_legacy_effects(&mut self) -> Result<()> {
        for index in 0..self.layers.len() {
            if self.layers[index].effect == Effect::None {
                continue;
            }
            ensure!(
                self.layers[index].effects.len() < MAX_EFFECTS_PER_LAYER,
                "maximum eight effects per image layer"
            );
            let mut mask = self.layers[index].mask.clone().filter(|mask| mask.effect);
            if let Some(mask) = &mut mask {
                if self.layers[index]
                    .mask
                    .as_ref()
                    .is_some_and(|mask| mask.composition)
                {
                    mask.asset = self.insert(self.assets[&mask.asset].clone())?;
                } else {
                    self.layers[index].mask = None;
                }
                mask.composition = false;
                mask.effect = true;
            }
            let settings = self.layers[index].motion.clone();
            let unmasked_motion = matches!(settings.effect, Effect::Hair | Effect::Clothing)
                && self.layers[index]
                    .mask
                    .as_ref()
                    .is_some_and(|mask| !mask.effect);
            self.layers[index].effects.insert(
                0,
                LayerEffect {
                    visible: true,
                    mask,
                    settings,
                },
            );
            if unmasked_motion {
                self.blank_mask_for(LayerTarget::Effect(index, 0), true)?;
            }
            self.layers[index].motion = EffectSettings::default();
        }
        self.prune();
        self.version = 3;
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
        self.paint_target(LayerTarget::Image(i), from, to, radius, value)
    }
    pub fn paint_target(
        &mut self,
        target: LayerTarget,
        from: [f32; 2],
        to: [f32; 2],
        radius: f32,
        value: u8,
    ) -> Result<()> {
        let id = self
            .target_mask(target)?
            .as_ref()
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
                    .chain(l.effects.iter().flat_map(|effect| {
                        effect
                            .mask
                            .iter()
                            .map(|mask| mask.asset.clone())
                            .chain(effect.settings.displacement.clone())
                    }))
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
            let mut weather = Vec::new();
            // Upper effects remap the image and effects below them, so mixed stacks keep their order.
            for effect in l.effects.iter().rev().filter(|effect| effect.visible) {
                let mask = b.mask(effect.mask.as_ref(), &sample_uv, &one);
                if matches!(
                    effect.settings.effect,
                    Effect::Droplets | Effect::Rain | Effect::Ripples
                ) {
                    weather.push((&effect.settings, sample_uv.clone(), mask));
                } else {
                    sample_uv =
                        b.effect_uv(&effect.settings, &sample_uv, &mask, effect.mask.is_some());
                }
            }
            let mut color = b.add(Op::Image {
                asset: l.asset.clone(),
                uv: sample_uv.clone(),
            });
            for (settings, weather_uv, mask) in weather.into_iter().rev() {
                color = b.effect_color(settings, color, &weather_uv, mask);
            }
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
    fn mask(&mut self, mask: Option<&Mask>, uv: &str, one: &str) -> String {
        let Some(mask) = mask else {
            return one.into();
        };
        let color = self.add(Op::Image {
            asset: mask.asset.clone(),
            uv: uv.into(),
        });
        let feather = self.constant(V::Float(mask.feather));
        let invert = self.constant(V::Boolean(mask.invert));
        self.add(Op::Mask {
            color,
            feather,
            invert,
        })
    }
    fn effect_uv(
        &mut self,
        settings: &EffectSettings,
        uv: &str,
        mask: &str,
        has_mask: bool,
    ) -> String {
        let mask = if matches!(settings.effect, Effect::Hair | Effect::Clothing) && !has_mask {
            self.constant(V::Float(0.))
        } else {
            mask.into()
        };
        match settings.effect {
            Effect::Hair | Effect::Clothing | Effect::Wave | Effect::Warp => {
                let direction = self.constant(V::Vector2([
                    settings.direction.to_radians().cos(),
                    settings.direction.to_radians().sin(),
                ]));
                let amplitude = self.constant(V::Float(settings.amplitude));
                let frequency = self.constant(V::Float(settings.frequency));
                let speed = self.constant(V::Float(settings.speed));
                self.add(
                    if matches!(settings.effect, Effect::Clothing | Effect::Warp) {
                        Op::Warp {
                            uv: uv.into(),
                            mask,
                            direction,
                            amplitude,
                            frequency,
                            speed,
                        }
                    } else {
                        Op::Wave {
                            uv: uv.into(),
                            mask,
                            direction,
                            amplitude,
                            frequency,
                            speed,
                        }
                    },
                )
            }
            Effect::Displacement if settings.displacement.is_some() => {
                let map = self.add(Op::Image {
                    asset: settings.displacement.clone().unwrap(),
                    uv: uv.into(),
                });
                let strength = self.constant(V::Float(settings.strength * 0.15));
                self.add(Op::Displacement {
                    uv: uv.into(),
                    map,
                    mask,
                    strength,
                })
            }
            _ => uv.into(),
        }
    }
    fn effect_color(
        &mut self,
        settings: &EffectSettings,
        color: String,
        uv: &str,
        mask: String,
    ) -> String {
        let kind = match settings.effect {
            Effect::Rain => WeatherKind::Rain,
            Effect::Ripples => WeatherKind::Ripples,
            Effect::Droplets => WeatherKind::Droplets,
            _ => return color,
        };
        let density = self.constant(V::Float(settings.density));
        let size = self.constant(V::Float(settings.size));
        let speed = self.constant(V::Float(settings.speed));
        let strength = self.constant(V::Float(settings.strength));
        self.add(Op::Weather {
            rain_direction: settings.rain_direction,
            size_x: settings.size_x,
            size_y: settings.size_y,
            color,
            uv: uv.into(),
            mask,
            density,
            size,
            speed,
            strength,
            kind,
        })
    }

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

#[cfg(test)]
mod control_tests {
    use super::*;
    #[test]
    fn every_control_round_trips_and_rejects_invalid_values_without_mutation() {
        let mut layer = Layer::new("test".into(), "image".into());
        layer.mask = Some(Mask {
            asset: "mask".into(),
            feather: 0.,
            invert: false,
            composition: false,
            effect: true,
        });
        for control in CONTROLS {
            assert_eq!(ControlId::try_from(control.key).unwrap(), control.id);
            for value in [control.min, (control.min + control.max) / 2., control.max] {
                layer.set_value(control.id, value).unwrap();
                assert_eq!(layer.value(control.id), value, "{}", control.key);
            }
            for value in [f32::NAN, f32::INFINITY, control.max + 1., control.min - 1.] {
                assert!(layer.set_value(control.id, value).is_err());
                assert_eq!(layer.value(control.id), control.max);
            }
        }
        assert!(layer.set_value("unknown", 0.).is_err());
        layer.mask = None;
        assert!(layer.set_value(ControlId::Feather, 0.01).is_err());
    }
}
