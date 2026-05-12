//! Sane UI layering: a single ordered command list per frame.
//!
//! `UiFrame` carries one `Vec<DrawCmd>` plus the per-frame data the renderer
//! needs that isn't a draw call (hit-test buttons, point lights, etc).
//! The ordering rule for `cmds` is simple:
//!
//! 1. **Elements pushed earlier render under elements pushed later.**
//! 2. **A widget's children render on top of the widget itself** — which falls
//!    out of rule 1 as long as the widget pushes itself before its children.
//!
//! There are no stages, no z-indexes, no overlay-split indices. Modals,
//! tooltips, and pause menus are just "more cmds pushed at the end."

use crate::core::relic::RelicId;
use crate::core::tile::Tile;
use crate::core::tile_pack::TilePackKind;
use crate::render::lit_mesh::MaterialParams;
use crate::render::wgpu_renderer::{GpuInstance, PointLight, SpotLight, TextLabel};
use crate::scenes::{BackgroundId, ButtonDef};
use glam;
use std::borrow::Cow;
use std::sync::Arc;

/// Per-frame camera override supplied by a scene that wants to draw the 3D
/// world from a perspective other than the renderer's default "person at the
/// table" camera. When `UiFrame.camera_override` is `None`, the renderer
/// builds the gameplay camera from the window size as before.
#[derive(Clone, Copy, Debug)]
pub struct CameraParams {
    /// Camera world-space position.
    pub eye: [f32; 3],
    /// Camera look target in world space.
    pub target: [f32; 3],
    /// Up vector (typically `[0, 0, 1]` for Z-up world space).
    pub up: [f32; 3],
    /// Vertical field of view in degrees.
    pub fovy_deg: f32,
}

/// One punctual entry in [`SceneLighting`]. Smooth lights match legacy gameplay/candle
/// falloff; inverse-square matches embedded `KHR_lights_punctual` from GLB rooms.
#[derive(Clone, Debug)]
pub enum ScenePunctualLight {
    Smooth(PointLight),
    InverseSquare(PointLight),
}

/// Unified per-frame lights for tiles, `lit_mesh`, and GLB room passes.
#[derive(Clone, Debug)]
pub struct SceneLighting {
    pub punctual: Vec<ScenePunctualLight>,
    pub spot_lights: Vec<SpotLight>,
    /// Room environment mesh uses `shop_glb.wgsl` when true, else `tile_3d.wgsl`.
    pub room_shop_glb_brdf: bool,
    /// Embedded `KHR_lights_punctual` active for this room (inverse-square lights + exposure path).
    pub embedded_gltf_punctual: bool,
}

impl Default for SceneLighting {
    fn default() -> Self {
        Self {
            punctual: Vec::new(),
            spot_lights: Vec::new(),
            room_shop_glb_brdf: false,
            embedded_gltf_punctual: false,
        }
    }
}

impl SceneLighting {
    pub fn set_smooth_points(&mut self, v: Vec<PointLight>) {
        self.punctual = v.into_iter().map(ScenePunctualLight::Smooth).collect();
    }

    pub fn push_smooth(&mut self, p: PointLight) {
        self.punctual.push(ScenePunctualLight::Smooth(p));
    }
}

/// GPU / layout hints when the renderer scene key is `showcase` (showcase overlay).
/// The renderer uses these instead of branching on legacy per-flow scene keys.
#[derive(Clone, Copy, Debug, Default)]
pub struct ShowcaseRenderHints {
    /// [`Object3d`] world placement uses [`crate::render::world_space::world_on_camera_ray_plane_z`]
    /// when [`UiFrame::camera_override`] is set (tile-pack pack mesh).
    pub object3d_use_camera_ray_plane_z: bool,
    /// [`DrawCmd::ShowcaseTileBatch`] uses the same ray→plane mapping when `camera_override` is set.
    pub showcase_tiles_use_camera_ray_plane_z: bool,
    /// Suppress gameplay table shadow map (legacy `shop` / `tile_pack_celebration`).
    pub suppress_table_shadows: bool,
    /// [`crate::render::wgpu_renderer::runtime::camera::WgpuRenderer::tile_hdr_tonemap`] pack-celebration path.
    pub tile_pack_celebration_tonemap: bool,
    /// Shop storeroom tonemap + lit_mesh inspect / glTF punctual branches.
    pub shop_tonemap_and_lit_mesh_context: bool,
    /// Collection corridor / table-like HDR (legacy `collection` key).
    pub collection_tonemap_context: bool,
}

impl CameraParams {
    /// Returns the visible world-X range `(min_x, max_x)` at a given world
    /// position `(world_y, world_z)` by unprojecting the left and right NDC
    /// edges through the same view-projection matrix the renderer uses.
    ///
    /// Useful for layout code that needs to keep 3D objects within the camera
    /// frustum.  Pass the shelf's world Y and Z so the frustum width is
    /// evaluated at the correct depth.
    ///
    /// Uses the same near/far planes as the renderer (`near = 1.0`,
    /// `far = h * 12.0`) and the same right-handed convention.
    pub fn frustum_x_range_at(&self, w: f32, h: f32, world_y: f32, world_z: f32) -> (f32, f32) {
        use glam::{Mat4, Vec3, Vec4};

        let aspect = w / h;
        let fov_y = self.fovy_deg.to_radians();
        let eye = Vec3::from_array(self.eye);
        let target = Vec3::from_array(self.target);
        let up = Vec3::from_array(self.up);

        let view = Mat4::look_at_rh(eye, target, up);
        let proj = Mat4::perspective_rh(fov_y, aspect, 1.0, h * 12.0);
        let view_proj = proj * view;
        let inv_vp = view_proj.inverse();

        // Project the shelf centre (0, world_y, world_z) → NDC to obtain the
        // correct clip depth for this horizontal plane.
        let centre_clip = view_proj * Vec4::new(0.0, world_y, world_z, 1.0);
        let ndc_z = centre_clip.z / centre_clip.w.max(1e-6);

        // Unproject the left (ndc_x = -1) and right (ndc_x = +1) edges at
        // that depth.  ndc_y = 0 (vertical midline) is fine — we only need X.
        let unproj = |ndc_x: f32| {
            let h4 = inv_vp * Vec4::new(ndc_x, 0.0, ndc_z, 1.0);
            h4.x / h4.w.max(1e-6)
        };

        (unproj(-1.0), unproj(1.0))
    }

