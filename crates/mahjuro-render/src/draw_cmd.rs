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
//! Debug overlays use [`DrawCmd::DebugOverlayQuad`] / [`DrawCmd::DebugOverlayText`]
//! and render in a separate post-UI pass so tuning panels stay readable.

use crate::lit_mesh::MaterialParams;
use crate::scene_keys;
use crate::theme::color;
use crate::wgpu_renderer::{GpuInstance, PointLight, SpotLight, TextLabel};
use glam;
use mahjuro_core::core::relic::RelicId;
use mahjuro_core::core::tile::Tile;
use mahjuro_core::core::tile_pack::TilePackKind;
use mahjuro_types::scene_draw::{BackgroundId, ButtonDef};
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
    /// Parallel to [`Self::punctual`]: glTF node name when sourced from embedded room lights.
    pub punctual_gltf_nodes: Vec<Option<String>>,
    pub spot_lights: Vec<SpotLight>,
    /// Set when [`Self::spot_lights`] were loaded from embedded glTF (`KHR_lights_punctual` spots).
    /// Programmatic spots must not set this — see [`Self::set_gltf_embedded_spot_lights`].
    pub spot_lights_from_gltf: bool,
    /// Room environment mesh uses `room_glb.wgsl` when true, else `tile_3d.wgsl`.
    pub room_glb_brdf: bool,
    /// Embedded `KHR_lights_punctual` active for this room (inverse-square lights + exposure path).
    pub embedded_gltf_punctual: bool,
}

impl SceneLighting {
    /// Single smooth point for showcase tile grids (stress lab, decimation picker).
    pub fn showcase_tile_picker(w: f32, h: f32) -> Self {
        let mut lit = Self::default();
        lit.push_smooth(PointLight {
            pos: [w * 0.5, h * 0.38, h * 1.35],
            radius: h * 3.1,
            color: color::rgb(color::PARCHMENT),
            intensity: 1.22,
        });
        lit
    }

    pub fn set_smooth_points(&mut self, v: Vec<PointLight>) {
        self.punctual = v.into_iter().map(ScenePunctualLight::Smooth).collect();
        self.punctual_gltf_nodes = vec![None; self.punctual.len()];
    }

    pub fn push_smooth(&mut self, p: PointLight) {
        self.punctual.push(ScenePunctualLight::Smooth(p));
        self.punctual_gltf_nodes.push(None);
    }

    pub fn push_inverse_square(&mut self, p: PointLight, gltf_node_name: Option<String>) {
        self.punctual.push(ScenePunctualLight::InverseSquare(p));
        self.punctual_gltf_nodes.push(gltf_node_name);
    }

    pub fn set_punctual_tagged(
        &mut self,
        entries: impl IntoIterator<Item = (ScenePunctualLight, Option<String>)>,
    ) {
        let tagged: Vec<_> = entries.into_iter().collect();
        self.punctual = tagged.iter().map(|(e, _)| e.clone()).collect();
        self.punctual_gltf_nodes = tagged.into_iter().map(|(_, n)| n).collect();
    }

    #[inline]
    pub fn punctual_gltf_node(&self, index: usize) -> Option<&str> {
        self.punctual_gltf_nodes
            .get(index)
            .and_then(|n| n.as_deref())
    }

    /// Assign spot lights decoded from a room `.glb` (unsupported on the punctual shadow path).
    pub fn set_gltf_embedded_spot_lights(&mut self, spots: Vec<SpotLight>) {
        self.spot_lights = spots;
        self.spot_lights_from_gltf = !self.spot_lights.is_empty();
    }

    #[inline]
    pub fn clear_spot_lights(&mut self) {
        self.spot_lights.clear();
        self.spot_lights_from_gltf = false;
    }
}

/// GPU / layout hints when the renderer scene key is `showcase` (showcase overlay).
/// The renderer uses these instead of branching on legacy per-flow scene keys.
///
/// GPU / tonemap hints for the showcase overlay and related flows.
#[derive(Clone, Copy, Debug, Default)]
pub struct ShowcaseRenderHints {
    /// Screen-pixel anchors (showcase tiles + smooth lights). See [`Self::layout_uses_ray_plane`].
    pub layout_use_ray_plane_z: bool,
    /// [`crate::wgpu_renderer::runtime::camera::WgpuRenderer::tile_hdr_tonemap`] pack-celebration path.
    pub tile_pack_celebration_tonemap: bool,
    /// Shop storeroom tonemap + lit_mesh glTF punctual branches (showcase overlay hosting shop).
    pub shop_tonemap_and_lit_mesh_context: bool,
    /// Archive grid HDR / tonemap branch for the showcase overlay.
    pub collection_tonemap_context: bool,
    /// Paginated relic unlock hero (`apply_modal_relic_staging`): black void +
    /// staging camera; keep lights/meshes on [`pixel_to_world`] and disable
    /// directional shadows (main menu would otherwise ray-map lights only).
    /// Hero relic meshes also skip the punctual shadow caster pass.
    pub modal_relic_staging: bool,
    /// Zodiac level-up ribbon on the showcase overlay: hero receives punctual
    /// light but should not cast a drop shadow on the black void (same as pack
    /// celebration tiles / pack mesh).
    pub zodiac_celebration_no_shadow: bool,
}

