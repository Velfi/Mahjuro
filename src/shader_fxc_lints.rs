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
