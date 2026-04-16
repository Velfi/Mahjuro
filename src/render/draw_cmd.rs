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
//!
//! ## Markers
//!
//! `FluidSmoke` is a marker that the renderer expands into pipeline-specific
//! draws. It obeys the same ordering rule: a marker draws *between* whatever
//! was pushed before and after it. Scenes place it in declarative order
//! alongside ordinary cmds.

use crate::core::relic::RelicId;
use crate::core::tile::Tile;
use crate::core::tile_pack::TilePackKind;
use crate::render::candle_mesh::CandlePlacement;
use crate::render::lit_mesh::MaterialParams;
use crate::render::wgpu_renderer::{GpuInstance, PointLight, RelicIcon, TextLabel};
use crate::scenes::{BackgroundId, ButtonDef};
use glam;

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
}

/// Packs `(pixel_x, pixel_y, lift)` for a point in **world space** (`lift` is **+Z** above the felt).
/// Consumed by [`crate::render::world_space::pixel_to_world`].
pub type WorldSurfaceAnchor = [f32; 3];

/// One soft wind impulse to inject into the volumetric smoke sim this frame.
///
/// Coordinates use the same `(pixel_x, pixel_y)` convention as the rest of the
/// scene draw output: the renderer projects them onto the felt plane (`z ≈ 0`, with
/// the optional `lift_px` height) using [`crate::render::world_space::pixel_to_world`] before
/// queueing the impulse on the fluid sim. Velocity is in **world** units (same
/// frame as [`crate::render::world_space`]): **+Z** is up from the felt; along the
/// felt, larger layout **py** (nearer the player / bottom of screen) maps to more
/// **negative world y** (see [`crate::render::world_space::pixel_to_world`]).
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
    /// Density delta to add at the impulse center. Negative values pull
    /// existing smoke apart; small positive values trail a faint puff.
    pub density: f32,
}

/// One physical relic placeholder sitting in the dish on the table.
///
/// Coordinates use the same convention as `CandlePlacement`: `world_pos` is
/// `(pixel_x, pixel_y, lift)` — the renderer maps the pixel x/y onto
/// the table plane and uses `lift` as the height above the wood (**+Z**).
#[derive(Clone, Copy, Debug)]
pub struct RelicPlacement {
    /// `(pixel_x, pixel_y, lift)` for the box's *base center*.
    pub world_pos: WorldSurfaceAnchor,
    /// Half-extents of the relic box in world units (x = width/2, y = height/2,
    /// z = depth/2). Each placeholder gets a slightly different size so the
    /// row reads as a collection of distinct objects.
    pub half_extents: [f32; 3],
    /// Tint color (linear RGBA). Driven by relic rarity in the gameplay scene.
    pub color: [f32; 4],
    /// Which relic this placeholder represents — used by the scene to look up
    /// the name + description for the hover tooltip.
    pub relic_id: RelicId,
    /// Activation glow intensity in [0, 1]. The gameplay scene drives this
    /// with a fast attack + smooth decay envelope when a scoring cascade
    /// step credits this relic. The renderer brightens the relic's base
    /// color and emits an additive halo around its projected screen rect.
    /// Zero (the default) means "not glowing" and skips both effects.
    pub glow: f32,
    /// Rotation around the local X axis in degrees. Positive tilts the top
    /// of the box backward (away from the camera). Used by the shop scene to
    /// lean for-sale relics against the back of the cabinet shelf.
    pub rotation_x_deg: f32,
    /// Rotation around the local Z axis in degrees. Used for the activation
    /// wiggle — the scene drives a decaying sinusoidal oscillation so the
    /// relic wobbles when its scoring effect triggers.
    pub rotation_z_deg: f32,
}

/// A centered 3D relic viewer placement used by collection cards, tutorial
/// panels, and unlock modals. Orientation is **`Rx * Ry * Rz`**
/// ([`crate::render::table_transform::rot_rx_ry_rz_deg`]).
#[derive(Clone, Copy, Debug)]
pub struct RelicShowcasePlacement {
    /// `(pixel_x, pixel_y, lift)` for the object's center.
    pub center_pos: [f32; 3],
    /// Width × height × depth in world units.
    pub extents: [f32; 3],
    /// Yaw rotation about world Y in degrees.
    pub rotation_y_deg: f32,
    /// Pitch rotation about world X in degrees.
    pub rotation_x_deg: f32,
    /// Roll rotation about world Z in degrees.
    pub rotation_z_deg: f32,
    /// Base tint (linear RGBA).
    pub color: [f32; 4],
    pub relic_id: RelicId,
    /// Optional reveal/activation glow in [0, 1].
    pub glow: f32,
}

/// A tile-pack box on the shop shelf. Rendered as an axis-aligned box (not
/// the relic cylinder mesh) with foil pack art. `rotation_x_deg` leans the box
/// against the shelf back.
#[derive(Clone, Copy, Debug)]
pub struct PackPlacement {
    pub world_pos: WorldSurfaceAnchor,
    pub half_extents: [f32; 3],
    pub color: [f32; 4],
    pub kind: TilePackKind,
    /// Rotation around the local X axis in degrees (tilt forward/back).
    pub rotation_x_deg: f32,
    /// Rotation around the local Y axis in degrees (turn left/right).
    pub rotation_y_deg: f32,
    /// Optional pick id so hit-testing can identify this pack.
    pub pick_id: Option<u32>,
}

