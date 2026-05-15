//! WGSL sources embedded at compile time. Kept out of [`super::init`] so the
//! giant `WgpuRenderer::new` function does not host every `include_str!`
//! literal (reduces LLVM pressure).

macro_rules! wgsl_file {
    ($file:literal) => {
        include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/shaders/", $file))
    };
}

pub const QUAD: &str = wgsl_file!("quad.wgsl");
pub const TEXT_QUAD: &str = wgsl_file!("text_quad.wgsl");
pub const GRADIENT_QUAD: &str = wgsl_file!("gradient_quad.wgsl");
pub const SQUIRCLE_QUAD: &str = wgsl_file!("squircle_quad.wgsl");
pub const FLAME: &str = wgsl_file!("flame.wgsl");
pub const STARFIELD: &str = wgsl_file!("starfield.wgsl");
pub const EMBER_DRIFT: &str = wgsl_file!("ember_drift.wgsl");
pub const GOLDEN_DUST: &str = wgsl_file!("golden_dust.wgsl");
pub const MOONLIT_WATER: &str = wgsl_file!("moonlit_water.wgsl");
pub const SUNLIT_WATER: &str = wgsl_file!("sunlit_water.wgsl");
pub const SHOOTING_STAR_CASCADE: &str = wgsl_file!("shooting_star_cascade.wgsl");
pub const SHOOTING_STAR_CASCADE_COMPOSITE: &str =
    wgsl_file!("shooting_star_cascade_composite.wgsl");
pub const SCENE_COLOR_DOWNSAMPLE: &str = wgsl_file!("scene_color_downsample.wgsl");
pub const TILE_GLOW: &str = wgsl_file!("tile_glow.wgsl");
pub const SHADOW: &str = wgsl_file!("shadow.wgsl");
pub const IMAGE_QUAD: &str = wgsl_file!("image_quad.wgsl");
pub const BLOOM_EXTRACT: &str = wgsl_file!("bloom_extract.wgsl");
pub const BLOOM_BLUR: &str = wgsl_file!("bloom_blur.wgsl");
pub const BLOOM_COMPOSITE: &str = wgsl_file!("bloom_composite.wgsl");
pub const TONEMAP_COMPOSITE: &str = wgsl_file!("tonemap_composite.wgsl");
pub const EMISSIVE_PROBE_UPDATE: &str = wgsl_file!("emissive_probe_update.wgsl");
pub const EMISSIVE_PROBE_APPLY: &str = wgsl_file!("emissive_probe_apply.wgsl");
pub const EMISSIVE_GI_COMPOSITE: &str = wgsl_file!("emissive_gi_composite.wgsl");

/// `scene_pbr_lights` + `scene_hdr_tonemap` + `tile_3d`
pub const TILE_3D: &str = concat!(
    include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/shaders/scene_pbr_lights.wgsl"
    )),
    "\n",
    include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/shaders/scene_hdr_tonemap.wgsl"
    )),
    "\n",
    include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/shaders/tile_3d.wgsl")),
);

/// `scene_pbr_lights` + `scene_hdr_tonemap` + `shop_glb`
pub const SHOP_GLB: &str = concat!(
    include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/shaders/scene_pbr_lights.wgsl"
    )),
    "\n",
    include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/shaders/scene_hdr_tonemap.wgsl"
    )),
    "\n",
    include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/shaders/shop_glb.wgsl"
    )),
);

/// `scene_hdr_tonemap` + `tile_outline`
pub const TILE_OUTLINE: &str = concat!(
    include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/shaders/scene_hdr_tonemap.wgsl"
    )),
    "\n",
    include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/shaders/tile_outline.wgsl"
    )),
);

/// `scene_pbr_lights` + `scene_hdr_tonemap` + `lit_mesh`
pub const LIT_MESH: &str = concat!(
    include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/shaders/scene_pbr_lights.wgsl"
    )),
    "\n",
    include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/shaders/scene_hdr_tonemap.wgsl"
    )),
    "\n",
    include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/shaders/lit_mesh.wgsl"
    )),
);
