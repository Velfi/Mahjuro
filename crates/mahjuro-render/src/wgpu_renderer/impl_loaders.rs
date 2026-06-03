use super::*;

impl WgpuRenderer {
    pub(crate) fn relic_mesh_for(&self, relic_id: RelicId) -> &LitMeshGpu {
        self.relic_meshes
            .get(&relic_id)
            .unwrap_or(&self.relic_box_mesh)
    }

    pub(crate) fn ordeal_icon_mesh_for(
        &self,
        kind: mahjuro_core::core::ordeal_kind::OrdealKind,
    ) -> &LitMeshGpu {
        self.ordeal_icon_meshes
            .get(&kind)
            .unwrap_or(&self.relic_box_mesh)
    }

    pub(crate) fn talisman_mesh_for_kind_idx(&self, kind_idx: u8) -> &LitMeshGpu {
        use crate::wgpu_renderer::constants::MEMORIAL_TALISMAN_TEXTURE_BASE;
        if kind_idx >= MEMORIAL_TALISMAN_TEXTURE_BASE {
            let idx = (kind_idx - MEMORIAL_TALISMAN_TEXTURE_BASE) as usize;
            if let Some(&kind) =
                mahjuro_core::core::memorial_talisman::MemorialTalismanKind::all().get(idx)
            {
                return self
                    .memorial_talisman_meshes
                    .get(&kind)
                    .unwrap_or(&self.talisman_mesh);
            }
        } else if let Some(&kind) =
            mahjuro_core::core::talisman::TalismanKind::all().get(kind_idx as usize)
        {
            return self
                .talisman_meshes
                .get(&kind)
                .unwrap_or(&self.talisman_mesh);
        }
        &self.talisman_mesh
    }

    /// Lazy-upload one boss atlas cell into [`Self::ordeal_icon_meshes`] /
    /// [`Self::ordeal_icon_textures`] (silhouette mesh via [`build_ordeal_icon_mesh_from_rgba`]).
    pub(crate) fn ensure_ordeal_icon_gpu(
        &mut self,
        kind: mahjuro_core::core::ordeal_kind::OrdealKind,
    ) {
        use crate::ordeal_icons::ordeal_icon_rgba;
        use crate::relic_dish::build_ordeal_icon_mesh_from_rgba;

        if self.ordeal_icon_meshes.contains_key(&kind)
            && self.ordeal_icon_textures.contains_key(&kind)
        {
            return;
        }
        let Some((rgba, w, h)) = ordeal_icon_rgba(kind) else {
            return;
        };
        let label = kind.atlas_slug();
        if !self.ordeal_icon_meshes.contains_key(&kind)
            && let Some(cpu) = build_ordeal_icon_mesh_from_rgba(&rgba, w, h, label)
        {
            self.ordeal_icon_meshes.insert(
                kind,
                LitMeshGpu::new(&self.device, &cpu, &format!("boss-icon-mesh-{label}")),
            );
        }
        if !self.ordeal_icon_textures.contains_key(&kind) {
            let (_, view) = upload_rgba_texture(
                &self.device,
                &self.queue,
                &format!("boss-icon-{label}"),
                &rgba,
                w,
                h,
            );
            let (_, relief_view) = upload_rgba_texture_linear(
                &self.device,
                &self.queue,
                &format!("boss-icon-relief-{label}"),
                &[128, 128, 128, 255],
                1,
                1,
            );
            self.ordeal_icon_textures
                .insert(kind, RelicTextureGpu { view, relief_view });
        }
    }

