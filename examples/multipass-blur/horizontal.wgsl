fn wallpaper(uv: vec2<f32>) -> vec4<f32> {
    let step = vec2<f32>(params.p0 / f32(textureDimensions(background).x), 0.0);
    var color = textureSample(background, background_sampler, uv) * 0.227027;
    color += textureSample(background, background_sampler, uv + step * 1.384615) * 0.316216;
    color += textureSample(background, background_sampler, uv - step * 1.384615) * 0.316216;
    color += textureSample(background, background_sampler, uv + step * 3.230769) * 0.070270;
    color += textureSample(background, background_sampler, uv - step * 3.230769) * 0.070270;
    return vec4<f32>(color.rgb, 1.0);
}