    /// Default "person at the table" camera when [`UiFrame::camera_override`] is
    /// `None` — must match `WgpuRenderer`'s resolve path.
    pub fn default_table_camera(window_h: f32) -> Self {
        let h = window_h.max(1.0);
        // ref_h: 2104
        let cs = h / 2104_f32;
        // Z-up world, standard right-hand conventions: +X right, +Y into table (far side),
        // −Y toward player. Camera sits behind the player at large −Y, elevated in +Z, looking
        // toward +Y. With look_at_rh: forward = +Y, right = forward × up = +Y × +Z = +X ✓.
        // Values derived from the legacy Y-up camera: (x, Y_zup, Z_zup) = (x, −old_z, old_y).
        Self {
            eye: [0.0 * cs, -2104.0 * cs, 1157.2 * cs],
            target: [0.0 * cs, -39.6 * cs, 105.2 * cs],
            up: [0.0, 0.0, 1.0],
            fovy_deg: 55.0,
        }
    }

    /// Project a world point to layout pixels (same contract as the internal render camera).
    pub fn project_world_to_screen(
        &self,
        window_w: f32,
        window_h: f32,
        world: glam::Vec3,
    ) -> (f32, f32) {
        let aspect = window_w / window_h.max(1e-6);
        let eye = glam::Vec3::from_array(self.eye);
        let target = glam::Vec3::from_array(self.target);
        let up = glam::Vec3::from_array(self.up);
        let view = glam::Mat4::look_at_rh(eye, target, up);
        let proj =
            glam::Mat4::perspective_rh(self.fovy_deg.to_radians(), aspect, 1.0, window_h * 12.0);
        let view_proj = proj * view;
        let clip = view_proj * glam::Vec4::new(world.x, world.y, world.z, 1.0);
        let inv_w = 1.0 / clip.w.max(1e-6);
        let nx = clip.x * inv_w;
        let ny = clip.y * inv_w;
        let sx = (nx * 0.5 + 0.5) * window_w;
        let sy = (1.0 - (ny * 0.5 + 0.5)) * window_h;
        (sx, sy)
    }
}

/// Packs `(pixel_x, pixel_y, lift)` for a point in **world space** (`lift` is **+Z** above the felt).
/// Consumed by [`crate::render::world_space::pixel_to_world`].
pub type WorldSurfaceAnchor = [f32; 3];

/// One soft wind impulse sampled by the procedural candle-flame particle system.
///
/// Coordinates use the same `(pixel_x, pixel_y)` convention as the rest of the
/// scene draw output: [`crate::render::world_space::pixel_to_world`] maps the
/// center to world space. Velocity is in **world** units (**+Z** up from the felt).
#[derive(Clone, Copy, Debug)]
pub struct WindGust {
    /// `(pixel_x, pixel_y)` center of the gust in layout-pixel space.
    pub center_px: (f32, f32),
    /// Height in world **+Z** above the felt.
    pub lift: f32,
    /// Velocity in world units per second (x, y, z).
    pub velocity: [f32; 3],
    /// Impulse radius in world units.
    pub radius: f32,
    /// Gameplay still supplies this for scripted gusts; flame bending uses `velocity` only.
    #[allow(dead_code)]
    pub density: f32,
}

// ── Skeuomorphic gameplay HUD placements ──────────────────────────────────
//
// Phase 1 of the in-game UI redesign: physical objects rendered through the
// `lit_mesh` pipeline replace the flat slate-blue HUD rects. Each variant has
// a sibling DrawCmd below.
//
// Phase 1 wires up the mesh + draw cmd infrastructure but no scene actually
// pushes these yet — phases 2-7 introduce the corresponding `gameplay.rs`
// pushes one region at a time. The unused-field/variant warnings stay
// suppressed via the `allow(dead_code)` until then.

/// Hanging blind/score plaque suspended above the gameplay table.
///
/// One yaku selector tablet (carved bone, sitting in a row below the hand).

#[derive(Clone, Debug)]
pub struct YakuTabletPlacement {
    /// `(pixel_x, pixel_y, lift)` for the tablet's *base center*.
    pub world_pos: WorldSurfaceAnchor,
    /// Width × height × depth in world units.
    pub extents: [f32; 3],
    /// Roll about table/world Z in degrees.
    pub rotation_z_deg: f32,
    /// Yaku display name (engraved on the face via decal).
    pub name: String,
    /// True when this yaku is the player's currently selected target.
    pub active: bool,
    /// Hover lift envelope in [0, 1] driven by the scene each frame.
    pub hover: f32,
}

/// Which counter fan an `Object3dKind::TallyFan` represents. Drives per-fan
/// focus rect slot and tooltip wiring on the gameplay scene side.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TallyFanKind {
    /// Plays-remaining fan, anchored in front of the mirror.
    Draws,
    /// Discards-remaining fan, anchored in front of the river.
    Discards,
}

/// Stack of facedown wall tiles at the back of the table.
#[derive(Clone, Copy, Debug)]
pub struct WallStackPlacement {
    /// `(pixel_x, pixel_y, lift)` for the bottom-back-left of the stack.
    pub world_pos: WorldSurfaceAnchor,
    /// Tile slot dimensions in world units (per-tile width/height/depth).
    pub tile_extents: [f32; 3],
    /// Number of facedown tiles still in the wall.
    pub remaining: u32,
    /// Number of tiles per row in the stack (the pile fans wide).
    pub row_len: u32,
}

/// Which cascade-readout axis a token represents — drives its tint and
/// the gameplay scene's pulse animation routing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CascadeTokenKind {
    Chips,
    Mult,
}

/// Material selector for the extruded-glyph score popup. Maps to the lit-mesh
/// shader's `MaterialKind` branch at render time.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GlyphMaterial {
    Plain,
    Metal,
    Polychrome,
}

