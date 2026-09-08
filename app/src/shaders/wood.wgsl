// Procedural hewn-wood grain, layered on top of Bevy's StandardMaterial PBR.
//
// Grain runs along the +X axis in world space. Growth rings are a warped
// sine over the Z coordinate; fine streaks add high-frequency variation
// along the grain; a slow low-frequency wash gives each plank a slightly
// different tone. Roughness follows the ring pattern so the late-wood bands
// catch the window light differently from the early wood.

#import bevy_pbr::{
    pbr_fragment::pbr_input_from_standard_material,
    pbr_functions::alpha_discard,
}

#ifdef PREPASS_PIPELINE
#import bevy_pbr::{
    prepass_io::{VertexOutput, FragmentOutput},
    pbr_deferred_functions::deferred_output,
}
#else
#import bevy_pbr::{
    forward_io::{VertexOutput, FragmentOutput},
    pbr_functions::{apply_pbr_lighting, main_pass_post_lighting_processing},
}
#endif

struct Wood {
    // Early wood (light) and late wood (dark) tones.
    light: vec4<f32>,
    dark: vec4<f32>,
    // World units → grain units.
    scale: f32,
    // Rings per grain unit across the grain.
    ring_freq: f32,
    // How much noise bends the rings.
    warp: f32,
    // Strength of fine streaks.
    streak: f32,
    // Plank width across the grain (grain units); 0 = one solid piece.
    plank_w: f32,
    _pad: vec3<f32>,
}

@group(#{MATERIAL_BIND_GROUP}) @binding(100)
var<uniform> wood: Wood;

fn hash21(p: vec2<f32>) -> f32 {
    var q = fract(p * vec2<f32>(123.34, 456.21));
    q = q + dot(q, q + 45.32);
    return fract(q.x * q.y);
}

fn vnoise(p: vec2<f32>) -> f32 {
    let i = floor(p);
    let f = fract(p);
    let u = f * f * (3.0 - 2.0 * f);
    let a = hash21(i);
    let b = hash21(i + vec2<f32>(1.0, 0.0));
    let c = hash21(i + vec2<f32>(0.0, 1.0));
    let d = hash21(i + vec2<f32>(1.0, 1.0));
    return mix(mix(a, b, u.x), mix(c, d, u.x), u.y);
}

fn fbm(p: vec2<f32>) -> f32 {
    var v = 0.0;
    var a = 0.5;
    var q = p;
    for (var i = 0; i < 4; i++) {
        v += a * vnoise(q);
        q = q * 2.03 + vec2<f32>(17.1, 9.7);
        a *= 0.5;
    }
    return v;
}

@fragment
fn fragment(
    in: VertexOutput,
    @builtin(front_facing) is_front: bool,
) -> FragmentOutput {
    var pbr_input = pbr_input_from_standard_material(in, is_front);

    var p = in.world_position.xz * wood.scale;

    // Planks: each strip across the grain is its own piece of timber with
    // its own grain offset and tone, and a dark seam at the joint.
    var seam = 0.0;
    var plank_tone = 0.0;
    if (wood.plank_w > 0.0) {
        let pid = floor(p.y / wood.plank_w);
        let f = fract(p.y / wood.plank_w);
        let h = hash21(vec2<f32>(pid, 3.7));
        // Staggered end joints along the grain.
        let along = p.x / (wood.plank_w * 6.0) + h * 3.0;
        let fa = fract(along);
        seam = max(
            1.0 - smoothstep(0.0, 0.035, min(f, 1.0 - f)),
            1.0 - smoothstep(0.0, 0.012, min(fa, 1.0 - fa)),
        );
        plank_tone = (h - 0.5) * 0.35;
        // Shift the ring pattern per plank so neighbours don't line up.
        p.y += h * 37.0;
        p.x += hash21(vec2<f32>(pid, floor(along))) * 91.0;
    }

    // Rings: sine across the grain (z), warped by low-frequency noise.
    let bend = fbm(p * vec2<f32>(0.15, 0.6)) - 0.5;
    let ring_phase = (p.y + bend * wood.warp) * wood.ring_freq;
    var rings = 0.5 + 0.5 * sin(ring_phase);
    // Sharpen: real late wood is a narrow dark band.
    rings = smoothstep(0.35, 0.9, rings);

    // Fine streaks along the grain: high frequency across, low along.
    let streaks = fbm(p * vec2<f32>(0.9, 14.0));
    // Slow tonal wash so long boards aren't uniform.
    let wash = fbm(p * vec2<f32>(0.05, 0.08));

    var t = rings * 0.7 + (streaks - 0.5) * wood.streak + (wash - 0.5) * 0.35 + plank_tone;
    t = clamp(t, 0.0, 1.0);

    var grain = mix(wood.light, wood.dark, t);
    grain = mix(grain, wood.dark * 0.55, seam * 0.85);
    pbr_input.material.base_color = pbr_input.material.base_color * grain;
    // Late wood is a touch glossier; seams are rough.
    pbr_input.material.perceptual_roughness =
        clamp(pbr_input.material.perceptual_roughness - rings * 0.12 + seam * 0.3, 0.05, 1.0);

    pbr_input.material.base_color = alpha_discard(pbr_input.material, pbr_input.material.base_color);

#ifdef PREPASS_PIPELINE
    let out = deferred_output(in, pbr_input);
#else
    var out: FragmentOutput;
    out.color = apply_pbr_lighting(pbr_input);
    out.color = main_pass_post_lighting_processing(pbr_input, out.color);
#endif
    return out;
}