    /// Drain any decoded relic images from the background loader and upload them
    /// to the GPU.  Called once per frame; a no-op once all images are loaded.
    pub(crate) fn poll_relic_textures(&mut self) {
        self.ensure_relic_loader_started();
        let Some(ref rx) = self.relic_rx else { return };
        let mut finished = false;
        // Non-blocking drain: upload every image that's ready this frame.
        loop {
            match rx.try_recv() {
                Ok(img) => {
                    if let Some(cpu) = img.mesh_cpu {
                        let tris = crate::relic_dish::relic_mesh_pick_triangles(&cpu);
                        self.relic_tri_lists.insert(img.id, tris);
                        self.relic_meshes.insert(
                            img.id,
                            LitMeshGpu::new(
                                &self.device,
                                &cpu,
                                &format!("relic-mesh-{:?}", img.id),
                            ),
                        );
                    }
                    let t_upload = std::time::Instant::now();
                    let (_tex, view) = upload_rgba_texture(
                        &self.device,
                        &self.queue,
                        img.name,
                        &img.rgba,
                        img.width,
                        img.height,
                    );
                    let (_relief_tex, relief_view) = upload_rgba_texture_linear(
                        &self.device,
                        &self.queue,
                        &format!("{}-relief", img.name),
                        &img.relief_rgba,
                        img.relief_width,
                        img.relief_height,
                    );
                    self.relic_profile_upload_cpu += t_upload.elapsed();
                    self.relic_textures
                        .insert(img.id, RelicTextureGpu { view, relief_view });
                }
                Err(mpsc::TryRecvError::Empty) => break,
                Err(mpsc::TryRecvError::Disconnected) => {
                    finished = true;
                    break;
                }
            }
        }
        if finished {
            crate::startup_profile::record(
                "relic.texture_upload_main",
                self.relic_profile_upload_cpu,
            );
            if let Some(start) = self.relic_load_start.take() {
                crate::startup_profile::record("async.relic_load_wall", start.elapsed());
            }
            log::debug!(
                "all {} relic textures uploaded to GPU (spawn → last upload)",
                self.relic_textures.len(),
            );
            self.relic_rx = None; // drop the channel
            self.relic_load_finished = true;
            if !self.is_loading() {
                crate::startup_profile::note_async_boot_complete();
            }
        }
    }

    /// Drain any decoded background images from the loader and upload to GPU.
    pub(crate) fn poll_background_textures(&mut self) {
        let Some(ref rx) = self.background_rx else {
            return;
        };
        let mut finished = false;
        loop {
            match rx.try_recv() {
                Ok(img) => {
                    let label = format!("bg-{:?}", img.id);
                    let (_tex, view) = upload_rgba_texture(
                        &self.device,
                        &self.queue,
                        &label,
                        &img.rgba,
                        img.width,
                        img.height,
                    );
                    let bind_group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                        label: Some(&label),
                        layout: &self.text_bind_group_layout,
                        entries: &[
                            wgpu::BindGroupEntry {
                                binding: 0,
                                resource: wgpu::BindingResource::TextureView(&view),
                            },
                            wgpu::BindGroupEntry {
                                binding: 1,
                                resource: wgpu::BindingResource::Sampler(&self.tile_sampler),
                            },
                        ],
                    });
                    self.background_textures
                        .insert(img.id, BackgroundTextureGpu { bind_group });
                }
                Err(mpsc::TryRecvError::Empty) => break,
                Err(mpsc::TryRecvError::Disconnected) => {
                    finished = true;
                    break;
                }
            }
        }
        if finished {
            for &id in super::resources::ASYNC_LOADED_BACKGROUNDS {
                self.require_background_texture(id);
            }
            if let Some(start) = self.background_load_start.take() {
                crate::startup_profile::record("async.background_gpu_uploads", start.elapsed());
            }
            log::debug!(
                "all {} background textures uploaded to GPU (spawn → last upload)",
                self.background_textures.len(),
            );
            self.background_rx = None;
            if !self.is_loading() {
                crate::startup_profile::note_async_boot_complete();
            }
        }
    }

    fn require_background_texture(&mut self, id: BackgroundId) {
        if self.background_textures.contains_key(&id) {
            return;
        }
        panic!("background {id:?} missing after async load");
    }
}