/// One zodiac/talisman ribbon hanging from an anchor point.
///
/// Used by the shop scene for both the wall-pinned for-sale ribbons (in the
/// curio cabinet) and the player's owned-consumable fan in front of the
/// counter. `rotation_y_deg` lets the same mesh be reused for the radial fan
/// (set per-ribbon to spread them) and for plain vertical drops (set 0).
///
/// Same **`Rz * Ry * Rx`** composition as [`TalismanPlacement`]
/// ([`crate::render::table_transform::rot_rz_ry_rx_deg`]).
#[derive(Clone, Copy, Debug)]
pub struct ZodiacRibbonPlacement {
    /// `(pixel_x, pixel_y, lift)` for the ribbon's *top anchor*.
    pub anchor_pos: [f32; 3],
    /// Length the ribbon hangs in world units.
    pub length: f32,
    /// Width of the ribbon in world units.
    pub width: f32,
    /// Yaw rotation about world Y around the anchor, in degrees. 0 = the
    /// ribbon hangs straight down with its face toward the camera.
    pub rotation_y_deg: f32,
    /// Pitch rotation about world X around the anchor, in degrees. Used by
    /// the inventory fan to drape ribbons forward toward the camera.
    pub rotation_x_deg: f32,
    /// World **Z** rotation in degrees — outer factor in **`Rz * Ry * Rx`**
    /// ([`crate::render::table_transform::rot_rz_ry_rx_deg`]). Shop wall ribbons
    /// usually leave this at 0; collection applies the slow spin here.
    pub rotation_z_deg: f32,
    /// Linear-space RGBA tint. When `kind` is `Some`, this is multiplied
    /// against the silk texture (use white to show the texture unmodified;
    /// drop alpha to dim sold ribbons).
    pub color: [f32; 4],
    /// Which zodiac silk texture to bind. `None` falls back to the flat
    /// untextured ribbon (used as a generic placeholder).
    pub kind: Option<crate::core::zodiac::ZodiacKind>,
}

/// One hanging talisman tablet (jade-amulet pendant). Used by the shop scene
/// for the for-sale talismans pinned in the curio cabinet — distinct from the
/// silken zodiac ribbons hanging next to them.
///
/// Rotations compose as **`Rz * Ry * Rx`** — see
/// [`crate::render::table_transform::rot_rz_ry_rx_deg`]. For a top-down
/// collection camera, **yaw (`rotation_y_deg`)** spins the tablet in the table
/// plane; **pitch (`rotation_x_deg`)** tips the face toward/away from the camera
/// (e.g. `-90` for face-on from above).
#[derive(Clone, Copy, Debug)]
pub struct TalismanPlacement {
    /// `(pixel_x, pixel_y, lift)` for the tablet's *center*.
    pub center_pos: [f32; 3],
    /// Width × height × thickness in world units.
    pub extents: [f32; 3],
    /// Yaw rotation about world Y in degrees (turntable in the table plane).
    pub rotation_y_deg: f32,
    /// Pitch rotation about world X in degrees. 0 = mesh-default standing pose.
    /// `-90` lays the tablet face-up on the table (face normal toward +Y).
    pub rotation_x_deg: f32,
    /// Roll rotation about world Z in degrees (in-plane wobble after pitch/yaw).
    pub rotation_z_deg: f32,
    /// Linear-space RGBA tint.
    pub color: [f32; 4],
    /// Which talisman variant — determines the heightmap motif.
    pub kind: crate::core::talisman::TalismanKind,
}

/// One physical gold coin sitting in (or on) the coin dish.
///
/// Coins are stamped via the lit-mesh pipeline using a shared 16-prism
/// cylinder mesh; per-instance position + rotation + scale supply the visual
/// variety in a stacked pile.
#[derive(Clone, Copy, Debug)]
pub struct CoinPlacement {
    /// `(pixel_x, pixel_y, lift)` for the coin's center.
    pub world_pos: WorldSurfaceAnchor,
    /// Yaw rotation about world Y in radians (small per-coin jitter).
    pub rotation_y: f32,
    /// World-units radius (typically a few pixels).
    pub radius: f32,
    /// World-units thickness.
    pub thickness: f32,
    /// Linear-space RGBA tint.
    pub color: [f32; 4],
}

/// One gold bar (big or mini) sitting in the coin dish area.
///
/// Bars are rendered as unit-box meshes via the lit-mesh pipeline with the
/// Metal material, giving them the same specular gold finish as coins.
#[derive(Clone, Copy, Debug)]
pub struct GoldBarPlacement {
    /// `(pixel_x, pixel_y, lift)` for the bar's base center.
    pub world_pos: WorldSurfaceAnchor,
    /// Yaw rotation about world Y in radians.
    pub rotation_y: f32,
    /// Half-extents of the bar in world units (width/2, height/2, depth/2).
    pub half_extents: [f32; 3],
    /// Linear-space RGBA tint.
    pub color: [f32; 4],
}

/// A standing book rendered via the book mesh (rounded spine, page inset).
/// Used by the shop scene for the Yaku Journal bookend on the inventory shelf.
#[derive(Clone, Copy, Debug)]
pub struct BookPlacement {
    /// `(pixel_x, pixel_y, lift)` for the book's base center.
    pub world_pos: WorldSurfaceAnchor,
    /// Yaw rotation about world Y in radians.
    pub rotation_y: f32,
    /// Half-extents in world units (width/2, height/2, depth/2).
    pub half_extents: [f32; 3],
    /// Linear-space RGBA tint for the cover.
    pub color: [f32; 4],
    /// Optional pick id for hit testing.
    pub pick_id: Option<u32>,
}

