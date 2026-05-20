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
    pub round_wind_plinth_rect: Option<[f32; 4]>,
}
