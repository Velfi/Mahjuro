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
use crate::render::theme::color;
use crate::render::wgpu_renderer::{GpuInstance, PointLight, SpotLight, TextLabel};
use crate::render::world_space::pixel_to_world;
use crate::scenes::{BackgroundId, ButtonDef};
use glam;
use std::borrow::Cow;
use std::sync::Arc;

/// `far = window_h × this` for [`CameraParams::frustum_x_range_at`], [`CameraParams::project_world_to_screen`],
/// and the renderer’s perspective matrix. glTF room verts scale ~with `window_h`, so long corridors need >12×.
pub const SCENE_PERSPECTIVE_FAR_MUL: f32 = 32.0;

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
    /// Perspective near plane in **world units**. `None` → `1.0` (table / legacy default).
    pub clip_near: Option<f32>,
    /// Perspective far plane in **world units**. `None` → `window_h ×`
    /// [`SCENE_PERSPECTIVE_FAR_MUL`] (table / legacy default).
    pub clip_far: Option<f32>,
}

/// One punctual entry in [`SceneLighting`]. Smooth lights match legacy gameplay/candle
/// falloff; inverse-square matches embedded `KHR_lights_punctual` from GLB rooms.
#[derive(Clone, Debug)]
pub enum ScenePunctualLight {
    Smooth(PointLight),
    InverseSquare(PointLight),
}

/// Unified per-frame lights for tiles, `lit_mesh`, and GLB room passes.
#[derive(Clone, Debug, Default)]
pub struct SceneLighting {
    pub punctual: Vec<ScenePunctualLight>,
    pub spot_lights: Vec<SpotLight>,
    /// Room environment mesh uses `room_glb.wgsl` when true, else `tile_3d.wgsl`.
    pub room_glb_brdf: bool,
    /// Embedded `KHR_lights_punctual` active for this room (inverse-square lights + exposure path).
    pub embedded_gltf_punctual: bool,
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
    /// [`crate::render::wgpu_renderer::runtime::camera::WgpuRenderer::tile_hdr_tonemap`] pack-celebration path.
    pub tile_pack_celebration_tonemap: bool,
    /// Shop storeroom tonemap + lit_mesh inspect / glTF punctual branches.
    pub shop_tonemap_and_lit_mesh_context: bool,
    /// Archive grid HDR / tonemap branch for the showcase overlay.
    pub collection_tonemap_context: bool,
}

impl CameraParams {
    /// `(near, far)` for [`glam::Mat4::perspective_rh`], in world units.
    #[inline]
    pub fn clip_planes(&self, window_h: f32) -> (f32, f32) {
        let h = window_h.max(1e-6);
        let near = self.clip_near.unwrap_or(1.0).max(1e-3);
        let far = self
            .clip_far
            .unwrap_or(h * SCENE_PERSPECTIVE_FAR_MUL)
            .max(near + 1.0);
        (near, far)
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
            clip_near: None,
            clip_far: None,
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
        let (near, far) = self.clip_planes(window_h);
        let proj = glam::Mat4::perspective_rh(self.fovy_deg.to_radians(), aspect, near, far);
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
/// Which gameplay plinth HUD rect slot to publish.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PlinthRole {
    Dora,
    Boss,
    RoundWind,
}

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

impl CascadeTokenKind {
    /// Canonical RGBA tint for this cascade token kind. Centralized so
    /// the score-popup glyphs, the cascade HUD label, and the 3D
    /// cascade-token meshes all stay in sync — any one of those reading
    /// the wrong color would break the warm-vs-cool reading of the
    /// score breakdown.
    pub fn color(self) -> [f32; 4] {
        match self {
            CascadeTokenKind::Chips => crate::render::theme::color::LAPIS,
            CascadeTokenKind::Mult => crate::render::theme::color::RUBY,
        }
    }
}

/// Material selector for the extruded-glyph score popup. Maps to the lit-mesh
/// shader's `MaterialKind` branch at render time.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GlyphMaterial {
    #[allow(dead_code)] // Score-reel digits (3D path); renderer arm kept in sync.
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
    /// When set, this tile contributes to a merged screen overlay rect (dora / round wind).
    pub overlay_rect_group: Option<TileOverlayRectGroup>,
}

