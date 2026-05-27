//! Lazy GPU pools and asset loads deferred past first frame to shorten sync boot.

use std::time::Instant;

use crate::lit_mesh::LitMeshInstance;
use crate::relic_pipeline::spawn_relic_loader;
use crate::wgpu_renderer::resources::load_metal_heightmap;
use crate::wgpu_renderer::{
    MAX_BOWL_SLOTS, MAX_EXTRUDED_GLYPH_SLOTS, MAX_MIRROR_SLOTS, MAX_ORDEAL_ICON_SLOTS,
    MAX_ORB_SLOTS, MAX_RELIC_SLOTS,
    MAX_TALLY_STICK_SLOTS, MAX_WALL_TILE_SLOTS, MAX_WOOD_TABLET_SLOTS, MAX_YAKU_TABLET_SLOTS,
    WgpuRenderer,
};

impl WgpuRenderer {
    fn make_lit_instance_pool(&self, count: usize) -> Vec<LitMeshInstance> {
        (0..count)
            .map(|_| {
                LitMeshInstance::new(
                    &self.device,
                    &self.lit_mesh_material_layout,
                    &self.shadow_caster_layout,
                    &self.lit_mesh_white_view,
                    &self.lit_mesh_relief_default_view,
                    &self.tile_sampler,
                )
            })
            .collect()
    }

    /// Start the background relic decode thread (first frame or first relic draw).
    pub(crate) fn ensure_relic_loader_started(&mut self) {
        if self.relic_load_finished || self.relic_rx.is_some() {
            return;
        }
        self.ensure_relic_instance_pool();
        self.relic_load_start = Some(Instant::now());
        self.relic_rx = Some(spawn_relic_loader());
    }

    pub(crate) fn ensure_relic_instance_pool(&mut self) {
        if self.relic_instances.len() >= MAX_RELIC_SLOTS {
            return;
        }
        let _scope = crate::startup_profile::scope("wgpu.defer.relic_instance_pool");
        self.relic_instances = self.make_lit_instance_pool(MAX_RELIC_SLOTS);
    }

    pub(crate) fn ensure_gameplay_hud_pools(&mut self) {
        if self.gameplay_hud_pools_ready {
            return;
        }
        let _scope = crate::startup_profile::scope("wgpu.defer.gameplay_hud_pools");
        self.yaku_tablet_instances = self.make_lit_instance_pool(MAX_YAKU_TABLET_SLOTS);
        self.wood_tablet_instances = self.make_lit_instance_pool(MAX_WOOD_TABLET_SLOTS);
        self.bowl_instances = self.make_lit_instance_pool(MAX_BOWL_SLOTS);
        let (_lit_mesh_mirror_height_tex, lit_mesh_mirror_height_view) =
            crate::wgpu_renderer::resources::load_mirror_heightmap(&self.device, &self.queue);
        self.mirror_instances = (0..MAX_MIRROR_SLOTS)
            .map(|_| {
                LitMeshInstance::new(
                    &self.device,
                    &self.lit_mesh_material_layout,
                    &self.shadow_caster_layout,
                    &lit_mesh_mirror_height_view,
                    &lit_mesh_mirror_height_view,
                    &self.tile_sampler,
                )
            })
            .collect();
        self.tally_stick_instances = self.make_lit_instance_pool(MAX_TALLY_STICK_SLOTS * 2);
        self.wall_tile_instances = self.make_lit_instance_pool(MAX_WALL_TILE_SLOTS);
        self.extruded_glyph_instances = self.make_lit_instance_pool(MAX_EXTRUDED_GLYPH_SLOTS);
        self.gameplay_hud_pools_ready = true;
    }

    pub(crate) fn ensure_talisman_textures(&mut self) {
        if self.talisman_textures_ready {
            return;
        }
        let _scope = crate::startup_profile::scope("wgpu.defer.talisman_textures");
        let talisman_height_paths = mahjuro_core::core::talisman::TalismanKind::heightmap_paths();
        for &(path, label) in talisman_height_paths {
            let (_tex, view) = load_metal_heightmap(&self.device, &self.queue, path, label);
            self.talisman_height_views.push(view);
        }
        let talisman_mask_paths = mahjuro_core::core::talisman::TalismanKind::mask_paths();
        for &(path, label) in talisman_mask_paths {
            let (_tex, view) = load_metal_heightmap(&self.device, &self.queue, path, label);
            self.talisman_mask_views.push(view);
        }
        let memorial_talisman_height_paths =
            mahjuro_core::core::memorial_talisman::MemorialTalismanKind::heightmap_paths();
        for &(path, label) in memorial_talisman_height_paths {
            let (_tex, view) = load_metal_heightmap(&self.device, &self.queue, path, label);
            self.memorial_talisman_height_views.push(view);
        }
        let memorial_talisman_mask_paths =
            mahjuro_core::core::memorial_talisman::MemorialTalismanKind::mask_paths();
        for &(path, label) in memorial_talisman_mask_paths {
            let (_tex, view) = load_metal_heightmap(&self.device, &self.queue, path, label);
            self.memorial_talisman_mask_views.push(view);
        }
        self.talisman_textures_ready = true;
    }

    pub(crate) fn ensure_orb_pool(&mut self) {
        if self.orb_instances.len() >= MAX_ORB_SLOTS {
            return;
        }
        let _scope = crate::startup_profile::scope("wgpu.defer.orb_pool");
        self.orb_instances = self.make_lit_instance_pool(MAX_ORB_SLOTS);
    }

    pub(crate) fn ensure_ordeal_icon_pool(&mut self) {
        if self.ordeal_icon_instances.len() >= MAX_ORDEAL_ICON_SLOTS {
            return;
        }
        let _scope = crate::startup_profile::scope("wgpu.defer.ordeal_icon_pool");
        self.ordeal_icon_instances = self.make_lit_instance_pool(MAX_ORDEAL_ICON_SLOTS);
    }
}