/// One free-standing dish placement in world space (no auto-sizing from a
/// relic batch). Used by the shop scene to draw the foreground relic dish
/// and the coin dish at fixed positions, independent of the for-sale relics
/// living up in the curio cabinet.
#[derive(Clone, Copy, Debug)]
pub struct DishExplicit {
    /// `(pixel_x, pixel_y, lift)` for the dish base center.
    pub center_pos: [f32; 3],
    /// Full extents in world units (width × height × depth). The dish mesh
    /// itself is a wide low box; height ≈ rim height.
    pub extents: [f32; 3],
    /// Optional id used by `pick_shop_object` to recognize a hit on this
    /// dish (e.g. so the scene can route a click on the coin dish to "show
    /// gold count" rather than to a relic).
    pub pick_id: Option<u32>,
    /// Rotation applied to the dish mesh in world space. Use
    /// `glam::Mat4::IDENTITY` (or omit via `..Default::default()`) for the
    /// default Y-up orientation. The pick-blind floor plane uses
    /// `mesh_y_thickness_along_local_y_to_z_up()` to lay flat.
    pub rotation: glam::Mat4,
    /// Name exposed to arrange mode (`pick_debug_object` + `apply_arrange_override`).
    /// `None` means the dish is not selectable in arrange mode.
    pub arrange_name: Option<&'static str>,
    /// If true, render with the round dish mesh (circular rim + recessed
    /// floor). Default false uses the legacy square dish mesh so existing
    /// callers (relic dish, ribbon/talisman trays) keep their look.
    pub round: bool,
}

impl Default for DishExplicit {
    fn default() -> Self {
        Self {
            center_pos: [0.0; 3],
            extents: [1.0, 1.0, 1.0],
            pick_id: None,
            rotation: glam::Mat4::IDENTITY,
            arrange_name: None,
            round: false,
        }
    }
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
#[allow(dead_code)]
///
/// The wood + chain mesh is uploaded once at renderer init; per-instance
/// position + extents drive a model matrix and the renderer rasterizes the
/// `text` payload into a decal texture sampled by the lit-mesh shader.
#[derive(Clone, Debug)]
pub struct PlaquePlacement {
    /// `(pixel_x, pixel_y, lift)` for the plaque's *center*.
    pub center_pos: [f32; 3],
    /// Width × height × thickness in world units.
    pub extents: [f32; 3],
    /// Yaw rotation about world Y in degrees (0 = facing the camera).
    pub rotation_y_deg: f32,
    /// Plaque text. May contain `\n` for hard line breaks; otherwise it's
    /// word-wrapped to fit the plaque face. The renderer picks the largest
    /// font size where the wrapped layout fits the face.
    pub text: String,
}

/// Hanging boss-rule ofuda paper.
#[allow(dead_code)]
#[derive(Clone, Debug)]
pub struct OfudaPlacement {
    /// `(pixel_x, pixel_y, lift)` for the ofuda's *center*.
    pub center_pos: [f32; 3],
    /// Width × height × thickness in world units.
    pub extents: [f32; 3],
    /// Yaw rotation about world Y in degrees.
    pub rotation_y_deg: f32,
    /// Boss name (large title at the top of the paper).
    pub title: String,
    /// Boss rule description (smaller body text below the title).
    pub rule: String,
}

/// One yaku selector tablet (carved bone, sitting in a row below the hand).
#[allow(dead_code)]
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
    /// Progress toward this yaku in [0, 1] — drives the inlay glow strip.
    pub progress: f32,
    /// True when this yaku is the player's currently selected target.
    pub active: bool,
    /// Hover lift envelope in [0, 1] driven by the scene each frame.
    pub hover: f32,
}

/// One sort/play action wood tablet. Same mesh as the yaku tablets but
/// lacquered wood; visually consistent with the table.
#[allow(dead_code)]
#[derive(Clone, Debug)]
pub struct WoodTabletPlacement {
    /// Tablet *base center* as [`WorldSurfaceAnchor`] (third component = **+Z** lift).
    pub world_pos: WorldSurfaceAnchor,
    /// Width × height × depth in world units.
    pub extents: [f32; 3],
    /// Engraved label (Sort by Suit, Sort by Rank, Play, …).
    pub label: String,
    /// Press envelope in [0, 1] (1.0 = fully pressed in).
    pub pressed: f32,
    /// Hover lift envelope in [0, 1].
    pub hover: f32,
    /// Rotation around the local Z axis in degrees. Used for excited idle
    /// wiggles such as the gameplay cash-in prompt when a full structure is ready.
    pub rotation_z_deg: f32,
    /// True if the action is currently disabled (e.g. no tiles selected).
    pub disabled: bool,
}

/// The discard bowl. Click target = drop selected tile in.
#[derive(Clone, Copy, Debug)]
pub struct BowlPlacement {
    /// Bowl *base center* as [`WorldSurfaceAnchor`].
    pub world_pos: WorldSurfaceAnchor,
    /// Width × height × depth in world units.
    pub extents: [f32; 3],
    /// Hover lift envelope in [0, 1].
    pub hover: f32,
    /// Base pitch rotation about world X in degrees, applied before the
    /// hover tilt animation. 0 = mesh default orientation.
    pub rotation_x_deg: f32,
}

/// The bronze mirror. Click target = play the selected hand. Visual
/// counterpart to the discard bowl, sharing the same flat-on-table
/// footprint and hover-lift convention.
#[derive(Clone, Copy, Debug)]
pub struct MirrorPlacement {
    /// Mirror *base center* as [`WorldSurfaceAnchor`].
    pub world_pos: WorldSurfaceAnchor,
    /// Width × height × depth in world units.
    pub extents: [f32; 3],
    /// Hover lift envelope in [0, 1].
    pub hover: f32,
    /// Base pitch toward/away from the camera in degrees. Positive tips the
    /// reflective face toward the player before hover animation is applied.
    pub rotation_x_deg: f32,
    /// Roll around the local Z axis in degrees. Used for scene-authored idle
    /// wobble without affecting the shared hover envelope.
    pub rotation_z_deg: f32,
}

