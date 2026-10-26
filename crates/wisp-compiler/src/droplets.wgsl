// A contact-angle meniscus gives beads a dark, steep rim and a broad lens interior.
// Pixel-sized edge coverage keeps fine condensation stable when the output resizes.
fn wisp_glass_bead(delta: vec2<f32>, radius: f32, stretch: vec2<f32>, seed: f32, pixel: f32) -> vec4<f32> {
    var q = delta / (radius * stretch);
    let bend = 0.13 * sin(q.y * 2.7 + seed * 6.2831853);
    q.x += bend * (1.0 - min(q.y * q.y, 1.0));
    let shoulder = 1.0 + 0.16 * q.y;
    q.x /= max(shoulder, 0.65);
    let distance = length(q);
    let edge = clamp(pixel / (radius * min(stretch.x, stretch.y)), 0.015, 0.65);
    let coverage = 1.0 - smoothstep(1.0 - edge, 1.0 + edge, distance);
    let height = sqrt(max(1.0 - min(distance * distance, 1.0), 0.0));
    let slope = q / stretch * (0.65 / max(height, 0.12));
    return vec4<f32>(slope, radius * max(height, 0.04) * coverage, coverage);
}

// Independent scales break up the visible lattice: fine pinned condensation,
// irregular resting beads, and sparse runners that leave a wet wake.
fn wisp_glass_droplets(uv: vec2<f32>, density: f32, size: f32, speed: f32, axis_scale: vec2<f32>) -> vec4<f32> {
    let grid = clamp(density, 1.0, 32.0);
    let radius = clamp(size, 0.03, 0.45);
    let shape = clamp(axis_scale, vec2<f32>(0.2), vec2<f32>(2.0));
    let aspect = globals.resolution.x / max(globals.resolution.y, 1.0);
    let physical = (uv - vec2<f32>(0.5)) * vec2<f32>(aspect, 1.0);
    let time = globals.time * clamp(speed, 0.0, 5.0);
    var result = vec4<f32>(0.0);
    for (var layer = 0; layer < 3; layer++) {
        let moving = layer == 2;
        let scale = grid * select(select(1.45, 7.3, layer == 0), 0.85, moving);
        let pixel = scale / max(globals.resolution.y, 1.0);
        let p = physical * scale + vec2<f32>(f32(layer) * 23.71);
        let reach = i32(ceil(radius * max(shape.x, shape.y) * select(1.8, 2.5, moving)));
        for (var x = -3; x <= 3; x++) {
            if abs(x) > reach { continue; }
            let column = i32(floor(p.x)) + x;
            let column_seed = wisp_hash(vec2<i32>(column, 391 + layer * 71));
            let travel = time * (0.25 + column_seed * 0.55);
            let local = p - vec2<f32>(0.0, select(0.0, travel, moving));
            for (var y = -3; y <= 3; y++) {
                if abs(y) > reach { continue; }
                let cell = vec2<i32>(column, i32(floor(local.y)) + y);
                let seed = wisp_hash(cell + vec2<i32>(layer * 101, 37));
                let seed2 = wisp_hash(cell + vec2<i32>(83, layer * 131));
                if seed < select(0.22, 0.68, moving) { continue; }
                let center = vec2<f32>(cell) + vec2<f32>(0.18 + seed * 0.64, 0.18 + seed2 * 0.64);
                var delta = local - center;
                let variation = mix(0.18, 0.95, seed2 * seed2);
                let drop_radius = radius * variation * select(select(0.7, 0.48, moving), 0.75, layer == 0);
                let stretch = shape * vec2<f32>(0.8 + seed * 0.25, select(1.0 + seed2 * 0.65, 1.8 + seed2 * 0.6, moving));
                if moving {
                    // The path is tied to glass coordinates so the wake stays behind the head.
                    delta.x += sin(p.y * 1.6 + column_seed * 6.2831853) * 0.035;
                }
                let bead = wisp_glass_bead(delta, drop_radius, stretch, seed, pixel);
                if bead.w > 0.0 && bead.z / scale > result.z {
                    result = vec4<f32>(bead.xy, bead.z / scale, bead.w);
                }
                if moving {
                    let behind = -delta.y;
                    let length = 0.7 + seed * 0.5;
                    let taper = smoothstep(0.0, drop_radius, behind) * (1.0 - smoothstep(0.0, length, behind));
                    let width = max(drop_radius * stretch.x * 0.22, pixel * 0.4);
                    let across = delta.x / width;
                    let wet = exp(-across * across * 2.0) * taper;
                    let wake = vec4<f32>(across * 0.7, 0.04, drop_radius * 0.12 / scale, wet * 0.65);
                    if wet > 0.01 && wake.z * wet > result.z {
                        result = vec4<f32>(wake.xy, wake.z * wet, wake.w);
                    }
                }
            }
        }
    }
    return result;
}
