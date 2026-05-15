//! Cursor-ray picking for the `WgpuRenderer`.
//!
//! Extracted from `wgpu_renderer.rs` as a sibling module. Picking is read-only
//! over the one-frame-stale `last_*` snapshots the renderer captures at end of
//! frame, so it can live next to the renderer without any special coordination.
//!
//! All methods here live in a second `impl WgpuRenderer` block — callers see
//! them as regular `renderer.pick_*(...)` methods.

use glam::{Mat4, Vec3};

use crate::render::mirror_mesh::{MIRROR_LOCAL_CENTER_Y, MIRROR_LOCAL_HALF};
use crate::render::river_mesh::{
    RIVER_LOCAL_CENTER_Y as BOWL_LOCAL_CENTER_Y, RIVER_LOCAL_HALF as BOWL_LOCAL_HALF,
};
use crate::render::talisman_mesh::TALISMAN_LOCAL_HALF;
use crate::render::wgpu_renderer::{
    GameplayPick, LOCAL_X_EXTENT, LOCAL_Y_EXTENT, LOCAL_Z_EXTENT, MAIN_MENU_PICK_OPTIONS,
    MAIN_MENU_PICK_PLAY, MAIN_MENU_PICK_QUIT, MainMenuPick, ShopHit, WgpuRenderer,
};
use crate::scenes::journal_transition::YAKU_JOURNAL_BOOK_PICK_ID;
use crate::scenes::shop::pick_ids::{PICK_LEAVE_PROP, PICK_REROLL_PROP};

fn shop_env_collision_node_to_hit(node_name: &str) -> Option<ShopHit> {
    match node_name {
        "journal_btn" => Some(ShopHit::Dish(YAKU_JOURNAL_BOOK_PICK_ID)),
        "restock_btn" => Some(ShopHit::Dish(PICK_REROLL_PROP)),
        "exit_btn" => Some(ShopHit::Dish(PICK_LEAVE_PROP)),
        _ if node_name.starts_with("shop_spawn_relic_") => {
            let n = node_name.strip_prefix("shop_spawn_relic_")?;
            let slot: usize = n.parse().ok()?;
            Some(ShopHit::EnvSpawnSlot(slot))
        }
        _ if node_name.starts_with("shop_player_relic_") => {
            let n = node_name.strip_prefix("shop_player_relic_")?;
            let slot: usize = n.parse().ok()?;
            Some(ShopHit::EnvInvSlot(slot))
        }
        _ if node_name.starts_with("shop_player_consumable_") => {
            let n = node_name.strip_prefix("shop_player_consumable_")?;
            let ord: usize = n.parse().ok()?;
            Some(ShopHit::EnvConsumableOrd(ord))
        }
        _ => None,
    }
}

/// Möller–Trumbore against triangles in local mesh space; compares hits by world-space ray
/// distance along `world_dir` (consistent with mixed pick candidates).
fn trimesh_hit_world_t_tris(
    tris: &[[Vec3; 3]],
    model: Mat4,
    world_origin: Vec3,
    world_dir: Vec3,
) -> Option<f32> {
    let inv = model.inverse();
    let lo = inv.transform_point3(world_origin);
    let ld = inv.transform_vector3(world_dir);
    const EPS: f32 = 1e-7;
    let mut best_wt: Option<f32> = None;
    for [a, b, c] in tris {
        let e1 = *b - *a;
        let e2 = *c - *a;
        let p = ld.cross(e2);
        let det = e1.dot(p);
        if det.abs() < EPS {
            continue;
        }
        let inv_det = 1.0 / det;
        let s = lo - *a;
        let u = s.dot(p) * inv_det;
        if !(0.0..=1.0).contains(&u) {
            continue;
        }
        let q = s.cross(e1);
        let v = ld.dot(q) * inv_det;
        if v < 0.0 || u + v > 1.0 {
            continue;
        }
        let t_loc = e2.dot(q) * inv_det;
        if t_loc <= EPS {
            continue;
        }
        let local_hit = lo + ld * t_loc;
        let world_hit = model.transform_point3(local_hit);
        let wt = (world_hit - world_origin).dot(world_dir);
        if wt > EPS {
            best_wt = Some(match best_wt {
                Some(b) if b <= wt => b,
                _ => wt,
            });
        }
    }
    best_wt
}

