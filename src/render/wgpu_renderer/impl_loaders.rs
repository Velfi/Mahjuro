use super::*;

impl WgpuRenderer {
    pub(crate) fn relic_mesh_for(&self, relic_id: RelicId) -> &LitMeshGpu {
        self.relic_meshes
            .get(&relic_id)
            .unwrap_or(&self.relic_box_mesh)
    }

    pub(crate) fn boss_icon_mesh_for(&self, kind: crate::core::boss::BossKind) -> &LitMeshGpu {
        self.boss_icon_meshes
            .get(&kind)
            .unwrap_or(&self.relic_box_mesh)
    }

    /// Lazy-upload one boss atlas cell into [`Self::boss_icon_meshes`] /
    /// [`Self::boss_icon_textures`] (silhouette mesh via [`build_boss_icon_mesh_from_rgba`]).
    pub(crate) fn ensure_boss_icon_gpu(&mut self, kind: crate::core::boss::BossKind) {
        use crate::render::relic_dish::build_boss_icon_mesh_from_rgba;
        use crate::ui::boss_icons::boss_icon_rgba;

        if self.boss_icon_meshes.contains_key(&kind) && self.boss_icon_textures.contains_key(&kind)
        {
            return;
        }
        let Some((rgba, w, h)) = boss_icon_rgba(kind) else {
            return;
        };
        let label = kind.atlas_slug();
        if !self.boss_icon_meshes.contains_key(&kind)
            && let Some(cpu) = build_boss_icon_mesh_from_rgba(&rgba, w, h, label)
        {
            self.boss_icon_meshes.insert(
                kind,
                LitMeshGpu::new(&self.device, &cpu, &format!("boss-icon-mesh-{label}")),
            );
        }
        if !self.boss_icon_textures.contains_key(&kind) {
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
            self.boss_icon_textures
                .insert(kind, RelicTextureGpu { view, relief_view });
        }
    }

    /// Drain any decoded relic images from the background loader and upload them
    /// to the GPU.  Called once per frame; a no-op once all images are loaded.
    pub(crate) fn poll_relic_textures(&mut self) {
        let Some(ref rx) = self.relic_rx else { return };
        let mut finished = false;
        // Non-blocking drain: upload every image that's ready this frame.
        loop {
            match rx.try_recv() {
                Ok(img) => {
                    let mesh_source = img
                        .mesh_rgba
                        .as_deref()
                        .map(|rgba| (rgba, img.mesh_width, img.mesh_height))
                        .unwrap_or((&img.rgba, img.width, img.height));
                    // Diagnostic label for `build_relic_mesh_from_rgba`: prefer
                    // the actual mask asset path (where artists need to look)
                    // and fall back to the albedo path when no mask file
                    // exists and we're falling back to the albedo's alpha.
                    let mesh_source_label = if img.mesh_rgba.is_some() {
                        img.id.source_mask_path()
                    } else {
                        format!("{} (alpha fallback)", img.id.render_texture_path())
                    };
                    let t_mesh = std::time::Instant::now();
                    if let Some(cpu) = build_relic_mesh_from_rgba(
                        mesh_source.0,
                        mesh_source.1,
                        mesh_source.2,
                        &mesh_source_label,
                    ) {
                        // Cache the CPU triangle list alongside the GPU mesh so
                        // `pick_collection_object` / `pick_shop_object` can do
                        // per-triangle ray casts against the real silhouette
                        // instead of a loose AABB slab.
                        let tris: Vec<[glam::Vec3; 3]> = cpu
                            .indices
                            .chunks_exact(3)
                            .map(|c| {
                                let a = cpu.vertices[c[0] as usize].position;
                                let b = cpu.vertices[c[1] as usize].position;
                                let d = cpu.vertices[c[2] as usize].position;
                                [
                                    glam::Vec3::from(a),
                                    glam::Vec3::from(b),
                                    glam::Vec3::from(d),
                                ]
                            })
                            .collect();
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
                    self.relic_profile_mesh_cpu += t_mesh.elapsed();
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
            crate::startup_profile::record("relic.mesh_build_main", self.relic_profile_mesh_cpu);
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
                self.insert_background_solid_fallback_if_missing(id);
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

    /// When async decode fails (missing pack, corrupt file, …), `is_loading()` still goes
    /// false once the loader thread ends — without a bind group the façade pass uses a solid
    /// fallback until a successful decode replaces it.
    fn insert_background_solid_fallback_if_missing(&mut self, id: BackgroundId) {
        if self.background_textures.contains_key(&id) {
            return;
        }
        log::warn!(
            "background {:?} missing after async load — using solid fallback",
            id
        );
        let label = format!("bg-fallback-{id:?}");
        let rgba = vec![10u8, 10u8, 14u8, 255u8];
        let (_tex, view) = upload_rgba_texture(&self.device, &self.queue, &label, &rgba, 1, 1);
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
            .insert(id, BackgroundTextureGpu { bind_group });
    }
}
