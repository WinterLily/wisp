use anyhow::{Result, bail, ensure};
use wisp_format::{MAX_IMAGES, Parameter, ParameterValue, validate_parameters};

const ABI: &str = include_str!("abi.wgsl");
pub const MAX_SHADER_BYTES: usize = 1024 * 1024;

/// Validated WGSL with host-generated entry points and resource declarations.
/// ABI 3 adds `compute(id)` while retaining the fragment `wallpaper(uv)` interface.
pub struct Shader {
    source: String,
    image_count: usize,
    parameters: Vec<Parameter>,
    pub(crate) storage: Option<Vec<bool>>,
}

impl Shader {
    pub fn new(body: &str) -> Result<Self> {
        Self::with_bindings(body, 1, &[])
    }

    /// Build host declarations from validated binding counts and parameter types.
    pub fn with_bindings(body: &str, image_count: usize, parameters: &[Parameter]) -> Result<Self> {
        Self::build(body, image_count, parameters, None)
    }

    pub(crate) fn compute(
        body: &str,
        image_count: usize,
        parameters: &[Parameter],
        storage: Vec<bool>,
    ) -> Result<Self> {
        Self::build(body, image_count, parameters, Some(storage))
    }

    fn build(
        body: &str,
        image_count: usize,
        parameters: &[Parameter],
        storage: Option<Vec<bool>>,
    ) -> Result<Self> {
        ensure!(
            image_count <= MAX_IMAGES && (image_count > 0 || storage.is_some()),
            "invalid image binding count"
        );
        validate_parameters(parameters)?;
        if body.len() > MAX_SHADER_BYTES {
            bail!("shader exceeds the 1 MiB source limit");
        }
        let mut declarations = String::new();
        for i in 1..image_count {
            declarations.push_str(&format!(
                "@group(1) @binding({}) var image_{i}: texture_2d<f32>;\n@group(1) @binding({}) var image_{i}_sampler: sampler;\n", i * 2, i * 2 + 1));
        }
        if !parameters.is_empty() {
            declarations.push_str("struct Parameters {\n");
            for (i, parameter) in parameters.iter().enumerate() {
                let ty = match parameter.default {
                    ParameterValue::Float(_) => "f32",
                    ParameterValue::Integer(_) => "i32",
                    ParameterValue::Boolean(_) => "u32",
                    ParameterValue::Color(_) | ParameterValue::Vector4(_) => "vec4<f32>",
                    ParameterValue::Vector2(_) => "vec2<f32>",
                    ParameterValue::Vector3(_) => "vec3<f32>",
                };
                // Explicit padding survives translation to GLSL std140; relying on
                // @size alone does not preserve the slots on the GL backend.
                declarations.push_str(&format!("@align(16) p{i}: {ty},\n"));
                let lanes = match parameter.default {
                    ParameterValue::Float(_)
                    | ParameterValue::Integer(_)
                    | ParameterValue::Boolean(_) => 1,
                    ParameterValue::Vector2(_) => 2,
                    ParameterValue::Vector3(_) => 3,
                    ParameterValue::Color(_) | ParameterValue::Vector4(_) => 4,
                };
                for lane in lanes..4 {
                    declarations.push_str(&format!("_padding_{i}_{lane}: u32,\n"));
                }
            }
            declarations.push_str("};\n@group(2) @binding(0) var<uniform> params: Parameters;\n");
        }
        let mut abi = ABI.to_owned();
        if let Some(buffers) = &storage {
            abi = ABI.split("struct VertexOutput").next().unwrap().to_owned();
            if image_count == 0 {
                abi = abi
                    .lines()
                    .filter(|line| !line.starts_with("@group(1)"))
                    .collect::<Vec<_>>()
                    .join("\n");
            }
            declarations.push_str(
                "@group(3) @binding(0) var output_image: texture_storage_2d<rgba8unorm, write>;\n",
            );
            for (i, writable) in buffers.iter().enumerate() {
                let access = if *writable { "read_write" } else { "read" };
                declarations.push_str(&format!(
                    "@group(3) @binding({}) var<storage, {access}> buffer_{i}: array<vec4<f32>>;\n",
                    i + 1
                ));
            }
            declarations.push_str("@compute @workgroup_size(8, 8, 1) fn cs_main(@builtin(global_invocation_id) id: vec3<u32>) { compute(id); }\n");
        }
        let source = format!("{abi}\n{declarations}\n// Wallpaper source begins here.\n{body}\n");
        let module = naga::front::wgsl::parse_str(&source)
            .map_err(|error| anyhow::anyhow!(error.emit_to_string(&source)))?;
        naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::empty(),
        )
        .validate(&module)
        .map_err(|error| anyhow::anyhow!(error.emit_to_string(&source)))?;
        if module.entry_points.len() != if storage.is_some() { 1 } else { 2 } {
            bail!("wallpaper source must not define additional shader entry points");
        }
        let mut layouter = naga::proc::Layouter::default();
        layouter.update(module.to_ctx())?;
        let mut workgroup_bytes = 0u64;
        for (_, global) in module.global_variables.iter() {
            if global.space == naga::AddressSpace::WorkGroup {
                // Each variable is rounded to 16 bytes as required by WebGPU's
                // workgroup-storage accounting, regardless of actual use.
                workgroup_bytes += u64::from(layouter[global.ty].size).div_ceil(16) * 16;
                ensure!(
                    workgroup_bytes <= 16384,
                    "workgroup storage exceeds Profile 1 limit"
                );
            }
            if let Some(binding) = &global.binding {
                let allowed = matches!(
                    (binding.group, binding.binding, global.name.as_deref()),
                    (0, 0, Some("globals"))
                ) || (image_count > 0
                    && matches!(
                        (binding.group, binding.binding, global.name.as_deref()),
                        (1, 0, Some("background")) | (1, 1, Some("background_sampler"))
                    ));
                let allowed = allowed
                    || (binding.group == 1
                        && binding.binding >= 2
                        && (binding.binding as usize) < image_count * 2
                        && global.name.as_deref()
                            == Some(
                                if binding.binding % 2 == 0 {
                                    format!("image_{}", binding.binding / 2)
                                } else {
                                    format!("image_{}_sampler", binding.binding / 2)
                                }
                                .as_str(),
                            ))
                    || (binding.group == 2
                        && binding.binding == 0
                        && !parameters.is_empty()
                        && global.name.as_deref() == Some("params"));
                let allowed = allowed
                    || (binding.group == 3
                        && storage.as_ref().is_some_and(|buffers| {
                            binding.binding == 0 && global.name.as_deref() == Some("output_image")
                                || binding.binding > 0
                                    && binding.binding as usize <= buffers.len()
                                    && global.name.as_deref()
                                        == Some(format!("buffer_{}", binding.binding - 1).as_str())
                        }));
                if !allowed {
                    bail!(
                        "unsupported resource binding: group {}, binding {}",
                        binding.group,
                        binding.binding
                    );
                }
            }
        }
        Ok(Self {
            source,
            image_count,
            parameters: parameters.to_vec(),
            storage,
        })
    }

    pub fn image_count(&self) -> usize {
        self.image_count
    }
    pub fn parameters(&self) -> &[Parameter] {
        &self.parameters
    }

    /// Change a validated initial value without recompiling the shader.
    pub fn set_parameter(&mut self, id: &str, value: ParameterValue) -> Result<()> {
        let parameter = self
            .parameters
            .iter_mut()
            .find(|p| p.id == id)
            .ok_or_else(|| anyhow::anyhow!("unknown parameter: {id}"))?;
        parameter.validate_value(&value)?;
        parameter.default = value;
        Ok(())
    }

    pub fn source(&self) -> &str {
        &self.source
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn example_is_valid() {
        Shader::new(crate::EXAMPLE_SHADER).unwrap();
    }

    #[test]
    fn invalid_shaders_have_diagnostics() {
        for body in [
            "this is not wgsl",
            "fn wallpaper(uv: vec2<f32>) -> f32 { return 0.0; }",
        ] {
            assert!(
                Shader::new(body)
                    .err()
                    .unwrap()
                    .to_string()
                    .contains("error")
            );
        }
    }

    #[test]
    fn rejects_extra_gpu_resources_and_entry_points() {
        let extra_resource = format!(
            "{}\n@group(2) @binding(0) var extra: texture_2d<f32>;",
            crate::EXAMPLE_SHADER
        );
        assert!(
            Shader::new(&extra_resource)
                .err()
                .unwrap()
                .to_string()
                .contains("unsupported resource")
        );
        let compute = format!(
            "{}\n@compute @workgroup_size(1) fn extra() {{}}",
            crate::EXAMPLE_SHADER
        );
        assert!(Shader::new(&compute).is_err());
    }

    #[test]
    fn compute_rejects_undeclared_images_and_excess_workgroup_memory() {
        for body in [
            "@group(1) @binding(0) var background: texture_2d<f32>; fn compute(id: vec3<u32>) {}",
            "var<workgroup> huge: array<vec4<f32>, 1025>; fn compute(id: vec3<u32>) {}",
        ] {
            assert!(Shader::compute(body, 0, &[], vec![]).is_err());
        }
    }

    #[test]
    fn rejects_oversized_source_before_parsing() {
        assert!(Shader::new(&" ".repeat(MAX_SHADER_BYTES + 1)).is_err());
    }
}