impl WgpuRenderer {
    /// Cast a ray from the camera through the cursor (in physical pixels,
    /// matching the renderer's surface size) and return the index of the
    /// closest hand tile whose OBB the ray hits, if any.
    ///
    /// The intersection is done in each tile's *local* mesh space: the world
    /// ray is transformed by the inverse model matrix and tested against the
    /// normalized mesh's local AABB (centered at origin, half-extents
    /// `LOCAL_*_EXTENT / 2`). Model matrices include
    /// [`crate::render::table_transform::tile_mesh_local_to_world`].
    ///
    /// Uses the previous frame's snapshot, so this is consistent with what
    /// the user actually saw last frame (one-frame-stale, like the projected
    /// hand rects).
    pub fn pick_hand_tile(&self, cursor_x: f32, cursor_y: f32) -> Option<usize> {
        let cam = self.last_pick_camera.as_ref()?;
        if self.last_pick_models.is_empty() {
            return None;
        }

        // Cursor → NDC. wgpu uses z ∈ [0, 1] (matches Mat4::perspective_rh).
        let nx = (cursor_x / cam.viewport_w) * 2.0 - 1.0;
        let ny = 1.0 - (cursor_y / cam.viewport_h) * 2.0;

        // Unproject near and far points to world space.
        let near_clip = glam::Vec4::new(nx, ny, 0.0, 1.0);
        let far_clip = glam::Vec4::new(nx, ny, 1.0, 1.0);
        let near_w = cam.inv_view_proj * near_clip;
        let far_w = cam.inv_view_proj * far_clip;
        if near_w.w.abs() < 1e-6 || far_w.w.abs() < 1e-6 {
            return None;
        }
        let near = near_w.truncate() / near_w.w;
        let far = far_w.truncate() / far_w.w;
        let world_origin = near;
        let world_dir = (far - near).normalize_or_zero();
        if world_dir.length_squared() < 1e-6 {
            return None;
        }

        // Local AABB of the normalized tile mesh.
        let hx = LOCAL_X_EXTENT * 0.5;
        let hy = LOCAL_Y_EXTENT * 0.5;
        let hz = LOCAL_Z_EXTENT * 0.5;

        let mut best: Option<(usize, f32)> = None;
        for &(i, model) in &self.last_pick_models {
            let inv = model.inverse();
            let lo = inv.transform_point3(world_origin);
            let ld = inv.transform_vector3(world_dir);

            // Slab test against [-h, h] on each axis.
            let mut t_min = f32::NEG_INFINITY;
            let mut t_max = f32::INFINITY;
            let bounds = [(lo.x, ld.x, hx), (lo.y, ld.y, hy), (lo.z, ld.z, hz)];
            let mut hit = true;
            for (o, d, h) in bounds {
                if d.abs() < 1e-8 {
                    if o < -h || o > h {
                        hit = false;
                        break;
                    }
                } else {
                    let inv_d = 1.0 / d;
                    let mut t1 = (-h - o) * inv_d;
                    let mut t2 = (h - o) * inv_d;
                    if t1 > t2 {
                        std::mem::swap(&mut t1, &mut t2);
                    }
                    if t1 > t_min {
                        t_min = t1;
                    }
                    if t2 < t_max {
                        t_max = t2;
                    }
                    if t_min > t_max {
                        hit = false;
                        break;
                    }
                }
            }
            if !hit {
                continue;
            }
            // Ignore boxes that are entirely behind the camera.
            let t_enter = if t_min >= 0.0 { t_min } else { t_max };
            if t_enter < 0.0 {
                continue;
            }
            match best {
                Some((_, bt)) if t_enter >= bt => {}
                _ => best = Some((i, t_enter)),
            }
        }
        best.map(|(i, _)| i)
    }

