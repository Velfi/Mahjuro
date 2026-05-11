use glam::Mat4;

/// Camera state captured at the end of a frame, for unprojecting cursor
/// positions into world-space rays in `pick_hand_tile`.
#[derive(Clone, Copy)]
pub(crate) struct PickCamera {
    pub(crate) inv_view_proj: Mat4,
    pub(crate) viewport_w: f32,
    pub(crate) viewport_h: f32,
}

/// Per-frame projected screen-space rects for every 3D element category.
/// Written during rendering, read one-frame-stale by scenes via
/// `DrawCtx` to anchor 2D overlays to visible 3D positions.
#[derive(Default)]
pub struct ProjectionCache {
    pub hand_rects: Vec<(usize, [f32; 4])>,
    pub relic_rects: Vec<[f32; 4]>,
    pub pack_rects: Vec<([f32; 4], Option<u32>)>,
    pub shrine_rects: Vec<[f32; 4]>,
    pub ribbon_rects: Vec<[f32; 4]>,
    pub talisman_rects: Vec<[f32; 4]>,
    pub plaque_rects: Vec<[f32; 4]>,
    pub peg_rects: [Option<[f32; 4]>; 2],
    pub yaku_tablet_rects: Vec<[f32; 4]>,
    pub wood_tablet_rects: Vec<[f32; 4]>,
    pub bowl_rect: Option<[f32; 4]>,
    pub mirror_rect: Option<[f32; 4]>,
    pub aux_dish_rects: Vec<(Option<u32>, [f32; 4])>,
    pub dora_plinth_rect: Option<[f32; 4]>,
}

/// Active arrange-mode override for the renderer. When set, the matching
/// object's model matrix is rebuilt each frame using these values instead of
/// the placement data from the scene's draw commands.
#[derive(Clone, Debug)]
pub struct DebugArrangeOverride {
    /// Name as registered in `last_debug_pickables` (e.g. `"BlindPlaque"`).
    pub name: String,
    /// Pixel nudge along X. Because world_x = pixel_x − w/2, this maps 1:1
    /// to world X regardless of window size.
    pub delta_px: f32,
    /// Pixel nudge along Y (positive = toward player). world_y = h/2 − pixel_y,
    /// so delta_py maps to −world_y, also 1:1 regardless of window size.
    pub delta_py: f32,
    /// World-Z nudge (lift above the felt). Window-size-independent.
    pub delta_lift: f32,
    /// Rotation delta around Z, degrees (additive on top of original).
    pub delta_rz_deg: f32,
    /// Rotation delta around X, degrees (additive on top of original).
    pub delta_rx_deg: f32,
    /// Rotation delta around Y, degrees (additive on top of original).
    pub delta_ry_deg: f32,
}