/// One tile in a showcase display (pack-opening celebration, hand strip, etc.).
/// The scene provides full per-tile 3D transforms each frame — the renderer
/// just draws what it's told, with no animation state of its own.
#[derive(Clone, Copy, Debug)]
pub struct ShowcaseTilePlacement {
    /// The tile to display (identity determines the rasterized decal).
    pub tile: Tile,
    /// `(pixel_x, pixel_y, lift)` — same coordinate space as every
    /// other 3D placement. [`crate::render::world_space::pixel_to_world`] maps px/py to world XY;
    /// `lift` is height above the table plane (**+Z**).
    pub center_pos: [f32; 3],
    /// Euler rotation `(rx, ry, rz)` in radians — same composition as
    /// [`crate::render::table_transform::rot_euler_xyz_rad`], after the
    /// standard tile basis. `[0, 0, 0]` = default tilted-toward-camera.
    pub rotation: [f32; 3],
    /// Uniform scale factor (`1.0` = standard hand-tile size at the given
    /// pixel footprint). Used for grow-in / shrink animations.
    pub scale: f32,
    /// Pixel-space footprint width — controls the physical world size of the
    /// tile via the active tile-preset ratios (face_long, thickness).
    pub size_px: f32,
    /// Brightness multiplier: `1.0` = normal, `< 1.0` = dimmed (e.g. blocked
    /// tiles in solitaire). Passed to the tile shader via `base_color_factor.x`.
    pub brightness: f32,
    /// Whether this tile is currently selected (e.g. first pick in solitaire).
    /// Drives a warm gold fresnel rim via `base_color_factor.y = 1.0`.
    pub selected: bool,
    /// Whether this tile is hovered (focused). Drives a cool fresnel rim via
    /// `base_color_factor.y = 0.5`. Takes lower priority than `selected`.
    pub hovered: bool,
    /// Draw a gold outline shell around this tile (hand-strip selection indicator).
    /// The renderer writes an inflated model matrix into the outline uniform buffer.
    pub outline: bool,
    /// Emit an additive radial glow halo behind this tile (warm champagne gold).
    pub glow: bool,
    /// Override color for the glow halo when `glow` is true. `None` falls
    /// back to the default warm champagne gold. Used to mark dora tiles
    /// with a red glow.
    pub glow_color: Option<[f32; 4]>,
    /// Logical slot index for ray-cast tile picking and `proj.hand_rects` tracking.
    /// `None` = not pickable (pack-open showcase tiles, etc.).
    pub pick_id: Option<usize>,
}

/// One flat tile-face decal drawn as a screen-space image quad.
///
/// Uses the same per-tile face rasterization as the 3D tile renderer but skips
/// lighting, shadows, and mesh submission entirely.
#[derive(Clone, Copy, Debug)]
pub struct TileFaceQuad {
    /// Tile identity selects the cached face decal texture.
    pub tile: Tile,
    /// Screen-space placement and tint. `color.a` controls overall opacity.
    pub inst: GpuInstance,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum PromptIconSource {
    /// Sub-rectangle of a baked Kenney Input Prompts sheet PNG. `sheet` is the
    /// asset-relative path (under `assets/`) to the `_sheet_double.png`; `name`
    /// is the matching `SubTexture name="…"` from the sibling XML index. The
    /// renderer decodes each sheet once on first use, then crops the named
    /// sub-rect into a per-glyph GPU texture (cached by [`Self::cache_key`]).
    AtlasSprite {
        sheet: &'static str,
        name: &'static str,
    },
    /// Absolute filesystem path to an SVG or PNG (rasterized at draw time).
    #[allow(dead_code)] // No current producer; renderer path kept for tooling / experiments.
    Filesystem(std::path::PathBuf),
}

impl PromptIconSource {
    pub fn cache_key(&self) -> String {
        match self {
            Self::AtlasSprite { sheet, name } => format!("atlas:{sheet}:{name}"),
            Self::Filesystem(path) => format!("file:{}", path.display()),
        }
    }
}

/// Input prompt drawn as a tinted image quad (`shaders/image_quad.wgsl`).
#[derive(Clone, Debug)]
pub struct PromptIconQuad {
    pub inst: GpuInstance,
    pub source: PromptIconSource,
}

/// One shrine placement (used by the pick-blind scene to draw the three
/// blind shrines side by side, each scaled by `extents`). Geometry is the
/// procedural shrine mesh in normalized -0.5..+0.5 local space, scaled by
/// `extents` and translated to `world_pos`.
#[derive(Clone, Copy, Debug)]
pub struct ShrinePlacement {
    /// `(pixel_x, pixel_y, lift)` for the shrine's *base center*.
    pub world_pos: WorldSurfaceAnchor,
    /// Full extents in world units (width × height × depth). Per-instance
    /// scaling is how the Small / Big / Boss shrines get visibly distinct
    /// sizes.
    pub extents: [f32; 3],
    /// Linear-space RGBA tint applied to every face of the mesh.
    pub color: [f32; 4],
    /// Brighten multiplier in [0, 1]. The upcoming shrine pushes this above
    /// 0 so it reads as the active choice even before the spotlight
    /// `PointLight` adds its bloom on top.
    pub glow: f32,
}

// ── General-purpose 3D placement ─────────────────────────────────────────

/// Euler **XYZ** radians for [`Object3d::rotation`] — same convention as
/// [`ShowcaseTilePlacement::rotation`] and [`crate::render::table_transform::rot_euler_xyz_rad`].
pub type Object3dEuler = [f32; 3];

/// Pitch-only euler so a **+Z-normal** panel faces the camera (wood plaques, shop slabs, …).
#[inline]
pub fn camera_facing_euler_xyz_rad(cam_eye: [f32; 3], look_target: [f32; 3]) -> Object3dEuler {
    let look = glam::Vec3::from(look_target) - glam::Vec3::from(cam_eye);
    let pitch = look.z.atan2(look.y.abs()) + std::f32::consts::PI;
    [pitch, 0.0, 0.0]
}

/// [`Mat4`] view of [`camera_facing_euler_xyz_rad`] — for legacy compose-on-left math.
///
/// Prefer storing [`Object3dEuler`] via [`camera_facing_euler_xyz_rad`] on new code paths.
#[inline]
pub fn camera_facing_rotation(cam_eye: [f32; 3], look_target: [f32; 3]) -> glam::Mat4 {
    let e = camera_facing_euler_xyz_rad(cam_eye, look_target);
    glam::Mat4::from_rotation_x(e[0])
}

/// Mesh-specific data carried alongside the common [`Object3d`] fields.
/// Each variant names only the fields that differ from the common set.
#[derive(Clone, Debug)]

pub enum Object3dKind {
    // ── Upright panels ──────────────────────────────────────────────
    // (Plaque is now modeled as `Primitive { shape: BeveledSlab, … }`.)
    // (Ofuda is now modeled as `Primitive { shape: Ofuda, material:
    // Plain + ParchmentInk TitleRule decal }`. Callers format the
    // decal text as `"{title}\n{rule}"`.)
    /// Carved bone tablet with a single engraved name and progress glow.
    YakuTablet {
        label: Cow<'static, str>,
        active: bool,
        hover: f32,
    },
    /// Lacquered wood action tablet with press/hover animation envelopes.
    WoodTablet {
        label: Cow<'static, str>,
        /// When `Some`, the tablet's screen-space rect is published to
        /// `aux_dish_rects` and its model matrix to
        /// `last_primitive_pick_models` keyed by this id. Lets scenes
        /// that route clicks via `ShopHit::Dish(pid)` (e.g. shop's
        /// journal button) reach a wood tablet without introducing a
        /// separate pick channel.
        pick_id: Option<u32>,
    },
    /// Closed leather-bound book with a calligraphy spine label.
    /// Used by the shop scene's journal prop. Same pick-routing
    /// pattern as `WoodTablet` (via `aux_dish_rects` +
    /// `last_primitive_pick_models`) so existing `ShopHit::Dish(pid)`
    /// click handlers reach it unchanged.
    Book {
        spine_label: Cow<'static, str>,
        pick_id: Option<u32>,
        /// Cover-open animation amount, 0.0 (closed, cover flush over
        /// the page surface) to 1.0 (fully open, cover swung ~170°
        /// around the spine axis to lay flat on the camera-right,
        /// exposing the page-content surface). The renderer applies
        /// `Rz(+open_amount * 170°)` around the spine hinge axis when
        /// uploading the cover sub-instance.
        open_amount: f32,
    },