/// Which counter fan this `TallyFanPlacement` represents. Drives per-fan
/// focus rect slot and tooltip wiring on the gameplay scene side.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TallyFanKind {
    /// Plays-remaining fan, anchored in front of the mirror.
    Draws,
    /// Discards-remaining fan, anchored in front of the river.
    Discards,
}

/// An upright fan of bone tally sticks — one fan per counter (draws in front of
/// the mirror, discards in front of the river). The fan's pivot is the narrow
/// base of each stick; sticks radiate upward/outward through a total spread
/// angle, evenly spaced across `max_count` slots. Sticks keep their *original*
/// angular slots as the count drops (the fan thins from the outermost stick
/// first) so consumption reads as a spent stick rather than a re-deal.
#[derive(Clone, Copy, Debug)]
pub struct TallyFanPlacement {
    /// Fan pivot — `(pixel_x, pixel_y, lift)` at the base of each stick.
    pub world_pos: WorldSurfaceAnchor,
    /// Stick height (world units, from narrow base to wide tip).
    pub stick_len: f32,
    /// Wide-end width (world units, at the tip). Narrow-end width is
    /// baked into the mesh at half the wide end (2:1 taper).
    pub stick_wide: f32,
    /// Thickness along the fan's forward axis (world units).
    pub stick_thickness: f32,
    /// Sticks currently visible (the live count).
    pub count: u32,
    /// Slot count the fan is sized for (determines stick spacing and outer
    /// angular slots).
    pub max_count: u32,
    /// Total arc the fan spreads across at `max_count` (degrees, symmetric
    /// about vertical). Typical value ~60°.
    pub spread_deg: f32,
    /// RGBA tint for the upper cap of each stick. The tip cap's length is
    /// baked into the tally-stick mesh (`TIP_FRAC`).
    pub tip_color: [f32; 4],
    /// Yaw rotation of the fan plane about world up (degrees). Lets scenes
    /// angle the fan to face the camera.
    pub rotation_y_deg: f32,
    /// Which counter this fan represents.
    pub kind: TallyFanKind,
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

/// One engraved bone token in the cascade scoring readout. Reuses the
/// `bone_tablet_mesh` geometry, tinted per axis (chips = cool indigo,
/// mult = warm crimson). The gameplay scene drives the per-frame `pulse`
/// envelope from the existing cascade timing math; the renderer turns it
/// into a brief uniform scale-up so the active token visibly pops on each
/// scoring step.
#[derive(Clone, Copy, Debug)]
pub struct CascadeTokenPlacement {
    /// `(pixel_x, pixel_y, lift)` for the token's *center*.
    pub world_pos: WorldSurfaceAnchor,
    /// Width × thickness × depth in world units.
    pub extents: [f32; 3],
    /// Which scoring axis this token shows.
    pub kind: CascadeTokenKind,
    /// Pulse envelope in [0, 1] from the cascade frame's pop-in/settle
    /// timing. 1.0 = freshly fired, 0.0 = settled.
    pub pulse: f32,
}

/// One floating 3D extruded-glyph score popup ("+50", "×3", "=12500"). The
/// renderer turns each placement into one indexed draw of the glyph mesh
/// cached for `label` (lazily built on first use), positioned and tinted
/// per-instance. Popups are short-lived: the gameplay scene's
/// `ScorePopupSystem` spawns them on each cascade reveal-edge and clears
/// them when the cascade ends.
#[derive(Clone, Debug)]
pub struct ExtrudedGlyphPlacement {
    /// `(pixel_x, pixel_y, lift)` for the popup's *center*. Pixel
    /// x/y resolve via [`crate::render::world_space::pixel_to_world`]; `lift` is the
    /// height above the table plane the popup currently floats at.
    pub world_pos: WorldSurfaceAnchor,
    /// Uniform world-units scale applied to the glyph mesh. The mesh itself
    /// is normalised to a height of 1.0 unit, so this directly sets the
    /// rendered character height in world space.
    pub scale: f32,
    /// Pitch rotation (radians) about world X (combined with `-π/2` camera-face in
    /// the renderer — see [`crate::render::table_transform::score_popup_glyph_rot_rad`]).
    pub rotation_x: f32,
    /// Yaw rotation (radians) about world Y — small per-popup random
    /// jitter so a chain of popups doesn't read as a stamped row.
    pub rotation_y: f32,
    /// The label string ("+50", "×3", "=12500"). Used as the cache key for
    /// the renderer's lazy `GlyphMeshCache` upload.
    pub label: String,
    /// Linear-space RGBA tint. Alpha is multiplied by the lit_mesh
    /// material's base alpha so the popup can fade out at end of life.
    pub color: [f32; 4],
    /// Emissive boost in [0, 1]. The renderer adds this to the lit base
    /// color so popups read against busy backgrounds without depending on
    /// candle illumination.
    pub emissive: f32,
    /// Which lit-mesh material to render the glyph with. `Plain` is the
    /// default flat/specular look used by debuff Xs and other simple popups.
    /// `Metal` gives Fresnel-tinted specular highlights for the score reel.
    /// `Polychrome` adds a thin-film rainbow sheen over the base color,
    /// used by the streaming chip/mult/gold popups.
    pub material: GlyphMaterial,
}

/// Material selector for [`ExtrudedGlyphPlacement`]. Maps to the lit-mesh
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
    /// Logical slot index for ray-cast tile picking and `proj.hand_rects` tracking.
    /// `None` = not pickable (pack-open showcase tiles, etc.).
    pub pick_id: Option<usize>,
}