    /// Cast a ray from the camera through the cursor and return the closest
    /// shop object hit. Uses the same one-frame-stale snapshot pattern as
    /// `pick_hand_tile`.
    pub fn pick_shop_object(&self, cursor_x: f32, cursor_y: f32) -> Option<ShopHit> {
        let cam = self.last_pick_camera.as_ref()?;
        if self.last_relic_models.is_empty()
            && self.last_ribbon_models.is_empty()
            && self.last_talisman_models.is_empty()
            && self.proj.aux_dish_rects.is_empty()
            && self.shop_env_collision_meshes.is_empty()
        {
            return None;
        }

        let nx = (cursor_x / cam.viewport_w) * 2.0 - 1.0;
        let ny = 1.0 - (cursor_y / cam.viewport_h) * 2.0;
        let near_clip = glam::Vec4::new(nx, ny, 0.0, 1.0);
        let far_clip = glam::Vec4::new(nx, ny, 1.0, 1.0);
        let near_w = cam.inv_view_proj * near_clip;
        let far_w = cam.inv_view_proj * far_clip;
        if near_w.w.abs() < 1e-6 || far_w.w.abs() < 1e-6 {
            return None;
        }
        let near = near_w.truncate() / near_w.w;
        let far = far_w.truncate() / far_w.w;
        let world_origin = near;
        let world_dir = (far - near).normalize_or_zero();
        if world_dir.length_squared() < 1e-6 {
            return None;
        }

        let slab_test = |model: glam::Mat4, hx: f32, hy: f32, hz: f32, oy: f32| -> Option<f32> {
            let inv = model.inverse();
            let lo = inv.transform_point3(world_origin);
            let ld = inv.transform_vector3(world_dir);
            let bounds = [
                (lo.x, ld.x, -hx, hx),
                (lo.y, ld.y, -hy + oy, hy + oy),
                (lo.z, ld.z, -hz, hz),
            ];
            let mut t_min = f32::NEG_INFINITY;
            let mut t_max = f32::INFINITY;
            for (o, d, lo_b, hi_b) in bounds {
                if d.abs() < 1e-8 {
                    if o < lo_b || o > hi_b {
                        return None;
                    }
                } else {
                    let inv_d = 1.0 / d;
                    let mut t1 = (lo_b - o) * inv_d;
                    let mut t2 = (hi_b - o) * inv_d;
                    if t1 > t2 {
                        std::mem::swap(&mut t1, &mut t2);
                    }
                    if t1 > t_min {
                        t_min = t1;
                    }
                    if t2 < t_max {
                        t_max = t2;
                    }
                    if t_min > t_max {
                        return None;
                    }
                }
            }
            let t_enter = if t_min >= 0.0 { t_min } else { t_max };
            if t_enter < 0.0 { None } else { Some(t_enter) }
        };

        let mut best: Option<(ShopHit, f32)> = None;
        let mut consider = |hit: ShopHit, t: f32| match best {
            Some((_, bt)) if t >= bt => {}
            _ => best = Some((hit, t)),
        };

        // Relics: per-triangle trimesh raycast against the silhouette-
        // derived mesh (falls back to the relic-box tris until the per-ID
        // mesh finishes loading). Picks the real outline, not a loose AABB.
        for (i, (model, rid)) in self.last_relic_models.iter().enumerate() {
            let tris = self.relic_tris(*rid);
            if let Some(t) = trimesh_hit_world_t_tris(tris, *model, world_origin, world_dir) {
                consider(ShopHit::Relic(i), t);
            }
        }
        // Ribbons — local bounds x ∈ [-0.5, 0.5], y ∈ [-0.5, 0.5], z ∈ [-0.05, 0.05].
        for (i, model) in self.last_ribbon_models.iter().enumerate() {
            if let Some(t) = slab_test(*model, 0.5, 0.5, 0.5, 0.0) {
                consider(ShopHit::Ribbon(i), t);
            }
        }
        // Talismans — local AABB from TALISMAN_LOCAL_HALF, centered at origin.
        for (i, model) in self.last_talisman_models.iter().enumerate() {
            if let Some(t) = slab_test(
                *model,
                TALISMAN_LOCAL_HALF[0],
                TALISMAN_LOCAL_HALF[1],
                TALISMAN_LOCAL_HALF[2],
                0.0,
            ) {
                consider(ShopHit::Talisman(i), t);
            }
        }
        // Shop action props (Leave / Reroll counter-end) — any
        // Primitive with a pick_id that isn't already covered by
        // `aux_dish_rects` falls through here. The shop scene maps
        // `ShopHit::Dish(pid)` to the right action downstream.
        for (pid, model) in &self.last_primitive_pick_models {
            if let Some(t) = slab_test(*model, 0.5, 0.5, 0.5, 0.0) {
                consider(ShopHit::Dish(*pid), t);
            }
        }
        // Auxiliary dishes (world-space AABB picks).
        for (i, (id, _rect)) in self.proj.aux_dish_rects.iter().enumerate() {
            let Some(pid) = id else { continue };
            let Some((center, half)) = self.last_aux_dish_aabbs.get(i) else {
                continue;
            };
            // World-space AABB slab test.
            let bounds = [
                (
                    world_origin.x,
                    world_dir.x,
                    center.x - half.x,
                    center.x + half.x,
                ),
                (
                    world_origin.y,
                    world_dir.y,
                    center.y - half.y,
                    center.y + half.y,
                ),
                (
                    world_origin.z,
                    world_dir.z,
                    center.z - half.z,
                    center.z + half.z,
                ),
            ];
            let mut t_min = f32::NEG_INFINITY;
            let mut t_max = f32::INFINITY;
            let mut hit = true;
            for (o, d, lo_b, hi_b) in bounds {
                if d.abs() < 1e-8 {
                    if o < lo_b || o > hi_b {
                        hit = false;
                        break;
                    }
                } else {
                    let inv_d = 1.0 / d;
                    let mut t1 = (lo_b - o) * inv_d;
                    let mut t2 = (hi_b - o) * inv_d;
                    if t1 > t2 {
                        std::mem::swap(&mut t1, &mut t2);
                    }
                    if t1 > t_min {
                        t_min = t1;
                    }
                    if t2 < t_max {
                        t_max = t2;
                    }
                    if t_min > t_max {
                        hit = false;
                        break;
                    }
                }
            }
            if !hit {
                continue;
            }
            let t_enter = if t_min >= 0.0 { t_min } else { t_max };
            if t_enter < 0.0 {
                continue;
            }
            consider(ShopHit::Dish(*pid), t_enter);
        }

        // Pack boxes — 2D projected rect hit test (packs are few, so a
        // simple screen-space check is sufficient).
        for (rect, pick_id) in &self.proj.pack_rects {
            let Some(pid) = pick_id else { continue };
            let [rx, ry, rw, rh] = *rect;
            if cursor_x >= rx && cursor_x <= rx + rw && cursor_y >= ry && cursor_y <= ry + rh {
                // Use a small t value so nearby 3D picks can still win.
                consider(ShopHit::TilePack(*pid), 0.5);
            }
        }

        // shop.glb invisible marker colliders (spawn slots, inventory anchors, …).
        let env_s = crate::render::shop_glb::shop_env_world_scale(
            cam.viewport_h,
            self.room_gltf_height_scale(),
        );
        let env_model = crate::render::shop_glb::with_shop_glb_cpu(|opt| {
            opt.map(|cpu| {
                crate::render::shop_glb::shop_env_model_matrix_from_cpu(
                    cam.viewport_h,
                    self.room_gltf_height_scale(),
                    cpu,
                )
            })
        })
        .unwrap_or_else(|| Mat4::from_scale(Vec3::splat(env_s)));
        for mesh in &self.shop_env_collision_meshes {
            let Some(hit) = shop_env_collision_node_to_hit(mesh.node_name.as_str()) else {
                continue;
            };
            if let Some(t_w) =
                trimesh_hit_world_t_tris(&mesh.triangles, env_model, world_origin, world_dir)
            {
                consider(hit, t_w);
            }
        }

        best.map(|(h, _)| h)
    }