    // ── Props ────────────────────────────────────────────────────────
    /// Procedural shrine altar (Small / Big / Boss scale via `extents`).
    Shrine { glow: f32 },
    /// Procedural ornate brass plinth used by the gameplay scene to display
    /// the dora indicator tile(s). The mesh has no roof; the indicator
    /// tile face(s) are pushed separately as `ShowcaseTilePlacement`s
    /// resting on the platform on top.
    DoraPlinth { glow: f32 },
    /// Relic medallion — shop for-sale, owned dish row, gameplay HUD
    /// tray, collection cards, tutorial panels, and unlock modals all use
    /// this single kind; callers pick the rotation so the face points at
    /// the scene's camera.
    Relic {
        relic_id: RelicId,
        glow: f32,
        /// Render as a near-black matte silhouette (no texture, no glow)
        /// for locked Collection entries.
        silhouette: bool,
        /// Boss Hex (and similar): draw the same debuff X overlay as debuffed tiles.
        debuffed: bool,
        /// Optional pick id. When `Some`, the renderer snapshots this
        /// relic's model matrix into `last_collection_relic_pickables` so
        /// `pick_collection_object` can return the pick id for clicks that
        /// land inside the relic's real silhouette. Leave `None` for
        /// relics that shouldn't be individually clickable (e.g., the
        /// featured pedestal showpiece that's already selected).
        pick_id: Option<u32>,
    },
    /// Tile-pack box on the shop shelf.
    Pack {
        kind: TilePackKind,
        pick_id: Option<u32>,
    },
    /// Silken zodiac ribbon hanging from an anchor.
    ZodiacRibbon {
        kind: Option<crate::core::zodiac::ZodiacKind>,
    },
    /// Talisman tablet (shop curio).
    Talisman {
        kind: crate::core::talisman::TalismanKind,
    },
    // (Coin is now modeled as `Primitive { shape: Cylinder,
    // material: MaterialSpec::metal(), shadow_caster: true }`. The
    // renderer registers the engraved-coin heightmap as a per-shape
    // texture override for MeshId::Cylinder.)
    // (GoldBar is now modeled as `Primitive { shape: Cube,
    // material: MaterialSpec::metal(), shadow_caster: true }`.)
    // (BrassRail is now modeled as `Primitive { shape: Cube,
    // material: MaterialSpec::brass() }`.)
    // (Standing book was removed; the shop now uses an
    // `Object3dKind::WoodTablet { label: "Journal", pick_id: Some(…)
    // }` to match gameplay's journal affordance.)
    /// Discard bowl. Hover animation is driven by [`Object3d::hover_target`].
    Bowl,
    /// Bronze "play hand" mirror. Hover animation is driven by [`Object3d::hover_target`].
    ///
    /// `rotation_x_deg` is the base pitch before hover tilt is added.
    /// `rotation_z_deg` is the Z-roll (idle wobble) applied simultaneously.
    /// The `Object3d::rotation` field is ignored for Mirror; use these two
    /// instead so hover can correctly modify only the X component.
    Mirror {
        rotation_x_deg: f32,
        rotation_z_deg: f32,
    },
    /// Engraved bone cascade token with a pulse-pop envelope.
    CascadeToken { kind: CascadeTokenKind, pulse: f32 },
    // (Dish is now modeled as `Primitive { shape: DiscSquare or
    // DiscRound, material: MaterialSpec::plain(), shadow_caster: true
    // }`. Callers set `pos[2]` to the dish center (base + extents[1] *
    // 0.5) rather than the base, since the generic dispatch no longer
    // auto-lifts.)
    // (ShopActionProp is now modeled as `Primitive { shape:
    // ShopActionProp, material: Plain + GoldGilded Fixed decal }`.
    // Disabled callers pre-apply an alpha of 0.45 to `obj.color[3]`.)
    /// Floating 3D extruded-glyph score popup ("+50", "×3", "=12500"). The
    /// renderer lazily builds a per-string mesh on first use and reuses it
    /// on subsequent frames. `Object3d::pos` sets the popup center;
    /// `Object3d::extents`/`rotation` are ignored — the fields below supply
    /// the full pose/material.
    ExtrudedGlyph {
        scale: f32,
        rotation_x: f32,
        rotation_y: f32,
        /// `Arc<str>` so per-frame clones (every popup, every reel digit, every
        /// cascade-HUD glyph) are a refcount bump, not a fresh allocation. The
        /// downstream consumer in `object3d_placement.rs` calls `as_ref()` /
        /// `&label` to get `&str` for the per-label mesh cache.
        label: Arc<str>,
        emissive: f32,
        material: GlyphMaterial,
    },
    /// Procedural candle mesh (wax body + wick). `Object3d::pos` sets the
    /// base; `extents`/`rotation` are ignored — the renderer uses the
    /// `scale` and `height_scale` fields to size the shared unit mesh.
    Candle {
        /// Uniform scale applied to the local-unit mesh.
        scale: f32,
        /// Extra Y-axis scale (height multiplier) on top of `scale`.
        height_scale: f32,
    },
    /// Upright fan of bone tally sticks. `Object3d::pos` is the fan pivot;
    /// `Object3d::extents` is unused (stick dimensions come from the fields
    /// below); `Object3d::rotation` is also unused — the fan is yawed via
    /// `rotation_y_deg` in the kind payload so the renderer can cleanly
    /// compose the per-stick angular layout.
    TallyFan {
        /// Stick length (world units, narrow base to wide tip).
        stick_len: f32,
        /// Wide-end width.
        stick_wide: f32,
        /// Fan-forward thickness.
        stick_thickness: f32,
        /// Sticks currently visible.
        count: u32,
        /// Total slot count the fan is sized for.
        max_count: u32,
        /// Total arc the fan spreads across (degrees, symmetric about vertical).
        spread_deg: f32,
        /// RGBA tint for the tip cap.
        tip_color: [f32; 4],
        /// Yaw of the fan plane about world up (degrees).
        rotation_y_deg: f32,
        /// Which counter this fan represents (drives arrange-name + peg_rects slot).
        kind: TallyFanKind,
    },
    /// One hovering insect near a light source (e.g. shop lamp). The scene emits
    /// one `Bug` per insect per frame, with `slot` ∈ `0..MAX_BUG_SLOTS`.
    ///
    /// `flap_rad` is the wing flap angle in radians about the body's local +X.
    /// `live_wing_alpha` / `blur_alpha` cross-fade crisp wings vs motion-blur fans.
    Bug {
        slot: usize,
        flap_rad: f32,
        live_wing_alpha: f32,
        blur_alpha: f32,
    },
    /// Material preview orb — a shared unit sphere drawn with a caller-supplied
    /// `MaterialParams`. Used only by the material viewer debug scene. The
    /// instance pool binds the 1×1 default albedo and relief textures so
    /// materials that sample heightmaps render as their base material with no
    /// displacement (i.e. every orb previews the shading model itself, not a
    /// per-asset heightmap).
    MaterialOrb { material: MaterialParams },
    /// Generic shape + material. Replaces the bespoke-per-shape pattern
    /// for simple slabs/cubes/cylinders — `obj.color` is the base tint
    /// (honored consistently across every shape) and the optional decal
    /// goes through a single unified rasterizer. New shapes/materials
    /// are additive: add a [`crate::render::primitive::MeshId`] variant
    /// and register the mesh in `WgpuRenderer::new`.
    Primitive {
        shape: crate::render::primitive::MeshId,
        material: crate::render::primitive::MaterialSpec,
        pick_id: Option<u32>,
        /// When true, this primitive is walked by the shadow pre-pass.
        /// Default off — thin slabs look better without self-shadow.
        shadow_caster: bool,
        /// When true, render as a near-black matte silhouette
        /// (locked-collection lock state). Decal and material kind
        /// are suppressed; `obj.color` alpha is preserved.
        silhouette: bool,
    },
}

/// A single lit mesh placed in the world.
///
/// Replaces all individual `XxxPlacement` structs for objects rendered through
/// the `lit_mesh_pipeline`.  Scenes set `pos`, `extents`, and `rotation`
/// as Euler **XYZ** radians — same as [`ShowcaseTilePlacement::rotation`].
/// Use [`camera_facing_euler_xyz_rad`] when the face should track the camera.
#[derive(Clone, Debug)]

pub struct Object3d {
    /// Center position as `(pixel_x, pixel_y, lift)`.
    /// Mapped to world space by [`crate::render::world_space::pixel_to_world`].
    pub pos: [f32; 3],
    /// Full extents `(width, height, depth)` in world units.
    pub extents: [f32; 3],
    /// Euler **XYZ** radians — [`crate::render::table_transform::rot_euler_xyz_rad`].
    /// `[0.0, 0.0, 0.0]` = no rotation.
    pub rotation: Object3dEuler,
    /// Base tint (linear RGBA). `[1.0, 1.0, 1.0, 1.0]` = mesh default color.
    pub color: [f32; 4],
    /// Which mesh + material to render, plus mesh-specific payload.
    pub kind: Object3dKind,
    /// Target hover intensity in \[0, 1\] for this frame.
    ///
    /// When non-zero or when `anim_id` is set, the renderer maintains a
    /// per-object smoothed envelope (exponential ease, rate ≈ 14) and uses the
    /// eased value to animate lift, tilt, and scale. The exact effect is
    /// kind-specific (Bowl tilts and lifts; Mirror tilts further; tablets lift
    /// and scale-up; etc.). `0.0` = not hovered; `1.0` = fully hovered.
    pub hover_target: f32,
    /// Stable logical ID for objects that need persistent animation state
    /// across frames (smoothed hover envelopes, etc.).
    ///
    /// The renderer stores per-ID smoothed state in a table keyed on this
    /// value.  Scenes should assign a small non-zero constant per logical
    /// object (e.g. `1` = discard bowl, `2` = bronze mirror, `3..N` = yaku
    /// tablets).  `0` means "no persistent state" — `hover_target` is used
    /// directly without easing.
    pub anim_id: u64,
    /// Canonical arrange-mode path (e.g. `"shop.counter"`, `"gameplay.hand.strip"`).
    /// When set, the renderer uses this for `apply_arrange_override` and
    /// `last_debug_pickables`. `None` = not arrangeable via the debug picker.
    pub arrange_name: Option<&'static str>,
}

impl Object3d {
    /// [`Mat4`] for this instance's euler triple (what the renderer multiplies into `translate_rot_scale`).
    #[inline]
    pub fn rotation_matrix(&self) -> glam::Mat4 {
        crate::render::table_transform::rot_euler_xyz_rad(
            self.rotation[0],
            self.rotation[1],
            self.rotation[2],
        )
    }
}

/// One drawable element in a `UiFrame`.
///
/// The renderer walks `UiFrame.cmds` in order and dispatches each variant to
/// the appropriate pipeline. Contiguous runs of the same variant (e.g.
/// several `Quad`s in a row) are batched into a single instanced draw, which
/// is invisible to scenes and preserves ordering exactly.
pub enum DrawCmd {
    /// Full-screen background image.
    Background(BackgroundId),
    /// Procedural constellation starfield (fullscreen triangle, no data).
    Starfield,
    /// Procedural rising-ember vignette (fullscreen triangle, no data).
    EmberDrift,
    /// Procedural golden-dust with god-rays vignette (fullscreen triangle, no data).
    GoldenDust,
    /// Procedural moon hovering above rippling water (fullscreen triangle, no data).
    MoonlitWater,
    /// Procedural sun hovering above rippling water (fullscreen triangle, no data).
    SunlitWater,
    /// Procedural mountain-haze atmosphere (fullscreen triangle, no data).
    /// FBM scrolling noise + vertical gradient, additively blended — reads as
    /// slow drifting mountain fog without the cost of a volumetric sim.
    MountainHaze,
    /// Procedural shooting-star cascade transition (fullscreen triangle, no data).
    /// Brightness driven by `UiFrame::transition_progress`.
    ShootingStarCascade,
    /// Procedural lacquered-wood table mesh (one per scene, drawn via
    /// `lit_mesh_pipeline`). Sized by the renderer from the current window.
    Table,
    /// 3D candle meshes for the gameplay scene. Each placement becomes one
    /// wax-body draw + one wick draw via the `lit_mesh_pipeline`. Limited to
    /// the renderer's pre-allocated candle slot pool (currently 7).
    /// Tile-pack boxes rendered on the shop shelf. Uses the same unit-box
    /// mesh and lit-mesh pipeline as relics, with pack art textures.
    /// Batch of zodiac/talisman ribbons drawn via the lit-mesh pipeline,
    /// instanced from the renderer's pre-allocated ribbon slot pool. Used by
    /// the shop scene for both the wall-pinned for-sale ribbons and the
    /// owned-consumable inventory fan.
    /// Batch of talisman tablets drawn via the lit-mesh pipeline,
    /// instanced from the renderer's pre-allocated talisman slot pool. Used
    /// by the shop scene for the for-sale talismans pinned in the curio
    /// cabinet next to the zodiac ribbons.
    /// Batch of physical gold coins drawn via the lit-mesh pipeline,
    /// instanced from the renderer's pre-allocated coin slot pool. Used by
    /// the shop scene to display the player's gold as a pile of coins in a
    /// dish.
    /// Imported `Shop.glb` room mesh (tile-textured pipeline, world-space vertices).
    ShopEnvironment,
    /// Imported `hallway.glb` pick-blind room (same GPU path as [`DrawCmd::ShopEnvironment`]).
    HallwayEnvironment,
    /// Reset the main scene depth target while keeping the HDR color buffer. Later 3D
    /// draws (same camera) composite by depth among themselves but no longer test
    /// against geometry drawn before this marker — e.g. pack celebration meshes over
    /// the shop room without hiding the room in color.
    ClearSceneDepth,
    /// After this marker, following `lit_mesh` draws use full shop inspect HDR on
    /// [`SsrGlobals.felt`]; earlier draws use a dimmed row. Splits Pass A so the
    /// GPU buffer can be updated between subpasses. See [`UiFrame::shop_inspect_lit_mesh_subject_hdr`].
    ShopInspectLitMeshSubjectHdr,
    /// Batch of showcase tiles with explicit 3D transforms — used for hand
    /// tiles, pack-opening celebrations, and any other 3D tile placement.
    ShowcaseTileBatch(Vec<ShowcaseTilePlacement>),
    /// Flat screen-space tile face using the real per-tile decal art.
    TileFaceQuad(TileFaceQuad),
    /// Generic 2D quad (panels, dimmers, borders, tooltip backgrounds…).
    Quad(GpuInstance),
    /// 2D quad with a superellipse (squircle) silhouette — same `GpuInstance`
    /// payload as [`DrawCmd::Quad`]. See `shaders/squircle_quad.wgsl`.
    SquircleQuad(GpuInstance),
    /// Alpha-feathered 2D quad — solid `color` in the centre, falling off
    /// to full transparency toward the edges. Used as a soft dark backer
    /// behind HUD content so panels read against busy backgrounds without
    /// a hard-edged letterbox. See `shaders/gradient_quad.wgsl`.
    GradientQuad(crate::render::wgpu_renderer::GradientQuadInstance),
    /// Procedural candle flame (additive blend, animated by globals.time).
    /// Instance `color.a` carries a per-flame phase offset in [0,1].
    Flame(GpuInstance),
    /// Rasterized text label.
    Text(TextLabel),
    /// Embedded SVG prompt icon (Kenney); rasterized once per `asset_rel_path`.
    PromptIconQuad(PromptIconQuad),
    // ── Skeuomorphic gameplay HUD ──
    /// Batch of wood action tablets (sort suit / sort rank / play).
    /// Floating 3D extruded-glyph score popups. Each placement carries its
    /// own label string; the renderer lazily builds a per-string mesh on
    /// first use and reuses it on subsequent frames.
    // ── General-purpose 3D objects ──────────────────────────────────
    /// Single general-purpose lit-mesh object. See [`Object3d`].
    Object3d(Object3d),
    /// Batch of general-purpose lit-mesh objects. See [`Object3d`].
    Object3dBatch(Vec<Object3d>),
}

/// Axis along which an [`ArrangeClamp`] constrains a placement.
#[derive(Copy, Clone, Debug)]

pub enum ClampAxis {
    Horizontal,
}

/// Scene-provided hint describing the clamp band that constrains a pickable's
/// effective position. When the named pickable is the current arrange-mode
/// selection, the renderer draws the band so the user can see why nudges may
/// stop having an effect. `center_frac` is the *unclamped* placement value; if
/// it's outside `[lo_frac, hi_frac]` the renderer flags the pinning wall.
#[derive(Clone, Debug)]
pub struct ArrangeClamp {
    pub name: String,
    pub axis: ClampAxis,
    pub lo_frac: f32,
    pub hi_frac: f32,
    pub center_frac: f32,
}

/// Everything a frame's draw needs: an ordered command list plus per-frame
/// state used by hand-tile markers, hit testing, and the main loop.
pub struct UiFrame {
    /// Drawn back-to-front in order. Push earlier = renders under.
    pub cmds: Vec<DrawCmd>,