/// Merged HUD overlay rect sources for groups of showcase tiles.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TileOverlayRectGroup {
    DoraTiles,
    RoundWindTiles,
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
pub enum ImageQuadSource {
    /// Sub-rectangle of a baked Kenney Input Prompts sheet PNG. `sheet` is the
    /// asset-relative path (under `assets/`) to the `_sheet_double.png`; `name`
    /// is the matching `SubTexture name="…"` from the sibling XML index. The
    /// renderer decodes each sheet once on first use, then crops the named
    /// sub-rect into a per-glyph GPU texture (cached by [`Self::cache_key`]).
    AtlasSprite {
        sheet: &'static str,
        name: &'static str,
    },
    /// Sub-rectangle of a Mahjuro `atlas.toml` + `atlas.png` grid (skip tags, …).
    /// `sheet` is the asset-relative PNG path; `name` is a layout cell id.
    PackedAtlas {
        sheet: &'static str,
        name: &'static str,
    },
    /// Full embedded PNG under `assets/` (via [`crate::asset_path::get`]).
    Asset { path: &'static str },
    /// Absolute filesystem path to an SVG or PNG (rasterized at draw time).
    #[allow(dead_code)] // No current producer; renderer path kept for tooling / experiments.
    Filesystem(std::path::PathBuf),
}

impl ImageQuadSource {
    pub fn cache_key(&self) -> String {
        match self {
            Self::AtlasSprite { sheet, name } => format!("atlas:{sheet}:{name}"),
            Self::PackedAtlas { sheet, name } => format!("packed-atlas:{sheet}:{name}"),
            Self::Asset { path } => format!("asset:{path}"),
            Self::Filesystem(path) => format!("file:{}", path.display()),
        }
    }
}

/// Screen-space RGBA texture quad (`shaders/image_quad.wgsl`).
#[derive(Clone, Debug)]
pub struct ImageQuad {
    pub inst: GpuInstance,
    pub source: ImageQuadSource,
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
    /// Procedural ornate brass plinth used by the gameplay scene to display
    /// the dora indicator tile(s). The mesh has no roof; the indicator
    /// tile face(s) are pushed separately as `ShowcaseTilePlacement`s
    /// resting on the platform on top.
    Plinth { glow: f32, role: PlinthRole },
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
    /// Silken zodiac ribbon; `Object3d::pos` is the mesh centroid.
    ZodiacRibbon {
        kind: Option<crate::core::zodiac::ZodiacKind>,
    },
    /// Talisman tablet (shop curio).
    Talisman {
        kind: crate::core::talisman::TalismanKind,
    },
    /// Boss encounter icon — extruded silhouette mesh from `textures/boss_icons/`
    /// (same mesh builder as [`Relic`], archive cubbies + pedestal close-up).
    BossIcon {
        kind: crate::core::boss::BossKind,
        glow: f32,
        pick_id: Option<u32>,
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
        /// Pitch / roll from scene layout (degrees, `Rz * Ry * Rx`).
        placement_rot_deg: [f32; 3],
        /// Which counter this fan represents (peg_rects slot).
        kind: TallyFanKind,
    },
    /// One hovering insect near a light source (main-menu door light). The scene emits
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
        /// Legacy flag — shadow participation is driven by
        /// [`crate::render::lit_mesh::material_casts_shadow`] on `material.kind`
        /// (everything except [`MaterialKind::Emissive`] casts).
        #[allow(dead_code)]
        shadow_caster: bool,
        /// When true, render as a near-black matte silhouette
        /// (locked-collection lock state). Decal and material kind
        /// are suppressed; `obj.color` alpha is preserved.
        silhouette: bool,
    },
}