    /// Cast a ray from the camera through the cursor and return the
    /// `pick_id` of the closest collection-scene relic the ray passes
    /// through. Per-triangle trimesh test against the silhouette-extruded
    /// relic mesh, so clicks that land in the empty space around a relic's
    /// shape don't register.
    pub fn pick_collection_object(&self, cursor_x: f32, cursor_y: f32) -> Option<u32> {
        let cam = self.last_pick_camera.as_ref()?;
        if self.last_pickable_relic_models.is_empty() {
            return None;
        }
        let nx = (cursor_x / cam.viewport_w) * 2.0 - 1.0;
        let ny = 1.0 - (cursor_y / cam.viewport_h) * 2.0;
        let near_clip = glam::Vec4::new(nx, ny, 0.0, 1.0);
        let far_clip = glam::Vec4::new(nx, ny, 1.0, 1.0);
        let near_w = cam.inv_view_proj * near_clip;
        let far_w = cam.inv_view_proj * far_clip;
        if near_w.w.abs() < 1e-6 || far_w.w.abs() < 1e-6 {
            return None;
        }
        let near = near_w.truncate() / near_w.w;
        let far = far_w.truncate() / far_w.w;
        let world_origin = near;
        let world_dir = (far - near).normalize_or_zero();
        if world_dir.length_squared() < 1e-6 {
            return None;
        }
        let mut best: Option<(u32, f32)> = None;
        for (pid, model, rid) in &self.last_pickable_relic_models {
            let tris = self.relic_tris(*rid);
            if let Some(t) = Self::trimesh_hit_t_tris(tris, *model, world_origin, world_dir) {
                match best {
                    Some((_, bt)) if t >= bt => {}
                    _ => best = Some((*pid, t)),
                }
            }
        }
        best.map(|(pid, _)| pid)
    }