    /// Scene lights: punctual (smooth + optional inverse-square) and spots — one GPU upload.
    pub scene_lighting: SceneLighting,
    /// How many of the leading entries in `scene_lighting.punctual` are candle lights
    /// (as opposed to hint lights, spot lights, etc.).
    pub candle_light_count: u32,
    /// Candle flame height in world units (derived from mm via `Layout::mm`).
    pub flame_height_world: f32,
    /// Mouse cursor position in pixel coordinates, if the scene tracks one.
    pub cursor_pos: Option<(f32, f32)>,
    /// Discrete wind impulses for candle flame particles (see [`WindGust`]).
    /// Gameplay uses these for scripted "breath" across the table after dealing.
    pub wind_gusts: Vec<WindGust>,
    /// Optional 3D camera override. When `Some`, the renderer uses this
    /// camera (eye/target/up/fovy) for all 3D draws this frame instead of
    /// the default "person at the table" gameplay camera. The shop scene
    /// uses this to frame the curio cabinet + foreground dishes.
    pub camera_override: Option<CameraParams>,
    /// Debug overlay: when true, the renderer draws three colored axis bars
    /// (red = +X, green = +Y, blue = +Z) anchored at the camera's look
    /// target. Toggled from the native Debug menu in the gameplay scene to
    /// help disambiguate world-space directions when iterating on
    /// placements.
    pub debug_axes: bool,
    /// When `Some`, overrides the tile material for this frame. Used by
    /// the tile-select scene to preview materials before a run starts.
    pub tile_material_override: Option<crate::persistence::TileMaterial>,

