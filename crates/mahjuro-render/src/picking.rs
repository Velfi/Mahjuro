//! Cursor-ray picking for the `WgpuRenderer`.
//!
//! Extracted from `wgpu_renderer.rs` as a sibling module. Picking is read-only
//! over the one-frame-stale `last_*` snapshots the renderer captures at end of
//! frame, so it can live next to the renderer without any special coordination.
//!
//! All methods here live in a second `impl WgpuRenderer` block — callers see
//! them as regular `renderer.pick_*(...)` methods.

use glam::{Mat4, Vec3};

use crate::gameplay_glb::BTN_CASH_IN;
use crate::mirror_mesh::{MIRROR_LOCAL_CENTER_Y, MIRROR_LOCAL_HALF};
use crate::river_mesh::{
    RIVER_LOCAL_CENTER_Y as BOWL_LOCAL_CENTER_Y, RIVER_LOCAL_HALF as BOWL_LOCAL_HALF,
};
use crate::talisman_mesh::TALISMAN_LOCAL_HALF;
use crate::wgpu_renderer::{
    GameplayPick, LOCAL_X_EXTENT, LOCAL_Y_EXTENT, LOCAL_Z_EXTENT, MAIN_MENU_PICK_OPTIONS,
    MAIN_MENU_PICK_PLAY, MAIN_MENU_PICK_QUIT, MainMenuPick, ShopHit, WgpuRenderer,
};
use mahjuro_types::shop_pick::{PICK_LEAVE_PROP, PICK_RESTOCK_PROP, YAKU_JOURNAL_BOOK_PICK_ID};

fn gameplay_env_collision_node_to_hit(node_name: &str) -> Option<GameplayPick> {
    match node_name {
        BTN_CASH_IN => Some(GameplayPick::CashInButton),
        _ => None,
    }
}

fn shop_env_collision_node_to_hit(node_name: &str) -> Option<ShopHit> {
    match node_name {
        "journal_btn" => Some(ShopHit::Dish(YAKU_JOURNAL_BOOK_PICK_ID)),
        "restock_btn" => Some(ShopHit::Dish(PICK_RESTOCK_PROP)),
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

#[inline]
fn trimesh_hit_world_t_tris(
    tris: &[[Vec3; 3]],
    model: Mat4,
    world_origin: Vec3,
    world_dir: Vec3,
) -> Option<f32> {
    crate::raycast::ray_hit_trimesh(tris, model, world_origin, world_dir).map(|h| h.t)
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
    /// [`crate::table_transform::tile_mesh_local_to_world`].
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
        // Shop action props (Leave / Restock counter-end) — any
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
        let env_s = crate::room_glb::room_env_world_scale(
            cam.viewport_h,
            self.active_frame_env().height_scale,
        );
        let env_model = crate::room_glb::with_shop_glb_cpu(|opt| {
            opt.map(|cpu| {
                crate::room_glb::room_env_model_matrix_from_cpu(
                    cam.viewport_h,
                    self.active_frame_env().height_scale,
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
            .contains_key(&mahjuro_types::shop_pick::YAKU_JOURNAL_BOOK_PICK_ID);
        let has_guide_book = self
            .last_primitive_pick_models
            .contains_key(&mahjuro_types::shop_pick::GUIDE_BOOK_PICK_ID);
        let has_main_menu_pick = [
            MAIN_MENU_PICK_PLAY,
            MAIN_MENU_PICK_OPTIONS,
            MAIN_MENU_PICK_QUIT,
        ]
        .into_iter()
        .any(|id| self.last_primitive_pick_models.contains_key(&id));
        let has_cash_in_btn = self.last_gameplay_cash_in_button_visible
            && self
                .gameplay_env_collision_meshes
                .iter()
                .any(|m| m.node_name == BTN_CASH_IN);
        if self.last_yaku_tablet_models.is_empty()
            && self.last_wood_tablet_models.is_empty()
            && self.last_bowl_model.is_none()
            && self.last_mirror_model.is_none()
            && !has_journal_book
            && !has_guide_book
            && !has_main_menu_pick
            && !has_cash_in_btn
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
        // Wood action tablets — tutorial / legacy procedural tablets only.
        for (i, model) in self.last_wood_tablet_models.iter().enumerate() {
            if let Some(t) = slab_test(*model, 0.5, 0.5, 0.5, 0.0) {
                consider(GameplayPick::WoodTablet(i), t);
            }
        }
        // Authored gameplay env buttons (`btn_cash_in`, …).
        let gameplay_env_model = crate::gameplay_glb::with_gameplay_glb_cpu(|opt| {
            opt.map(|cpu| {
                crate::room_glb::room_env_model_matrix_from_cpu(
                    cam.viewport_h,
                    self.active_frame_env().height_scale,
                    cpu,
                )
            })
        })
        .unwrap_or_else(|| {
            Mat4::from_scale(Vec3::splat(crate::room_glb::room_env_world_scale(
                cam.viewport_h,
                self.active_frame_env().height_scale,
            )))
        });
        if self.last_gameplay_cash_in_button_visible {
            for mesh in &self.gameplay_env_collision_meshes {
                let Some(hit) = gameplay_env_collision_node_to_hit(mesh.node_name.as_str()) else {
                    continue;
                };
                if let Some(t_w) = trimesh_hit_world_t_tris(
                    &mesh.triangles,
                    gameplay_env_model,
                    world_origin,
                    world_dir,
                ) {
                    consider(hit, t_w);
                }
            }
        }
        // Yaku journal book — same unit-cube proxy as shop (`Object3dKind::Book`).
        if let Some(model) = self
            .last_primitive_pick_models
            .get(&mahjuro_types::shop_pick::YAKU_JOURNAL_BOOK_PICK_ID)
            && let Some(t) = slab_test(*model, 0.5, 0.5, 0.5, 0.0)
        {
            consider(GameplayPick::JournalBook, t);
        }
        // Guide book — same mesh/proxy as the yaku journal.
        if let Some(model) = self
            .last_primitive_pick_models
            .get(&mahjuro_types::shop_pick::GUIDE_BOOK_PICK_ID)
            && let Some(t) = slab_test(*model, 0.5, 0.5, 0.5, 0.0)
        {
            consider(GameplayPick::GuideBook, t);
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
    pub(super) fn relic_tris(
        &self,
        relic_id: mahjuro_core::core::relic::RelicId,
    ) -> &[[glam::Vec3; 3]] {
        self.relic_tri_lists
            .get(&relic_id)
            .map(|v| v.as_slice())
            .unwrap_or(&self.relic_box_tris)
    }
}