    /// Cast a ray from the camera through the cursor and return the closest
    /// gameplay-scene object hit (yaku tablet, wood action tablet, or
    /// discard bowl). One-frame-stale snapshot pattern, mirroring
    /// `pick_hand_tile` and `pick_shop_object`. The per-class local AABBs
    /// are precomputed mesh constants — there is no per-frame screen-space
    /// projection in the hit-test path.
    pub fn pick_gameplay_object(&self, cursor_x: f32, cursor_y: f32) -> Option<GameplayPick> {
        let cam = self.last_pick_camera.as_ref()?;
        let has_journal_book = self
            .last_primitive_pick_models
            .contains_key(&crate::scenes::journal_transition::YAKU_JOURNAL_BOOK_PICK_ID);
        let has_main_menu_pick = [
            MAIN_MENU_PICK_PLAY,
            MAIN_MENU_PICK_OPTIONS,
            MAIN_MENU_PICK_QUIT,
        ]
        .into_iter()
        .any(|id| self.last_primitive_pick_models.contains_key(&id));
        if self.last_yaku_tablet_models.is_empty()
            && self.last_wood_tablet_models.is_empty()
            && self.last_bowl_model.is_none()
            && self.last_mirror_model.is_none()
            && !has_journal_book
            && !has_main_menu_pick
        {
            return None;
        }
        let nx = (cursor_x / cam.viewport_w) * 2.0 - 1.0;
        let ny = 1.0 - (cursor_y / cam.viewport_h) * 2.0;
        let near_clip = glam::Vec4::new(nx, ny, 0.0, 1.0);
        let far_clip = glam::Vec4::new(nx, ny, 1.0, 1.0);
        let near_w = cam.inv_view_proj * near_clip;
        let far_w = cam.inv_view_proj * far_clip;
        if near_w.w.abs() < 1e-6 || far_w.w.abs() < 1e-6 {
            return None;
        }
        let near = near_w.truncate() / near_w.w;
        let far = far_w.truncate() / far_w.w;
        let world_origin = near;
        let world_dir = (far - near).normalize_or_zero();
        if world_dir.length_squared() < 1e-6 {
            return None;
        }
        // Local-space slab test against an AABB centered at `(0, oy, 0)` with
        // half-extents `(hx, hy, hz)`. Returns the entry distance along the
        // world ray when the ray hits the box.
        let slab_test = |model: glam::Mat4, hx: f32, hy: f32, hz: f32, oy: f32| -> Option<f32> {
            let inv = model.inverse();
            let lo = inv.transform_point3(world_origin);
            let ld = inv.transform_vector3(world_dir);
            let bounds = [
                (lo.x, ld.x, -hx, hx),
                (lo.y, ld.y, -hy + oy, hy + oy),
                (lo.z, ld.z, -hz, hz),
            ];
            let mut t_min = f32::NEG_INFINITY;
            let mut t_max = f32::INFINITY;
            for (o, d, lo_b, hi_b) in bounds {
                if d.abs() < 1e-8 {
                    if o < lo_b || o > hi_b {
                        return None;
                    }
                } else {
                    let inv_d = 1.0 / d;
                    let mut t1 = (lo_b - o) * inv_d;
                    let mut t2 = (hi_b - o) * inv_d;
                    if t1 > t2 {
                        std::mem::swap(&mut t1, &mut t2);
                    }
                    if t1 > t_min {
                        t_min = t1;
                    }
                    if t2 < t_max {
                        t_max = t2;
                    }
                    if t_min > t_max {
                        return None;
                    }
                }
            }
            let t_enter = if t_min >= 0.0 { t_min } else { t_max };
            if t_enter < 0.0 { None } else { Some(t_enter) }
        };

        let mut best: Option<(GameplayPick, f32)> = None;
        let mut consider = |hit: GameplayPick, t: f32| match best {
            Some((_, bt)) if t >= bt => {}
            _ => best = Some((hit, t)),
        };

        // Yaku tablets — unit cube `[-0.5, 0.5]^3` (push_box convention).
        for (i, model) in self.last_yaku_tablet_models.iter().enumerate() {
            if let Some(t) = slab_test(*model, 0.5, 0.5, 0.5, 0.0) {
                consider(GameplayPick::YakuTablet(i), t);
            }
        }
        // Wood action tablets — same unit cube as the yaku tablets.
        for (i, model) in self.last_wood_tablet_models.iter().enumerate() {
            if let Some(t) = slab_test(*model, 0.5, 0.5, 0.5, 0.0) {
                consider(GameplayPick::WoodTablet(i), t);
            }
        }
        // Yaku Journal book — same unit-cube proxy as shop (`Object3dKind::Book`).
        if let Some(model) = self
            .last_primitive_pick_models
            .get(&crate::scenes::journal_transition::YAKU_JOURNAL_BOOK_PICK_ID)
            && let Some(t) = slab_test(*model, 0.5, 0.5, 0.5, 0.0)
        {
            consider(GameplayPick::JournalBook, t);
        }
        // Discard bowl — tighter local AABB from the bowl mesh constants.
        if let Some(model) = self.last_bowl_model.as_ref()
            && let Some(t) = slab_test(
                *model,
                BOWL_LOCAL_HALF[0],
                BOWL_LOCAL_HALF[1],
                BOWL_LOCAL_HALF[2],
                BOWL_LOCAL_CENTER_Y,
            )
        {
            consider(GameplayPick::DiscardBowl, t);
        }
        // Bronze mirror — flat disc local AABB from the mirror mesh constants.
        if let Some(model) = self.last_mirror_model.as_ref()
            && let Some(t) = slab_test(
                *model,
                MIRROR_LOCAL_HALF[0],
                MIRROR_LOCAL_HALF[1],
                MIRROR_LOCAL_HALF[2],
                MIRROR_LOCAL_CENTER_Y,
            )
        {
            consider(GameplayPick::BronzeMirror, t);
        }
        for &pid in &[
            MAIN_MENU_PICK_PLAY,
            MAIN_MENU_PICK_OPTIONS,
            MAIN_MENU_PICK_QUIT,
        ] {
            if let Some(model) = self.last_primitive_pick_models.get(&pid)
                && let Some(t) = slab_test(*model, 0.5, 0.5, 0.5, 0.0)
                && let Some(m) = MainMenuPick::from_pick_id(pid)
            {
                consider(GameplayPick::MainMenu(m), t);
            }
        }

        best.map(|(h, _)| h)
    }

