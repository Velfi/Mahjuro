use super::super::*;

use mahjuro_gfx_types::ShadowQuality;

use crate::moths_to_a_light::{build_bug_body_mesh, build_bug_wing_blur_mesh, build_bug_wing_mesh};

#[cfg(feature = "windowed")]
fn present_boot_progress<'a, 'b>(
    boot_splash: &mut Option<super::super::boot_splash::BootSplash<'a>>,
    target: &super::super::targets::RenderTarget,
    config: &wgpu::SurfaceConfiguration,
    progress: f32,
    boot_poll_slot: &mut Option<&'b mut dyn FnMut()>,
) {
    super::super::boot_splash::boot_present(boot_splash, target, config, progress, boot_poll_slot);
}

fn load_startup_tile_meshes(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    tile_default_normal_view: &wgpu::TextureView,
    tile_glb_default_mr_view: &wgpu::TextureView,
    tile_glb_default_emissive_view: &wgpu::TextureView,
) -> [TileMeshGpuSet; crate::tile_glb::TILE_MATERIAL_MESH_COUNT] {
    use crate::tile_glb::{load_glb_tile_from_bytes, normalize_mesh, tile_glb_asset_path, tile_material_index};
    use mahjuro_gfx_types::TileMaterial;

    let tile_glb_defaults = crate::gltf_prop::GltfTileGpuDefaults {
        device,
        queue,
        default_normal_view: tile_default_normal_view,
        default_mr_view: tile_glb_default_mr_view,
        default_emissive_view: tile_glb_default_emissive_view,
    };
    let mut sets: [Option<TileMeshGpuSet>; crate::tile_glb::TILE_MATERIAL_MESH_COUNT] =
        std::array::from_fn(|_| None);
    for material in [
        TileMaterial::Bamboo,
        TileMaterial::Plastic,
        TileMaterial::TortoiseShell,
    ] {
        let path = tile_glb_asset_path(material);
        let label = format!("tile-{material:?}");
        let empty = crate::tile_glb::LoadedTile {
            primitives: Vec::new(),
        };
        let mesh_set = match mahjuro_assets::asset_path::get(path) {
            Some(file) => match load_glb_tile_from_bytes(&file.data) {
                Ok(mut mesh) => {
                    normalize_mesh(&mut mesh);
                    log::info!(
                        "Loaded 3D tile {:?}: {} primitive(s) from {path}",
                        material,
                        mesh.primitives.len()
                    );
                    for (i, prim) in mesh.primitives.iter().enumerate() {
                        log::info!(
                            "  {:?} prim {i}: {} verts, {} idx, face={}",
                            material,
                            prim.vertices.len(),
                            prim.indices.len(),
                            prim.vertices.first().is_some_and(|v| v.color[3] > 0.5),
                        );
                    }
                    crate::gltf_prop::upload_tile_mesh_gpu_set(&tile_glb_defaults, &label, &mesh)
                }
                Err(e) => {
                    log::warn!("Could not load tile mesh GLB {path}: {e:#}");
                    crate::gltf_prop::upload_tile_mesh_gpu_set(&tile_glb_defaults, &label, &empty)
                }
            },
            None => {
                log::warn!("Tile mesh GLB missing at {path} (packs or assets/)");
                crate::gltf_prop::upload_tile_mesh_gpu_set(&tile_glb_defaults, &label, &empty)
            }
        };
        sets[tile_material_index(material)] = Some(mesh_set);
    }
    sets.map(|slot| slot.expect("tile mesh slot filled above"))
}

fn build_primitive_mesh_registry(
    device: &wgpu::Device,
) -> rustc_hash::FxHashMap<MeshId, std::sync::Arc<LitMeshGpu>> {
    let mut primitive_meshes: rustc_hash::FxHashMap<MeshId, std::sync::Arc<LitMeshGpu>> =
        rustc_hash::FxHashMap::default();
    let unit_cube_cpu = {
        let mut verts: Vec<crate::tile_glb::Vertex3dTex> = Vec::new();
        let mut idx: Vec<u32> = Vec::new();
        push_box(
            &mut verts,
            &mut idx,
            Aabb::new(-0.5, 0.5, -0.5, 0.5, -0.5, 0.5),
        );
        MeshCpu {
            vertices: verts,
            indices: idx,
            default_material: MaterialParams {
                kind: MaterialKind::Plain,
                base_color: [1.0, 1.0, 1.0, 1.0],
                specular_strength: 0.25,
                specular_power: 32.0,
            },
        }
    };
    primitive_meshes.insert(
        MeshId::Cube,
        std::sync::Arc::new(LitMeshGpu::new(device, &unit_cube_cpu, "primitive-cube")),
    );
    primitive_meshes.insert(
        MeshId::BeveledSlab,
        std::sync::Arc::new(LitMeshGpu::new(device, &build_plaque_mesh(), "primitive-slab")),
    );
    primitive_meshes.insert(
        MeshId::CabinetColumn,
        std::sync::Arc::new(LitMeshGpu::new(
            device,
            &build_cabinet_mesh(),
            "primitive-cabinet-column",
        )),
    );
    primitive_meshes.insert(
        MeshId::CabinetRails,
        std::sync::Arc::new(LitMeshGpu::new(
            device,
            &build_cabinet_rails_mesh(),
            "primitive-cabinet-rails",
        )),
    );
    primitive_meshes.insert(
        MeshId::ShopActionProp,
        std::sync::Arc::new(LitMeshGpu::new(
            device,
            &build_shop_action_prop_mesh(),
            "primitive-shop-action-prop",
        )),
    );
    primitive_meshes.insert(
        MeshId::DiscSquare,
        std::sync::Arc::new(LitMeshGpu::new(
            device,
            &build_dish_mesh(),
            "primitive-dish-square",
        )),
    );
    primitive_meshes.insert(
        MeshId::DiscRound,
        std::sync::Arc::new(LitMeshGpu::new(
            device,
            &build_round_dish_mesh(),
            "primitive-dish-round",
        )),
    );
    primitive_meshes.insert(
        MeshId::PorcelainDish,
        std::sync::Arc::new(LitMeshGpu::new(
            device,
            &build_porcelain_dish_mesh(),
            "primitive-porcelain-dish",
        )),
    );
    // Cylinder — generic Y-up unit disc (legacy); yen coins use [`MeshId::Coin`].
    primitive_meshes.insert(
        MeshId::Cylinder,
        std::sync::Arc::new(LitMeshGpu::new(
            device,
            &build_coin_mesh(),
            "primitive-cylinder",
        )),
    );
    primitive_meshes.insert(
        MeshId::Ofuda,
        std::sync::Arc::new(LitMeshGpu::new(device, &build_ofuda_mesh(), "primitive-ofuda")),
    );
    primitive_meshes.insert(
        MeshId::Abacus,
        std::sync::Arc::new(LitMeshGpu::new(device, &build_abacus_mesh(), "primitive-abacus")),
    );
    primitive_meshes.insert(
        MeshId::AbacusHeavenBeads,
        std::sync::Arc::new(LitMeshGpu::new(
            device,
            &build_abacus_heaven_beads_mesh(),
            "primitive-abacus-heaven-beads",
        )),
    );
    primitive_meshes.insert(
        MeshId::AbacusEarthBeads,
        std::sync::Arc::new(LitMeshGpu::new(
            device,
            &build_abacus_earth_beads_mesh(),
            "primitive-abacus-earth-beads",
        )),
    );
    primitive_meshes.insert(
        MeshId::ShopBell,
        std::sync::Arc::new(LitMeshGpu::new(device, &build_shop_bell_mesh(), "primitive-shop-bell")),
    );
    primitive_meshes.insert(
        MeshId::BellTassel,
        std::sync::Arc::new(LitMeshGpu::new(
            device,
            &build_bell_tassel_mesh(),
            "primitive-bell-tassel",
        )),
    );
    primitive_meshes.insert(
        MeshId::ProgressMeterRail,
        std::sync::Arc::new(LitMeshGpu::new(
            device,
            &build_progress_meter_rail_mesh(),
            "primitive-progress-meter-rail",
        )),
    );
    primitive_meshes.insert(
        MeshId::ProgressMeterPip,
        std::sync::Arc::new(LitMeshGpu::new(
            device,
            &build_progress_meter_pip_mesh(),
            "primitive-progress-meter-pip",
        )),
    );
    primitive_meshes
}

