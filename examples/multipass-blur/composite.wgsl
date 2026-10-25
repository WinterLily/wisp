fn wallpaper(uv: vec2<f32>) -> vec4<f32> {
    let blurred = textureSample(background, background_sampler, uv);
    let original = textureSample(image_1, image_1_sampler, uv);
    // A moving split shows the original on the left and the blurred output on the right.
    let split = 0.5 + sin(globals.time * 0.4) * 0.15;
    let amount = smoothstep(split - 0.01, split + 0.01, uv.x) * params.p1;
    return vec4<f32>(mix(original.rgb, blurred.rgb, amount), 1.0);
}
