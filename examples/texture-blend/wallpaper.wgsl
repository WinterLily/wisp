// Program parameter order: blend, tint, animate, speed, offset.
fn wallpaper(uv: vec2<f32>) -> vec4<f32> {
    let movement = select(0.0, sin(globals.time * params.p3) * 0.025, params.p2 != 0u);
    let shifted = uv + params.p4 + vec2<f32>(movement, 0.0);
    let base = textureSample(background, background_sampler, shifted);
    let accent = textureSample(image_1, image_1_sampler, uv);
    return vec4<f32>(mix(base.rgb, accent.rgb, params.p0) * params.p1.rgb, 1.0);
}