/// `Object3d::anim_id` on the inspected stock mesh — sole shadow caster during storeroom inspect.
pub const SHOP_INSPECT_SUBJECT_ANIM_ID: u64 = 0x5348_4F50_5F49; // "SHOPI"
/// Archive pedestal / HUD featured close-up (casts dynamic shadow; grid cubbies do not).
pub const ARCHIVE_FEATURED_ANIM_ID: u64 = 0xC105_E0;

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
    #[allow(dead_code)]
    EmberDrift,
    /// Procedural golden-dust with god-rays vignette (fullscreen triangle, no data).
    GoldenDust,
    /// Procedural moon hovering above rippling water (fullscreen triangle, no data).
    MoonlitWater,
    /// Procedural sun hovering above rippling water (fullscreen triangle, no data).
    SunlitWater,
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
    /// Imported `shop.glb` room mesh (tile-textured pipeline, world-space vertices).
    ShopEnvironment,
    /// Imported `hallway.glb` pick-blind room (same GPU path as [`DrawCmd::ShopEnvironment`]).
    HallwayEnvironment,
    /// Imported `archive.glb` Archive room (same GPU path as [`DrawCmd::ShopEnvironment`]).
    ArchiveEnvironment,
    /// Imported `main_menu.glb` hub waterfront (same GPU path as [`DrawCmd::ShopEnvironment`]).
    MainMenuEnvironment,
    /// Reset the main scene depth target while keeping the HDR color buffer. Later 3D
    /// draws (same camera) composite by depth among themselves but no longer test
    /// against geometry drawn before this marker — e.g. pack celebration meshes over
    /// the shop room without hiding the room in color.
    ClearSceneDepth,
    /// Batch of showcase tiles with explicit 3D transforms — used for hand
    /// tiles, pack-opening celebrations, and any other 3D tile placement.
    ShowcaseTileBatch(Vec<ShowcaseTilePlacement>),
    /// Flat screen-space tile face using the real per-tile decal art.
    TileFaceQuad(TileFaceQuad),
    /// Generic 2D quad (panels, dimmers, borders…).
    Quad(GpuInstance),
    /// Screen-space quad drawn after tonemap (tooltip frames, etc.) so bright
    /// HDR bloom cannot paint over UI panels.
    OverlayQuad(GpuInstance),
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
    /// Tinted RGBA texture quad (logos, Kenney prompts, atlas sprites, …).
    ImageQuad(ImageQuad),
    // ── Skeuomorphic gameplay HUD ──
    /// Batch of wood action tablets (cash-in).
    /// Floating 3D extruded-glyph score popups. Each placement carries its
    /// own label string; the renderer lazily builds a per-string mesh on
    /// first use and reuses it on subsequent frames.
    // ── General-purpose 3D objects ──────────────────────────────────
    /// Single general-purpose lit-mesh object. See [`Object3d`].
    Object3d(Object3d),
    /// Batch of general-purpose lit-mesh objects. See [`Object3d`].
    Object3dBatch(Vec<Object3d>),
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
    /// Barrel / fisheye lens distortion applied in the final composite.
    /// 0.0 = off (no distortion). Positive = outward barrel (center
    /// magnified, edges compressed). Typical range 0.0..=0.6. Scenes
    /// that use barrel distortion (e.g. Archive browser) set this to pull
    /// the viewport toward a fisheye lens.
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
    /// lights are tuned for table-scale HDR — not the `/512` crush used for bright `shop.glb`.
    /// When set, [`crate::render::wgpu_renderer::runtime::camera::WgpuRenderer::tile_hdr_tonemap`]
    /// applies gameplay-style linear exposure for `lit_mesh` so shelf props stay visible.
    pub shop_inspect_lit_mesh_hdr: bool,
    /// Set by showcase overlay presenters; read by shadow / placement / tonemap paths.
    pub showcase_render_hints: ShowcaseRenderHints,
    /// Archive description quads: `Some(true)` = show left sign only; `Some(false)` = right only;
    /// `None` = draw both (or procedural Archive). Renderer culls the hidden GLB primitive index.
    pub archive_description_sign_use_left: Option<bool>,
    /// When set, description copy is rasterized into the archive room decal atlas and composited
    /// on the `sign_description_left` / `sign_description_right` meshes in `room_glb.wgsl`.
    pub archive_sign_description_decal_text: Option<String>,
    /// Pick-blind hallway vertex warp (`room_glb.wgsl` @group(0) @binding(8)); `None` elsewhere.
    pub hallway_distortion: Option<crate::render::hallway_glb::HallwayDistortion>,
    /// When true, skip offline room GI probe bake and run dynamic `emissive-probe-update`.
    pub room_gi_dynamic: bool,
    /// Shop item inspect: baked GI stays on; shadow pre-pass uses a tight frustum at this
    /// pivot and only draws the mesh tagged with [`SHOP_INSPECT_SUBJECT_ANIM_ID`].
    pub shop_inspect_shadow_target: Option<[f32; 3]>,
    /// World-space flame emitters without a [`Object3dKind::Candle`] mesh (e.g. shop
    /// `light_candle_*` punctual lights). Merged into the particle system each frame.
    pub procedural_flame_emitters: Vec<crate::render::flame_volume::FlameEmitter>,
}

impl UiFrame {
    pub fn new() -> Self {
        Self {
            cmds: Vec::new(),
            scene_lighting: SceneLighting::default(),
            candle_light_count: 0,
            flame_height_world: 0.0,
            cursor_pos: None,
            camera_override: None,
            debug_axes: false,
            tile_material_override: None,
            buttons: Vec::new(),
            window_title: String::new(),
            transition_progress: 0.0,
            fisheye_strength: 0.0,
            journal_prepass_frame: None,
            shop_inspect_lit_mesh_hdr: false,
            showcase_render_hints: ShowcaseRenderHints::default(),
            archive_description_sign_use_left: None,
            archive_sign_description_decal_text: None,
            hallway_distortion: None,
            room_gi_dynamic: false,
            shop_inspect_shadow_target: None,
            procedural_flame_emitters: Vec::new(),
        }
    }

