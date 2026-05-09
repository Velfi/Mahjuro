// Stephen Hill / Narkowicz ACES fitted — single definition prepended to `shop_glb`, `lit_mesh`,
// `tile_3d`, and `tile_outline` in `wgpu_renderer/init.rs` (keep in sync with `tonemap_composite.wgsl`).

fn aces_fitted(color: vec3<f32>) -> vec3<f32> {
    let a = 2.51;
    let b = 0.03;
    let c = 2.43;
    let d = 0.59;
    let e = 0.14;
    return clamp(
        (color * (a * color + b)) / (color * (c * color + d) + e),
        vec3<f32>(0.0),
        vec3<f32>(1.0),
    );
}
