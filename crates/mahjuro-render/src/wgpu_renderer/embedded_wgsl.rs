//! WGSL sources embedded at compile time. Kept out of [`super::init`] so the
//! giant `WgpuRenderer::new` function does not host every `include_str!`
//! literal (reduces LLVM pressure).

macro_rules! wgsl_file {
    ($file:literal) => {
        include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../shaders/",
            $file
        ))
    };
}

/// Lit room/tile shaders: hallway warp VS + PBR core + scene body + shadow atlas.
macro_rules! scene_pbr_with_hallway_warp {
    ($room_shader:literal) => {
        concat!(
            wgsl_file!("hallway_vertex_warp.wgsl"),
            "\n",
            wgsl_file!("scene_pbr_core.wgsl"),
            "\n",
            wgsl_file!("scene_pbr_lights.wgsl"),
            "\n",
            wgsl_file!("rainbow_swirl.wgsl"),
            "\n",
            wgsl_file!("moon_phase.wgsl"),
            "\n",
            wgsl_file!($room_shader),
            "\n",
            wgsl_file!("projected_shadow.wgsl"),
        )
    };
}

pub const QUAD: &str = wgsl_file!("quad.wgsl");
#[cfg(feature = "windowed")]
pub const BOOT_SPLASH: &str = wgsl_file!("boot_splash.wgsl");
pub const DEPTH_QUAD: &str = wgsl_file!("depth_quad.wgsl");
pub const DEPTH_QUAD_DEBUG: &str = wgsl_file!("depth_quad_debug.wgsl");
pub const TEXT_QUAD: &str = wgsl_file!("text_quad.wgsl");
pub const GRADIENT_QUAD: &str = wgsl_file!("gradient_quad.wgsl");
pub const ARC_RING_QUAD: &str = wgsl_file!("arc_ring_quad.wgsl");
pub const SQUIRCLE_QUAD: &str = wgsl_file!("squircle_quad.wgsl");
pub const FLAME: &str = wgsl_file!("flame.wgsl");
pub const STARFIELD: &str = concat!(
    wgsl_file!("rainbow_swirl.wgsl"),
    "\n",
    wgsl_file!("starfield.wgsl"),
);
pub const GOLDEN_DUST: &str = wgsl_file!("golden_dust.wgsl");
pub const MOONLIT_WATER: &str = concat!(
    wgsl_file!("moon_phase.wgsl"),
    "\n",
    wgsl_file!("moonlit_water.wgsl"),
);
pub const SUNLIT_WATER: &str = wgsl_file!("sunlit_water.wgsl");
pub const SHOOTING_STAR_CASCADE: &str = wgsl_file!("shooting_star_cascade.wgsl");
pub const SHOOTING_STAR_CASCADE_COMPOSITE: &str =
    wgsl_file!("shooting_star_cascade_composite.wgsl");
pub const DEPTH_TO_R32: &str = wgsl_file!("depth_to_r32.wgsl");
pub const TILE_GLOW: &str = wgsl_file!("tile_glow.wgsl");
pub const SHADOW: &str = concat!(
    wgsl_file!("hallway_vertex_warp.wgsl"),
    "\n",
    wgsl_file!("shadow.wgsl")
);
pub const ROOM_SHADOW_MASK: &str = concat!(
    wgsl_file!("hallway_vertex_warp.wgsl"),
    "\n",
    wgsl_file!("room_shadow_mask.wgsl")
);
pub const IMAGE_QUAD: &str = wgsl_file!("image_quad.wgsl");
pub const BLOOM_EXTRACT: &str = wgsl_file!("bloom_extract.wgsl");
pub const BLOOM_BLUR: &str = wgsl_file!("bloom_blur.wgsl");
pub const BLOOM_COMPOSITE: &str = wgsl_file!("bloom_composite.wgsl");
pub const TONEMAP_COMPOSITE: &str = wgsl_file!("tonemap_composite.wgsl");

#[cfg(test)]
mod composition_drift {
    //! Guards against the offline bake stamps (`mahjuro_bake_stamp`) drifting out
    //! of sync with the shaders embedded here. If a prepended dependency is added
    //! to a macro below but not to the matching `shader_program` list (or vice
    //! versa), these tests fail — that list is what the room shadow / GI
    //! `.inputs_stamp` hashes, so the mismatch would otherwise let a stale bake
    //! ship after a shader change.
    use std::path::PathBuf;