/// One physical scoring bone tumbling onto the play space during a cascade.
/// Reuses the `bone_tablet_mesh` geometry; per-instance tint matches the
/// cascade token kind so chips bones read cool indigo and mult bones read
/// warm crimson, tying the falling pile back to the HUD readout it spawned
/// from. The simulation lives in `crate::render::falling_bones` and is
/// driven by the gameplay scene's cascade reveal events.
#[derive(Clone, Copy, Debug)]
pub struct FallingBonePlacement {
    /// `(pixel_x, pixel_y, world_z)` for the bone's *center*. Pixel x/y
    /// resolve via [`crate::render::world_space::pixel_to_world`]; `world_z` is the live
    /// height above the table plane that the simulation drives under
    /// gravity.
    pub world_pos: WorldSurfaceAnchor,
    /// Width × thickness × depth in world units.
    pub extents: [f32; 3],
    /// Tumble euler angles `(rot_x, rot_y, rot_z)` in radians — composition
    /// **`Ry * Rx * Rz`**, see [`crate::render::table_transform::rot_ry_rx_rz_rad`].
    pub rotation: [f32; 3],
    /// Which scoring axis this bone represents (drives the tint).
    pub kind: CascadeTokenKind,
    /// Linear alpha multiplier in [0, 1]. Stays at 1.0 in flight; ramps to
    /// 0.0 once the bone has landed and its rest timer is bleeding out.
    pub alpha: f32,
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

/// One curio-cabinet placement (single instance — the back wall of the shop).
#[derive(Clone, Copy, Debug)]
pub struct CurioCabinetPlacement {
    /// `(pixel_x, pixel_y, lift)` for the cabinet's *center*.
    pub center_pos: [f32; 3],
    /// Full extents in world units (width × height × depth).
    pub extents: [f32; 3],
}

// ── General-purpose 3D placement ─────────────────────────────────────────

/// Rotation matrix for an [`Object3d`].  Scenes compose this directly using
/// [`glam::Mat4`] rotation constructors — no implicit axis order is assumed.
///
/// ```no_run
/// // Flat panel facing the camera (pitch only):
/// use glam::Mat4;
/// let pitch = camera_facing_rotation(cam.eye, cam.target);
/// // Extra yaw: multiply on the left (applied after pitch).
/// let rot = Mat4::from_rotation_z(yaw_rad) * pitch;
/// ```
pub type Rotation3d = glam::Mat4;

/// Returns a rotation [`Mat4`] that pitches a +Z-normal flat mesh to face
/// the given camera.  Equivalent to the formula wood tablets have always used.
///
/// Compose additional rotations by multiplying on the left:
/// `extra_rot * camera_facing_rotation(eye, target)`.
pub fn camera_facing_rotation(cam_eye: [f32; 3], look_target: [f32; 3]) -> glam::Mat4 {
    let look = glam::Vec3::from(look_target) - glam::Vec3::from(cam_eye);
    let pitch_deg = look.z.atan2(look.y.abs()).to_degrees() + 180.0;
    glam::Mat4::from_rotation_x(pitch_deg.to_radians())
}

/// Mesh-specific data carried alongside the common [`Object3d`] fields.
/// Each variant names only the fields that differ from the common set.
#[derive(Clone, Debug)]
#[allow(dead_code)]
pub enum Object3dKind {
    // ── Upright panels ──────────────────────────────────────────────
    /// Lacquered wood slab + chain nubs with a dynamically-scaled engraved
    /// decal. The renderer word-wraps `text` and picks the largest font size
    /// that fits the face; callers format with explicit `\n` if they want a
    /// forced line break between fields.
    Plaque {
        text: String,
        pick_id: Option<u32>,
    },
    /// Paper slab + eyelet with a title + body-rule decal.
    Ofuda {
        title: String,
        rule: String,
        pick_id: Option<u32>,
    },
    /// Carved bone tablet with a single engraved name and progress glow.
    YakuTablet {
        label: String,
        active: bool,
        hover: f32,
        progress: f32,
    },
    /// Lacquered wood action tablet with press/hover animation envelopes.
    WoodTablet {
        label: String,
        hover: f32,
        pressed: f32,
        disabled: bool,
    },

