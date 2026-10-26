fn wisp_srgb(c: vec3<f32>) -> vec3<f32> {
    let c0 = clamp(c, vec3<f32>(0.0), vec3<f32>(1.0));
    return select(1.055 * pow(c0, vec3<f32>(1.0 / 2.4)) - vec3<f32>(0.055), c0 * 12.92, c0 <= vec3<f32>(0.0031308));
}
fn wisp_gray(c: vec4<f32>) -> f32 { return dot(wisp_srgb(c.rgb), vec3<f32>(0.2126, 0.7152, 0.0722)) * c.a; }
fn wisp_transform(uv: vec2<f32>, position: vec2<f32>, scale: vec2<f32>, angle: f32, pivot: vec2<f32>) -> vec2<f32> {
    let p = uv - pivot - position;
    let c = cos(angle); let s = sin(angle);
    return vec2<f32>(c * p.x + s * p.y, -s * p.x + c * p.y) / max(abs(scale), vec2<f32>(0.001)) + pivot;
}
// Linear RGB blending avoids dark fringes around translucent layer edges.
fn wisp_blend(back: vec4<f32>, front: vec4<f32>, mode: u32) -> vec4<f32> {
    let ab = clamp(back.a, 0.0, 1.0); let af = clamp(front.a, 0.0, 1.0);
    var mixed = front.rgb;
    switch mode {
        case 1u: { mixed = back.rgb + front.rgb; }
        case 2u: { mixed = back.rgb * front.rgb; }
        case 3u: { mixed = vec3<f32>(1.0) - (vec3<f32>(1.0) - back.rgb) * (vec3<f32>(1.0) - front.rgb); }
        case 4u: { mixed = select(vec3<f32>(1.0) - 2.0 * (vec3<f32>(1.0) - back.rgb) * (vec3<f32>(1.0) - front.rgb), 2.0 * back.rgb * front.rgb, back.rgb <= vec3<f32>(0.5)); }
        default: {}
    }
    let alpha = af + ab * (1.0 - af);
    let premult = (1.0 - af) * ab * back.rgb + af * ((1.0 - ab) * front.rgb + ab * mixed);
    return vec4<f32>(premult / max(alpha, 0.000001), alpha);
}
fn wisp_hash(cell: vec2<i32>) -> f32 {
    var h = bitcast<u32>(cell.x) * 1597334677u ^ bitcast<u32>(cell.y) * 3812015801u;
    h = (h ^ (h >> 16u)) * 2246822519u;
    h = h ^ (h >> 13u);
    return f32(h & 65535u) / 65535.0;
}
// Irregular ellipses and a tapered shoulder avoid uniform outlined circles.
fn wisp_water_cap(d: vec2<f32>, radius: f32, elongation: f32, seed: f32) -> vec4<f32> {
    let stretch = vec2<f32>(0.82 + seed * 0.3, elongation);
    var q = d / (radius * stretch);
    let shoulder = 1.0 + 0.12 * clamp(q.y, -1.0, 1.0);
    q.x /= shoulder;
    let r2 = dot(q, q);
    let coverage = 1.0 - smoothstep(0.84, 1.0, r2);
    let cap = sqrt(max(1.0 - r2, 0.0001));
    let slope = q / (stretch * vec2<f32>(shoulder, 1.0)) * 0.72 / max(cap, 0.22);
    return vec4<f32>(slope * coverage, cap * radius * 0.65 * coverage, coverage);
}
// Bias toward small beads so occasional large drops do not dominate the surface.
fn wisp_water_size(seed: f32) -> f32 {
    return mix(0.10, 1.40, pow(seed, 1.5));
}
// Curved track normals keep wet streaks consistent with droplet highlights.
fn wisp_water_track(d: vec2<f32>, radius: f32, shape: vec2<f32>, seed: f32, activity: f32) -> vec4<f32> {
    let behind = -d.y;
    let length = min(0.85, 0.20 + radius * shape.y * 2.0);
    let taper = (1.0 - smoothstep(0.0, length, behind)) * smoothstep(0.0, radius * shape.y * 0.45, behind);
    let trail_x = d.x + sin(behind * 14.0 + seed * 6.0) * radius * shape.x * 0.06;
    let q = trail_x / max(radius * shape.x * 0.12, 0.001);
    let wet = exp(-q * q * 2.0) * taper * activity;
    return vec4<f32>(q * wet * 1.1, wet * 0.08, wet * radius * 0.06, wet * 0.8);
}
// Separate depths prevent the equally sharp, equally spaced streaks of a flat overlay.
fn wisp_rain(uv: vec2<f32>, density: f32, size: f32, speed: f32, direction: f32, axis_scale: vec2<f32>) -> f32 {
    let radius = clamp(size, 0.03, 0.45);
    let shape = clamp(axis_scale, vec2<f32>(0.2), vec2<f32>(2.0));
    let angle = radians(clamp(direction, -180.0, 180.0));
    let across = vec2<f32>(cos(angle), -sin(angle));
    let along = vec2<f32>(sin(angle), cos(angle));
    let aspect = globals.resolution.x / max(globals.resolution.y, 1.0);
    let physical = (uv - vec2<f32>(0.5)) * vec2<f32>(aspect, 1.0);
    let rain_position = vec2<f32>(dot(physical, across), dot(physical, along));
    let t = globals.time * clamp(speed, 0.0, 5.0);
    var coverage = 0.0;
    for (var layer = 0; layer < 3; layer++) {
        let depth = f32(layer);
        let length_variation = 2.8 * (1.0 + 0.9 / (1.0 + depth * 2.0));
        // Include the stretched footprint so long streaks do not pop at cell boundaries.
        let reach = i32(ceil(radius * (0.7 + length_variation) * shape.y + 0.5));
        let grid = clamp(density, 1.0, 32.0) * (1.0 + depth * 1.3);
        let p = rain_position * grid + vec2<f32>(depth * 17.13, depth * 31.79);
        let pixel = grid / max(globals.resolution.y, 1.0);
        let base_x = i32(floor(p.x));
        for (var x = -1; x <= 1; x++) {
            let cell_x = base_x + x;
            let column_seed = wisp_hash(vec2<i32>(cell_x, 137 + layer * 31));
            // Distant drops cross the image more slowly while retaining rapid downward travel.
            let velocity = (1.25 + column_seed * 0.8) * 1.7 / (1.0 + depth * 0.25);
            let column_p = p - vec2<f32>(0.0, t * grid * velocity);
            let base_y = i32(floor(column_p.y));
            for (var y = -reach; y <= reach; y++) {
                let cell = vec2<i32>(cell_x, base_y + y);
                let seed = wisp_hash(cell + vec2<i32>(layer * 43, layer * 59));
                let seed2 = wisp_hash(cell + vec2<i32>(71 + layer * 29, 19));
                let variation = wisp_hash(cell + vec2<i32>(109, 83 + layer * 37));
                if variation < 0.24 { continue; }
                let center = vec2<f32>(cell) + vec2<f32>(seed, seed2);
                let d = column_p - center;
                let half_length = radius * (0.7 + seed2 * seed2 * length_variation) * shape.y;
                let half_width = radius * (0.025 + seed * seed * 0.065) * shape.x;
                let blur = half_width + pixel * (0.55 + (2.0 - depth) * 0.2);
                if abs(d.x) > blur * 2.5 || abs(d.y) > half_length { continue; }
                let longitudinal = d.y / half_length;
                let tail = smoothstep(-1.0, -0.35, longitudinal);
                let head = 1.0 - smoothstep(0.6, 1.0, longitudinal);
                let cross_section = exp(-2.0 * d.x * d.x / (blur * blur));
                // Subpixel drops lose brightness instead of becoming equally bright one-pixel lines.
                let visibility = half_width / blur;
                let opacity = (0.15 + variation * variation * 0.85) / (1.0 + depth * 0.6);
                coverage += cross_section * tail * head * visibility * opacity;
            }
        }
    }
    return 1.0 - exp(-coverage * 1.6);
}
// Neighbor reach follows the maximum footprint (at most 7 x 7 cells), preventing
// large or elongated drops from popping at cell boundaries.
fn wisp_weather(uv: vec2<f32>, density: f32, size: f32, speed: f32, kind: u32, axis_scale: vec2<f32>) -> vec4<f32> {
    let grid = clamp(density, 1.0, 32.0);
    let radius = clamp(size, 0.03, 0.45);
    let shape = clamp(axis_scale, vec2<f32>(0.2), vec2<f32>(2.0));
    let t = globals.time * clamp(speed, 0.0, 5.0);
    let aspect = globals.resolution.x / max(globals.resolution.y, 1.0);
    let p = (uv - vec2<f32>(0.5)) * vec2<f32>(aspect, 1.0) * grid;
    let footprint = select(radius * 1.4 * shape * vec2<f32>(1.12, 1.85), vec2<f32>(0.8 * 1.4) * shape, kind == 2u);
    let reach = vec2<i32>(ceil(max(footprint, vec2<f32>(0.0, 0.85)) + vec2<f32>(0.33)));
    let base_x = i32(floor(p.x));
    var result = vec4<f32>(0.0);
    for (var x = -3; x <= 3; x++) {
        if abs(x) > reach.x { continue; }
        let cell_x = base_x + x;
        let column_seed = wisp_hash(vec2<i32>(cell_x, 137));
        let cycle = t * (0.07 + column_seed * 0.04) + column_seed * 5.0;
        let phase = fract(cycle);
        let slide = floor(cycle) * 1.2 + smoothstep(0.62, 0.92, phase) * 1.2 + t * 0.01;
        let column_p = p - vec2<f32>(0.0, select(slide, 0.0, kind == 2u));
        let activity = (smoothstep(0.60, 0.70, phase) * (1.0 - smoothstep(0.94, 1.0, phase))) * select(0.0, 1.0, speed > 0.0);
        let base_y = i32(floor(column_p.y));
        for (var y = -3; y <= 3; y++) {
            if abs(y) > reach.y { continue; }
            let cell = vec2<i32>(cell_x, base_y + y);
            let seed = wisp_hash(cell);
            let seed2 = wisp_hash(cell + vec2<i32>(71, 19));
            let center = vec2<f32>(cell) + vec2<f32>(0.18 + seed * 0.64, 0.18 + seed2 * 0.64);
            let d = column_p - center;
            let variation = wisp_water_size(seed2);
            let age = fract(t * (0.12 + seed2 * 0.09) + seed);
            if kind == 2u {
                let q = d / shape;
                let r = length(q);
                let spread = age * 0.8 * variation;
                let width = max(radius * 0.15 * sqrt(variation), 0.005);
                let ring = exp(-pow((r - spread) / width, 2.0)) * (1.0 - age);
                let slope = q / max(r, 0.001) / shape * ring * (r - spread) / width * 1.7;
                result += vec4<f32>(slope, ring * radius * variation * 0.18, ring);
            } else {
                let drop_radius = radius * variation;
                let breathing = 0.94 + 0.06 * sin(t * 0.35 + seed * 6.2831853);
                let elongation = 0.9 + seed * 0.35;
                var cap = wisp_water_cap(d / shape, drop_radius * breathing, elongation, seed);
                cap = vec4<f32>(cap.xy / shape, cap.zw);
                if cap.z > result.z { result = cap; }
                let track = wisp_water_track(d, drop_radius, shape, seed, activity);
                if track.z > result.z { result = track; }
            }
        }
    }
    return vec4<f32>(result.xy, result.z / grid, clamp(result.w, 0.0, 1.0));
}
fn wisp_water_light(normal: vec3<f32>, coverage: f32) -> vec4<f32> {
    let reflection = reflect(vec3<f32>(0.0, 0.0, -1.0), normal);
    let fresnel = 0.0204 + 0.9796 * pow(1.0 - max(normal.z, 0.0), 5.0);
    // A few distinct reflection shapes make the curved water surface readable.
    let sky = mix(vec3<f32>(0.07, 0.09, 0.12), vec3<f32>(0.45, 0.58, 0.7), clamp(-reflection.y * 0.5 + 0.5, 0.0, 1.0));
    let window_x = 1.0 - smoothstep(0.13, 0.19, abs(reflection.x + 0.38));
    let window_y = 1.0 - smoothstep(0.28, 0.38, abs(reflection.y + 0.34));
    let half_light = normalize(vec3<f32>(-0.38, -0.52, 1.7));
    let specular = pow(max(dot(normal, half_light), 0.0), 160.0) * 0.85;
    let secondary = pow(max(dot(normal, normalize(vec3<f32>(0.65, 0.8, 1.4))), 0.0), 90.0) * 0.22;
    let reflected = sky * fresnel + vec3<f32>(window_x * window_y) * (0.08 + fresnel * 0.8) + vec3<f32>(specular + secondary);
    return vec4<f32>(reflected * coverage, fresnel * coverage);
}