    /// Concatenate repo-relative WGSL files with the same `"\n"` separator the
    /// `concat!` macros use, reproducing the embedded program byte-for-byte.
    fn compose(files: &[&str]) -> String {
        let repo = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
        files
            .iter()
            .map(|rel| {
                std::fs::read_to_string(repo.join(rel))
                    .unwrap_or_else(|e| panic!("read {rel}: {e}"))
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn shadow_matches_shader_program_list() {
        assert_eq!(
            super::SHADOW,
            compose(mahjuro_bake_stamp::shader_program::SHADOW),
            "embedded_wgsl::SHADOW drifted from shader_program::SHADOW; \
             update both the macro and the bake stamp input list together"
        );
    }

    #[test]
    fn room_shadow_mask_matches_shader_program_list() {
        assert_eq!(
            super::ROOM_SHADOW_MASK,
            compose(mahjuro_bake_stamp::shader_program::ROOM_SHADOW_MASK),
            "embedded_wgsl::ROOM_SHADOW_MASK drifted from \
             shader_program::ROOM_SHADOW_MASK; update both the macro and the bake \
             stamp input list together"
        );
    }

    #[test]
    fn shop_glb_matches_shader_program_list() {
        assert_eq!(
            super::SHOP_GLB,
            compose(
                &mahjuro_bake_stamp::shader_program::scene_pbr_with_hallway_warp(
                    "shaders/room_glb.wgsl"
                )
            ),
            "embedded_wgsl::SHOP_GLB drifted from \
             shader_program::scene_pbr_with_hallway_warp; update both the macro \
             and the bake stamp input list together"
        );
    }
}

// Scene shaders all write linear HDR to `scene_color` now —
// `tonemap_composite.wgsl` owns the single ACES pass, so `scene_hdr_tonemap.wgsl`
// no longer needs to be prepended here.

/// `scene_pbr_core` + `scene_pbr_lights` + `tile_3d`
pub const TILE_3D: &str = scene_pbr_with_hallway_warp!("tile_3d.wgsl");

/// `scene_pbr_core` + `scene_pbr_lights` + `room_glb`
pub const SHOP_GLB: &str = scene_pbr_with_hallway_warp!("room_glb.wgsl");

/// `tile_outline`
pub const TILE_OUTLINE: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../shaders/tile_outline.wgsl"
));

/// `scene_pbr_core` + `scene_pbr_lights` + `lit_mesh`
pub const LIT_MESH: &str = concat!(
    include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../shaders/scene_pbr_core.wgsl"
    )),
    "\n",
    include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../shaders/scene_pbr_lights.wgsl"
    )),
    "\n",
    include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../shaders/lit_mesh.wgsl"
    )),
    "\n",
    include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../shaders/projected_shadow.wgsl"
    )),
);

#[cfg(test)]
mod shader_validation {
    //! Parse + validate every embedded WGSL program with the same `naga` version
    //! `wgpu` uses. This catches shader breakage in CI on any host (no GPU needed)
    //! — including platforms we don't run in CI like the Steam Deck (RADV/Vulkan),
    //! since they share this front-end. Each program here is a complete module
    //! (post-composition) exactly as handed to `wgpu`.

    fn validate(label: &str, src: &str) {
        let module = naga::front::wgsl::parse_str(src)
            .unwrap_or_else(|e| panic!("{label}: WGSL parse error:\n{}", e.emit_to_string(src)));
        let mut validator = naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::all(),
        );
        validator
            .validate(&module)
            .unwrap_or_else(|e| panic!("{label}: WGSL validation error:\n{}", e.emit_to_string(src)));
    }

    macro_rules! validate_program {
        ($name:ident) => {{
            validate(stringify!($name), super::$name);
        }};
    }

    #[test]
    fn flame_validates() {
        validate_program!(FLAME);
    }

    #[test]
    fn all_embedded_programs_validate() {
        validate_program!(QUAD);
        validate_program!(DEPTH_QUAD);
        validate_program!(DEPTH_QUAD_DEBUG);
        validate_program!(TEXT_QUAD);
        validate_program!(GRADIENT_QUAD);
        validate_program!(ARC_RING_QUAD);
        validate_program!(SQUIRCLE_QUAD);
        validate_program!(FLAME);
        validate_program!(STARFIELD);
        validate_program!(GOLDEN_DUST);
        validate_program!(MOONLIT_WATER);
        validate_program!(SUNLIT_WATER);
        validate_program!(SHOOTING_STAR_CASCADE);
        validate_program!(SHOOTING_STAR_CASCADE_COMPOSITE);
        validate_program!(DEPTH_TO_R32);
        validate_program!(TILE_GLOW);
        validate_program!(SHADOW);
        validate_program!(ROOM_SHADOW_MASK);
        validate_program!(IMAGE_QUAD);
        validate_program!(BLOOM_EXTRACT);
        validate_program!(BLOOM_BLUR);
        validate_program!(BLOOM_COMPOSITE);
        validate_program!(TONEMAP_COMPOSITE);
        validate_program!(TILE_3D);
        validate_program!(SHOP_GLB);
        validate_program!(TILE_OUTLINE);
        validate_program!(LIT_MESH);
        #[cfg(feature = "windowed")]
        validate_program!(BOOT_SPLASH);
    }
}
