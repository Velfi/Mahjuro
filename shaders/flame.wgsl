// Procedural candle flame on an additive quad.
//
// Instance data:
//   rect    — bounding box of the flame in screen pixels
//   color.rgb — flame tint multiplier (use [1,1,1] for the default look)
//   color.a   — per-instance phase offset in [0,1]; randomises noise + flicker
//               so neighbouring candles don't beat in sync.
//
// The fragment shader builds a teardrop-shaped flame, displaces its boundary
// with 2-octave value noise advected upward over time, and shades a hot
// inner core surrounded by an orange halo. Output is premultiplied: the
// pipeline blend is (SrcAlpha, One) so the flame *adds* light to whatever's
// behind it without overwriting the table.

struct Globals {
    screen: vec2<f32>,
    time: f32,
    _pad: f32,
};

@group(0) @binding(0) var<uniform> globals: Globals;

struct VsOut {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) tint: vec3<f32>,
    @location(2) phase: f32,
};

@vertex
fn vs_main(
    @location(0) corner: vec2<f32>,
    @location(1) rect: vec4<f32>,
    @location(2) color: vec4<f32>,
) -> VsOut {
    let x = rect.x + corner.x * rect.z;
    let y = rect.y + corner.y * rect.w;
    let nx = (x / globals.screen.x) * 2.0 - 1.0;
    let ny = 1.0 - (y / globals.screen.y) * 2.0;
    var out: VsOut;
    out.clip_position = vec4<f32>(nx, ny, 0.0, 1.0);
    out.uv = corner;
    out.tint = color.rgb;
    out.phase = color.a;
    return out;
}

// ── Noise helpers ──────────────────────────────────────────────────────────
fn hash21(p: vec2<f32>) -> f32 {
    let h = dot(p, vec2<f32>(127.1, 311.7));
    return fract(sin(h) * 43758.5453123);
}

fn vnoise(p: vec2<f32>) -> f32 {
    let i = floor(p);
    let f = fract(p);
    let u = f * f * (3.0 - 2.0 * f);
    let a = hash21(i + vec2<f32>(0.0, 0.0));
    let b = hash21(i + vec2<f32>(1.0, 0.0));
    let c = hash21(i + vec2<f32>(0.0, 1.0));
    let d = hash21(i + vec2<f32>(1.0, 1.0));
    return mix(mix(a, b, u.x), mix(c, d, u.x), u.y);
}