impl ShowcaseRenderHints {
    /// Screen-pixel anchors: showcase tiles and smooth punctual lights.
    ///
    /// **Ray-plane:** guide, tutorial, shop lights/tiles, pick-blind, main menu, tile-pack, anchor lab.
    /// **Pixel-to-world:** gameplay, yaku journal, wall ledger, archive collection.
    #[inline]
    pub fn layout_uses_ray_plane(self, active_scene_key: Option<&str>) -> bool {
        if self.modal_relic_staging {
            return false;
        }
        if self.layout_use_ray_plane_z {
            return true;
        }
        let key = active_scene_key.map(scene_keys::normalize_scene_key);
        if matches!(
            key,
            Some(scene_keys::GAMEPLAY | "yaku_journal" | "wall_ledger" | scene_keys::ARCHIVE)
        ) {
            return false;
        }
        matches!(
            key,
            Some(
                scene_keys::SHOP
                    | "tutorial"
                    | scene_keys::HALLWAY
                    | scene_keys::MAIN_MENU
                    | "tile_pack_celebration"
                    | "guide"
                    | "tile_anchor_lab"
                    | "tile_stress_lab"
                    | scene_keys::STAIRWAY
            )
        ) || (active_scene_key == Some("showcase") && self.tile_pack_celebration_tonemap)
    }

    /// Alias for [`Self::layout_uses_ray_plane`] — showcase tile batch placement.
    #[inline]
    pub fn showcase_tiles_use_ray_plane(self, active_scene_key: Option<&str>) -> bool {
        self.layout_uses_ray_plane(active_scene_key)
    }

    /// [`Object3d::pos`] decoding — almost always [`pixel_to_world`].
    ///
    /// Shop / archive encode world centers via [`crate::world_space::object3d_pos_triple_for_world_center`];
    /// only tile-pack celebration uses raw screen pixels + ray-plane here.
    #[inline]
    pub fn object3d_uses_ray_plane(self, active_scene_key: Option<&str>) -> bool {
        matches!(active_scene_key, Some("tile_pack_celebration"))
            || (active_scene_key == Some("showcase") && self.tile_pack_celebration_tonemap)
    }
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

    /// Cached view×projection for many world→screen projections in one frame.
    #[inline]
    pub fn screen_projector(&self, window_w: f32, window_h: f32) -> ScreenProjector {
        ScreenProjector::new(self, window_w, window_h)
    }

    /// Project a world point to layout pixels (same contract as the internal render camera).
    pub fn project_world_to_screen(
        &self,
        window_w: f32,
        window_h: f32,
        world: glam::Vec3,
    ) -> (f32, f32) {
        self.screen_projector(window_w, window_h).project(world)
    }
}

/// Per-frame view×projection cache for rain and other batched screen projections.
#[derive(Clone, Copy, Debug)]
pub struct ScreenProjector {
    view_proj: glam::Mat4,
    window_w: f32,
    window_h: f32,
}

impl ScreenProjector {
    pub fn new(cam: &CameraParams, window_w: f32, window_h: f32) -> Self {
        let aspect = window_w / window_h.max(1e-6);
        let eye = glam::Vec3::from_array(cam.eye);
        let target = glam::Vec3::from_array(cam.target);
        let up = glam::Vec3::from_array(cam.up);
        let view = glam::Mat4::look_at_rh(eye, target, up);
        let (near, far) = cam.clip_planes(window_h);
        let proj = glam::Mat4::perspective_rh(cam.fovy_deg.to_radians(), aspect, near, far);
        Self {
            view_proj: proj * view,
            window_w,
            window_h,
        }
    }

    #[inline]
    pub fn window_w(self) -> f32 {
        self.window_w
    }

    #[inline]
    pub fn window_h(self) -> f32 {
        self.window_h
    }

    #[inline]
    pub fn project(&self, world: glam::Vec3) -> (f32, f32) {
        let (sx, sy, _) = self.project_with_depth(world);
        (sx, sy)
    }

    #[inline]
    pub fn project_with_depth(&self, world: glam::Vec3) -> (f32, f32, f32) {
        let clip = self.view_proj * glam::Vec4::new(world.x, world.y, world.z, 1.0);
        let inv_w = 1.0 / clip.w.max(1e-6);
        let nx = clip.x * inv_w;
        let ny = clip.y * inv_w;
        let depth = clip.z * inv_w;
        let sx = (nx * 0.5 + 0.5) * self.window_w;
        let sy = (1.0 - (ny * 0.5 + 0.5)) * self.window_h;
        (sx, sy, depth)
    }
}

