fn compute(id: vec3<u32>) {
    let size = vec2<u32>(globals.resolution);
    if id.x >= size.x || id.y >= size.y || id.z != 0u { return; }
    let color = buffer_0[id.y * size.x + id.x];
    let mirrored = textureLoad(background, vec2<i32>(vec2<u32>(size.x - id.x - 1u, id.y)), 0);
    textureStore(output_image, vec2<i32>(id.xy), mix(color, mirrored, params.p1));
}