    /// Resolve the cached triangle list for a relic. Falls back to the
    /// fallback box mesh when the per-relic silhouette mesh hasn't finished
    /// loading yet (the first few frames after the relic texture shows up).
    pub(super) fn relic_tris(&self, relic_id: crate::core::relic::RelicId) -> &[[glam::Vec3; 3]] {
        self.relic_tri_lists
            .get(&relic_id)
            .map(|v| v.as_slice())
            .unwrap_or(&self.relic_box_tris)
    }

    /// Möller–Trumbore ray vs. triangle list, with tris supplied directly.
    /// Used for relics where the mesh varies per-ID.
    pub(super) fn trimesh_hit_t_tris(
        tris: &[[glam::Vec3; 3]],
        model: glam::Mat4,
        world_origin: glam::Vec3,
        world_dir: glam::Vec3,
    ) -> Option<f32> {
        let inv = model.inverse();
        let lo = inv.transform_point3(world_origin);
        let ld = inv.transform_vector3(world_dir);
        const EPS: f32 = 1e-7;
        let mut best: Option<f32> = None;
        for [a, b, c] in tris {
            let e1 = *b - *a;
            let e2 = *c - *a;
            let p = ld.cross(e2);
            let det = e1.dot(p);
            if det.abs() < EPS {
                continue;
            }
            let inv_det = 1.0 / det;
            let s = lo - *a;
            let u = s.dot(p) * inv_det;
            if !(0.0..=1.0).contains(&u) {
                continue;
            }
            let q = s.cross(e1);
            let v = ld.dot(q) * inv_det;
            if v < 0.0 || u + v > 1.0 {
                continue;
            }
            let t = e2.dot(q) * inv_det;
            if t > EPS {
                best = Some(match best {
                    Some(bt) if bt <= t => bt,
                    _ => t,
                });
            }
        }
        best
    }

