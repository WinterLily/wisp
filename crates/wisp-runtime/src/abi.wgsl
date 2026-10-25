struct Globals {
    resolution: vec2<f32>,
    mouse: vec2<f32>,
    time: f32,
    delta_time: f32,
    audio_bass: f32,
    audio_mid: f32,
    audio_treble: f32,
    audio_volume: f32,
    frame: u32,
};

@group(0) @binding(0) var<uniform> globals: Globals;
@group(1) @binding(0) var background: texture_2d<f32>;
@group(1) @binding(1) var background_sampler: sampler;

struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex
fn vs_main(@builtin(vertex_index) index: u32) -> VertexOutput {
    // One oversized triangle covers the viewport without a diagonal seam.
    let uv = vec2<f32>(f32((index << 1u) & 2u), f32(index & 2u));
    var out: VertexOutput;
    out.position = vec4<f32>(uv * vec2<f32>(2.0, -2.0) + vec2<f32>(-1.0, 1.0), 0.0, 1.0);
    out.uv = uv;
    return out;
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    return wallpaper(in.uv);
}