    /// `room_glb.wgsl` for room meshes vs `tile_3d` (includes shop inspect storeroom path).
    #[inline]
    pub fn uses_room_glb_shader(&self) -> bool {
        self.scene_lighting.room_glb_brdf || self.shop_inspect_lit_mesh_hdr
    }

    // ── Push helpers ────────────────────────────────────────────────────
    pub fn background(&mut self, bg: BackgroundId) {
        self.cmds.push(DrawCmd::Background(bg));
    }
    /// Draw the 3D shop from embedded [`shop.glb`](../../assets/3d/shop.glb). No-op if the asset failed to load.
    pub fn shop_environment(&mut self) {
        self.cmds.push(DrawCmd::ShopEnvironment);
    }
    /// Draw the pick-blind hallway from embedded [`hallway.glb`](../../assets/3d/hallway.glb).
    pub fn hallway_environment(&mut self) {
        self.cmds.push(DrawCmd::HallwayEnvironment);
    }
    /// Draw [`archive.glb`](../../assets/3d/archive.glb). No-op if the asset failed to load.
    pub fn archive_environment(&mut self) {
        self.cmds.push(DrawCmd::ArchiveEnvironment);
    }
    /// Draw [`main_menu.glb`](../../assets/3d/main_menu.glb). No-op if the asset failed to load.
    pub fn main_menu_environment(&mut self) {
        self.cmds.push(DrawCmd::MainMenuEnvironment);
    }
    /// See [`DrawCmd::ClearSceneDepth`].
    pub fn clear_scene_depth(&mut self) {
        self.cmds.push(DrawCmd::ClearSceneDepth);
    }
    pub fn starfield(&mut self) {
        self.cmds.push(DrawCmd::Starfield);
    }
    #[allow(dead_code)]
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
    pub fn overlay_quads<I: IntoIterator<Item = GpuInstance>>(&mut self, iter: I) {
        self.cmds.extend(iter.into_iter().map(DrawCmd::OverlayQuad));
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

    pub fn image_quads<I: IntoIterator<Item = ImageQuad>>(&mut self, iter: I) {
        self.cmds.extend(iter.into_iter().map(DrawCmd::ImageQuad));
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
                DrawCmd::OverlayQuad(inst) => inst.color[3] *= alpha,
                DrawCmd::SquircleQuad(inst) => inst.color[3] *= alpha,
                DrawCmd::TileFaceQuad(face) => face.inst.color[3] *= alpha,
                DrawCmd::ImageQuad(icon) => icon.inst.color[3] *= alpha,
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
                | DrawCmd::ArchiveEnvironment
                | DrawCmd::MainMenuEnvironment
                | DrawCmd::ClearSceneDepth
                | DrawCmd::Table
                | DrawCmd::ShowcaseTileBatch(_)
                | DrawCmd::Object3d(_)
                | DrawCmd::Object3dBatch(_) => {}
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
                | DrawCmd::ArchiveEnvironment
                | DrawCmd::MainMenuEnvironment
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
        clip_near: None,
        clip_far: None,
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
            color: color::rgb(color::CHAMPAGNE),
            intensity: 1.0,
        },
    ]);
    let relic = &modal_relic_objects[0];
    let cx = relic.pos[0];
    let cy = relic.pos[1];
    let lift = relic.pos[2];
    let cos_outer = (36.0_f32).to_radians().cos();
    let cos_inner = (22.0_f32).to_radians().cos();
    let spot_lift = lift + h * 0.42;
    let spot_pos = [cx, cy - h * 0.06, spot_lift];
    let tw = pixel_to_world(w, h, cx, cy, lift);
    let lw = pixel_to_world(w, h, spot_pos[0], spot_pos[1], spot_pos[2]);
    let dir = (tw - lw).normalize_or_zero();
    let dir = if dir.length_squared() < 1e-4 {
        glam::Vec3::new(0.0, 0.4, -1.0).normalize()
    } else {
        dir
    };
    frame.scene_lighting.spot_lights = vec![SpotLight {
        pos: spot_pos,
        dir: dir.to_array(),
        radius: w.max(h) * 2.2,
        cos_outer,
        cos_inner,
        color: color::rgb(color::PARCHMENT),
        intensity: 6.0,
    }];
    frame.object3d_batch(modal_relic_objects);
}

impl Default for UiFrame {
    fn default() -> Self {
        Self::new()
    }
}