/// Packs `(pixel_x, pixel_y, lift)` for a point in **world space** (`lift` is **+Z** above the felt).
/// Consumed by [`crate::world_space::pixel_to_world`].
pub type WorldSurfaceAnchor = [f32; 3];

// ── Skeuomorphic gameplay HUD placements ──────────────────────────────────
//
// Phase 1 of the in-game UI redesign: physical objects rendered through the
// `lit_mesh` pipeline replace the flat slate-blue HUD rects. Each variant has
// a sibling DrawCmd below.
//
// Phase 1 wires up the mesh + draw cmd infrastructure but no scene actually
// pushes these yet — phases 2-7 introduce the corresponding `gameplay.rs`
// pushes one region at a time.

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

/// Material selector for the extruded-glyph score popup. Maps to the lit-mesh
/// shader's `MaterialKind` branch at render time.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GlyphMaterial {
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
    /// other 3D placement via [`crate::world_space::layout_anchor_to_world`]
    /// (gameplay → [`crate::world_space::pixel_to_world`]; showcase/guide → ray → `plane_z`).
    /// `lift` is height above the felt (**+Z**).
    pub center_pos: [f32; 3],
    /// Euler rotation `(rx, ry, rz)` in radians — same composition as
    /// [`crate::table_transform::rot_euler_xyz_rad`], after the
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
    /// Override [`tile_outline.wgsl`] rim mode (`base_color_factor.y`). `None`
    /// derives from `selected` / `hovered` (gold / blue). `2.0` = decimation red.
    pub outline_sel: Option<f32>,
    /// Logical slot index for ray-cast tile picking and `proj.hand_rects` tracking.
    /// `None` = not pickable (pack-open showcase tiles, etc.).
    pub pick_id: Option<usize>,
    /// When set, this tile contributes to a merged screen overlay rect (dora / round wind).
    pub overlay_rect_group: Option<TileOverlayRectGroup>,
}

/// One [`DrawCmd::ShowcaseTileBatch`] — optional screen scissor for scroll panels.
#[derive(Clone, Debug)]
pub struct ShowcaseTileBatchCmd {
    pub placements: Vec<ShowcaseTilePlacement>,
    pub clip_rect: Option<[f32; 4]>,
}

