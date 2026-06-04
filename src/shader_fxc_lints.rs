//! Guardrails for DX12/FXC shader compatibility.
//!
//! Steam Deck running the Windows build through Proton/gamescope can end up on
//! DX12 + FXC when DXC is unavailable. Some WGSL patterns that are otherwise
//! valid have tripped FXC with:
//!   "Shader model 5.1+ resource array"
//!   "Array size is not a positive integer constant"
//!
//! These tests keep known-problem startup shaders on a safer `textureLoad` path
//! so regressions are caught in `cargo test` / CI before runtime.

fn assert_fxc_sampler_safe(shader_name: &str, src: &str) {
    // Plain samplers are the risky path here (`sampler_comparison` is unrelated
    // and not used by these shaders).
    assert!(
        !src.contains(": sampler;"),
        "{shader_name} reintroduced a plain sampler binding. \
         Keep this shader on textureLoad-only sampling for DX12/FXC safety."
    );
    assert!(
        !src.contains("textureSample("),
        "{shader_name} reintroduced textureSample(). \
         Keep this shader on textureLoad-only sampling for DX12/FXC safety."
    );
    assert!(
        src.contains("textureLoad("),
        "{shader_name} should use textureLoad() so FXC path stays compatible."
    );
}

fn assert_no_implicit_compare_sampling(shader_name: &str, src: &str) {
    assert!(
        !src.contains("textureSampleCompare("),
        "{shader_name} reintroduced textureSampleCompare(). \
         Use textureSampleCompareLevel() to avoid FXC loop-unroll failures."
    );
}

#[test]
fn boot_splash_wgsl_stays_fxc_safe() {
    let src = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/shaders/boot_splash.wgsl"
    ));
    assert_fxc_sampler_safe("boot_splash.wgsl", src);
}

#[test]
fn moonlit_water_wgsl_stays_fxc_safe() {
    let src = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/shaders/moonlit_water.wgsl"
    ));
    assert_fxc_sampler_safe("moonlit_water.wgsl", src);
}

#[test]
fn projected_shadow_wgsl_stays_compare_level_only() {
    let src = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/shaders/projected_shadow.wgsl"
    ));
    assert_no_implicit_compare_sampling("projected_shadow.wgsl", src);
    assert!(
        src.contains("textureSampleCompareLevel("),
        "projected_shadow.wgsl should use textureSampleCompareLevel() for FXC safety."
    );
}

#[test]
fn main_material_shaders_do_not_use_implicit_compare_sampling() {
    let tile_3d = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/shaders/tile_3d.wgsl"));
    let room_glb = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/shaders/room_glb.wgsl"));
    let lit_mesh = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/shaders/lit_mesh.wgsl"));
    assert_no_implicit_compare_sampling("tile_3d.wgsl", tile_3d);
    assert_no_implicit_compare_sampling("room_glb.wgsl", room_glb);
    assert_no_implicit_compare_sampling("lit_mesh.wgsl", lit_mesh);
}
