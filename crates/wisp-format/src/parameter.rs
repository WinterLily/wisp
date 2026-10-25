use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};

pub const MAX_IMAGES: usize = 8;
pub const MAX_PARAMETERS: usize = 64;

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[serde(
    tag = "type",
    content = "value",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum ParameterValue {
    Float(f32),
    Integer(i32),
    Boolean(bool),
    Color([f32; 4]),
    Vector2([f32; 2]),
    Vector3([f32; 3]),
    Vector4([f32; 4]),
}

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Parameter {
    pub id: String,
    pub default: ParameterValue,
    /// Inclusive scalar bounds; absent bounds allow every finite value of the type.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max: Option<f64>,
}

impl Parameter {
    pub fn validate(&self) -> Result<()> {
        crate::identifier(&self.id)?;
        let scalar = matches!(
            self.default,
            ParameterValue::Float(_) | ParameterValue::Integer(_)
        );
        ensure!(
            scalar || (self.min.is_none() && self.max.is_none()),
            "only scalar parameters support bounds"
        );
        for bound in [self.min, self.max].into_iter().flatten() {
            ensure!(bound.is_finite(), "parameter bounds must be finite");
            match self.default {
                ParameterValue::Integer(_) => ensure!(
                    bound.fract() == 0.0 && bound >= i32::MIN as f64 && bound <= i32::MAX as f64,
                    "integer bounds must fit i32"
                ),
                ParameterValue::Float(_) => {
                    ensure!(bound.abs() <= f32::MAX as f64, "float bounds must fit f32")
                }
                _ => {}
            }
        }
        if let (Some(min), Some(max)) = (self.min, self.max) {
            ensure!(min <= max, "parameter minimum exceeds maximum");
        }
        self.validate_value(&self.default)
    }

    /// Check an override against an already validated declaration.
    pub fn validate_value(&self, value: &ParameterValue) -> Result<()> {
        use ParameterValue::*;
        ensure!(
            std::mem::discriminant(value) == std::mem::discriminant(&self.default),
            "parameter {} has the wrong value type",
            self.id
        );
        let components: &[f32] = match value {
            Float(v) => std::slice::from_ref(v),
            Color(v) | Vector4(v) => v,
            Vector2(v) => v,
            Vector3(v) => v,
            _ => &[],
        };
        ensure!(
            components.iter().all(|v| v.is_finite()),
            "parameter {} must be finite",
            self.id
        );
        if let Color(v) = value {
            ensure!(
                v.iter().all(|v| (0.0..=1.0).contains(v)),
                "color components must be in 0..=1"
            );
        }
        let scalar = match value {
            Float(v) => {
                ensure!(
                    self.min.is_none_or(|min| *v >= min as f32)
                        && self.max.is_none_or(|max| *v <= max as f32),
                    "parameter {} is outside its bounds",
                    self.id
                );
                None
            }
            Integer(v) => Some(f64::from(*v)),
            _ => None,
        };
        if let Some(v) = scalar {
            ensure!(
                self.min.is_none_or(|min| v >= min) && self.max.is_none_or(|max| v <= max),
                "parameter {} is outside its bounds",
                self.id
            );
        }
        Ok(())
    }
}

pub fn validate_parameters(parameters: &[Parameter]) -> Result<()> {
    ensure!(
        parameters.len() <= MAX_PARAMETERS,
        "too many parameters (maximum {MAX_PARAMETERS})"
    );
    let mut ids = std::collections::BTreeSet::new();
    for parameter in parameters {
        parameter.validate()?;
        ensure!(
            ids.insert(&parameter.id),
            "duplicate parameter: {}",
            parameter.id
        );
    }
    Ok(())
}
