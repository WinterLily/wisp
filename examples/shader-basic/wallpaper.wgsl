// The runtime supplies globals, background and background_sampler.
// UV and mouse coordinates run from (0, 0) at top left to (1, 1) at bottom right.
fn wallpaper(uv: vec2<f32>) -> vec4<f32> {
    let parallax = (globals.mouse - vec2<f32>(0.5)) * 0.045;
    let wave = vec2<f32>(
        sin(uv.y * 8.0 + globals.time * 0.35),
        cos(uv.x * 6.0 + globals.time * 0.25)
    ) * 0.018;
    let color = textureSample(background, background_sampler, uv + parallax + wave);
    let pulse = 0.94 + 0.06 * sin(globals.time * 0.7);
    let vignette = 1.0 - 0.65 * dot(uv - vec2<f32>(0.5), uv - vec2<f32>(0.5));
    return vec4<f32>(color.rgb * pulse * vignette, 1.0);
}
