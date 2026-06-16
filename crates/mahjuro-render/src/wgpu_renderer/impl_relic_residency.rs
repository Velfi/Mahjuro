//! Relic GPU residency: on-demand loads (Low memory) and LRU eviction.

use std::collections::HashSet;
use std::sync::mpsc;

use mahjuro_core::core::relic::RelicId;

use crate::gpu_types::{DecodedRelicImage, RelicGpuMeta, RelicTextureGpu};
use crate::lit_mesh::LitMeshGpu;
use crate::relic_gpu_residency::{total_relic_gpu_bytes, touch_relic_lru, trim_relic_lru};
use crate::wgpu_renderer::WgpuRenderer;

impl WgpuRenderer {
    pub(super) fn memory_budget_counters(&self) -> crate::gpu_memory_profile::MemoryBudgetCounters {
        crate::gpu_memory_profile::MemoryBudgetCounters {
            relic_gpu_bytes: total_relic_gpu_bytes(&self.relic_gpu_meta),
            decal_atlas_cpu_bytes: crate::gpu_memory_profile::decal_atlas_cpu_bytes(),
            music_pcm_bytes: crate::gpu_memory_profile::music_pcm_bytes(),
        }
    }

    pub(super) fn ensure_relic_ondemand_channel(&mut self) {
        if self.relic_ondemand_tx.is_some() {
            return;
        }
        let (tx, rx) = mpsc::channel();
        self.relic_ondemand_tx = Some(tx);
        self.relic_rx = Some(rx);
        // Persistent on-demand channel — not boot batch loading (see `is_loading`).
        self.relic_load_finished = true;
    }

    pub(super) fn request_relic_gpu(&mut self, id: RelicId) {
        if self.relic_textures.contains_key(&id) {
            touch_relic_lru(&mut self.relic_gpu_lru, id);
            return;
        }
        if self.relic_loading.contains(&id) {
            return;
        }
        if self.graphics_mode != mahjuro_gfx_types::GraphicsMode::LowMemory {
            self.ensure_relic_loader_started();
            return;
        }
        self.ensure_relic_ondemand_channel();
        self.relic_loading.insert(id);
        let tx = self
            .relic_ondemand_tx
            .as_ref()
            .expect("relic on-demand channel")
            .clone();
        crate::loader_pool::submit_relic_batch(move || {
            match crate::relic_bake::load_baked_relic_uncached(id) {
                Ok(msg) => {
                    let _ = tx.send(msg);
                }
                Err(e) => {
                    log::error!("relic on-demand load {id:?}: {e:#}");
                }
            }
        });
    }

    pub(super) fn touch_relic_gpu(&mut self, id: RelicId) {
        if self.relic_textures.contains_key(&id) {
            touch_relic_lru(&mut self.relic_gpu_lru, id);
        }
    }

    pub(super) fn trim_relic_gpu_residency(&mut self) {
        let Some(cap) = self.graphics_mode.max_relic_gpu_residents() else {
            return;
        };
        let protected = HashSet::new();
        let mut evict = Vec::new();
        trim_relic_lru(&mut self.relic_gpu_lru, cap, &protected, &mut |id| {
            evict.push(id);
        });
        for id in evict {
            self.evict_relic_gpu(id);
        }
    }

    pub(super) fn evict_relic_gpu(&mut self, id: RelicId) {
        self.relic_textures.remove(&id);
        self.relic_meshes.remove(&id);
        self.relic_gpu_meta.remove(&id);
        if let Some(i) = self.relic_gpu_lru.iter().position(|&r| r == id) {
            self.relic_gpu_lru.remove(i);
        }
        log::debug!("relic gpu evict: {id:?}");
    }

    pub(super) fn install_relic_gpu(&mut self, mut img: DecodedRelicImage) {
        let id = img.id;
        self.relic_loading.remove(&id);

        let mut mesh_bytes = 0usize;
        if let Some(cpu) = img.mesh_cpu.take() {
            let tris = crate::relic_dish::relic_mesh_pick_triangles(&cpu);
            self.relic_tri_lists.insert(id, tris);
            mesh_bytes = cpu.vertices.len() * std::mem::size_of::<crate::tile_glb::Vertex3dTex>()
                + cpu.indices.len() * std::mem::size_of::<u32>();
            self.relic_meshes.insert(
                id,
                LitMeshGpu::new(&self.device, &cpu, &format!("relic-mesh-{id:?}")),
            );
        }

        let t_upload = std::time::Instant::now();
        let (_albedo_tex, albedo_view, albedo_bytes) =
            crate::wgpu_renderer::resources::upload_relic_albedo_texture(
                &self.device,
                &self.queue,
                img.name,
                &img,
                self.bc7_textures_supported,
            );
        let (_relief_tex, relief_view, relief_bytes) =
            crate::wgpu_renderer::resources::upload_relic_relief_texture(
                &self.device,
                &self.queue,
                &format!("{}-relief", img.name),
                &img,
                self.bc7_textures_supported,
            );
        self.relic_profile_upload_cpu += t_upload.elapsed();
        self.relic_textures.insert(
            id,
            RelicTextureGpu {
                view: albedo_view,
                relief_view,
            },
        );
        self.relic_gpu_meta.insert(
            id,
            RelicGpuMeta {
                albedo_bytes,
                relief_bytes,
                mesh_bytes,
            },
        );
        touch_relic_lru(&mut self.relic_gpu_lru, id);
        self.trim_relic_gpu_residency();
    }
}