fn fbm(p: vec2<f32>) -> f32 {
    var v = 0.0;
    var amp = 0.5;
    var pp = p;
    for (var i = 0; i < 3; i = i + 1) {
        v = v + amp * vnoise(pp);
        pp = pp * 2.02;
        amp = amp * 0.5;
    }
    return v;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    // uv is [0,1] across the flame quad. Centre x at 0, y up = bottom.
    let cx = in.uv.x - 0.5;
    let cy = 1.0 - in.uv.y;          // 0 at base, 1 at tip
    let phase = in.phase * 6.2831853;
    let t = globals.time;

    // ── Outer envelope ──────────────────────────────────────────────
    // Asymmetric teardrop: narrow cup at the very base (where it
    // meets the wick), bulging out slightly above, then tapering to
    // a sharp tip. The narrow base reads as "the flame springs from
    // a point on the wick" — broad bases look like clouds.
    let bulge = smoothstep(0.0, 0.25, cy) * (1.0 - smoothstep(0.25, 1.0, cy));
    let half_width = mix(0.05, 0.40, bulge) * (1.0 - cy * cy * 0.6) + 0.04;

    // Two octaves of advected fbm so the silhouette ripples like real
    // combustion. Lateral wobble shears the upper half so the tip
    // dances in the air current.
    let n_uv = vec2<f32>(cx * 2.5 + sin(t * 1.7 + phase) * 0.08, cy * 3.0 - t * 2.4 + phase);
    let n = fbm(n_uv) - 0.5;
    let lateral_wobble = sin(t * 9.0 + phase + cy * 6.0) * 0.05 * cy;

    let dx_norm = abs(cx + lateral_wobble) - (half_width + n * 0.16 * cy);

    // Soft outer mask — clamp the negative-distance region to alpha.
    let outer = clamp(-dx_norm * 9.0, 0.0, 1.0);

    // ── Inner volumetric layer ──────────────────────────────────────
    // A second smaller flame INSIDE the first, sampled with a
    // different noise phase. Compositing two layers of differing size
    // is the cheapest way to fake volume in 2D — the inner layer
    // brightens the core where they overlap and creates plume detail.
    let inner_half = (half_width * 0.55 + n * 0.05) * (1.0 - smoothstep(0.0, 0.85, cy));
    let n2_uv = vec2<f32>(cx * 4.0 + cos(t * 2.3 + phase * 1.7) * 0.06, cy * 5.0 - t * 3.6 + phase * 1.3);
    let n2 = fbm(n2_uv) - 0.5;
    let dx_inner = abs(cx) - (inner_half + n2 * 0.04);
    let inner = clamp(-dx_inner * 14.0, 0.0, 1.0);

    // Vertical envelope: cup the base (so the wick joint reads dark),
    // peak low, fade to a point at the tip.
    let base_fade = smoothstep(0.0, 0.10, cy);
    let tip_fade = 1.0 - smoothstep(0.55, 1.00, cy);
    let env = base_fade * tip_fade;

    // Per-frame brightness flicker.
    let flicker = 0.85 + 0.15 * sin(t * 11.0 + phase) + 0.08 * sin(t * 19.0 + phase * 1.3);

    let alpha = outer * env * flicker;

    // ── Volumetric shading ──────────────────────────────────────────
    // Build a "depth into the flame" coordinate from the inner mask
    // and the radial distance from the silhouette: pixels deep inside
    // the inner layer + low in the flame = the hot core. This is
    // what gives the 2D quad its 3D feel.
    let depth = inner * (1.0 - smoothstep(0.0, 0.55, cy));
    let core_dist = length(vec2<f32>(cx * 2.2, (cy - 0.16) * 1.4));
    let core_mask = pow(clamp(1.0 - core_dist * 1.8, 0.0, 1.0), 2.2);
    let hot = clamp(depth * 0.9 + core_mask * 0.7, 0.0, 1.0);

    // Palette: deep red at the silhouette → orange in the body →
    // bright yellow in the inner plume → near-white at the wick cup.
    // Real saturated colors instead of pale washes so the flame stops
    // looking like fog when blended additively over the bright table.
    let rim    = vec3<f32>(0.85, 0.18, 0.04);
    let orange = vec3<f32>(1.00, 0.50, 0.10);
    let yellow = vec3<f32>(1.00, 0.86, 0.32);
    let white  = vec3<f32>(1.00, 0.98, 0.80);

    // Color from outside-in. `rim_t` is high on the silhouette edge.
    let rim_t = clamp(-dx_norm * 4.0, 0.0, 1.0);
    var body = mix(rim, orange, smoothstep(0.0, 0.55, rim_t));
    body = mix(body, yellow, hot);
    body = mix(body, white, pow(hot, 3.0) * 0.85);

    // Tiny dark notch right where the flame meets the wick — a
    // thin desaturation band for the first ~6% of cy. This is the
    // classic dark cup at the base of every real candle flame and
    // it sells the volumetric look more than any noise term.
    let cup = 1.0 - smoothstep(0.00, 0.06, cy);
    body = mix(body, vec3<f32>(0.18, 0.06, 0.02), cup * 0.6);

    let rgb = body * in.tint;

    // Premultiplied output for additive blending. Boost slightly so
    // saturated cores still pop on the bright wood table; the alpha
    // term keeps the silhouette soft.
    return vec4<f32>(rgb * alpha * 1.6, alpha);
}