    // ── Props ────────────────────────────────────────────────────────
    /// Procedural shrine altar (Small / Big / Boss scale via `extents`).
    Shrine { glow: f32 },
    /// Colored relic placeholder box sitting in the dish.
    Relic { relic_id: RelicId, glow: f32 },
    /// 3D relic viewer (collection card, modal, tutorial).
    RelicShowcase { relic_id: RelicId, glow: f32 },
    /// Tile-pack box on the shop shelf.
    Pack {
        kind: TilePackKind,
        pick_id: Option<u32>,
    },
    /// Silken zodiac ribbon hanging from an anchor.
    ZodiacRibbon {
        kind: Option<crate::core::zodiac::ZodiacKind>,
    },
    /// Jade talisman tablet.
    Talisman {
        kind: crate::core::talisman::TalismanKind,
    },
    /// Stamped coin (extents encode `[radius*2, thickness, radius*2]`).
    Coin,
    /// Gold bar rendered as a Metal-material box.
    GoldBar,
    /// Standing book (Yaku Journal).
    Book { pick_id: Option<u32> },
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
    /// Free-standing dish (explicit size/position, not auto-sized from relics).
    Dish { pick_id: Option<u32> },
    /// Curio cabinet back-wall shadow box.
    CurioCabinet,
    /// Counter-end action prop (Leave / Restock) rendered as a labelled rectangle.
    ShopActionProp {
        label: String,
        pick_id: Option<u32>,
        disabled: bool,
    },
    /// Sell-return tray on the counter far-left end.
    SellTray { pick_id: Option<u32> },
    /// Overhead shop lamp — brass pole + conical shade (body mesh) plus a
    /// glass bulb.  The scene should also push a warm `PointLight` at the
    /// bulb position via `UiFrame::point_lights` (or `own_light`).
    ///
    /// `glow` drives the bulb brightness envelope \[0, 1\]; 1.0 = full lamp-on
    /// emission boost on the glass material.
    ShopLamp { glow: f32 },
    /// One hovering insect near the lamp.  The scene emits one `Bug` per bug
    /// per frame, with `slot` ∈ `0..MAX_BUG_SLOTS` identifying the instance
    /// buffer entry.  `rotation` orients the body so +X faces the orbit tangent.
    /// Body and wings share the same `pos` / `extents` / `rotation`.
    Bug { slot: usize },
    /// Ghost trail copy of a bug, rendered through the alpha-blended pipeline.
    /// `slot` ∈ `0..MAX_BUG_GHOST_SLOTS` indexes a separate instance pool;
    /// `alpha` ∈ [0, 1] scales the body/wing material's RGBA output.
    BugGhost { slot: usize, alpha: f32 },
    /// Material preview orb — a shared unit sphere drawn with a caller-supplied
    /// `MaterialParams`. Used only by the material viewer debug scene. The
    /// instance pool binds the 1×1 default albedo and relief textures so
    /// materials that sample heightmaps render as their base material with no
    /// displacement (i.e. every orb previews the shading model itself, not a
    /// per-asset heightmap).
    MaterialOrb { material: MaterialParams },
}

/// A single lit mesh placed in the world.
///
/// Replaces all individual `XxxPlacement` structs for objects rendered through
/// the `lit_mesh_pipeline`.  Scenes set `pos`, `extents`, and `rotation`
/// directly — use [`camera_facing_rotation`] when the face should track the
/// active camera.
#[derive(Clone, Debug)]
#[allow(dead_code)]
pub struct Object3d {
    /// Center position as `(pixel_x, pixel_y, lift)`.
    /// Mapped to world space by [`crate::render::world_space::pixel_to_world`].
    pub pos: [f32; 3],
    /// Full extents `(width, height, depth)` in world units.
    pub extents: [f32; 3],
    /// Rotation matrix. Build with [`glam::Mat4`] rotation constructors and
    /// compose with `*`. Use [`camera_facing_rotation`] for camera-facing panels.
    /// [`glam::Mat4::IDENTITY`] = no rotation.
    pub rotation: Rotation3d,
    /// Base tint (linear RGBA). `[1.0, 1.0, 1.0, 1.0]` = mesh default color.
    pub color: [f32; 4],
    /// Which mesh + material to render, plus mesh-specific payload.
    pub kind: Object3dKind,
    /// When `true`, this object participates in focus-ring and hover
    /// hit-testing (rendered rect is exposed via `proj.object3d_rects`).
    pub focusable: bool,
    /// When `true`, the scene's ambient lighting and shadow map apply.
    /// Set `false` for self-illuminated / unshaded objects.
    pub scene_shaded: bool,
    /// Optional point light emitted from this object's center this frame.
    pub own_light: Option<PointLight>,
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
    /// Canonical arrange-mode path for this object (e.g.
    /// `"shop.counter"`, `"gameplay.hand.strip"`). When set, the renderer
    /// uses this name both for `apply_arrange_override` lookups and for
    /// `last_debug_pickables` so click-to-select lands on the correct
    /// scene placement without per-kind hard-coded name tables.
    ///
    /// `None` = object is not arrangeable via the debug picker.
    pub arrange_name: Option<&'static str>,
}

/// One drawable element in a `UiFrame`.
///
/// The renderer walks `UiFrame.cmds` in order and dispatches each variant to
/// the appropriate pipeline. Contiguous runs of the same variant (e.g.
/// several `Quad`s in a row) are batched into a single instanced draw, which
/// is invisible to scenes and preserves ordering exactly.
#[allow(dead_code)]
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
    /// Procedural shooting-star cascade transition (fullscreen triangle, no data).
    /// Brightness driven by `UiFrame::transition_progress`.
    ShootingStarCascade,
    /// Procedural lacquered-wood table mesh (one per scene, drawn via
    /// `lit_mesh_pipeline`). Sized by the renderer from the current window.
    Table,
    /// 3D candle meshes for the gameplay scene. Each placement becomes one
    /// wax-body draw + one wick draw via the `lit_mesh_pipeline`. Limited to
    /// the renderer's pre-allocated candle slot pool (currently 7).
    CandleBatch(Vec<CandlePlacement>),
    /// 3D dish mesh sitting on the table — a wide low brass tray that holds
    /// the physical relic placeholders. The renderer reads the placement out
    /// of `RelicBatch` (the dish auto-sizes to enclose the row).
    Dish,
    /// Batch of physical relic placeholders sitting in the dish. Each
    /// placement is a colored axis-aligned box rendered via the
    /// `lit_mesh_pipeline`, instanced from the renderer's pre-allocated
    /// relic slot pool.
    RelicBatch(Vec<RelicPlacement>),
    /// Tile-pack boxes rendered on the shop shelf. Uses the same unit-box
    /// mesh and lit-mesh pipeline as relics, with pack art textures.
    PackBatch(Vec<PackPlacement>),
    /// Free-standing dish placement (alternative to `Dish` which auto-sizes
    /// from `RelicBatch`). Used by the shop scene to draw multiple dishes at
    /// fixed positions in the same frame.
    DishExplicit(DishExplicit),
    /// Single curio-cabinet (back-wall shadow box) placement. The shop scene
    /// uses one per frame; gameplay leaves this empty.
    #[allow(dead_code)]
    CurioCabinet(CurioCabinetPlacement),
    /// Batch of shrine meshes drawn via the lit-mesh pipeline, instanced
    /// from the renderer's pre-allocated shrine slot pool. The pick-blind
    /// scene uses one batch of three (Small / Big / Boss) per frame.
    ShrineBatch(Vec<ShrinePlacement>),
    /// Batch of zodiac/talisman ribbons drawn via the lit-mesh pipeline,
    /// instanced from the renderer's pre-allocated ribbon slot pool. Used by
    /// the shop scene for both the wall-pinned for-sale ribbons and the
    /// owned-consumable inventory fan.
    ZodiacBatch(Vec<ZodiacRibbonPlacement>),
    /// Batch of jade talisman tablets drawn via the lit-mesh pipeline,
    /// instanced from the renderer's pre-allocated talisman slot pool. Used
    /// by the shop scene for the for-sale talismans pinned in the curio
    /// cabinet next to the zodiac ribbons.
    TalismanBatch(Vec<TalismanPlacement>),
    /// Batch of physical gold coins drawn via the lit-mesh pipeline,
    /// instanced from the renderer's pre-allocated coin slot pool. Used by
    /// the shop scene to display the player's gold as a pile of coins in a
    /// dish.
    CoinBatch(Vec<CoinPlacement>),
    /// Gold bars rendered as unit-box meshes with Metal material. Used by
    /// the shop scene when the player has ≥100 gold.
    GoldBarBatch(Vec<GoldBarPlacement>),
    /// A standing book mesh (Yaku Journal). Single placement per frame.
    Book(BookPlacement),
    /// Fluid smoke overlay. Renderer owns the simulation state.
    FluidSmoke,
    /// Batch of showcase tiles with explicit 3D transforms — used for hand
    /// tiles, pack-opening celebrations, and any other 3D tile placement.
    ShowcaseTileBatch(Vec<ShowcaseTilePlacement>),
    /// Batch of 3D relic viewer placements for collection cards, tutorial
    /// panels, and reward reveals.
    RelicShowcaseBatch(Vec<RelicShowcasePlacement>),
    /// Generic 2D quad (panels, dimmers, borders, tooltip backgrounds…).
    Quad(GpuInstance),
    /// Procedural candle flame (additive blend, animated by globals.time).
    /// Instance `color.a` carries a per-flame phase offset in [0,1].
    Flame(GpuInstance),
    /// Rasterized text label.
    Text(TextLabel),
    /// Pre-loaded relic icon texture.
    #[allow(dead_code)]
    RelicIcon(RelicIcon),
    // ── Skeuomorphic gameplay HUD ──
    /// Hanging blind/score plaque (gameplay HUD). Single placement per cmd.
    Plaque(PlaquePlacement),
    /// Hanging boss-rule ofuda paper (gameplay HUD). Single placement per cmd.
    Ofuda(OfudaPlacement),
    /// Batch of bone yaku tablets sitting in a row below the hand.
    YakuTabletBatch(Vec<YakuTabletPlacement>),
    /// Batch of wood action tablets (sort suit / sort rank / play).
    WoodTabletBatch(Vec<WoodTabletPlacement>),
    /// The discard bowl.
    Bowl(BowlPlacement),
    /// The bronze "play hand" mirror.
    Mirror(MirrorPlacement),
    /// A counter fan — upright bone tally sticks, one fan per counter.
    /// Gameplay pushes two per frame: draws (jade tips, at the mirror) and
    /// discards (amber tips, at the river).
    TallyFan(TallyFanPlacement),
    /// The wall stack (facedown tiles at the back of the table).
    #[allow(dead_code)]
    WallStack(WallStackPlacement),
    /// Engraved bone scoring tokens that pop in during a cascade. Reuses
    /// the bone-tablet mesh; per-instance tint distinguishes chips vs mult.
    CascadeTokenBatch(Vec<CascadeTokenPlacement>),
    /// Physical scoring bones tumbling onto the play space during a cascade.
    /// Same bone-tablet mesh as the cascade tokens, with full 3D model
    /// matrices (gravity-driven world_z + euler tumble) instead of static
    /// HUD positioning.
    FallingBoneBatch(Vec<FallingBonePlacement>),
    /// Floating 3D extruded-glyph score popups. Each placement carries its
    /// own label string; the renderer lazily builds a per-string mesh on
    /// first use and reuses it on subsequent frames.
    ExtrudedGlyphBatch(Vec<ExtrudedGlyphPlacement>),
    /// Non-rendered hover region anchored at a screen rect that resolves
    /// to a glossary term. Lets scenes attach a tooltip to a 3D object
    /// (e.g. the coin pile, the wall stack) by giving its approximate
    /// 2D screen footprint and the glossary term to look up.
    GlossaryAnchor { rect: [f32; 4], term: &'static str },

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

    /// Active point lights this frame. Uploaded to the tile pipeline so the
    /// 3D hand-tile shader can apply candle / spot illumination.
    pub point_lights: Vec<PointLight>,
    /// How many of the leading entries in `point_lights` are candle lights
    /// (as opposed to hint lights, spot lights, etc.). The volumetric flame
    /// emission in the lightbake shader only fires for the first
    /// `candle_light_count` lights.
    pub candle_light_count: u32,
    /// Candle flame height in world units (derived from mm via `Layout::mm`).
    /// Passed to the volumetric lightbake shader so the analytic flame
    /// envelope is physically sized.
    pub flame_height_world: f32,
    /// Mouse cursor position in pixel coordinates, if the scene tracks one.
    /// The renderer projects this onto the table plane and feeds it into the
    /// volumetric smoke sim as a continuous wind impulse.
    pub cursor_pos: Option<(f32, f32)>,
    /// Discrete wind impulses to inject into the smoke sim this frame, on
    /// top of the per-cursor wind. Used by gameplay to "blow" smoke off the
    /// hand strip a few seconds after dealing.
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
}

impl UiFrame {
    pub fn new() -> Self {
        Self {
            cmds: Vec::new(),
            point_lights: Vec::new(),
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
        }
    }