    // ── Non-draw scene metadata ─────────────────────────────────────────
    /// Hit-test rects for clickable buttons (not drawn).
    pub buttons: Vec<ButtonDef>,
    /// Title shown in the OS window chrome.
    pub window_title: String,
    /// Scene-transition progress (0.0 = inactive, >0.0 = animating).
    /// Uploaded to the GPU `Globals` uniform for the cascade shader.
    pub transition_progress: f32,
    /// Arrange-mode clamp hints. Drawn as a faint band when the named
    /// pickable is the current selection — see [`ArrangeClamp`].
    pub arrange_clamps: Vec<ArrangeClamp>,
    /// Gameplay only: [`MountainHaze`] horizon height in normalized screen Y
    /// (0 = top, 1 = bottom). Overrides volumetric `haze_horizon_y` for this
    /// frame; sourced from gameplay scene layout `fog_wall.ny`.
    pub gameplay_fog_wall_horizon_y: Option<f32>,
    /// Gameplay only: horizontal center of the fog slab in normalized screen X
    /// (0 = left, 1 = right). From `fog_wall.nx`; drives `set_haze_tuning` with
    /// [`GAMEPLAY_FOG_WALL_HALF_WIDTH_UV`](crate::ui::scene_layout::GAMEPLAY_FOG_WALL_HALF_WIDTH_UV).
    pub gameplay_fog_wall_center_x: Option<f32>,
    /// Barrel / fisheye lens distortion applied in the final composite.
    /// 0.0 = off (no distortion). Positive = outward barrel (center
    /// magnified, edges compressed). Typical range 0.0..=0.6. Scenes
    /// that want the "looking into infinity" effect (e.g. the collection
    /// corridor) set this to pull the viewport toward a fisheye lens.
    pub fisheye_strength: f32,
    /// Optional pre-pass `UiFrame` rendered into `journal_scene_texture`
    /// before the main frame. Set by the shop while the journal book is
    /// open: the embedded `YakuJournalScene` builds its own UiFrame and
    /// the application loop runs it through `render_to(.., Some(view))`
    /// before rendering the shop. The shop's open-book mesh samples the
    /// resulting texture in screen space, so the page region reads as a
    /// window cut through the page mesh into a live render of the
    /// post-transition scene.
    pub journal_prepass_frame: Option<Box<UiFrame>>,
    /// Shop [`ItemInspectScene`] uses synthetic point lights only (GLB punctual off). Those
    /// lights are tuned for table-scale HDR — not the `/512` crush used for bright `Shop.glb`.
    /// When set, [`crate::render::wgpu_renderer::runtime::camera::WgpuRenderer::tile_hdr_tonemap`]
    /// applies gameplay-style linear exposure for `lit_mesh` so shelf props stay visible.
    pub shop_inspect_lit_mesh_hdr: bool,
    /// Set by showcase overlay presenters; read by shadow / placement / tonemap paths.
    pub showcase_render_hints: ShowcaseRenderHints,
}

impl UiFrame {
    pub fn new() -> Self {
        Self {
            cmds: Vec::new(),
            scene_lighting: SceneLighting::default(),
            candle_light_count: 0,
            flame_height_world: 0.0,
            cursor_pos: None,
            wind_gusts: Vec::new(),
            camera_override: None,
            debug_axes: false,
            tile_material_override: None,
            buttons: Vec::new(),
            window_title: String::new(),
            transition_progress: 0.0,
            arrange_clamps: Vec::new(),
            fisheye_strength: 0.0,
            journal_prepass_frame: None,
            gameplay_fog_wall_horizon_y: None,
            gameplay_fog_wall_center_x: None,
            shop_inspect_lit_mesh_hdr: false,
            showcase_render_hints: ShowcaseRenderHints::default(),
        }
    }