    /// Debug "what is the cursor over?" picker. Walks
    /// `last_debug_pickables` (populated as the renderer processes the
    /// frame's draw cmds) and returns the closest hit's name. Hand tiles
    /// are checked separately because they have their own pick path.
    /// Returns `None` if nothing was hit.
    pub fn pick_debug_object(&self, cursor_x: f32, cursor_y: f32) -> Option<String> {
        // Hand tiles first — they have their own dedicated picker that
        // already handles per-tile OBBs.
        if let Some(idx) = self.pick_hand_tile(cursor_x, cursor_y) {
            return Some(format!("gameplay.hand.tile[{}]", idx));
        }

        let cam = self.last_pick_camera.as_ref()?;

        let mut best: Option<(&str, f32)> = None;

        if !self.last_debug_pickables.is_empty() {
            let nx = (cursor_x / cam.viewport_w) * 2.0 - 1.0;
            let ny = 1.0 - (cursor_y / cam.viewport_h) * 2.0;
            let near_clip = glam::Vec4::new(nx, ny, 0.0, 1.0);
            let far_clip = glam::Vec4::new(nx, ny, 1.0, 1.0);
            let near_w = cam.inv_view_proj * near_clip;
            let far_w = cam.inv_view_proj * far_clip;
            if near_w.w.abs() > 1e-6 && far_w.w.abs() > 1e-6 {
                let near = near_w.truncate() / near_w.w;
                let far = far_w.truncate() / far_w.w;
                let world_origin = near;
                let world_dir = (far - near).normalize_or_zero();
                if world_dir.length_squared() >= 1e-6 {
                    let slab_test = |model: glam::Mat4, half: glam::Vec3, oy: f32| -> Option<f32> {
                        let inv = model.inverse();
                        let lo = inv.transform_point3(world_origin);
                        let ld = inv.transform_vector3(world_dir);
                        let bounds = [
                            (lo.x, ld.x, -half.x, half.x),
                            (lo.y, ld.y, -half.y + oy, half.y + oy),
                            (lo.z, ld.z, -half.z, half.z),
                        ];
                        let mut t_min = f32::NEG_INFINITY;
                        let mut t_max = f32::INFINITY;
                        for (o, d, lo_b, hi_b) in bounds {
                            if d.abs() < 1e-8 {
                                if o < lo_b || o > hi_b {
                                    return None;
                                }
                            } else {
                                let inv_d = 1.0 / d;
                                let mut t1 = (lo_b - o) * inv_d;
                                let mut t2 = (hi_b - o) * inv_d;
                                if t1 > t2 {
                                    std::mem::swap(&mut t1, &mut t2);
                                }
                                if t1 > t_min {
                                    t_min = t1;
                                }
                                if t2 < t_max {
                                    t_max = t2;
                                }
                                if t_min > t_max {
                                    return None;
                                }
                            }
                        }
                        let t_enter = if t_min >= 0.0 { t_min } else { t_max };
                        if t_enter < 0.0 { None } else { Some(t_enter) }
                    };

                    for (name, model, half, oy) in &self.last_debug_pickables {
                        if let Some(t) = slab_test(*model, *half, *oy) {
                            match best {
                                Some((_, bt)) if t > bt => {}
                                _ => best = Some((name.as_str(), t)),
                            }
                        }
                    }
                }
            }
        }

        if let Some((n, _)) = best {
            return Some(n.to_string());
        }

        None
    }

    /// Raycast the cursor ray against registered pickables and return the
    /// world-space hit point of the nearest intersection. Used by arrange
    /// mode's click-to-move so teleport targets land on actual geometry.
    pub fn pick_debug_world_point(&self, cursor_x: f32, cursor_y: f32) -> Option<glam::Vec3> {
        let cam = self.last_pick_camera.as_ref()?;
        if self.last_debug_pickables.is_empty() {
            return None;
        }
        let ndc_x = (cursor_x / cam.viewport_w) * 2.0 - 1.0;
        let ndc_y = 1.0 - (cursor_y / cam.viewport_h) * 2.0;
        let near_clip = glam::Vec4::new(ndc_x, ndc_y, 0.0, 1.0);
        let far_clip = glam::Vec4::new(ndc_x, ndc_y, 1.0, 1.0);
        let near_w = cam.inv_view_proj * near_clip;
        let far_w = cam.inv_view_proj * far_clip;
        if near_w.w.abs() < 1e-6 || far_w.w.abs() < 1e-6 {
            return None;
        }
        let near = near_w.truncate() / near_w.w;
        let far = far_w.truncate() / far_w.w;
        let world_origin = near;
        let world_dir = (far - near).normalize_or_zero();
        if world_dir.length_squared() < 1e-6 {
            return None;
        }

        let slab_test = |model: glam::Mat4, half: glam::Vec3, oy: f32| -> Option<f32> {
            let inv = model.inverse();
            let lo = inv.transform_point3(world_origin);
            let ld = inv.transform_vector3(world_dir);
            let bounds = [
                (lo.x, ld.x, -half.x, half.x),
                (lo.y, ld.y, -half.y + oy, half.y + oy),
                (lo.z, ld.z, -half.z, half.z),
            ];
            let mut t_min = f32::NEG_INFINITY;
            let mut t_max = f32::INFINITY;
            for (o, d, lo_b, hi_b) in bounds {
                if d.abs() < 1e-8 {
                    if o < lo_b || o > hi_b {
                        return None;
                    }
                } else {
                    let inv_d = 1.0 / d;
                    let mut t1 = (lo_b - o) * inv_d;
                    let mut t2 = (hi_b - o) * inv_d;
                    if t1 > t2 {
                        std::mem::swap(&mut t1, &mut t2);
                    }
                    if t1 > t_min {
                        t_min = t1;
                    }
                    if t2 < t_max {
                        t_max = t2;
                    }
                    if t_min > t_max {
                        return None;
                    }
                }
            }
            let t_enter = if t_min >= 0.0 { t_min } else { t_max };
            if t_enter < 0.0 { None } else { Some(t_enter) }
        };

        let mut best_t: Option<f32> = None;
        for (_n, model, half, oy) in &self.last_debug_pickables {
            if let Some(t) = slab_test(*model, *half, *oy) {
                match best_t {
                    Some(bt) if t > bt => {}
                    _ => best_t = Some(t),
                }
            }
        }
        best_t.map(|t| world_origin + world_dir * t)
    }