impl From<Vec<ShowcaseTilePlacement>> for ShowcaseTileBatchCmd {
    fn from(placements: Vec<ShowcaseTilePlacement>) -> Self {
        Self {
            placements,
            clip_rect: None,
        }
    }
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
    /// Sub-rectangle of a Mahjuro `atlas.toml` + `atlas.png` grid (temptations, …).
    /// `sheet` is the asset-relative PNG path; `name` is a layout cell id.
    PackedAtlas {
        sheet: &'static str,
        name: &'static str,
    },
    /// Full embedded PNG under `assets/` (via [`mahjuro_assets::asset_path::get`]).
    Asset { path: &'static str },
    /// Relic icon: albedo + mask cut via [`crate::relic_pipeline`].
    Relic(mahjuro_core::core::relic::RelicId),
    /// Procedural debuff X ([`crate::decal::rasterize_debuff_marker_overlay`]) — same mark as on tiles.
    DebuffMarker,
}

impl ImageQuadSource {
    pub fn cache_key(&self) -> String {
        match self {
            Self::AtlasSprite { sheet, name } => format!("atlas:{sheet}:{name}"),
            Self::PackedAtlas { sheet, name } => format!("packed-atlas:{sheet}:{name}"),
            Self::Asset { path } => format!("asset:{path}"),
            Self::Relic(id) => format!("relic:{id:?}"),
            Self::DebuffMarker => "debuff-marker".to_string(),
        }
    }
}

/// Screen-space RGBA texture quad (`shaders/image_quad.wgsl`).
#[derive(Clone, Debug)]
pub struct ImageQuad {
    pub inst: GpuInstance,
    pub source: ImageQuadSource,
    /// When set, the renderer applies a scissor rect for this draw.
    pub clip_rect: Option<[f32; 4]>,
}

/// Screen-space debuff X centered on `anchor_rect` (matches relic/tile overlay sizing).
pub fn debuff_marker_image_quad(anchor_rect: [f32; 4]) -> ImageQuad {
    let [rx, ry, rw, rh] = anchor_rect;
    let side = (rw.min(rh) * 0.42).max(14.0).min(rw.min(rh) * 0.92);
    let cx = rx + rw * 0.5;
    let cy = ry + rh * 0.48;
    ImageQuad {
        inst: GpuInstance {
            rect: [cx - side * 0.5, cy - side * 0.5, side, side],
            color: [1.0, 1.0, 1.0, 1.0],
            user: 0,
        },
        source: ImageQuadSource::DebuffMarker,
        clip_rect: None,
    }
}

// ── General-purpose 3D placement ─────────────────────────────────────────

/// Euler **XYZ** radians for [`Object3d::rotation`] — same convention as
/// [`ShowcaseTilePlacement::rotation`] and [`crate::table_transform::rot_euler_xyz_rad`].
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
    },
    /// Tile-pack box on the shop shelf.
    Pack {
        kind: TilePackKind,
        pick_id: Option<u32>,
    },
    /// Silken zodiac ribbon; `Object3d::pos` is the mesh centroid.
    ZodiacRibbon {
        kind: Option<mahjuro_core::core::zodiac::ZodiacKind>,
    },
    /// Talisman tablet (shop curio).
    Talisman {
        kind: mahjuro_core::core::talisman::TalismanKind,
    },
    /// Memorial remnant tablet (defeat / dish).
    MemorialTalisman {
        kind: mahjuro_core::core::memorial_talisman::MemorialTalismanKind,
    },
    /// Boss encounter icon — extruded silhouette mesh from `textures/ordeal_icons/`
    /// (same mesh builder as [`Relic`], archive cubbies + pedestal close-up).
    BossIcon {
        kind: mahjuro_core::core::ordeal_kind::OrdealKind,
        glow: f32,
        pick_id: Option<u32>,
    },
    // (Coin uses `Primitive { shape: Coin }` routed to [`DrawKind::GltfCoin`]
    // and rendered with full glTF PBR from [`coin.glb`](../../../assets/3d/coin.glb).)
    // (GoldBar is now modeled as `Primitive { shape: Cube,
    // material: MaterialSpec::metal() }`.)
    // (BrassRail is now modeled as `Primitive { shape: Cube,
    // material: MaterialSpec::brass() }`.)
    // (Standing book was removed; the shop now uses an
    // `Object3dKind::WoodTablet { label: "Journal", pick_id: Some(…)
    // }` to match gameplay's journal affordance.)
    /// Discard bowl. Hover animation is driven by [`Object3d::hover_target`].
    Bowl,
    /// Bronze "play hand" mirror. Hover animation is driven by [`Object3d::hover_target`].
    /// [`valid_play_glow`] drives the jade fresnel rim when the hand selection is
    /// playable (`lit_mesh.wgsl` `instance_params.x`).
    Mirror {
        /// Valid-hand jade fresnel intensity in \[0, 1\]. `0.0` when idle.
        valid_play_glow: f32,
    },
    // (Dish is now modeled as `Primitive { shape: DiscSquare or
    // DiscRound, material: MaterialSpec::plain() }`. Callers set `pos[2]` to the dish center (base + extents[1] *
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
        /// RGBA tint for the stick body (lower ~80% of each stick).
        base_color: [f32; 4],
        /// RGBA tint for the tip cap (upper ~20%).
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
    /// are additive: add a [`crate::primitive::MeshId`] variant
    /// and register the mesh in `WgpuRenderer::new`.
    Primitive {
        shape: crate::primitive::MeshId,
        material: crate::primitive::MaterialSpec,
        pick_id: Option<u32>,
        /// When true, render as a near-black matte silhouette
        /// (locked-collection lock state). Decal and material kind
        /// are suppressed; `obj.color` alpha is preserved.
        silhouette: bool,
    },
}

/// `Object3d::anim_id` on the inspected stock mesh — sole shadow caster during storeroom inspect.
pub const SHOP_INSPECT_SUBJECT_ANIM_ID: u64 = 0x5348_4F50_5F49; // "SHOPI"
/// Archive inspect-orbit subject only (grid featured close-up and cubbies do not cast).
pub const ARCHIVE_FEATURED_ANIM_ID: u64 = 0x00C1_05E0;

/// A single lit mesh placed in the world.
///
/// Replaces all individual `XxxPlacement` structs for objects rendered through
/// the `lit_mesh_pipeline`.  Scenes set `pos`, `extents`, and `rotation`
/// as Euler **XYZ** radians — same as [`ShowcaseTilePlacement::rotation`].
/// Use [`camera_facing_euler_xyz_rad`] when the face should track the camera.
#[derive(Clone, Debug)]