    /// `shop_glb.wgsl` for room meshes vs `tile_3d` (includes shop inspect storeroom path).
    #[inline]
    pub fn room_uses_shop_glb_shader(&self) -> bool {
        self.scene_lighting.room_shop_glb_brdf || self.shop_inspect_lit_mesh_hdr
    }

    // ── Push helpers ────────────────────────────────────────────────────
    pub fn background(&mut self, bg: BackgroundId) {
        self.cmds.push(DrawCmd::Background(bg));
    }
    /// Draw the 3D shop from embedded [`Shop.glb`](../../assets/Shop.glb). No-op if the asset failed to load.
    pub fn shop_environment(&mut self) {
        self.cmds.push(DrawCmd::ShopEnvironment);
    }
    /// Draw the pick-blind hallway from embedded [`hallway.glb`](../../assets/hallway.glb).
    pub fn hallway_environment(&mut self) {
        self.cmds.push(DrawCmd::HallwayEnvironment);
    }
    /// See [`DrawCmd::ClearSceneDepth`].
    pub fn clear_scene_depth(&mut self) {
        self.cmds.push(DrawCmd::ClearSceneDepth);
    }
    /// Dimmed vs full inspect HDR boundary for shop [`ItemInspectScene`] (see [`DrawCmd::ShopInspectLitMeshSubjectHdr`]).
    pub fn shop_inspect_lit_mesh_subject_hdr(&mut self) {
        self.cmds.push(DrawCmd::ShopInspectLitMeshSubjectHdr);
    }
    pub fn starfield(&mut self) {
        self.cmds.push(DrawCmd::Starfield);
    }
    pub fn ember_drift(&mut self) {
        self.cmds.push(DrawCmd::EmberDrift);
    }
    pub fn golden_dust(&mut self) {
        self.cmds.push(DrawCmd::GoldenDust);
    }
    pub fn moonlit_water(&mut self) {
        self.cmds.push(DrawCmd::MoonlitWater);
    }
    pub fn sunlit_water(&mut self) {
        self.cmds.push(DrawCmd::SunlitWater);
    }
    pub fn mountain_haze(&mut self) {
        self.cmds.push(DrawCmd::MountainHaze);
    }
    pub fn shooting_star_cascade(&mut self) {
        self.cmds.push(DrawCmd::ShootingStarCascade);
    }
    pub fn table(&mut self) {
        self.cmds.push(DrawCmd::Table);
    }
    pub fn quad(&mut self, inst: GpuInstance) {
        self.cmds.push(DrawCmd::Quad(inst));
    }
    pub fn quads<I: IntoIterator<Item = GpuInstance>>(&mut self, iter: I) {
        self.cmds.extend(iter.into_iter().map(DrawCmd::Quad));
    }