struct RenderScaleDepthResources {
    suggested_graphics_mode: mahjuro_gfx_types::GraphicsMode,
    render_scale: f32,
    render_size: crate::physical_size::PhysicalSize,
    depth_texture: wgpu::Texture,
    depth_view: wgpu::TextureView,
    ssr_prev_depth_texture: wgpu::Texture,
    ssr_prev_depth_view: wgpu::TextureView,
    depth_r32_snapshot_texture: wgpu::Texture,
    depth_r32_snapshot_view: wgpu::TextureView,
    overlay_depth_texture: wgpu::Texture,
    overlay_depth_view: wgpu::TextureView,
    depth_copy_staging_buffer: wgpu::Buffer,
}

fn init_render_scale_and_depth_resources(
    device: &wgpu::Device,
    target: &super::super::targets::RenderTarget,
    adapter_name: &str,
    integrated_gpu: bool,
    size: crate::physical_size::PhysicalSize,
    depth_texture: wgpu::Texture,
    depth_view: wgpu::TextureView,
    ssr_prev_depth_texture: wgpu::Texture,
    ssr_prev_depth_view: wgpu::TextureView,
    depth_r32_snapshot_texture: wgpu::Texture,
    depth_r32_snapshot_view: wgpu::TextureView,
) -> RenderScaleDepthResources {
    let _phase = crate::startup_profile::scope("wgpu.phase.render_scale_depth");
    // Offline bakes/screenshots must match `room_gi` / `room_shadow` stamp resolution (native).
    let suggested_graphics_mode = if matches!(
        target,
        super::super::targets::RenderTarget::Offscreen { .. }
    ) {
        mahjuro_gfx_types::GraphicsMode::Visuals
    } else {
        mahjuro_gfx_types::GraphicsMode::suggest_for_adapter(adapter_name, integrated_gpu)
    };
    if !mahjuro_gfx_types::GraphicsMode::adapter_meets_minimum_support(
        adapter_name,
        integrated_gpu,
    ) {
        log::warn!(
            "adapter '{}' appears below minimum supported graphics memory ({} MiB); runtime behavior is unsupported",
            adapter_name,
            mahjuro_gfx_types::MIN_SUPPORTED_GPU_MEMORY_MIB
        );
    }
    let render_scale = suggested_graphics_mode.render_scale();
    let render_size = super::super::constants::scaled_render_size(size, render_scale);
    // `early_gpu_and_depth` allocates scene depth at window size; recreate when scaled down.
    let (
        depth_texture,
        depth_view,
        ssr_prev_depth_texture,
        ssr_prev_depth_view,
        depth_r32_snapshot_texture,
        depth_r32_snapshot_view,
    ) = if render_size.width != size.width || render_size.height != size.height {
        depth_texture.destroy();
        ssr_prev_depth_texture.destroy();
        depth_r32_snapshot_texture.destroy();
        let (dt, dv) = super::super::resources::create_depth(
            device,
            render_size.width.max(1),
            render_size.height.max(1),
        );
        let (sdt, sdv) = super::super::resources::create_depth_r32_snapshot(
            device,
            render_size.width.max(1),
            render_size.height.max(1),
            "ssr-prev-depth",
        );
        let (drt, drv) = super::super::resources::create_depth_r32_snapshot(
            device,
            render_size.width.max(1),
            render_size.height.max(1),
            "depth-r32-snapshot",
        );
        (dt, dv, sdt, sdv, drt, drv)
    } else {
        (
            depth_texture,
            depth_view,
            ssr_prev_depth_texture,
            ssr_prev_depth_view,
            depth_r32_snapshot_texture,
            depth_r32_snapshot_view,
        )
    };
    let (overlay_depth_texture, overlay_depth_view) =
        super::super::resources::create_depth(device, size.width.max(1), size.height.max(1));
    let depth_copy_staging_buffer = super::super::resources::create_depth_copy_staging(
        device,
        render_size.width.max(1),
        render_size.height.max(1),
    );
    RenderScaleDepthResources {
        suggested_graphics_mode,
        render_scale,
        render_size,
        depth_texture,
        depth_view,
        ssr_prev_depth_texture,
        ssr_prev_depth_view,
        depth_r32_snapshot_texture,
        depth_r32_snapshot_view,
        overlay_depth_texture,
        overlay_depth_view,
        depth_copy_staging_buffer,
    }
}

fn load_startup_fonts_with_profile() -> (
    Option<fontdue::Font>,
    Option<fontdue::Font>,
    Option<fontdue::Font>,
    Option<fontdue::Font>,
) {
    let _phase = crate::startup_profile::scope("wgpu.phase.fonts");
    let t_fonts = Instant::now();
    let ui_font = load_ui_font().cloned();
    if ui_font.is_some() {
        log::debug!("UI font loaded.");
    } else {
        log::warn!("No UI font found; panel text will be blank.");
    }
    let ui_font_italic = crate::decal::load_ui_font_italic().cloned();
    let mono_font = load_mono_font().cloned();
    if mono_font.is_some() {
        log::debug!("Mono UI font loaded.");
    } else {
        log::warn!("No mono UI font found; tabular Chronicle text falls back to serif.");
    }
    let emoji_font = load_noto_emoji_font();
    if emoji_font.is_some() {
        log::debug!("Noto Emoji font loaded.");
    } else {
        log::warn!("No Noto Emoji font found; tile symbols may be blank.");
    }
    crate::startup_profile::record("wgpu.fonts", t_fonts.elapsed());
    (ui_font, ui_font_italic, mono_font, emoji_font)
}

