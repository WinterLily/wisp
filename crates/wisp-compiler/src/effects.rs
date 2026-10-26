//! Fused layer/effect lowering. These functions exist only in generated shaders.
use super::*;
use wisp_ir::{BlendMode, WeatherKind};
impl Compiler<'_> {
    pub(super) fn effect(&self, op: &Op, source: &mut String) -> Option<String> {
        let v = |id: &str| format!("v{}", self.graph.index(id));
        let n = |id: &str| format!("n{}", self.graph.index(id));
        Some(match op {
            Op::Transform {
                uv,
                position,
                scale,
                rotation,
                pivot,
            } => format!(
                "wisp_transform({}, {}, {}, {}, {})",
                v(uv),
                v(position),
                v(scale),
                v(rotation),
                v(pivot)
            ),
            Op::Mask {
                color,
                feather,
                invert,
            } => {
                let color_fn = n(color);
                source.push_str(&format!(
                    "let f = clamp({}, 0.0, 0.1);\nvar coverage = 0.0;\n",
                    v(feather)
                ));
                // Fixed nine taps; white/black remain exact and grayscale is perceptual,
                // undoing the shared image resource's sRGB decode before luminance.
                for y in -1..=1 {
                    for x in -1..=1 {
                        source.push_str(&format!("coverage += wisp_gray({color_fn}(uv + vec2<f32>({x}.0, {y}.0) * f)) / 9.0;\n"));
                    }
                }
                format!(
                    "select(clamp(coverage, 0.0, 1.0), 1.0 - clamp(coverage, 0.0, 1.0), {})",
                    v(invert)
                )
            }
            Op::Layer {
                color,
                uv,
                opacity,
                mask,
            } => format!(
                "vec4<f32>({}.rgb, {}.a * clamp({}, 0.0, 1.0) * clamp({}, 0.0, 1.0) * select(0.0, 1.0, all({} >= vec2<f32>(0.0)) && all({} <= vec2<f32>(1.0))))",
                v(color),
                v(color),
                v(opacity),
                v(mask),
                v(uv),
                v(uv)
            ),
            Op::Blend { a, b, mode } => {
                let mode = match mode {
                    BlendMode::Alpha => 0,
                    BlendMode::Add => 1,
                    BlendMode::Multiply => 2,
                    BlendMode::Screen => 3,
                    BlendMode::Overlay => 4,
                };
                format!("wisp_blend({}, {}, {mode}u)", v(a), v(b))
            }
            Op::Wave {
                uv,
                mask,
                direction,
                amplitude,
                frequency,
                speed,
            }
            | Op::Warp {
                uv,
                mask,
                direction,
                amplitude,
                frequency,
                speed,
            } => {
                source.push_str(&format!("let dir = {} / max(length({}), 0.0001);\nlet phase = dot({}, vec2<f32>(-dir.y, dir.x)) * clamp({}, 0.0, 64.0) * 6.2831853 + globals.time * clamp({}, -10.0, 10.0);\n", v(direction), v(direction), v(uv), v(frequency), v(speed)));
                let wave = if matches!(op, Op::Warp { .. }) {
                    "sin(phase + sin(phase * 0.73) * 0.7)"
                } else {
                    "sin(phase)"
                };
                format!(
                    "{} + dir * {wave} * clamp({}, -0.25, 0.25) * clamp({}, 0.0, 1.0)",
                    v(uv),
                    v(amplitude),
                    v(mask)
                )
            }
            Op::Displacement {
                uv,
                map,
                mask,
                strength,
            } => format!(
                "{} + (wisp_srgb({}.rgb).rg * 2.0 - vec2<f32>(1.0)) * clamp({}, -0.25, 0.25) * clamp({}, 0.0, 1.0)",
                v(uv),
                v(map),
                v(strength),
                v(mask)
            ),
            Op::Weather {
                color,
                uv,
                mask,
                density,
                size,
                speed,
                strength,
                kind,
                rain_direction,
                size_x,
                size_y,
            } => {
                if *kind == WeatherKind::Droplets {
                    source.push_str(&format!(
                        "let water = wisp_glass_droplets({}, {}, {}, {}, vec2<f32>({size_x:?}, {size_y:?}));\n\
                         let domain = select(0.0, 1.0, all({} >= vec2<f32>(0.0)) && all({} <= vec2<f32>(1.0)));\n\
                         let amount = clamp({}, 0.0, 1.0) * clamp({}, 0.0, 1.0) * domain;\n\
                         let normal = normalize(vec3<f32>(water.xy, 1.0));\n\
                         let aspect = globals.resolution.x / max(globals.resolution.y, 1.0);\n\
                         let ray = refract(vec3<f32>(0.0, 0.0, -1.0), normal, 0.7501875);\n\
                         let offset = ray.xy / max(abs(ray.z), 0.1) * (0.3 + water.z * 1.8) / vec2<f32>(aspect, 1.0);\n\
                         let lens_uv = clamp(uv + offset * amount, vec2<f32>(0.000001), vec2<f32>(0.999999));\n\
                         let refracted = {}(lens_uv);\n\
                         let reflected_uv = clamp(uv + normal.xy * vec2<f32>(0.25 / aspect, -0.25), vec2<f32>(0.000001), vec2<f32>(0.999999));\n\
                         let reflected = {}(reflected_uv);\n\
                         let fresnel = 0.02 + 0.98 * pow(1.0 - normal.z, 5.0);\n\
                         let rim = smoothstep(0.45, 0.95, length(normal.xy));\n\
                         let caustic = pow(max(dot(normal, normalize(vec3<f32>(-0.35, -0.6, 1.0))), 0.0), 32.0);\n\
                         let glint = pow(max(dot(normal, normalize(vec3<f32>(-0.4, -0.65, 1.0))), 0.0), 96.0) * 0.25;\n\
                         let lens = refracted.rgb * (1.0 - rim * 0.85) * (1.0 - fresnel) + reflected.rgb * (fresnel + caustic * 0.35) + vec3<f32>(glint);\n\
                         let blur_step = vec2<f32>(0.0035 / aspect, 0.0035);\n\
                         var mist = {}.rgb * {}.a * 0.2;\n\
                         var mist_alpha = {}.a * 0.2;\n",
                        v(uv), v(density), v(size), v(speed), v(uv), v(uv), v(mask), v(strength),
                        n(color), n(color), v(color), v(color), v(color),
                    ));
                    // Premultiplied filtering prevents transparent image RGB bleeding into the glass.
                    for (x, y) in [
                        (1.0, 0.0),
                        (-1.0, 0.0),
                        (0.0, 1.0),
                        (0.0, -1.0),
                        (0.707, 0.707),
                        (-0.707, 0.707),
                        (0.707, -0.707),
                        (-0.707, -0.707),
                    ] {
                        source.push_str(&format!(
                            "{{ let tap = {}(clamp(uv + blur_step * vec2<f32>({x:?}, {y:?}), vec2<f32>(0.000001), vec2<f32>(0.999999))); mist += tap.rgb * tap.a * 0.1; mist_alpha += tap.a * 0.1; }}\n",
                            n(color),
                        ));
                    }
                    source.push_str(&format!(
                        "let backdrop = mix({}.rgb, mist / max(mist_alpha, 0.000001), amount * 0.85);\n",
                        v(color),
                    ));
                    // Coverage mixes the complete lens, keeping antialiased rims free of displaced halos.
                    return Some(format!(
                        "vec4<f32>(mix(backdrop, lens, water.w * amount), {}.a)",
                        v(color),
                    ));
                }
                let kind = match kind {
                    WeatherKind::Droplets => 0,
                    WeatherKind::Rain => 1,
                    WeatherKind::Ripples => 2,
                };
                source.push_str(&format!("let water = wisp_weather({}, {}, {}, {}, {kind}u, {rain_direction:?}, vec2<f32>({size_x:?}, {size_y:?}));\nlet domain = select(0.0, 1.0, all({} >= vec2<f32>(0.0)) && all({} <= vec2<f32>(1.0)));\nlet amount = clamp({}, 0.0, 1.0) * clamp({}, 0.0, 1.0) * domain;\nlet normal = normalize(vec3<f32>(water.xy, 1.0));\nlet ray = refract(vec3<f32>(0.0, 0.0, -1.0), normal, 0.7501875);\nlet aspect = globals.resolution.x / max(globals.resolution.y, 1.0);\nlet offset = ray.xy / max(abs(ray.z), 0.1) * water.z * 2.8 / vec2<f32>(aspect, 1.0);\nlet refracted = {}(clamp(uv + offset * amount, vec2<f32>(0.000001), vec2<f32>(0.999999)));\nlet lighting = wisp_water_light(normal, water.w) * amount;\n", v(uv), v(density), v(size), v(speed), v(uv), v(uv), v(mask), v(strength), n(color)));
                "vec4<f32>(refracted.rgb * (1.0 - lighting.a * 0.85) + lighting.rgb, refracted.a)"
                    .into()
            }
            _ => return None,
        })
    }
}
