//! Stateful authoring operations lower to ordinary ABI 3 compute and fragment passes.
use super::*;
use wisp_format::StorageBinding;
use wisp_ir::Node;
impl Compiler<'_> {
    fn buffer(&mut self, elements: u32) -> Result<String> {
        let count = self
            .resources
            .iter()
            .filter(|r| matches!(r, Resource::StorageBuffer { .. }))
            .count();
        ensure!(
            count < wisp_format::MAX_STORAGE_BUFFERS,
            "graph exceeds persistent buffer limit"
        );
        let bytes = u64::from(elements) * 16;
        self.storage_bytes += bytes;
        ensure!(
            self.storage_bytes <= self.options.runtime_limits.max_storage_bytes,
            "graph exceeds persistent storage limit"
        );
        let id = format!("state{count}");
        self.resources.push(Resource::StorageBuffer {
            id: id.clone(),
            elements,
        });
        Ok(id)
    }
    fn compute_pass(
        &mut self,
        source: String,
        images: Vec<String>,
        buffer: String,
        output: String,
        width: u32,
        height: u32,
    ) {
        let path = format!("shaders/pass_{}.wgsl", self.passes.len());
        self.files.insert(path.clone(), source.into_bytes());
        self.passes.push(Pass::Compute {
            shader: path,
            images,
            buffers: vec![StorageBinding {
                resource: buffer,
                writable: true,
            }],
            output,
            dispatch: [width.div_ceil(8), height.div_ceil(8), 1],
        });
    }
    pub(super) fn simulation(&mut self, node: &Node) -> Result<()> {
        match &node.op {
            Op::Feedback {
                color,
                decay,
                width,
                height,
            } => {
                let input = self.target(*width, *height)?;
                self.emit(color, Output::Target(input.clone()), None)?;
                let output = self.target(*width, *height)?;
                let buffer = self.buffer(width * height)?;
                let (mut source, mut bindings) = self.source(&[decay], None, true)?;
                let image = bind(&mut bindings, &input);
                let decay = self.graph.index(decay);
                // Each invocation owns exactly one history element. No cross-invocation
                // read/write hazard and no texture self-sampling or graph cycle.
                source.push_str(&format!(
                    r#"
fn compute(id: vec3<u32>) {{
    if (id.x >= {width}u || id.y >= {height}u) {{ return; }}
    let uv = (vec2<f32>(id.xy) + vec2<f32>(0.5)) / vec2<f32>({width}.0, {height}.0);
    let index = id.y * {width}u + id.x;
    let current = clamp(textureLoad({image}, vec2<i32>(id.xy), 0), vec4<f32>(0.0), vec4<f32>(1.0));
    let dt = clamp(globals.delta_time, 0.0, 0.25);
    var history = buffer_0[index];
    if (dt > 0.0) {{ history *= pow(clamp(n{decay}(uv), 0.0, 0.9999), dt * 60.0); }}
    let value = max(current, history);
    buffer_0[index] = value;
    textureStore(output_image, vec2<i32>(id.xy), value);
}}
"#
                ));
                self.compute_pass(source, bindings, buffer, output.clone(), *width, *height);
                self.targets.insert(node.id.clone(), output);
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
                let state = self.target(*count, 1)?;
                let buffer = self.buffer(*count)?;
                let (mut source, bindings) = self.source(&[position, speed], None, true)?;
                let position = self.graph.index(position);
                let speed = self.graph.index(speed);
                // Integer hash keeps spawning deterministic across shader backends.
                source.push_str(&format!(r#"
fn random01(seed: u32) -> f32 {{
    var h = seed * 747796405u + 2891336453u;
    h = ((h >> ((h >> 28u) + 4u)) ^ h) * 277803737u;
    return f32((h >> 22u) ^ h) / 4294967295.0;
}}
fn compute(id: vec3<u32>) {{
    if (id.x >= {count}u || id.y != 0u) {{ return; }}
    let uv = vec2<f32>(random01(id.x + 1u), random01(id.x + 983u));
    let emitter = clamp(n{position}(uv), vec2<f32>(0.0), vec2<f32>(1.0));
    let dt = clamp(globals.delta_time, 0.0, 0.1);
    var p = buffer_0[id.x];
    if (p.w == 0.0) {{ p = vec4<f32>(emitter, uv.x * 3.0, 1.0); }}
    p.z += dt;
    if (p.z >= 3.0 || any(p.xy < vec2<f32>(0.0)) || any(p.xy > vec2<f32>(1.0))) {{
        p = vec4<f32>(emitter, 0.0, 1.0);
    }}
    let angle = uv.y * 6.28318530718;
    p = vec4<f32>(p.xy + vec2<f32>(cos(angle), sin(angle)) * clamp(n{speed}(uv), 0.0, 2.0) * dt, p.zw);
    buffer_0[id.x] = p;
    textureStore(output_image, vec2<i32>(i32(id.x), 0), vec4<f32>(p.xy, 1.0 - p.z / 3.0, 1.0));
}}
"#));
                self.compute_pass(source, bindings, buffer, state.clone(), *count, 1);
                let output = self.target(*width, *height)?;
                let (mut source, mut bindings) = self.source(&[color, size], None, false)?;
                let image = bind(&mut bindings, &state);
                let color = self.graph.index(color);
                let size = self.graph.index(size);
                source.push_str(&format!(
                    r#"
fn wallpaper(uv: vec2<f32>) -> vec4<f32> {{
    let color = clamp(n{color}(uv), vec4<f32>(0.0), vec4<f32>(1.0));
    let radius = clamp(n{size}(uv), 0.001, 0.25);
    let aspect = vec2<f32>({width}.0 / {height}.0, 1.0);
    var light = 0.0;
    for (var i = 0u; i < {count}u; i++) {{
        let p = textureLoad({image}, vec2<i32>(i32(i), 0), 0);
        let d = length((uv - p.xy) * aspect);
        light += (1.0 - smoothstep(radius * 0.25, radius, d)) * p.z;
    }}
    return color * clamp(light, 0.0, 1.0);
}}
"#
                ));
                Shader::with_bindings(&source, bindings.len(), &self.parameters)?;
                let path = format!("shaders/pass_{}.wgsl", self.passes.len());
                self.files.insert(path.clone(), source.into_bytes());
                self.passes.push(Pass::Fragment {
                    shader: path,
                    background: bindings[0].clone(),
                    images: bindings[1..].to_vec(),
                    output: Output::Target(output.clone()),
                });
                self.targets.insert(node.id.clone(), output);
            }
            _ => {}
        }
        Ok(())
    }
}
fn bind(bindings: &mut Vec<String>, resource: &str) -> String {
    let i = if let Some(i) = bindings.iter().position(|r| r == resource) {
        i
    } else {
        bindings.push(resource.into());
        bindings.len() - 1
    };
    if i == 0 {
        "background".into()
    } else {
        format!("image_{i}")
    }
}