    pub fn squircle_quads<I: IntoIterator<Item = GpuInstance>>(&mut self, iter: I) {
        self.cmds
            .extend(iter.into_iter().map(DrawCmd::SquircleQuad));
    }

    pub fn gradient_quads<
        I: IntoIterator<Item = crate::render::wgpu_renderer::GradientQuadInstance>,
    >(
        &mut self,
        iter: I,
    ) {
        self.cmds
            .extend(iter.into_iter().map(DrawCmd::GradientQuad));
    }
    pub fn flames<I: IntoIterator<Item = GpuInstance>>(&mut self, iter: I) {
        self.cmds.extend(iter.into_iter().map(DrawCmd::Flame));
    }
    pub fn text(&mut self, label: TextLabel) {
        self.cmds.push(DrawCmd::Text(label));
    }
    pub fn texts<I: IntoIterator<Item = TextLabel>>(&mut self, iter: I) {
        self.cmds.extend(iter.into_iter().map(DrawCmd::Text));
    }

    pub fn object3d(&mut self, obj: Object3d) {
        self.cmds.push(DrawCmd::Object3d(obj));
    }
    pub fn object3d_batch(&mut self, objs: Vec<Object3d>) {
        self.cmds.push(DrawCmd::Object3dBatch(objs));
    }
    pub fn showcase_tile_batch(&mut self, placements: Vec<ShowcaseTilePlacement>) {
        self.cmds.push(DrawCmd::ShowcaseTileBatch(placements));
    }
    pub fn tile_face_quads<I: IntoIterator<Item = TileFaceQuad>>(&mut self, iter: I) {
        self.cmds
            .extend(iter.into_iter().map(DrawCmd::TileFaceQuad));
    }

    pub fn prompt_icon_quads<I: IntoIterator<Item = PromptIconQuad>>(&mut self, iter: I) {
        self.cmds
            .extend(iter.into_iter().map(DrawCmd::PromptIconQuad));
    }

    /// Apply a global alpha multiplier to every queued cmd's color.
    /// Used by the main loop for scene transition fades.
    pub fn apply_alpha(&mut self, alpha: f32) {
        if alpha >= 1.0 {
            return;
        }
        for cmd in self.cmds.iter_mut() {
            match cmd {
                DrawCmd::Quad(inst) => inst.color[3] *= alpha,
                DrawCmd::SquircleQuad(inst) => inst.color[3] *= alpha,
                DrawCmd::TileFaceQuad(face) => face.inst.color[3] *= alpha,
                DrawCmd::PromptIconQuad(icon) => icon.inst.color[3] *= alpha,
                DrawCmd::GradientQuad(inst) => inst.color[3] *= alpha,
                // Flame `color.a` is a phase offset, not a transparency.
                // Don't scale it on transitions — the flame fades naturally
                // because the underlying scene quads behind it fade.
                DrawCmd::Flame(_) => {}
                DrawCmd::Text(lbl) => lbl.color[3] *= alpha,
                DrawCmd::Background(_)
                | DrawCmd::Starfield
                | DrawCmd::EmberDrift
                | DrawCmd::GoldenDust
                | DrawCmd::MoonlitWater
                | DrawCmd::SunlitWater
                | DrawCmd::ShootingStarCascade
                | DrawCmd::ShopEnvironment
                | DrawCmd::HallwayEnvironment
                | DrawCmd::ClearSceneDepth
                | DrawCmd::ShopInspectLitMeshSubjectHdr
                | DrawCmd::Table
                | DrawCmd::ShowcaseTileBatch(_)
                | DrawCmd::Object3d(_)
                | DrawCmd::Object3dBatch(_)
                | DrawCmd::MountainHaze => {}
            }
        }
    }
}

/// Strip scene depth-writing 3D cmds and stage the relic celebration modal:
/// orthographic-style camera, key/fill/rim lights, then the relic meshes.
///
/// Shared by the interactive `App::draw` path and headless screenshot capture
/// so the two cannot drift.
pub fn apply_modal_relic_staging(
    frame: &mut UiFrame,
    window_w: f32,
    window_h: f32,
    modal_relic_objects: Vec<Object3d>,
) {
    if modal_relic_objects.is_empty() {
        return;
    }
    frame.cmds.retain(|cmd| {
        !matches!(
            cmd,
            DrawCmd::Object3d(_)
                | DrawCmd::Object3dBatch(_)
                | DrawCmd::ShowcaseTileBatch(_)
                | DrawCmd::TileFaceQuad(_)
                | DrawCmd::ShopEnvironment
                | DrawCmd::HallwayEnvironment
                | DrawCmd::Table
        )
    });
    let w = window_w;
    let h = window_h;
    frame.camera_override = Some(CameraParams {
        eye: [0.0, -h * 3.0, 0.0],
        target: [0.0, 0.0, 0.0],
        up: [0.0, 0.0, 1.0],
        fovy_deg: 20.0,
    });
    frame.scene_lighting.set_smooth_points(vec![
        PointLight {
            pos: [w * 0.5 + w * 0.18, h * 0.5 + h * 0.45, h * 0.45],
            radius: h * 1.6,
            color: [1.00, 0.94, 0.82],
            intensity: 2.0,
        },
        PointLight {
            pos: [w * 0.5 - w * 0.22, h * 0.5 + h * 0.35, h * 0.30],
            radius: h * 1.3,
            color: [0.78, 0.86, 1.00],
            intensity: 0.9,
        },
        PointLight {
            pos: [w * 0.5, h * 0.5 - h * 0.30, h * 0.05],
            radius: h * 1.0,
            color: [1.00, 0.78, 0.42],
            intensity: 1.0,
        },
    ]);
    frame.object3d_batch(modal_relic_objects);
}

impl Default for UiFrame {
    fn default() -> Self {
        Self::new()
    }
}
