@group(0) @binding(2) var<uniform> crop: vec4<f32>;
struct Vertex { @builtin(position) position: vec4<f32>, @location(0) uv: vec2<f32> };
@vertex fn vertex(@builtin(vertex_index) index: u32) -> Vertex {
    let positions = array<vec2<f32>,3>(vec2<f32>(-1.0,-1.0),vec2<f32>(3.0,-1.0),vec2<f32>(-1.0,3.0));
    let p = positions[index];
    return Vertex(vec4<f32>(p,0.0,1.0),crop.xy + crop.zw * vec2<f32>((p.x+1.0)*0.5,(1.0-p.y)*0.5));
}
@group(0) @binding(0) var preview: texture_2d<f32>;
@group(0) @binding(1) var preview_sampler: sampler;
@fragment fn fragment(input: Vertex) -> @location(0) vec4<f32> {
    return textureSample(preview,preview_sampler,input.uv);
}
