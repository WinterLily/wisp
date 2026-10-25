fn wallpaper(uv: vec2<f32>) -> vec4<f32> {
    return textureSample(background, background_sampler, uv);
}
