fn compute(id: vec3<u32>) {
    let size = vec2<u32>(globals.resolution);
    if id.x >= size.x || id.y >= size.y || id.z != 0u { return; }
    let uv = vec2<f32>(id.xy) / vec2<f32>(size - vec2<u32>(1u));
    let wave = 0.5 + 0.5 * sin(uv.x * 12.0 + globals.time * params.p0);
    let color = vec4<f32>(uv.x * 0.5, uv.y * 0.4, wave, 1.0);
    buffer_0[id.y * size.x + id.x] = color;
    textureStore(output_image, vec2<i32>(id.xy), color);
}