pub(super) fn build_renderer_new(
    target_init: TargetInit,
    #[cfg(feature = "windowed")] present_boot_frame: bool,
    #[cfg(feature = "windowed")] boot_input_poll: Option<&mut dyn FnMut()>,
) -> anyhow::Result<WgpuRenderer> {
    // Instance, adapter, device, surface/offscreen target, depth — see
    // `init_phases::early_gpu_and_depth`.
    let super::super::init_phases::EarlyGpuState {
        device,
        queue,
        adapter_name,
        integrated_gpu,
        size,
        target,
        config,
        format,
        swapchain_sdr_format,
        swapchain_hdr_available,
        timestamp_supported,
        gpu_profiler_backend,
        depth_texture,
        depth_view,
        ssr_prev_depth_texture,
        ssr_prev_depth_view,
        depth_r32_snapshot_texture,
        depth_r32_snapshot_view,
    } = {
        let _early = crate::startup_profile::scope("wgpu.early_gpu");
        super::super::init_phases::early_gpu_and_depth(target_init)?
    };
    #[cfg(feature = "windowed")]
    let mut boot_splash: Option<super::super::boot_splash::BootSplash<'_>> = if present_boot_frame {
        Some(super::super::boot_splash::BootSplash::new(
            &device,
            &queue,
            format,
            size.width,
            size.height,
        )?)
    } else {
        None
    };
    #[cfg(feature = "windowed")]
    let mut boot_poll_slot = boot_input_poll;
    #[cfg(feature = "windowed")]
    present_boot_progress(&mut boot_splash, &target, &config, 0.06, &mut boot_poll_slot);
    let RenderScaleDepthResources {
        suggested_graphics_mode,
        render_scale,
        render_size,
        depth_texture,
        depth_view,
        ssr_prev_depth_texture,
        ssr_prev_depth_view,
        depth_r32_snapshot_texture,
        depth_r32_snapshot_view,
        overlay_depth_texture,
        overlay_depth_view,
        depth_copy_staging_buffer,
    } = init_render_scale_and_depth_resources(
        &device,
        &target,
        &adapter_name,
        integrated_gpu,
        size,
        depth_texture,
        depth_view,
        ssr_prev_depth_texture,
        ssr_prev_depth_view,
        depth_r32_snapshot_texture,
        depth_r32_snapshot_view,
    );

    {
        let _bakes = crate::startup_profile::scope("wgpu.offline_bakes");
        crate::offline_bakes::require_all_at_startup()?;
    }
    #[cfg(feature = "windowed")]
    present_boot_progress(&mut boot_splash, &target, &config, 0.12, &mut boot_poll_slot);


    let super::shaders_and_pipelines::ShadersAndPipelinesInit {
        arc_ring_quad_pipeline,
        arc_ring_quad_pipeline_display,
        bloom_bind_group_layout,
        bloom_blur_h_params_buffer,
        bloom_blur_pipeline,
        bloom_blur_v_params_buffer,
        bloom_composite_bind_group,
        bloom_composite_bind_group_layout,
        bloom_composite_params_buffer,
        bloom_composite_pipeline,
        bloom_extract_bind_group_layout,
        bloom_extract_params_buffer,
        bloom_extract_pipeline,
        bloom_ping_bind_group,
        bloom_ping_texture,
        bloom_ping_view,
        bloom_pong_bind_group,
        bloom_pong_texture,
        bloom_pong_view,
        bloom_sampler,
        bloom_scene_bind_group,
        cascade_composite_bind_group,
        cascade_composite_layout,
        cascade_composite_pipeline,
        cascade_composite_sampler,
        cascade_offscreen_texture,
        cascade_offscreen_view,
        default_shadow_quality,
        depth_quad_debug_pipeline,
        depth_quad_debug_pipeline_display,
        depth_quad_pipeline,
        depth_quad_pipeline_display,
        emissive_gi_composite_bind_group,
        emissive_gi_composite_bind_group_layout,
        emissive_gi_composite_pipeline,
        emissive_gi_texture,
        emissive_gi_view,
        emissive_probe_apply_bind_group,
        emissive_probe_apply_bind_group_layout,
        emissive_probe_apply_pipeline,
        emissive_probe_update_bind_group,
        emissive_probe_update_bind_group_layout,
        emissive_probe_update_pipeline,
        flame_pipeline,
        flame_view_bind_group,
        flame_view_buffer,
        flame_volume_mesh,
        globals_bind_group,
        globals_buffer,
        golden_dust_pipeline,
        gradient_quad_pipeline,
        image_pipeline,
        image_pipeline_scene_hdr,
        image_shader,
        journal_scene_texture,
        journal_scene_view,
        lit_mesh_blended_pipeline,
        lit_mesh_material_layout,
        lit_mesh_pipeline,
        lit_mesh_spot_ssr_bind_group,
        lit_mesh_spot_ssr_layout,
        lit_mesh_ssr_buffer,
        lit_mesh_ssr_sampler,
        moonlit_water_bind_group,
        moonlit_water_pipeline,
        point_lights_bind_group,
        point_lights_buffer,
        point_shadow_array,
        post_bloom_texture,
        post_bloom_view,
        probe_gi_frame_uniform_buffer,
        probe_sh_buffer,
        quad_pipeline,
        quad_pipeline_display,
        room_baked_shadow_gpu,
        room_emissive_texture,
        room_emissive_view,
        scene_color_downsample_bind_group,
        scene_color_downsample_pipeline,
        scene_color_texture,
        scene_color_view,
        scene_prev_texture,
        scene_prev_view,
        shadow_ao_sampler,
        shadow_ao_white_texture,
        shadow_ao_white_view,
        shadow_baked_depth_dummy_texture,
        shadow_baked_depth_dummy_view,
        shadow_caster_layout,
        shadow_compare_sampler,
        shadow_globals_buffer,
        shadow_pipeline,
        shadow_pipeline_room_env,
        shadow_sample_bind_group,
        shadow_sample_layout,
        shadow_warp_disabled_bind_group,
        shadow_warp_layout,
        shooting_star_cascade_pipeline,
        shop_pipeline_blend_cull,
        shop_pipeline_blend_double,
        shop_pipeline_mrt_blend_cull,
        shop_pipeline_mrt_blend_double,
        shop_pipeline_mrt_opaque_cull,
        shop_pipeline_opaque_cull,
        spot_lights_bind_group,
        spot_lights_buffer,
        spot_shadow_array,
        squircle_quad_pipeline,
        squircle_quad_pipeline_display,
        starfield_pipeline,
        sunlit_water_pipeline,
        text_bind_group_layout,
        text_overlay_pipeline_layout,
        text_pipeline,
        text_pipeline_scene_hdr,
        text_shader,
        tile_base_color_factor,
        tile_default_normal_texture,
        tile_default_normal_view,
        tile_env_distortion_placeholder,
        tile_glow_pipeline,
        tile_material_layout,
        tile_occluders_buffer,
        tile_outline_frame_bind_group,
        tile_outline_frame_uniform_buffer,
        tile_outline_instance_buffer,
        tile_outline_pipeline,
        tile_pipeline_blend_cull,
        tile_pipeline_blend_double,
        tile_pipeline_opaque_cull,
        tile_sampler,
        tonemap_bind_group,
        tonemap_bind_group_layout,
        tonemap_bind_group_scene,
        tonemap_params_buffer,
        tonemap_pipeline,
        tonemap_pipeline_layout,
        tonemap_rgba16f_pipeline,
        tonemap_shader_module,
    } = super::shaders_and_pipelines::init_shaders_and_pipelines(
        super::shaders_and_pipelines::ShadersAndPipelinesParams {
            device: &device,
            queue: &queue,
            size,
            render_size,
            format,
            ssr_prev_depth_view: &ssr_prev_depth_view,
            depth_r32_snapshot_view: &depth_r32_snapshot_view,
        },
    );

    #[cfg(feature = "windowed")]
    present_boot_progress(&mut boot_splash, &target, &config, 0.48, &mut boot_poll_slot);

    let (ui_font, ui_font_italic, mono_font, emoji_font) = load_startup_fonts_with_profile();
    #[cfg(feature = "windowed")]
    present_boot_progress(&mut boot_splash, &target, &config, 0.55, &mut boot_poll_slot);

    let quad_v: [[f32; 2]; 4] = [[0.0, 0.0], [1.0, 0.0], [0.0, 1.0], [1.0, 1.0]];
    let vertex_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("quad-verts"),
        contents: bytemuck::cast_slice(&quad_v),
        usage: wgpu::BufferUsages::VERTEX,
    });

    let idx: [u16; 6] = [0, 1, 2, 2, 1, 3];
    let index_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("quad-idx"),
        contents: bytemuck::cast_slice(&idx),
        usage: wgpu::BufferUsages::INDEX,
    });

    let (_tile_glb_default_mr_tex, tile_glb_default_mr_view) =
        default_metallic_roughness_map(&device, &queue);
    let (_tile_glb_default_emissive_tex, tile_glb_default_emissive_view) =
        default_emissive_map(&device, &queue);
    let tile_env_normal_view = tile_default_normal_view.clone();
    let tile_env_mr_view = tile_glb_default_mr_view.clone();
    let tile_env_emissive_view = tile_glb_default_emissive_view.clone();

    let tile_meshes = {
        let _tile = crate::startup_profile::scope("wgpu.tile_mesh");
        load_startup_tile_meshes(
            &device,
            &queue,
            &tile_default_normal_view,
            &tile_glb_default_mr_view,
            &tile_glb_default_emissive_view,
        )
    };
    #[cfg(feature = "windowed")]
    present_boot_progress(&mut boot_splash, &target, &config, 0.66, &mut boot_poll_slot);

    // Deferred room GPU uploads — see `room_gpu_load.rs` (`ensure_*_room_gpu`).
    let shop_gltf_anim = crate::room_gltf_anim::RoomGltfAnimGpu::default();
    let (shop_env_primitives, shop_environment, shop_eyeball_prim_indices) =
        (Vec::new(), None, Vec::new());
    let (hallway_env_primitives, hallway_environment) = (Vec::new(), None);
    let (staircase_env_primitives, staircase_environment) = (Vec::new(), None);
    let (
        archive_env_primitives,
        archive_environment,
        archive_sign_left_prim_idx,
        archive_sign_right_prim_idx,
        archive_inspect_plaque_prim_idx,
        archive_plaque_backing_prim_idx,
        archive_page_left_prim_indices,
        archive_page_right_prim_indices,
    ) = (
        Vec::new(),
        None,
        None,
        None,
        None,
        None,
        Vec::new(),
        Vec::new(),
    );
    let (main_menu_env_primitives, main_menu_environment) = (Vec::new(), None);
    let main_menu_env_collision_meshes = Vec::new();
    let (
        gameplay_env_primitives,
        gameplay_environment,
        gameplay_cash_in_prim_indices,
        gameplay_score_roller_prim_groups,
        gameplay_score_roller_pivots_doc,
        gameplay_score_roller_axes_doc,
    ) = (
        Vec::new(),
        None,
        Vec::new(),
        Vec::new(),
        Vec::new(),
        Vec::new(),
    );
    let gameplay_env_collision_meshes = Vec::new();
    let shop_env_collision_meshes = Vec::new();

    // Relic decode starts on first frame (`ensure_relic_loader_started`) so sync boot
    // is not competing with 100+ PNG decodes on a background thread.
    let relic_load_start = None;
    let relic_rx = None;
    let (_lit_mesh_relief_default_tex, lit_mesh_relief_default_view) =
        flat_relief_height(&device, &queue);
    let pack_textures_map = {
        let _pack = crate::startup_profile::scope("wgpu.pack_textures");
        load_pack_textures(&device, &queue, &lit_mesh_relief_default_view)
    };
    #[cfg(feature = "windowed")]
    present_boot_progress(&mut boot_splash, &target, &config, 0.72, &mut boot_poll_slot);
    let (background_load_start, background_rx) =
        if super::resources::ASYNC_LOADED_BACKGROUNDS.is_empty() {
            (None, None)
        } else {
            (Some(Instant::now()), Some(spawn_background_loader()))
        };

    // ---- Lit-mesh procedural geometry (candles) ----
    let t_lit_meshes = Instant::now();
    let t_lit_mesh_geometry = Instant::now();
    let relic_box_cpu = build_relic_mesh();
    let relic_box_tris: Vec<[glam::Vec3; 3]> = relic_box_cpu
        .indices
        .chunks_exact(3)
        .map(|c| {
            let a = relic_box_cpu.vertices[c[0] as usize].position;
            let b = relic_box_cpu.vertices[c[1] as usize].position;
            let d = relic_box_cpu.vertices[c[2] as usize].position;
            [
                glam::Vec3::from(a),
                glam::Vec3::from(b),
                glam::Vec3::from(d),
            ]
        })
        .collect();
    let relic_box_mesh = LitMeshGpu::new(&device, &relic_box_cpu, "relic-mesh");
    let pack_mesh = LitMeshGpu::new(&device, &build_pack_mesh(), "pack-mesh");
    let ribbon_mesh = LitMeshGpu::new(&device, &build_ribbon_mesh(), "ribbon");
    let bug_body_mesh = LitMeshGpu::new(&device, &build_bug_body_mesh(), "bug-body");
    let bug_wing_mesh = LitMeshGpu::new(&device, &build_bug_wing_mesh(), "bug-wing");
    let bug_wing_blur_mesh = LitMeshGpu::new(&device, &build_bug_wing_blur_mesh(), "bug-wing-blur");
    let coin_glb_file = mahjuro_assets::asset_path::get("3d/coin.glb")
        .expect("3d/coin.glb not embedded (packs or assets/)");
    let mut coin_tile = crate::tile_glb::load_glb_tile_from_node_name(
        &coin_glb_file.data,
        Some(crate::coin_glb::COIN_GLB_NODE),
    )
    .expect("coin.glb node decode");
    crate::tile_glb::reorient_mesh_to_engine_axes(&mut coin_tile);
    crate::tile_glb::center_mesh_at_origin(&mut coin_tile);
    let coin_half = crate::tile_glb::mesh_local_half_extents(&coin_tile);
    crate::coin_glb::init_coin_glb_half_extents(coin_half);
    log::info!(
        "Loaded coin.glb: {} material slot(s), half_extents={coin_half:?}",
        coin_tile.primitives.len()
    );
    // Phase-1 primitive registry: parallel GPU copies of meshes
    // the generic `Object3dKind::Primitive` dispatch can reach by
    // `MeshId`. Legacy named fields above still own their own
    // allocations during the migration window.
    let primitive_meshes = {
        let _primitive_registry = crate::startup_profile::scope("wgpu.lit_meshes.primitive_registry");
        build_primitive_mesh_registry(&device)
    };
    let primitive_textures: rustc_hash::FxHashMap<
        crate::primitive::MeshId,
        (wgpu::TextureView, wgpu::TextureView),
    > = rustc_hash::FxHashMap::default();
    let bone_tablet_mesh = LitMeshGpu::new(&device, &build_bone_tablet_mesh(), "bone-tablet");
    let wood_tablet_mesh = LitMeshGpu::new(&device, &build_wood_tablet_mesh(), "wood-tablet");
    let book_mesh = LitMeshGpu::new(&device, &build_book_body_mesh(), "book");
    let book_cover_mesh = LitMeshGpu::new(&device, &build_book_cover_mesh(), "book-cover");
    // The legacy "bowl" slot now hosts the discard river mesh — a stone
    // trough with an animated water surface. Field/variant names stayed
    // (`bowl_mesh`, `BowlPlacement`, `GameplayPick::DiscardBowl`) to keep
    // this swap to a single mesh substitution; renaming is a follow-up.
    let bowl_mesh = LitMeshGpu::new(&device, &build_river_mesh(), "river");
    let mirror_mesh = LitMeshGpu::new(&device, &build_mirror_mesh(), "mirror");
    let tally_stick_base_mesh =
        LitMeshGpu::new(&device, &build_tally_stick_base_mesh(), "tally-stick-base");
    let tally_stick_tip_mesh =
        LitMeshGpu::new(&device, &build_tally_stick_tip_mesh(), "tally-stick-tip");
    // Shared 1×1 white texture for procedural meshes that don't sample.
    let (_lit_mesh_white_tex, lit_mesh_white_view) = white_albedo(&device, &queue);
    let coin_glb_primitives = crate::gltf_prop::upload_gltf_tile_primitives(
        &crate::gltf_prop::GltfTileGpuDefaults {
            device: &device,
            queue: &queue,
            default_normal_view: &tile_default_normal_view,
            default_mr_view: &tile_glb_default_mr_view,
            default_emissive_view: &tile_glb_default_emissive_view,
        },
        "coin-glb",
        &coin_tile.primitives,
    );
    let (main_menu_rain_hit_debug_mesh, main_menu_rain_hit_debug_instance) =
        super::super::resources::init_main_menu_rain_hit_debug(
            &device,
            &lit_mesh_material_layout,
            &shadow_caster_layout,
            &lit_mesh_white_view,
            &lit_mesh_relief_default_view,
            &tile_sampler,
        );
    crate::startup_profile::record("wgpu.lit_meshes.geometry", t_lit_mesh_geometry.elapsed());

    let t_lit_mesh_instance_pools = Instant::now();
    let relic_instances: Vec<LitMeshInstance> = Vec::new();
    let ordeal_icon_instances: Vec<LitMeshInstance> = Vec::new();
    let mut pack_instances: Vec<LitMeshInstance> = Vec::with_capacity(4);
    for _ in 0..4 {
        pack_instances.push(LitMeshInstance::new(
            &device,
            &lit_mesh_material_layout,
            &shadow_caster_layout,
            &lit_mesh_white_view,
            &lit_mesh_relief_default_view,
            &tile_sampler,
        ));
    }
    let ribbon_zodiac_tex = load_zodiac_ribbon_textures(&device, &queue);
    let mut ribbon_instances: Vec<LitMeshInstance> = Vec::with_capacity(MAX_RIBBON_SLOTS);
    for _ in 0..MAX_RIBBON_SLOTS {
        ribbon_instances.push(LitMeshInstance::new(
            &device,
            &lit_mesh_material_layout,
            &shadow_caster_layout,
            &lit_mesh_white_view,
            &lit_mesh_relief_default_view,
            &tile_sampler,
        ));
    }
    let ribbon_slot_zodiac: Vec<Option<u8>> = vec![None; MAX_RIBBON_SLOTS];
    let mut bug_body_instances: Vec<LitMeshInstance> = Vec::with_capacity(MAX_BUG_SLOTS);
    let mut bug_wing_instances: Vec<LitMeshInstance> = Vec::with_capacity(MAX_BUG_SLOTS);
    let mut bug_wing_r_instances: Vec<LitMeshInstance> = Vec::with_capacity(MAX_BUG_SLOTS);
    let mut bug_wing_blur_instances: Vec<LitMeshInstance> = Vec::with_capacity(MAX_BUG_SLOTS);
    let mut bug_wing_blur_r_instances: Vec<LitMeshInstance> = Vec::with_capacity(MAX_BUG_SLOTS);
    for _ in 0..MAX_BUG_SLOTS {
        bug_body_instances.push(LitMeshInstance::new(
            &device,
            &lit_mesh_material_layout,
            &shadow_caster_layout,
            &lit_mesh_white_view,
            &lit_mesh_relief_default_view,
            &tile_sampler,
        ));
        bug_wing_instances.push(LitMeshInstance::new(
            &device,
            &lit_mesh_material_layout,
            &shadow_caster_layout,
            &lit_mesh_white_view,
            &lit_mesh_relief_default_view,
            &tile_sampler,
        ));
        bug_wing_r_instances.push(LitMeshInstance::new(
            &device,
            &lit_mesh_material_layout,
            &shadow_caster_layout,
            &lit_mesh_white_view,
            &lit_mesh_relief_default_view,
            &tile_sampler,
        ));
        bug_wing_blur_instances.push(LitMeshInstance::new(
            &device,
            &lit_mesh_material_layout,
            &shadow_caster_layout,
            &lit_mesh_white_view,
            &lit_mesh_relief_default_view,
            &tile_sampler,
        ));
        bug_wing_blur_r_instances.push(LitMeshInstance::new(
            &device,
            &lit_mesh_material_layout,
            &shadow_caster_layout,
            &lit_mesh_white_view,
            &lit_mesh_relief_default_view,
            &tile_sampler,
        ));
    }
    let orb_mesh = LitMeshGpu::new(&device, &build_orb_mesh(), "material-orb");
    let orb_instances: Vec<LitMeshInstance> = Vec::new();
    let talisman_height_views: Vec<wgpu::TextureView> = Vec::new();
    let talisman_mask_views: Vec<wgpu::TextureView> = Vec::new();
    let memorial_talisman_height_views: Vec<wgpu::TextureView> = Vec::new();
    let memorial_talisman_mask_views: Vec<wgpu::TextureView> = Vec::new();
    let talisman_slot_kind: Vec<Option<u8>> = vec![None; MAX_TALISMAN_SLOTS];
    let mut talisman_instances: Vec<LitMeshInstance> = Vec::with_capacity(MAX_TALISMAN_SLOTS);
    for _ in 0..MAX_TALISMAN_SLOTS {
        talisman_instances.push(LitMeshInstance::new(
            &device,
            &lit_mesh_material_layout,
            &shadow_caster_layout,
            &lit_mesh_white_view,
            &lit_mesh_relief_default_view,
            &tile_sampler,
        ));
    }
    // Shop journal books are cheap; gameplay HUD instance pools are deferred
    // until first gameplay draw (`ensure_gameplay_hud_pools`).
    let make_pool = |n: usize| -> Vec<LitMeshInstance> {
        (0..n)
            .map(|_| {
                LitMeshInstance::new(
                    &device,
                    &lit_mesh_material_layout,
                    &shadow_caster_layout,
                    &lit_mesh_white_view,
                    &lit_mesh_relief_default_view,
                    &tile_sampler,
                )
            })
            .collect()
    };
    let book_instances = make_pool(MAX_BOOK_SLOTS);
    let book_cover_instances = make_pool(MAX_BOOK_SLOTS);
    let yaku_tablet_instances: Vec<LitMeshInstance> = Vec::new();
    let wood_tablet_instances: Vec<LitMeshInstance> = Vec::new();
    let bowl_instances: Vec<LitMeshInstance> = Vec::new();
    let mirror_instances: Vec<LitMeshInstance> = Vec::new();
    let tally_stick_instances: Vec<LitMeshInstance> = Vec::new();
    let wall_tile_instances: Vec<LitMeshInstance> = Vec::new();
    let extruded_glyph_instances: Vec<LitMeshInstance> = Vec::new();
    let debug_axes_instances = make_pool(3);
    crate::startup_profile::record(
        "wgpu.lit_meshes.instance_pools",
        t_lit_mesh_instance_pools.elapsed(),
    );
    crate::startup_profile::record("wgpu.lit_meshes_and_pools", t_lit_meshes.elapsed());
    #[cfg(feature = "windowed")]
    present_boot_progress(&mut boot_splash, &target, &config, 0.92, &mut boot_poll_slot);

    // Build the GPU profiler up-front while we still have a borrow of
    // device/queue (the struct literal below moves them).
    let gpu_profiler = crate::gpu_profiler::GpuProfiler::new(
        &device,
        &queue,
        timestamp_supported,
        gpu_profiler_backend,
    );

    queue.write_buffer(
        &tonemap_params_buffer,
        0,
        bytemuck::bytes_of(&TonemapParams {
            exposure: 1.0,
            mode: 0.0,
            vhs_enabled: 0.0,
            time: 0.0,
            vhs_chromatic: 0.0,
            vhs_scanline: 0.0,
            vhs_grain: 0.0,
            vhs_vignette: 0.0,
            grain_frame: 0.0,
            gamma: 1.0,
        }),
    );

    // Per-frame bump-allocated buffer pool used by the highest-frequency
    // per-frame instance vertex uploads in `runtime/render.rs`. Created
    // before the struct literal so the `&device` borrow doesn't conflict
    // with `device` being moved into the `WgpuRenderer.device` field.
    let frame_buffer_pool =
        super::super::frame_pool::FrameBufferPool::new(&device, "frame-buffer-pool", 1 << 20);

    #[cfg(feature = "windowed")]
    present_boot_progress(&mut boot_splash, &target, &config, 1.0, &mut boot_poll_slot);
    #[cfg(feature = "windowed")]
    drop(boot_splash);

    Ok(WgpuRenderer {
        target,
        device,
        queue,
        config,
        depth_texture,
        depth_view,
        ssr_prev_depth_texture,
        ssr_prev_depth_view,
        depth_r32_snapshot_texture,
        depth_r32_snapshot_view,
        depth_copy_staging_buffer,
        quad_pipeline,
        quad_pipeline_display,
        depth_quad_pipeline,
        depth_quad_pipeline_display,
        depth_quad_debug_pipeline,
        depth_quad_debug_pipeline_display,
        gradient_quad_pipeline,
        arc_ring_quad_pipeline,
        arc_ring_quad_pipeline_display,
        squircle_quad_pipeline,
        squircle_quad_pipeline_display,
        flame_pipeline,
        flame_volume_mesh,
        flame_view_buffer,
        flame_view_bind_group,
        flame_instance_staging: Vec::with_capacity(32),
        starfield_pipeline,
        golden_dust_pipeline,
        moonlit_water_pipeline,
        moonlit_water_bind_group,
        sunlit_water_pipeline,
        shooting_star_cascade_pipeline,
        cascade_composite_pipeline,
        cascade_composite_layout,
        cascade_composite_sampler,
        cascade_offscreen_texture,
        cascade_offscreen_view,
        cascade_composite_bind_group,
        scene_color_downsample_pipeline,
        scene_color_downsample_bind_group,
        tile_pipeline_opaque_cull,
        tile_pipeline_blend_double,
        tile_pipeline_blend_cull,
        shop_pipeline_opaque_cull,
        shop_pipeline_blend_double,
        shop_pipeline_blend_cull,
        shop_pipeline_mrt_opaque_cull,
        shop_pipeline_mrt_blend_double,
        shop_pipeline_mrt_blend_cull,
        tile_outline_pipeline,
        tile_glow_pipeline,
        globals_buffer,
        globals_bind_group,
        tile_material_layout,
        tile_env_distortion_placeholder,
        tile_outline_frame_uniform_buffer,
        tile_outline_instance_buffer,
        tile_outline_frame_bind_group,
        tile_outline_instances_staging: Vec::new(),
        tile_outline_batch_ranges: Vec::new(),
        point_lights_buffer,
        tile_occluders_buffer,
        point_lights_bind_group,
        spot_lights_buffer,
        spot_lights_bind_group,
        tile_sampler,
        _tile_default_normal_texture: tile_default_normal_texture,
        _tile_glb_default_mr_texture: _tile_glb_default_mr_tex,
        _tile_glb_default_emissive_texture: _tile_glb_default_emissive_tex,
        tile_meshes,
        active_tile_material: mahjuro_gfx_types::TileMaterial::Bamboo,
        shop_env_primitives,
        shop_environment,
        shop_gltf_anim,
        shop_gltf_anim_missing_clip_warned: std::cell::Cell::new(false),
        shop_eyeball_prim_indices,
        shop_room_gpu_upload: None,
        rooms_gpu_loaded: 0,
        poll_pinned_room_gpu_bit: None,
        gpu_memory_pressure: crate::gpu_memory_pressure::GpuMemoryPressure::Normal,
        room_profile_frame_dt_ms: 1000.0 / 60.0,
        shadow_warp_layout,
        tile_env_normal_view,
        tile_env_mr_view,
        tile_env_emissive_view,
        hallway_env_primitives,
        hallway_environment,
        hallway_room_gpu_upload: None,
        staircase_env_primitives,
        staircase_environment,
        archive_env_primitives,
        archive_environment,
        main_menu_env_primitives,
        main_menu_environment,
        gameplay_env_primitives,
        gameplay_cash_in_prim_indices,
        gameplay_score_roller_prim_groups,
        gameplay_score_roller_pivots_doc,
        gameplay_score_roller_axes_doc,
        gameplay_score_roller_drive_values: std::cell::RefCell::new([0.0; 2]),
        gameplay_score_roller_drive_initialized: std::cell::RefCell::new([false; 2]),
        gameplay_score_roller_roll_elapsed: std::cell::RefCell::new(0.0),
        gameplay_environment,
        gameplay_room_gpu_upload: None,
        archive_sign_left_prim_idx,
        archive_sign_right_prim_idx,
        archive_inspect_plaque_prim_idx,
        archive_plaque_backing_prim_idx,
        archive_page_left_prim_indices,
        archive_page_right_prim_indices,
        archive_sign_decal_upload_key: 0,
        archive_inspect_plaque_decal_upload_key: 0,
        frame_env_tunes: rustc_hash::FxHashMap::default(),
        active_frame_env: crate::room_glb::RoomEnvFrameTune::default(),
        shop_env_collision_meshes,
        main_menu_env_collision_meshes,
        gameplay_env_collision_meshes,
        tile_base_color_factor,
        // Populated on first render() from RenderSettings.tileset_name.
        tile_set: None,
        showcase_tiles: Vec::new(),
        tile_face_overlays: rustc_hash::FxHashMap::default(),
        image_quad_overlays: rustc_hash::FxHashMap::default(),
        image_quad_missing: rustc_hash::FxHashSet::default(),
        debuff_marker_overlay: None,
        text_label_cache: rustc_hash::FxHashMap::default(),
        text_cache_frame: 0,
        vertex_buffer,
        index_buffer,
        text_pipeline,
        text_pipeline_scene_hdr,
        text_bind_group_layout,
        text_overlay_pipeline_layout,
        text_shader_module: text_shader,
        image_shader_module: image_shader,
        image_pipeline,
        image_pipeline_scene_hdr,
        ui_font,
        emoji_font,
        ui_font_italic,
        mono_font,
        size,
        render_size,
        render_scale,
        overlay_depth_texture,
        overlay_depth_view,
        graphics_mode: suggested_graphics_mode,
        suggested_graphics_mode,
        room_gpu_lru: std::collections::VecDeque::new(),
        tile_anim_y: Vec::new(),
        tile_anim_x: Vec::new(),
        tile_uids: Vec::new(),
        proj: ProjectionCache::default(),
        last_pick_models: Vec::new(),
        last_pick_camera: None,
        last_gameplay_cash_in_button_visible: false,
        last_relic_models: Vec::new(),
        relic_slot_texture: vec![None; MAX_RELIC_SLOTS],
        ordeal_icon_instances,
        ordeal_icon_meshes: rustc_hash::FxHashMap::default(),
        ordeal_icon_textures: rustc_hash::FxHashMap::default(),
        ordeal_icon_slot_texture: vec![None; MAX_ORDEAL_ICON_SLOTS],
        pack_instances,
        pack_slot_texture: vec![None; 4],
        ribbon_mesh,
        ribbon_instances,
        ribbon_slot_zodiac,
        ribbon_zodiac_tex,
        talisman_instances,
        bug_body_mesh,
        bug_wing_mesh,
        bug_body_instances,
        bug_wing_instances,
        bug_wing_r_instances,
        bug_wing_blur_mesh,
        bug_wing_blur_instances,
        bug_wing_blur_r_instances,
        orb_mesh,
        orb_instances,
        last_ribbon_models: Vec::new(),
        last_ribbon_batch_slot_counts: Vec::new(),
        last_talisman_models: Vec::new(),
        last_aux_dish_aabbs: Vec::new(),
        bone_tablet_mesh,
        wood_tablet_mesh,
        book_mesh,
        book_cover_mesh,
        bowl_mesh,
        mirror_mesh,
        tally_stick_base_mesh,
        tally_stick_tip_mesh,
        yaku_tablet_instances,
        wood_tablet_instances,
        book_instances,
        book_cover_instances,
        bowl_instances,
        mirror_instances,
        tally_stick_instances,
        wall_tile_instances,
        extruded_glyph_instances,
        glyph_cpu_cache: crate::glyph_mesh::GlyphMeshCache::new(),
        extruded_glyph_meshes: rustc_hash::FxHashMap::default(),
        primitive_meshes,
        primitive_instances: rustc_hash::FxHashMap::default(),
        primitive_textures,
        coin_glb_primitives,
        coin_glb_instances: Vec::new(),
        last_primitive_pick_models: rustc_hash::FxHashMap::default(),
        debug_axes_instances,
        last_yaku_tablet_models: Vec::new(),
        last_wood_tablet_models: Vec::new(),
        last_bowl_model: None,
        last_mirror_model: None,
        active_scene_key: None,
        last_frame: Instant::now(),
        frame_dt: 0.0,
        obj3d_hover_state: rustc_hash::FxHashMap::default(),
        creation_time: Instant::now(),
        vhs_grain_frame: 0,
        relic_textures: rustc_hash::FxHashMap::default(),
        gameplay_hud_pools_ready: false,
        talisman_textures_ready: false,
        relic_rx,
        relic_load_finished: false,
        relic_load_start,
        relic_profile_upload_cpu: std::time::Duration::ZERO,
        pack_textures: pack_textures_map,
        background_textures: rustc_hash::FxHashMap::default(),
        background_rx,
        background_load_start,
        prev_tile_world: rustc_hash::FxHashMap::default(),
        tile_uid_scratch: rustc_hash::FxHashSet::default(),
        prev_shadow_quality: ShadowQuality::Off,
        cached_shadow_light_view_proj: glam::Mat4::IDENTITY.to_cols_array(),
        projected_shadow_lights: Vec::new(),
        cached_projected_shadow_hash: 0,
        showcase_decal_atlas: None,
        showcase_decal_atlas_tileset: None,
        showcase_decal_atlas_cache: std::collections::VecDeque::new(),
        lit_mesh_material_layout,
        lit_mesh_spot_ssr_layout,
        lit_mesh_ssr_buffer,
        lit_mesh_spot_ssr_bind_group,
        lit_mesh_ssr_sampler,
        scene_prev_texture,
        scene_prev_view,
        scene_color_texture,
        scene_color_view,
        room_emissive_texture,
        room_emissive_view,
        emissive_gi_texture,
        emissive_gi_view,
        emissive_probe_update_pipeline,
        emissive_probe_update_bind_group_layout,
        emissive_probe_update_bind_group,
        emissive_probe_apply_pipeline,
        emissive_probe_apply_bind_group_layout,
        emissive_probe_apply_bind_group,
        probe_gi_frame_uniform_buffer,
        probe_sh_buffer,
        probe_gi_tick: 0,
        probe_gi_last_view_proj: glam::Mat4::IDENTITY.to_cols_array(),
        probe_gi_last_size: (0, 0),
        probe_gi_had_room: false,
        probe_gi_gpu_room: None,
        room_gi_capture_pending: None,
        room_gi_capture_meta: None,
        room_gi_captured: None,
        room_baked_shadow_gpu,
        active_room_baked_shadow: None,
        lab_baked_shadow: None,
        active_lab_baked_shadow: false,
        room_shadow_capture_pending: None,
        room_shadow_captured: None,
        shadow_probe_last_log: Instant::now(),
        shadow_probe_last_caster_count: usize::MAX,
        emissive_gi_composite_pipeline,
        emissive_gi_composite_bind_group_layout,
        emissive_gi_composite_bind_group,
        journal_scene_texture,
        journal_scene_view,
        journal_scene_view_generation: 0,
        bloom_extract_pipeline,
        bloom_blur_pipeline,
        bloom_composite_pipeline,
        bloom_bind_group_layout,
        bloom_extract_bind_group_layout,
        bloom_composite_bind_group_layout,
        bloom_extract_params_buffer,
        bloom_blur_h_params_buffer,
        bloom_blur_v_params_buffer,
        bloom_composite_params_buffer,
        bloom_sampler,
        bloom_scene_bind_group,
        bloom_ping_bind_group,
        bloom_pong_bind_group,
        bloom_composite_bind_group,
        bloom_ping_texture,
        bloom_ping_view,
        bloom_pong_texture,
        bloom_pong_view,
        post_bloom_texture,
        post_bloom_view,
        tonemap_pipeline,
        tonemap_rgba16f_pipeline,
        tonemap_bind_group_layout,
        tonemap_params_buffer,
        tonemap_bind_group,
        tonemap_bind_group_scene,
        frame_buffer_pool,
        tonemap_shader_module,
        tonemap_pipeline_layout,
        swapchain_sdr_format,
        swapchain_hdr_available,
        tonemap_exposure: 1.0,
        tonemap_vhs_enabled: false,
        tonemap_vhs_chromatic: 0.001,
        tonemap_vhs_scanline: 0.040,
        tonemap_vhs_grain: 0.020,
        tonemap_vhs_vignette: 0.100,
        main_menu_effects: crate::main_menu_effects_tuning::MainMenuEffectsTuning::load(),
        lit_mesh_pipeline,
        lit_mesh_blended_pipeline,
        lit_mesh_white_view,
        lit_mesh_relief_default_view,
        talisman_height_views,
        talisman_mask_views,
        memorial_talisman_height_views,
        memorial_talisman_mask_views,
        talisman_meshes: rustc_hash::FxHashMap::default(),
        memorial_talisman_meshes: rustc_hash::FxHashMap::default(),
        talisman_meshes_ready: false,
        talisman_slot_kind,
        relic_box_mesh,
        relic_box_tris,
        relic_tri_lists: rustc_hash::FxHashMap::default(),
        pack_mesh,
        relic_meshes: rustc_hash::FxHashMap::default(),
        relic_instances,
        point_shadow_array,
        spot_shadow_array,
        shadow_sample_layout,
        shadow_compare_sampler,
        shadow_ao_sampler,
        _shadow_ao_white_texture: shadow_ao_white_texture,
        shadow_ao_white_view,
        _shadow_baked_depth_dummy_texture: shadow_baked_depth_dummy_texture,
        shadow_baked_depth_dummy_view,
        shadow_caster_layout,
        shadow_warp_disabled_bind_group,
        shadow_globals_buffer,
        shadow_sample_bind_group,
        shadow_pipeline,
        shadow_pipeline_room_env,
        gpu_profiler,
        pending_screenshot: std::cell::Cell::new(None),
        acquire_telemetry: super::super::runtime::AcquireTelemetry::new(),
        shadow_quality: default_shadow_quality,
        flame_tuning: crate::flame_tuning::FlameTuning::load(),
        main_menu_pride_rainbow_debug:
            crate::main_menu_glb::main_menu_pride_rainbow_default_enabled(),
        main_menu_moon_phase_debug: crate::main_menu_moon_tuning::MainMenuMoonPhaseDebug::default(),
        main_menu_rain_hit_debug_mesh,
        main_menu_rain_hit_debug_instance,
        probe_gi_stale_aabb_warned_room: None,
    })
}
