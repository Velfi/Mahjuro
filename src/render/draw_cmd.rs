//! Sane UI layering: a single ordered command list per frame.
//!
//! `UiFrame` carries one `Vec<DrawCmd>` plus the per-frame data the renderer
//! needs that isn't a draw call (hand tile mesh state, hit-test buttons, etc).
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
//! A few `DrawCmd` variants are markers (`HandTileBackdrop`, `HandTileFaces`,
//! `FluidSmoke`) that the renderer expands into pipeline-specific draws using
//! its own internal animation state. They obey the same ordering rule: a
//! marker draws *between* whatever was pushed before and after it. Scenes
//! place them in declarative order alongside ordinary cmds.

use crate::core::relic::RelicId;
use crate::core::tile::Tile;
use crate::core::tile_pack::TilePackKind;
use crate::render::candle_mesh::CandlePlacement;
use crate::render::wgpu_renderer::{GpuInstance, PointLight, RelicIcon, TextLabel};
use crate::scenes::{BackgroundId, ButtonDef};

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
    /// Up vector (typically `[0, 1, 0]`).
    pub up: [f32; 3],
    /// Vertical field of view in degrees.
    pub fovy_deg: f32,
}

/// One soft wind impulse to inject into the volumetric smoke sim this frame.
///
/// Coordinates use the same `(pixel_x, pixel_y)` convention as the rest of the
/// scene draw output: the renderer projects them onto the table plane (with
/// the optional `lift_px` height) using its `pixel_to_world` helper before
/// queueing the impulse on the fluid sim. Velocity is in world units (the same
/// space the existing candle plumes and cursor wind use), so a small upward +Z
/// push reads as a gentle breath flowing toward the back of the table.
#[derive(Clone, Copy, Debug)]
pub struct WindGust {
    /// `(pixel_x, pixel_y)` center of the gust in layout-pixel space.
    pub center_px: (f32, f32),
    /// Height above the table plane in world units.
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
/// `(pixel_x, pixel_y, world_y_lift)` — the renderer maps the pixel x/y onto
/// the table plane and uses world_y as the height above the wood.
#[derive(Clone, Copy, Debug)]
pub struct RelicPlacement {
    /// `(pixel_x, pixel_y, world_y_lift)` for the box's *base center*.
    pub world_pos: [f32; 3],
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

/// A tile-pack box on the shop shelf. Rendered using the same unit-box mesh
/// and lit-mesh pipeline as relics, with the pack's art texture wrapped on
/// every face. `rotation_x_deg` leans the box against the shelf back.
#[derive(Clone, Copy, Debug)]
pub struct PackPlacement {
    pub world_pos: [f32; 3],
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
#[derive(Clone, Copy, Debug)]
pub struct ZodiacRibbonPlacement {
    /// `(pixel_x, pixel_y, world_y)` for the ribbon's *top anchor*.
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
    /// Roll rotation about world Z around the anchor, in degrees. Used by
    /// the collection viewer's top-down camera for a turntable spin.
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
#[derive(Clone, Copy, Debug)]
pub struct TalismanPlacement {
    /// `(pixel_x, pixel_y, world_y)` for the tablet's *center*.
    pub center_pos: [f32; 3],
    /// Width × height × thickness in world units.
    pub extents: [f32; 3],
    /// Yaw rotation about world Y in degrees (0 = facing the camera).
    pub rotation_y_deg: f32,
    /// Pitch rotation about world X in degrees. 0 = upright. -90 lays
    /// the tablet face-up on the table (long axis flat, face normal
    /// rotated from +Z to +Y).
    pub rotation_x_deg: f32,
    /// Roll rotation about world Z in degrees. Used by the collection
    /// viewer's top-down camera for a turntable spin.
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
    /// `(pixel_x, pixel_y, world_y)` for the coin's center.
    pub world_pos: [f32; 3],
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
    /// `(pixel_x, pixel_y, world_y)` for the bar's base center.
    pub world_pos: [f32; 3],
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
    /// `(pixel_x, pixel_y, world_y)` for the book's base center.
    pub world_pos: [f32; 3],
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
    /// `(pixel_x, pixel_y, world_y)` for the dish base center.
    pub center_pos: [f32; 3],
    /// Full extents in world units (width × height × depth). The dish mesh
    /// itself is a wide low box; height ≈ rim height.
    pub extents: [f32; 3],
    /// Optional id used by `pick_shop_object` to recognize a hit on this
    /// dish (e.g. so the scene can route a click on the coin dish to "show
    /// gold count" rather than to a relic).
    pub pick_id: Option<u32>,
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
/// `top_text` / `bot_text` payload into a decal texture sampled by the
/// lit-mesh shader.
#[derive(Clone, Debug)]
pub struct PlaquePlacement {
    /// `(pixel_x, pixel_y, world_y)` for the plaque's *center*.
    pub center_pos: [f32; 3],
    /// Width × height × thickness in world units.
    pub extents: [f32; 3],
    /// Yaw rotation about world Y in degrees (0 = facing the camera).
    pub rotation_y_deg: f32,
    /// Display string for the large top line (blind name + ante + score/target).
    pub top_text: String,
    /// Display string for the smaller bottom line (gold · wind · shanten · ...).
    pub bot_text: String,
}

/// Hanging boss-rule ofuda paper.
#[allow(dead_code)]
#[derive(Clone, Debug)]
pub struct OfudaPlacement {
    /// `(pixel_x, pixel_y, world_y)` for the ofuda's *center*.
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
    /// `(pixel_x, pixel_y, world_y)` for the tablet's *base center*.
    pub world_pos: [f32; 3],
    /// Width × height × depth in world units.
    pub extents: [f32; 3],
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
    /// `(pixel_x, pixel_y, world_y)` for the tablet's *base center*.
    pub world_pos: [f32; 3],
    /// Width × height × depth in world units.
    pub extents: [f32; 3],
    /// Engraved label (Sort by Suit, Sort by Rank, Play, …).
    pub label: String,
    /// Press envelope in [0, 1] (1.0 = fully pressed in).
    pub pressed: f32,
    /// Hover lift envelope in [0, 1].
    pub hover: f32,
    /// True if the action is currently disabled (e.g. no tiles selected).
    pub disabled: bool,
}

/// The discard bowl. Click target = drop selected tile in.
#[derive(Clone, Copy, Debug)]
pub struct BowlPlacement {
    /// `(pixel_x, pixel_y, world_y)` for the bowl's *base center*.
    pub world_pos: [f32; 3],
    /// Width × height × depth in world units.
    pub extents: [f32; 3],
    /// Hover lift envelope in [0, 1].
    pub hover: f32,
}

/// The bronze mirror. Click target = play the selected hand. Visual
/// counterpart to the discard bowl, sharing the same flat-on-table
/// footprint and hover-lift convention.
#[derive(Clone, Copy, Debug)]
pub struct MirrorPlacement {
    /// `(pixel_x, pixel_y, world_y)` for the mirror's *base center*.
    pub world_pos: [f32; 3],
    /// Width × height × depth in world units.
    pub extents: [f32; 3],
    /// Hover lift envelope in [0, 1].
    pub hover: f32,
}

/// The plays/discards remaining peg block. The block itself is a single
/// wooden mesh; pegs (small cylinders) are emitted as separate coin-mesh
/// instances by the renderer based on the live counts.
#[derive(Clone, Copy, Debug)]
pub struct PegBlockPlacement {
    /// `(pixel_x, pixel_y, world_y)` for the block's *base center*.
    pub world_pos: [f32; 3],
    /// Width × height × depth in world units.
    pub extents: [f32; 3],
    /// Number of plays remaining (left peg row, capped at `plays_max`).
    pub plays_left: u32,
    /// Maximum number of play pegs (the row length).
    pub plays_max: u32,
    /// Number of discards remaining (right peg row, capped at `discards_max`).
    pub discards_left: u32,
    /// Maximum number of discard pegs (the row length).
    pub discards_max: u32,
}

/// Stack of facedown wall tiles at the back of the table.
#[derive(Clone, Copy, Debug)]
pub struct WallStackPlacement {
    /// `(pixel_x, pixel_y, world_y)` for the bottom-back-left of the stack.
    pub world_pos: [f32; 3],
    /// Tile slot dimensions in world units (per-tile width/height/depth).
    pub tile_extents: [f32; 3],
    /// Number of facedown tiles still in the wall.
    pub remaining: u32,
    /// Number of tiles per row in the stack (the pile fans wide).
    pub row_len: u32,
}

/// The dora indicator stand: a brass plinth + a single face-up tile resting
/// against the back board.
#[derive(Clone, Copy, Debug)]
pub struct DoraStandPlacement {
    /// `(pixel_x, pixel_y, world_y)` for the stand's *base center*.
    pub world_pos: [f32; 3],
    /// Width × height × depth in world units.
    pub extents: [f32; 3],
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
    /// `(pixel_x, pixel_y, world_y)` for the token's *center*.
    pub world_pos: [f32; 3],
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
    /// `(pixel_x, pixel_y, world_y_lift)` for the popup's *center*. Pixel
    /// x/y resolve to world xz via `pixel_to_world`; `world_y_lift` is the
    /// height above the table plane the popup currently floats at.
    pub world_pos: [f32; 3],
    /// Uniform world-units scale applied to the glyph mesh. The mesh itself
    /// is normalised to a height of 1.0 unit, so this directly sets the
    /// rendered character height in world space.
    pub scale: f32,
    /// Pitch rotation (radians) about world X. The popup defaults to lying
    /// flat on the table (face-up), so a small positive tilt rocks the top
    /// edge toward the camera for legibility.
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
}

/// One physical scoring bone tumbling onto the play space during a cascade.
/// Reuses the `bone_tablet_mesh` geometry; per-instance tint matches the
/// cascade token kind so chips bones read cool indigo and mult bones read
/// warm crimson, tying the falling pile back to the HUD readout it spawned
/// from. The simulation lives in `crate::render::falling_bones` and is
/// driven by the gameplay scene's cascade reveal events.
#[derive(Clone, Copy, Debug)]
/// One tile in a showcase display (pack-opening celebration, etc.).
/// The scene provides full per-tile 3D transforms each frame — the renderer
/// just draws what it's told, with no animation state of its own.
pub struct ShowcaseTilePlacement {
    /// The tile to display (identity determines the rasterized decal).
    pub tile: Tile,
    /// `(pixel_x, pixel_y, world_y_lift)` — same coordinate space as every
    /// other 3D placement. `pixel_to_world` maps px/py to world xz;
    /// `world_y_lift` is height above the table plane.
    pub center_pos: [f32; 3],
    /// Euler rotation `(rx, ry, rz)` in radians, applied after the standard
    /// tile basis orientation. `[0, 0, 0]` = default tilted-toward-camera.
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
    /// Passed to the tile shader via `base_color_factor.y`.
    pub selected: bool,
}

pub struct FallingBonePlacement {
    /// `(pixel_x, pixel_y, world_y)` for the bone's *center*. Pixel x/y
    /// resolve to world xz via `pixel_to_world`; `world_y` is the live
    /// height above the table plane that the simulation drives under
    /// gravity.
    pub world_pos: [f32; 3],
    /// Width × thickness × depth in world units.
    pub extents: [f32; 3],
    /// Tumble euler angles (rot_x, rot_y, rot_z) in radians.
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
    /// `(pixel_x, pixel_y, world_y)` for the shrine's *base center*.
    pub world_pos: [f32; 3],
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
    /// `(pixel_x, pixel_y, world_y)` for the cabinet's *center*.
    pub center_pos: [f32; 3],
    /// Full extents in world units (width × height × depth).
    pub extents: [f32; 3],
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
    /// Procedural shooting-star cascade transition (fullscreen triangle, no data).
    /// Brightness driven by `UiFrame::transition_progress`.
    ShootingStarCascade,
    /// Procedural lacquered-wood table mesh (one per scene, drawn via
    /// `lit_mesh_pipeline`). Sized by the renderer from the current window.
    Table,
    /// 3D candle meshes for the gameplay scene. Each placement becomes one
    /// wax-body draw + one wick draw via the `lit_mesh_pipeline`. Limited to
    /// the renderer's pre-allocated candle slot pool (currently 4).
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
    /// Light beams + hand tile body quads (drawn via `light_beam_pipeline` +
    /// `tile_quad_pipeline`). Renderer pulls hand state from `UiFrame`.
    HandTileBackdrop,
    /// Fluid smoke overlay. Renderer owns the simulation state.
    FluidSmoke,
    /// Hand tile face text + emoji indicators (text_pipeline). Splitting from
    /// the backdrop lets scenes draw 2D UI panels between hand tile bodies and
    /// their face labels — preserving the existing visual semantics where
    /// tile faces appear on top of overlay panels.
    HandTileFaces,
    /// Batch of showcase tiles with explicit 3D transforms. Used by the
    /// pack-opening celebration to display tiles at arbitrary positions
    /// (not constrained to the table plane like hand tiles).
    ShowcaseTileBatch(Vec<ShowcaseTilePlacement>),
    /// Generic 2D quad (panels, dimmers, borders, tooltip backgrounds…).
    Quad(GpuInstance),
    /// Procedural candle flame (additive blend, animated by globals.time).
    /// Instance `color.a` carries a per-flame phase offset in [0,1].
    Flame(GpuInstance),
    /// Rasterized text label.
    Text(TextLabel),
    /// Pre-loaded relic icon texture.
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
    /// The plays/discards remaining peg block.
    #[allow(dead_code)]
    PegBlock(PegBlockPlacement),
    /// The wall stack (facedown tiles at the back of the table).
    #[allow(dead_code)]
    WallStack(WallStackPlacement),
    /// The dora indicator stand.
    #[allow(dead_code)]
    DoraStand(DoraStandPlacement),
    /// Engraved bone scoring tokens that pop in during a cascade. Reuses
    /// the bone-tablet mesh; per-instance tint distinguishes chips vs mult.
    CascadeTokenBatch(Vec<CascadeTokenPlacement>),
    /// Physical scoring bones tumbling onto the play space during a cascade.
    /// Same bone-tablet mesh as the cascade tokens, with full 3D model
    /// matrices (gravity-driven world_y + euler tumble) instead of static
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
}

/// Everything a frame's draw needs: an ordered command list plus per-frame
/// state used by hand-tile markers, hit testing, and the main loop.
pub struct UiFrame {
    /// Drawn back-to-front in order. Push earlier = renders under.
    pub cmds: Vec<DrawCmd>,