pub struct Object3d {
    /// Center position as `(pixel_x, pixel_y, lift)`.
    /// Mapped to world space by [`crate::world_space::pixel_to_world`].
    pub pos: [f32; 3],
    /// Full extents `(width, height, depth)` in world units.
    pub extents: [f32; 3],
    /// Euler **XYZ** radians — [`crate::table_transform::rot_euler_xyz_rad`].
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
        crate::table_transform::rot_euler_xyz_rad(
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
    /// Procedural golden-dust with god-rays vignette (fullscreen triangle, no data).
    GoldenDust,
    /// Procedural moon hovering above rippling water (fullscreen triangle, no data).
    MoonlitWater,
    /// Procedural sun hovering above rippling water (fullscreen triangle, no data).
    SunlitWater,
    /// Procedural shooting-star cascade transition (fullscreen triangle, no data).
    /// Brightness driven by `UiFrame::transition_progress`.
    ShootingStarCascade,
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
    /// Imported `staircase.glb` post-ordeal interstitial (same GPU path as [`DrawCmd::ShopEnvironment`]).
    StaircaseEnvironment,
    /// Imported `archive.glb` Archive room (same GPU path as [`DrawCmd::ShopEnvironment`]).
    ArchiveEnvironment,
    /// Imported `main_menu.glb` hub waterfront (same GPU path as [`DrawCmd::ShopEnvironment`]).
    MainMenuEnvironment,
    /// Imported [`gameplay.glb`](../../assets/3d/gameplay.glb) table room (same GPU path as shop).
    GameplayEnvironment,
    /// Reset the main scene depth target while keeping the HDR color buffer. Later 3D
    /// draws (same camera) composite by depth among themselves but no longer test
    /// against geometry drawn before this marker — e.g. pack celebration meshes over
    /// the shop room without hiding the room in color.
    ClearSceneDepth,
    /// Batch of showcase tiles with explicit 3D transforms — used for hand
    /// tiles, pack-opening celebrations, and any other 3D tile placement.
    ShowcaseTileBatch(ShowcaseTileBatchCmd),
    /// Flat screen-space tile face using the real per-tile decal art.
    TileFaceQuad(TileFaceQuad),
    /// Generic 2D quad (panels, dimmers, borders…).
    Quad(GpuInstance),
    /// Screen-space quad that reads scene depth (used by world-projected rain).
    DepthQuad(GpuInstance),
    /// Screen-space quad drawn after tonemap (tooltip frames, etc.) so bright
    /// HDR bloom cannot paint over UI panels.
    OverlayQuad(GpuInstance),
    /// Post-tonemap squircle panel — same payload as [`DrawCmd::SquircleQuad`].
    OverlaySquircleQuad(GpuInstance),
    /// 2D quad with a superellipse (squircle) silhouette — same `GpuInstance`
    /// payload as [`DrawCmd::Quad`]. See `shaders/squircle_quad.wgsl`.
    SquircleQuad(GpuInstance),
    /// Alpha-feathered 2D quad — solid `color` in the centre, falling off
    /// to full transparency toward the edges. Used as a soft dark backer
    /// behind HUD content so panels read against busy backgrounds without
    /// a hard-edged letterbox. See `shaders/gradient_quad.wgsl`.
    GradientQuad(crate::wgpu_renderer::GradientQuadInstance),
    /// Circular hold-to-act progress ring. See `shaders/arc_ring_quad.wgsl`.
    ArcRingQuad(crate::wgpu_renderer::ArcRingQuadInstance),
    /// Triggers the volumetric flame batch (`procedural_flame_emitters` on the frame).
    Flame,
    /// Rasterized text label.
    Text(TextLabel),
    /// Post-tonemap debug panel quad — drawn after normal UI overlay pass.
    DebugOverlayQuad(GpuInstance),
    /// Post-tonemap debug panel label — drawn after normal UI overlay pass.
    DebugOverlayText(TextLabel),
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
/// Pick-ray proxies for authored [`gameplay.glb`](../../assets/3d/gameplay.glb) action meshes.
/// The renderer seeds `last_bowl_model` / `proj` rects without drawing duplicate geometry.
#[derive(Clone, Debug, Default)]
pub struct GameplayActionPickProxies {
    pub bowl: Option<Object3d>,
    pub mirror: Option<Object3d>,
    pub journal: Option<Object3d>,
    pub guidebook: Option<Object3d>,
    pub cash_in_tablet: Option<Object3d>,
}

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
    /// When set together with [`DrawCmd::ClearSceneDepth`], the first Pass A
    /// subpass keeps [`Self::camera_override`] (e.g. room background); later
    /// subpasses and showcase-tile placement use this camera instead.
    pub camera_override_after_depth_clear: Option<CameraParams>,
    /// When set with [`DrawCmd::ClearSceneDepth`], Pass A subpasses after the
    /// first depth clear use this lighting instead of [`Self::scene_lighting`]
    /// (e.g. decimation tiles must not inherit staircase embedded punctual).
    pub scene_lighting_after_depth_clear: Option<SceneLighting>,
    /// Debug overlay: when true, the renderer draws three colored axis bars
    /// (red = +X, green = +Y, blue = +Z) anchored at the camera's look
    /// target. Toggled from the native Debug menu in the gameplay scene to
    /// help disambiguate world-space directions when iterating on
    /// placements.
    pub debug_axes: bool,
    /// Rain debug menu: draw emissive overlay on main-menu `rain_hit_*` collision shells.
    pub debug_rain_hit_colliders: bool,
    /// Rain debug menu: visualize rain quad depth as grayscale.
    pub debug_rain_depth: bool,
    /// When `Some`, overrides the tile material for this frame. Used by
    /// the tile-select scene to preview materials before a run starts.
    pub tile_material_override: Option<mahjuro_gfx_types::TileMaterial>,

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
    /// Set by showcase overlay presenters; read by shadow / placement / tonemap paths.
    pub showcase_render_hints: ShowcaseRenderHints,
    /// Archive description quads: `Some(true)` = show left sign only; `Some(false)` = right only;
    /// `None` = draw both sign meshes. Renderer culls the hidden GLB primitive index.
    pub archive_description_sign_use_left: Option<bool>,
    /// Archive cabinet page buttons (`btn_page_left` / `btn_page_right`). When false, the matching
    /// GLB mesh is culled (e.g. first page hides left, last page hides right).
    pub archive_page_left_visible: bool,
    pub archive_page_right_visible: bool,
    /// When set, browse-mode copy is rasterized into the archive room decal atlas and composited
    /// on the active `sign_description_*` mesh in `room_glb.wgsl`.
    pub archive_sign_description_decal_text: Option<String>,
    /// Item inspect overlay: show `inspect_plaque` / `plaque_backing` and rasterize copy onto the plaque.
    pub archive_inspect_plaque_visible: bool,
    /// Inspect-mode description copy for [`archive_inspect_plaque_visible`].
    pub archive_inspect_plaque_decal_text: Option<String>,
    /// Pick-blind hallway vertex warp (`room_glb.wgsl` @group(0) @binding(8)); `None` elsewhere.
    pub hallway_distortion: Option<crate::hallway_glb::HallwayDistortion>,
    /// When true, skip offline room GI probe bake and run dynamic `emissive-probe-update`.
    pub room_gi_dynamic: bool,
    /// World-space flame emitters without procedural candle meshes (e.g. shop
    /// `light_candle_*` punctual lights). Merged into the particle system each frame.
    pub procedural_flame_emitters: Vec<crate::flame_volume::FlameEmitter>,
    /// Authored-table gameplay: raycast + projected rects for discard river, mirror, journal, cash-in.
    pub gameplay_action_picks: Option<GameplayActionPickProxies>,
    /// When false, `btn_cash_in` / `label_cash_in` env meshes are not drawn or pickable.
    pub gameplay_cash_in_button_visible: bool,
    /// When true, gameplay room env draws only cash-in control meshes (guide scoring flow).
    pub gameplay_env_cash_in_only: bool,
    /// Guide scoring flow: isolated cash-in env draw uses this camera after showcase tiles.
    pub gameplay_cash_in_overlay_camera: Option<CameraParams>,
    /// Lighting paired with [`Self::gameplay_cash_in_overlay_camera`] (gameplay embedded punctual).
    pub gameplay_cash_in_overlay_lighting: Option<SceneLighting>,
    /// Sustained pulse (0..1) for the cash-in control; scales with played meld count.
    pub gameplay_cash_in_glow: f32,
    /// Vertical wiggle (screen px) for the cash-in control; scales with played meld count.
    pub gameplay_cash_in_wiggle: f32,
    /// The House ordeal: animate `btn_cash_in` with glossary polychrome bands until discards are spent.
    pub gameplay_cash_in_blocked: bool,
    /// Active shop glTF node TRS animation samples (clip name, playback time in seconds).
    pub shop_gltf_anim_samples: Vec<(String, f32)>,
    /// When true, shop room env draws only the `Eyeball` node mesh (animation lab).
    pub shop_env_eyeball_only: bool,
    /// When true, main-menu room env draws only `MoonObject` (victory run summary).
    pub main_menu_env_moon_only: bool,
    /// Extra model matrix applied after hub room centering (victory moon recenter / scale).
    pub main_menu_env_model_delta: glam::Mat4,
    /// When true, `moonlit_water.wgsl` skips the procedural 2D moon disc (3D moon replaces it).
    pub moonlit_water_hide_disc: bool,
    /// Animation lab: flat albedo + simple N·L in `room_glb.wgsl` (no punctual PBR).
    pub shop_env_unlit_debug: bool,
    /// Gameplay score roller digits `(live_score, target_score)` when the HUD roller is active.
    pub gameplay_score_roller_values: Option<(u64, u64)>,
    /// Debug [`shadow_ao_lab`] overlay — synthetic geometry + contact AO bake.
    pub shadow_ao_lab_layout: Option<crate::shadow_ao_lab::ShadowAoLabLayout>,
    /// Debug labs: override global shadow quality for one frame (`None` = use settings).
    pub shadow_quality_override: Option<mahjuro_gfx_types::ShadowQuality>,
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
            camera_override_after_depth_clear: None,
            scene_lighting_after_depth_clear: None,
            debug_axes: false,
            debug_rain_hit_colliders: false,
            debug_rain_depth: false,
            tile_material_override: None,
            buttons: Vec::new(),
            window_title: String::new(),
            transition_progress: 0.0,
            fisheye_strength: 0.0,
            journal_prepass_frame: None,
            showcase_render_hints: ShowcaseRenderHints::default(),
            archive_description_sign_use_left: None,
            archive_page_left_visible: false,
            archive_page_right_visible: false,
            archive_sign_description_decal_text: None,
            archive_inspect_plaque_visible: false,
            archive_inspect_plaque_decal_text: None,
            hallway_distortion: None,
            room_gi_dynamic: false,
            procedural_flame_emitters: Vec::new(),
            gameplay_action_picks: None,
            gameplay_cash_in_button_visible: false,
            gameplay_env_cash_in_only: false,
            gameplay_cash_in_overlay_camera: None,
            gameplay_cash_in_overlay_lighting: None,
            gameplay_cash_in_glow: 0.0,
            gameplay_cash_in_wiggle: 0.0,
            gameplay_cash_in_blocked: false,
            shop_gltf_anim_samples: Vec::new(),
            shop_env_eyeball_only: false,
            main_menu_env_moon_only: false,
            main_menu_env_model_delta: glam::Mat4::IDENTITY,
            moonlit_water_hide_disc: false,
            shop_env_unlit_debug: false,
            gameplay_score_roller_values: None,
            shadow_ao_lab_layout: None,
            shadow_quality_override: None,
        }
    }

    /// Lighting for showcase tiles and post–depth-clear 3D when set.
    #[inline]
    pub fn foreground_scene_lighting(&self) -> &SceneLighting {
        self.scene_lighting_after_depth_clear
            .as_ref()
            .unwrap_or(&self.scene_lighting)
    }

    /// Camera for showcase tiles and post–depth-clear 3D when set; otherwise
    /// [`Self::camera_override`].
    #[inline]
    pub fn foreground_camera(&self) -> Option<&CameraParams> {
        self.camera_override_after_depth_clear
            .as_ref()
            .or(self.camera_override.as_ref())
    }

    /// `true` when a depth reset precedes the first showcase tile batch.
    pub fn depth_clear_before_showcase(&self) -> bool {
        let mut seen_clear = false;
        for cmd in &self.cmds {
            match cmd {
                DrawCmd::ClearSceneDepth => seen_clear = true,
                DrawCmd::ShowcaseTileBatch(_) if seen_clear => return true,
                _ => {}
            }
        }
        false
    }

    /// Pass A subpass `chunk_index` camera: background subpass uses
    /// [`Self::camera_override`]; later subpasses use [`Self::foreground_camera`].
    #[inline]
    pub fn pass_a_chunk_camera(&self, chunk_index: usize) -> Option<&CameraParams> {
        if chunk_index > 0 && self.camera_override_after_depth_clear.is_some() {
            self.foreground_camera()
        } else {
            self.camera_override.as_ref()
        }
    }

    #[inline]
    pub fn uses_room_glb_shader(&self) -> bool {
        self.scene_lighting.room_glb_brdf
            || self
                .gameplay_cash_in_overlay_lighting
                .as_ref()
                .is_some_and(|l| l.room_glb_brdf)
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
    /// Draw the post-ordeal staircase from embedded [`staircase.glb`](../../assets/3d/staircase.glb).
    pub fn staircase_environment(&mut self) {
        self.cmds.push(DrawCmd::StaircaseEnvironment);
    }
    /// Draw [`archive.glb`](../../assets/3d/archive.glb). No-op if the asset failed to load.
    pub fn archive_environment(&mut self) {
        self.cmds.push(DrawCmd::ArchiveEnvironment);
    }
    /// Draw [`main_menu.glb`](../../assets/3d/main_menu.glb). No-op if the asset failed to load.
    pub fn main_menu_environment(&mut self) {
        self.cmds.push(DrawCmd::MainMenuEnvironment);
    }
    /// Draw [`gameplay.glb`](../../assets/3d/gameplay.glb). No-op if the asset failed to load.
    pub fn gameplay_environment(&mut self) {
        self.cmds.push(DrawCmd::GameplayEnvironment);
    }
    /// See [`DrawCmd::ClearSceneDepth`].
    pub fn clear_scene_depth(&mut self) {
        self.cmds.push(DrawCmd::ClearSceneDepth);
    }
    pub fn starfield(&mut self) {
        self.cmds.push(DrawCmd::Starfield);
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
    pub fn quad(&mut self, inst: GpuInstance) {
        self.cmds.push(DrawCmd::Quad(inst));
    }
    pub fn quads<I: IntoIterator<Item = GpuInstance>>(&mut self, iter: I) {
        self.cmds.extend(iter.into_iter().map(DrawCmd::Quad));
    }
    pub fn depth_quads<I: IntoIterator<Item = GpuInstance>>(&mut self, iter: I) {
        self.cmds.extend(iter.into_iter().map(DrawCmd::DepthQuad));
    }
    pub fn overlay_quads<I: IntoIterator<Item = GpuInstance>>(&mut self, iter: I) {
        self.cmds.extend(iter.into_iter().map(DrawCmd::OverlayQuad));
    }

    pub fn debug_overlay_quads<I: IntoIterator<Item = GpuInstance>>(&mut self, iter: I) {
        self.cmds
            .extend(iter.into_iter().map(DrawCmd::DebugOverlayQuad));
    }

    pub fn debug_overlay_texts<I: IntoIterator<Item = TextLabel>>(&mut self, iter: I) {
        self.cmds
            .extend(iter.into_iter().map(DrawCmd::DebugOverlayText));
    }

    pub fn overlay_squircle_quads<I: IntoIterator<Item = GpuInstance>>(&mut self, iter: I) {
        self.cmds
            .extend(iter.into_iter().map(DrawCmd::OverlaySquircleQuad));
    }

    pub fn squircle_quads<I: IntoIterator<Item = GpuInstance>>(&mut self, iter: I) {
        self.cmds
            .extend(iter.into_iter().map(DrawCmd::SquircleQuad));
    }

    pub fn gradient_quads<I: IntoIterator<Item = crate::wgpu_renderer::GradientQuadInstance>>(
        &mut self,
        iter: I,
    ) {
        self.cmds
            .extend(iter.into_iter().map(DrawCmd::GradientQuad));
    }
    pub fn arc_ring_quads<I: IntoIterator<Item = crate::wgpu_renderer::ArcRingQuadInstance>>(
        &mut self,
        iter: I,
    ) {
        self.cmds.extend(iter.into_iter().map(DrawCmd::ArcRingQuad));
    }
    pub fn flame_batch(&mut self) {
        self.cmds.push(DrawCmd::Flame);
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
        self.showcase_tile_batch_clipped(placements, None);
    }

    pub fn showcase_tile_batch_clipped(
        &mut self,
        placements: Vec<ShowcaseTilePlacement>,
        clip_rect: Option<[f32; 4]>,
    ) {
        self.cmds.push(DrawCmd::ShowcaseTileBatch(ShowcaseTileBatchCmd {
            placements,
            clip_rect,
        }));
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
                DrawCmd::DepthQuad(inst) => inst.color[3] *= alpha,
                DrawCmd::OverlayQuad(inst) => inst.color[3] *= alpha,
                DrawCmd::DebugOverlayQuad(inst) => inst.color[3] *= alpha,
                DrawCmd::OverlaySquircleQuad(inst) => inst.color[3] *= alpha,
                DrawCmd::SquircleQuad(inst) => inst.color[3] *= alpha,
                DrawCmd::TileFaceQuad(face) => face.inst.color[3] *= alpha,
                DrawCmd::ImageQuad(icon) => icon.inst.color[3] *= alpha,
                DrawCmd::GradientQuad(inst) => inst.color[3] *= alpha,
                DrawCmd::ArcRingQuad(inst) => {
                    inst.fill_color[3] *= alpha;
                    inst.track_color[3] *= alpha;
                }
                // Flame `color.a` is a phase offset, not a transparency.
                // Don't scale it on transitions — the flame fades naturally
                // because the underlying scene quads behind it fade.
                DrawCmd::Flame => {}
                DrawCmd::Text(lbl) => lbl.color[3] *= alpha,
                DrawCmd::DebugOverlayText(lbl) => lbl.color[3] *= alpha,
                DrawCmd::Background(_)
                | DrawCmd::Starfield
                | DrawCmd::GoldenDust
                | DrawCmd::MoonlitWater
                | DrawCmd::SunlitWater
                | DrawCmd::ShootingStarCascade
                | DrawCmd::ShopEnvironment
                | DrawCmd::HallwayEnvironment
                | DrawCmd::StaircaseEnvironment
                | DrawCmd::ArchiveEnvironment
                | DrawCmd::MainMenuEnvironment
                | DrawCmd::GameplayEnvironment
                | DrawCmd::ClearSceneDepth
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
    // Hero relic + punctual lights share the staging camera on a black void.
    frame.showcase_render_hints.modal_relic_staging = true;
    frame.scene_lighting.embedded_gltf_punctual = false;
    frame.scene_lighting.room_glb_brdf = false;
    frame.cmds.retain(|cmd| {
        !matches!(
            cmd,
            DrawCmd::Object3d(_)
                | DrawCmd::Object3dBatch(_)
                | DrawCmd::ShowcaseTileBatch(_)
                | DrawCmd::TileFaceQuad(_)
                | DrawCmd::ShopEnvironment
                | DrawCmd::HallwayEnvironment
                | DrawCmd::StaircaseEnvironment
                | DrawCmd::ArchiveEnvironment
                | DrawCmd::MainMenuEnvironment
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
    frame.object3d_batch(modal_relic_objects);
}

impl Default for UiFrame {
    fn default() -> Self {
        Self::new()
    }
}