    // ── Push helpers ────────────────────────────────────────────────────
    pub fn background(&mut self, bg: BackgroundId) {
        self.cmds.push(DrawCmd::Background(bg));
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
    pub fn shooting_star_cascade(&mut self) {
        self.cmds.push(DrawCmd::ShootingStarCascade);
    }
    pub fn fluid_smoke(&mut self) {
        self.cmds.push(DrawCmd::FluidSmoke);
    }
    pub fn table(&mut self) {
        self.cmds.push(DrawCmd::Table);
    }
    pub fn candles(&mut self, placements: Vec<CandlePlacement>) {
        self.cmds.push(DrawCmd::CandleBatch(placements));
    }
    pub fn pack_batch(&mut self, placements: Vec<PackPlacement>) {
        self.cmds.push(DrawCmd::PackBatch(placements));
    }
    pub fn relic_showcase_batch(&mut self, placements: Vec<RelicShowcasePlacement>) {
        self.cmds.push(DrawCmd::RelicShowcaseBatch(placements));
    }
    pub fn dish_explicit(&mut self, dish: DishExplicit) {
        self.cmds.push(DrawCmd::DishExplicit(dish));
    }
    pub fn zodiac_batch(&mut self, placements: Vec<ZodiacRibbonPlacement>) {
        self.cmds.push(DrawCmd::ZodiacBatch(placements));
    }
    pub fn talisman_batch(&mut self, placements: Vec<TalismanPlacement>) {
        self.cmds.push(DrawCmd::TalismanBatch(placements));
    }
    pub fn coin_batch(&mut self, placements: Vec<CoinPlacement>) {
        self.cmds.push(DrawCmd::CoinBatch(placements));
    }
    #[allow(dead_code)]
    pub fn gold_bar_batch(&mut self, placements: Vec<GoldBarPlacement>) {
        self.cmds.push(DrawCmd::GoldBarBatch(placements));
    }
    pub fn wood_tablet_batch(&mut self, placements: Vec<WoodTabletPlacement>) {
        self.cmds.push(DrawCmd::WoodTabletBatch(placements));
    }
    pub fn bowl(&mut self, p: BowlPlacement) {
        self.cmds.push(DrawCmd::Bowl(p));
    }
    pub fn mirror(&mut self, p: MirrorPlacement) {
        self.cmds.push(DrawCmd::Mirror(p));
    }
    pub fn tally_fan(&mut self, p: TallyFanPlacement) {
        self.cmds.push(DrawCmd::TallyFan(p));
    }
    #[allow(dead_code)]
    pub fn wall_stack(&mut self, p: WallStackPlacement) {
        self.cmds.push(DrawCmd::WallStack(p));
    }
    pub fn cascade_token_batch(&mut self, placements: Vec<CascadeTokenPlacement>) {
        self.cmds.push(DrawCmd::CascadeTokenBatch(placements));
    }
    pub fn extruded_glyph_batch(&mut self, placements: Vec<ExtrudedGlyphPlacement>) {
        self.cmds.push(DrawCmd::ExtrudedGlyphBatch(placements));
    }
    pub fn quad(&mut self, inst: GpuInstance) {
        self.cmds.push(DrawCmd::Quad(inst));
    }
    pub fn quads<I: IntoIterator<Item = GpuInstance>>(&mut self, iter: I) {
        self.cmds.extend(iter.into_iter().map(DrawCmd::Quad));
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
    #[allow(dead_code)]
    pub fn relic_icons<I: IntoIterator<Item = RelicIcon>>(&mut self, iter: I) {
        self.cmds.extend(iter.into_iter().map(DrawCmd::RelicIcon));
    }

    pub fn glossary_anchor(&mut self, rect: [f32; 4], term: &'static str) {
        self.cmds.push(DrawCmd::GlossaryAnchor { rect, term });
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

    /// Apply a global alpha multiplier to every queued cmd's color.
    /// Used by the main loop for scene transition fades.
    pub fn apply_alpha(&mut self, alpha: f32) {
        if alpha >= 1.0 {
            return;
        }
        for cmd in self.cmds.iter_mut() {
            match cmd {
                DrawCmd::Quad(inst) => inst.color[3] *= alpha,
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
                | DrawCmd::FluidSmoke
                | DrawCmd::Table
                | DrawCmd::CandleBatch(_)
                | DrawCmd::Dish
                | DrawCmd::RelicBatch(_)
                | DrawCmd::PackBatch(_)
                | DrawCmd::RelicShowcaseBatch(_)
                | DrawCmd::DishExplicit(_)
                | DrawCmd::CurioCabinet(_)
                | DrawCmd::ShrineBatch(_)
                | DrawCmd::ZodiacBatch(_)
                | DrawCmd::TalismanBatch(_)
                | DrawCmd::CoinBatch(_)
                | DrawCmd::RelicIcon(_)
                | DrawCmd::Plaque(_)
                | DrawCmd::Ofuda(_)
                | DrawCmd::YakuTabletBatch(_)
                | DrawCmd::WoodTabletBatch(_)
                | DrawCmd::Bowl(_)
                | DrawCmd::Mirror(_)
                | DrawCmd::TallyFan(_)
                | DrawCmd::WallStack(_)
                | DrawCmd::CascadeTokenBatch(_)
                | DrawCmd::FallingBoneBatch(_)
                | DrawCmd::ExtrudedGlyphBatch(_)
                | DrawCmd::ShowcaseTileBatch(_)
                | DrawCmd::GlossaryAnchor { .. }
                | DrawCmd::GoldBarBatch(_)
                | DrawCmd::Book(_)
                | DrawCmd::Object3d(_)
                | DrawCmd::Object3dBatch(_) => {}
            }
        }
    }
}

impl Default for UiFrame {
    fn default() -> Self {
        Self::new()
    }
}