    /// position and rotation from the object's current transform.
    /// Hand-tile hits return `None` for the model matrix (they don't have a
    /// single rigid placement matrix available here).
    pub fn pick_debug_object_with_model(
        &self,
        cursor_x: f32,
        cursor_y: f32,
    ) -> Option<(String, Option<Mat4>)> {
        // Hand tiles: clicking any tile selects the whole strip as
        // `gameplay.hand.strip` so arrange mode can move/rotate all tiles
        // together as a group.
        if self.pick_hand_tile(cursor_x, cursor_y).is_some() {
            let target = "gameplay.hand.strip";
            let model = self
                .last_debug_pickables
                .iter()
                .find(|(n, _, _, _)| n == target)
                .map(|(_, m, _, _)| *m);
            return Some((target.to_string(), model));
        }

        let cam = self.last_pick_camera.as_ref()?;

        let mut best: Option<(&str, f32, Mat4)> = None;

        if !self.last_debug_pickables.is_empty() {
            let nx = (cursor_x / cam.viewport_w) * 2.0 - 1.0;
            let ny = 1.0 - (cursor_y / cam.viewport_h) * 2.0;
            let near_clip = glam::Vec4::new(nx, ny, 0.0, 1.0);
            let far_clip = glam::Vec4::new(nx, ny, 1.0, 1.0);
            let near_w = cam.inv_view_proj * near_clip;
            let far_w = cam.inv_view_proj * far_clip;
            if near_w.w.abs() > 1e-6 && far_w.w.abs() > 1e-6 {
                let near = near_w.truncate() / near_w.w;
                let far = far_w.truncate() / far_w.w;
                let world_origin = near;
                let world_dir = (far - near).normalize_or_zero();
                if world_dir.length_squared() >= 1e-6 {
                    let slab_test = |model: glam::Mat4, half: glam::Vec3, oy: f32| -> Option<f32> {
                        let inv = model.inverse();
                        let lo = inv.transform_point3(world_origin);
                        let ld = inv.transform_vector3(world_dir);
                        let bounds = [
                            (lo.x, ld.x, -half.x, half.x),
                            (lo.y, ld.y, -half.y + oy, half.y + oy),
                            (lo.z, ld.z, -half.z, half.z),
                        ];
                        let mut t_min = f32::NEG_INFINITY;
                        let mut t_max = f32::INFINITY;
                        for (o, d, lo_b, hi_b) in bounds {
                            if d.abs() < 1e-8 {
                                if o < lo_b || o > hi_b {
                                    return None;
                                }
                            } else {
                                let inv_d = 1.0 / d;
                                let mut t1 = (lo_b - o) * inv_d;
                                let mut t2 = (hi_b - o) * inv_d;
                                if t1 > t2 {
                                    std::mem::swap(&mut t1, &mut t2);
                                }
                                if t1 > t_min {
                                    t_min = t1;
                                }
                                if t2 < t_max {
                                    t_max = t2;
                                }
                                if t_min > t_max {
                                    return None;
                                }
                            }
                        }
                        let t_enter = if t_min >= 0.0 { t_min } else { t_max };
                        if t_enter < 0.0 { None } else { Some(t_enter) }
                    };

                    for (name, model, half, oy) in &self.last_debug_pickables {
                        if let Some(t) = slab_test(*model, *half, *oy) {
                            match best {
                                Some((_, bt, _)) if t > bt => {}
                                _ => best = Some((name.as_str(), t, *model)),
                            }
                        }
                    }
                }
            }
        }

        if let Some((n, _, m)) = best {
            return Some((n.to_string(), Some(m)));
        }

        None
    }
}
