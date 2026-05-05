// 3D billboarded candle-flame particles.
//
// Ported from a Godot "stylized flame" particle shader. Each live
// particle is rendered as a camera-facing quad in world space. The
// vertex stage constructs the billboard axes from the camera's inverse
// view matrix so the quad always faces the viewer; the fragment stage
// samples procedural noise at the quad's local UV, distorts the UVs,
// and dissolves the shape against an age threshold.
//
// Instance data (see [`crate::render::flame_particles::GpuFlameParticle`]):
//   inst_pos_age.xyz  — world-space particle center
//   inst_pos_age.w    — normalized age in [0,1] (0 = just spawned)
//   inst_params.x     — particle half-extent in world units (billboard size)
//   inst_params.y     — per-particle random phase in [0, 2π]
//   inst_params.z     — brightness multiplier [0, 1+]
//   inst_params.w     — cross_slice: 0 / 1 — second vertical plane 90° in XY
//
// The vertex buffer is a [0..1]² unit quad (the same four verts the 2D
// UI uses). We recentre to [-0.5..0.5] for the billboard-X axis (so the
// particle is horizontally centered on the wick) but keep the raw [0..1]
// range for billboard-Y (so the particle's BASE is at the wick and it
// grows upward from there).
//
// Output is premultiplied: the pipeline blends (SrcAlpha, One) so the
// additive contribution stacks across overlapping particles — which is
// how the full flame silhouette is built.

struct Globals {
    screen: vec2<f32>,
    time: f32,
    gamma: f32,
};
@group(0) @binding(0) var<uniform> globals: Globals;

// View uniform for the flame pipeline. Populated by the renderer each
// frame from the same camera matrices the lit-mesh pipeline uses — so
// flame billboards project to exactly the same pixels the 3D scene
// does — but packaged as its own small uniform so the binding can be
// visible to the vertex stage (the lit-mesh SSR layout exposes its
// matrices to fragment only).
struct FlameView {
    view_proj: mat4x4<f32>,
    view_pos: vec4<f32>,
};
@group(1) @binding(0) var<uniform> view: FlameView;

struct VsOut {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) age: f32,
    @location(2) phase: f32,
    @location(3) brightness: f32,
    @location(4) cross_slice: f32,
};