    // ── Hand tile state (consumed by HandTileBackdrop / HandTileFaces) ──
    /// Logical hand tiles for `update_hand_tiles`.
    pub hand_tiles: Vec<Tile>,
    /// Screen-space slot rects parallel with `hand_tiles`.
    pub hand_slots: Vec<(f32, f32, f32, f32)>,
    /// Focused hand tile index.
    pub focus: usize,
    /// Selection bitmask parallel with `hand_tiles`.
    pub selected_tiles: Vec<bool>,
    /// Tile indices that should glow with a directional hint this frame.
    pub hint_indices: Vec<usize>,
    /// Tile indices that started departing this frame; consumed by
    /// `WgpuRenderer::depart_tiles` before `update_hand_tiles` removes them.
    pub departing_indices: Vec<usize>,
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
            hand_tiles: Vec::new(),
            hand_slots: Vec::new(),
            focus: 0,
            selected_tiles: Vec::new(),
            hint_indices: Vec::new(),
            departing_indices: Vec::new(),
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
    pub fn shooting_star_cascade(&mut self) {
        self.cmds.push(DrawCmd::ShootingStarCascade);
    }
    pub fn hand_tile_backdrop(&mut self) {
        self.cmds.push(DrawCmd::HandTileBackdrop);
    }
    pub fn hand_tile_faces(&mut self) {
        self.cmds.push(DrawCmd::HandTileFaces);
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
    pub fn dish(&mut self) {
        self.cmds.push(DrawCmd::Dish);
    }
    pub fn relic_batch(&mut self, placements: Vec<RelicPlacement>) {
        self.cmds.push(DrawCmd::RelicBatch(placements));
    }
    pub fn pack_batch(&mut self, placements: Vec<PackPlacement>) {
        self.cmds.push(DrawCmd::PackBatch(placements));
    }
    pub fn dish_explicit(&mut self, dish: DishExplicit) {
        self.cmds.push(DrawCmd::DishExplicit(dish));
    }
    pub fn curio_cabinet(&mut self, cabinet: CurioCabinetPlacement) {
        self.cmds.push(DrawCmd::CurioCabinet(cabinet));
    }
    pub fn shrine_batch(&mut self, placements: Vec<ShrinePlacement>) {
        self.cmds.push(DrawCmd::ShrineBatch(placements));
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
    pub fn gold_bar_batch(&mut self, placements: Vec<GoldBarPlacement>) {
        self.cmds.push(DrawCmd::GoldBarBatch(placements));
    }
    pub fn book(&mut self, placement: BookPlacement) {
        self.cmds.push(DrawCmd::Book(placement));
    }
    pub fn plaque(&mut self, p: PlaquePlacement) {
        self.cmds.push(DrawCmd::Plaque(p));
    }
    pub fn ofuda(&mut self, p: OfudaPlacement) {
        self.cmds.push(DrawCmd::Ofuda(p));
    }
    pub fn yaku_tablet_batch(&mut self, placements: Vec<YakuTabletPlacement>) {
        self.cmds.push(DrawCmd::YakuTabletBatch(placements));
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
    #[allow(dead_code)]
    pub fn peg_block(&mut self, p: PegBlockPlacement) {
        self.cmds.push(DrawCmd::PegBlock(p));
    }
    #[allow(dead_code)]
    pub fn wall_stack(&mut self, p: WallStackPlacement) {
        self.cmds.push(DrawCmd::WallStack(p));
    }
    #[allow(dead_code)]
    pub fn dora_stand(&mut self, p: DoraStandPlacement) {
        self.cmds.push(DrawCmd::DoraStand(p));
    }
    pub fn cascade_token_batch(&mut self, placements: Vec<CascadeTokenPlacement>) {
        self.cmds.push(DrawCmd::CascadeTokenBatch(placements));
    }
    pub fn falling_bone_batch(&mut self, placements: Vec<FallingBonePlacement>) {
        self.cmds.push(DrawCmd::FallingBoneBatch(placements));
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
    pub fn relic_icons<I: IntoIterator<Item = RelicIcon>>(&mut self, iter: I) {
        self.cmds.extend(iter.into_iter().map(DrawCmd::RelicIcon));
    }
    pub fn glossary_anchor(&mut self, rect: [f32; 4], term: &'static str) {
        self.cmds.push(DrawCmd::GlossaryAnchor { rect, term });
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
                | DrawCmd::ShootingStarCascade
                | DrawCmd::HandTileBackdrop
                | DrawCmd::HandTileFaces
                | DrawCmd::FluidSmoke
                | DrawCmd::Table
                | DrawCmd::CandleBatch(_)
                | DrawCmd::Dish
                | DrawCmd::RelicBatch(_)
                | DrawCmd::PackBatch(_)
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
                | DrawCmd::PegBlock(_)
                | DrawCmd::WallStack(_)
                | DrawCmd::DoraStand(_)
                | DrawCmd::CascadeTokenBatch(_)
                | DrawCmd::FallingBoneBatch(_)
                | DrawCmd::ExtrudedGlyphBatch(_)
                | DrawCmd::ShowcaseTileBatch(_)
                | DrawCmd::GlossaryAnchor { .. }
                | DrawCmd::GoldBarBatch(_)
                | DrawCmd::Book(_) => {}
            }
        }
    }
}

impl Default for UiFrame {
    fn default() -> Self {
        Self::new()
    }
}