@vertex
fn vs_main(
    // Unit-quad corner. The scene's vertex buffer supplies `[-0.5..0.5]²`
    // in a single f32x2 attribute; we need to map that to a uv in [0,1].
    @location(0) corner: vec2<f32>,
    @location(1) inst_pos_age: vec4<f32>,
    @location(2) inst_params: vec4<f32>,
) -> VsOut {
    let center_world = inst_pos_age.xyz;
    let age = inst_pos_age.w;
    let half_extent = inst_params.x;
    let phase = inst_params.y;
    let brightness = inst_params.z;
    let cross_slice = inst_params.w;

    // Build camera-aligned billboard axes from the view matrix (Z-up
    // world space). `cam_forward` is -Z in view space, which we pull
    // out of the inverse view-projection matrix by unprojecting a
    // point slightly in front of the camera. Simpler: use the rows of
    // the view_proj to recover camera-local axes.
    //
    // We want:
    //   up_billboard    — roughly world up (flame rises visibly even
    //                     when camera looks straight down).
    //   right_billboard — perpendicular to both world up and the view
    //                     direction, so the billboard faces the camera.
    //
    // Compute the view direction from particle → camera.
    let cam_pos = view.view_pos.xyz;
    let view_dir = normalize(cam_pos - center_world);

    // Constrain the billboard up to world +Z (flame always points up).
    // Right axis = view_dir × world_up, then re-orthogonalize up to
    // maintain a clean square quad.
    let world_up = vec3<f32>(0.0, 0.0, 1.0);
    var right = cross(world_up, view_dir);
    let right_len = length(right);
    if (right_len < 1e-4) {
        // Degenerate case: camera looks straight up/down. Pick an
        // arbitrary horizontal axis so the billboard still has area.
        right = vec3<f32>(1.0, 0.0, 0.0);
    } else {
        right = right / right_len;
    }
    // Second instance: same vertical axis, horizontal billboard axis
    // rotated 90° in the world XY plane → cross-shaped slices read as
    // volumetric from the gameplay camera without a full 3D fluid solve.
    if (cross_slice >= 0.5) {
        right = normalize(cross(world_up, right));
    }
    // Force billboard up to world +Z (stretch the flame vertically).
    let up = world_up;

    // Particle quads are much taller than wide so that stacked
    // billboards form a continuous column rather than reading as a
    // row of pancakes. Narrowed further on x (0.75×) so each slice is
    // visibly narrower than the wick itself — the aggregate plume
    // then builds up silhouette from the overlap pattern, not from
    // each particle being big.
    let w = half_extent * 0.75;
    let h = half_extent * 2.4;
    // corner is in [0,1]. Recentre x → [-0.5, 0.5] so the billboard is
    // horizontally centered on the wick. Keep y as [0,1] so the base
    // of the billboard sits at the particle center and the top rises
    // upward — particles spawn at the wick and should grow from it, not
    // straddle it.
    let local_x = (corner.x - 0.5) * w * 2.0;
    let local_y = corner.y * h * 2.0;
    let world = center_world + right * local_x + up * local_y;

    var out: VsOut;
    out.clip_position = view.view_proj * vec4<f32>(world, 1.0);
    out.uv = corner;
    out.age = age;
    out.phase = phase;
    out.brightness = brightness;
    out.cross_slice = cross_slice;
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
    let u = f * f * f * (f * (f * 6.0 - 15.0) + 10.0);
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
    // UV is [0,1] across the billboard. (0,0) = bottom-left corner.
    // The vertex shader puts the particle base near y=0, so y≈0 is the
    // hot bottom and y≈1 is the cool fading tip.
    let u = in.uv.x;
    let v = in.uv.y;
    // De-correlate procedural noise between the two cross planes.
    let slice_phase = in.phase + in.cross_slice * 1.57079632679;

    // ── Particle silhouette ─────────────────────────────────────────
    // Asymmetric teardrop — wide at the base, tapers to the top. Two
    // key shape tweaks vs. the previous cauliflower:
    //   * The horizontal falloff is quadratic + raised to a power, so
    //     the particle silhouette is very soft at its edges and
    //     overlapping particles blend additively into a continuous
    //     column instead of showing as rows of lumps.
    //   * The taper is explicitly `1 - v` so the particle narrows as
    //     it rises — the tip of a real flame-like particle disappears
    //     to a point before the dissolve even fires.
    let cx = u - 0.5;
    // Vertical profile: bulge at v≈0.15, taper through v≈1.0.
    let profile = smoothstep(0.0, 0.12, v) * (1.0 - pow(v, 1.4));
    let half_width = profile * 0.5 + 0.04;
    let dx = abs(cx) - half_width;
    // `shape` is a soft silhouette mask — not a hard distance SDF —
    // so overlap reads as continuous light rather than stacked blobs.
    let shape = pow(clamp(-dx * 6.0, 0.0, 1.0), 1.2);
    if (shape < 0.005) {
        discard;
    }

    // ── Noise: distortion + dissolve ────────────────────────────────
    let t = globals.time;
    // Distortion field — scrolls upward with time + per-particle phase.
    let dist_uv = vec2<f32>(
        u * 0.8 + sin(slice_phase + t * 0.3) * 0.05,
        v * 0.7 - t * 0.9 + slice_phase * 0.15,
    );
    let dist = fbm(dist_uv) - 0.5;
    let distortion = vec2<f32>(dist * 0.14, dist * 0.22);

    // Dissolve field — finer + faster scroll.
    let dissolve_uv = vec2<f32>(
        u * 1.8 + distortion.x + cos(slice_phase * 1.7 + t * 0.5) * 0.04,
        v * 2.1 + distortion.y - t * 1.4 + slice_phase * 0.25,
    );
    let dissolve = fbm(dissolve_uv);

    // Age-driven threshold. Young particles (age≈0) survive almost any
    // noise value; old particles (age≈1) only the densest peaks.
    let feather = 0.10;
    let threshold = pow(in.age, 0.85);
    let threshold_min = threshold - feather;
    let threshold_max = threshold + feather;
    let field = dissolve * 0.9 + 0.12;
    let dissolve_mask = smoothstep(threshold_min, threshold_max, field);

    // Alpha: shape × dissolve. Soft power so bleeding into neighbours
    // looks like volumetric light, not discrete blobs.
    var alpha = shape * dissolve_mask;
    alpha = pow(alpha, 0.7);
    // Two orthogonal slices per particle — scale so stacked brightness
    // stays close to the old single-plane look.
    alpha = alpha * 0.64;
    if (alpha < 0.005) {
        discard;
    }

    // ── Colour gradient along the particle ──────────────────────────
    // Real candle flame colour chart, bottom → top:
    //   * tiny near-white nucleus right at the wick (thin blue-white
    //     band in real life; rendered as warm-white here because the
    //     table's ambient lighting is already warm).
    //   * bright yellow for the hot body — the visible bulk of the flame.
    //   * saturated orange on the outside of the body.
    //   * deep red at the cool tip and rim where combustion is ending.
    //
    // We blend by vertical position `v`, warped by particle age so
    // older particles are cooler even if they're still near the base.
    let white_hot = vec3<f32>(1.00, 0.97, 0.82);
    let yellow    = vec3<f32>(1.00, 0.85, 0.35);
    let orange    = vec3<f32>(1.00, 0.48, 0.08);
    let red       = vec3<f32>(0.95, 0.18, 0.02);

    // `height_t`: 0 near base, 1 near tip. Age slides everyone toward
    // the tip (cooler) end of the palette as they rise.
    let height_t = clamp(v + in.age * 0.4, 0.0, 1.0);

    // Build the gradient in two stops: yellow→orange in the lower
    // half, orange→red in the upper half.
    var col = mix(yellow, orange, smoothstep(0.15, 0.55, height_t));
    col = mix(col, red, smoothstep(0.55, 1.00, height_t));

    // White-hot nucleus: a narrow spot at the very bottom-centre of
    // the particle, gated to youth (only young particles have it) and
    // to the core of the shape (so it doesn't bloom outward). Low
    // strength — stacked additive particles push it brighter on their
    // own where they overlap.
    let nucleus_d = length(vec2<f32>(cx * 3.0, (v - 0.05) * 2.4));
    let nucleus = pow(clamp(1.0 - nucleus_d * 2.2, 0.0, 1.0), 2.5)
                * (1.0 - smoothstep(0.0, 0.4, in.age));
    col = mix(col, white_hot, nucleus * 0.6);

    // ── Emission ────────────────────────────────────────────────────
    // Keep the per-particle emission modest — the final brightness
    // comes from many overlapping particles stacking additively, not
    // from any one particle blowing out to white. The previous 2.4×
    // was clipping the nucleus everywhere.
    //
    // Age ramp: hottest while young (1.0×), dim by end of life (0.3×).
    let age_warmth = mix(1.0, 0.3, in.age);
    // Height ramp: the body of the flame is brighter than the tip; the
    // outer wisps are visibly cooler in a real flame.
    let height_warmth = 1.0 - height_t * 0.55;
    // Fast sub-flicker on top of the CPU-driven per-emitter envelope.
    let hf_flicker = 1.0
        + 0.065 * sin(t * 12.8 + slice_phase * 1.1) * sin(t * 5.05 + in.phase * 0.8);
    let emission = 1.35 * age_warmth * height_warmth * in.brightness * hf_flicker;

    // ── Hot dissolve edge ───────────────────────────────────────────
    // The dissolving frontier (noise ≈ threshold) is hotter than the
    // solid body: it's where combustion is actively happening. Pulls
    // toward yellow with a small bias to white so the edge reads
    // brighter than whatever pixel is next to it.
    let rim_falloff = smoothstep(threshold_max, threshold_max + feather * 1.5, field);
    let edge_strength = clamp(dissolve_mask - rim_falloff, 0.0, 1.0);
    let edge_tint = mix(yellow, white_hot, 0.4);
    let edge_light = edge_tint * edge_strength * 1.2 * age_warmth;

    let rgb = col * emission + edge_light;

    // Premultiplied output for the additive (SrcAlpha, One) blend.
    // No gamma baked in — the sRGB colour target applies the transfer
    // itself; double-applying crushed the cores in the 2D predecessor.
    let out_rgb = rgb * alpha;
    return vec4<f32>(out_rgb, clamp(alpha, 0.0, 1.0));
}
