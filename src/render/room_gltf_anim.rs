//! glTF node TRS animation sampling for embedded room meshes.

use std::time::Instant;

use anyhow::Context as _;
use glam::{Mat4, Quat, Vec3};
use rustc_hash::FxHashMap;

use crate::render::room_env_gltf::{RoomEnvPrimitiveCpu, RoomNodeBindPose};

/// TRS channels for one animated glTF node (`translation` / `rotation`; scale from bind or keys).
#[derive(Clone, Debug)]
pub struct RoomNodeAnimClip {
    pub times: Vec<f32>,
    pub translations: Vec<Vec3>,
    pub rotations: Vec<Quat>,
    pub scales: Vec<Vec3>,
    pub duration_secs: f32,
}

impl RoomNodeAnimClip {
    /// Local TRS in glTF node space at `t` (seconds; missing channels use bind pose).
    pub fn local_matrix_at(
        &self,
        t: f32,
        bind_translation: Vec3,
        bind_rotation: Quat,
        bind_scale: Vec3,
    ) -> Mat4 {
        let (i0, i1, alpha) = segment_index(&self.times, t);
        let translation = if self.translations.is_empty() {
            bind_translation
        } else {
            self.translations[i0].lerp(self.translations[i1], alpha)
        };
        let rotation = if self.rotations.is_empty() {
            bind_rotation
        } else {
            self.rotations[i0].slerp(self.rotations[i1], alpha)
        };
        let scale = if self.scales.is_empty() {
            bind_scale
        } else {
            self.scales[i0].lerp(self.scales[i1], alpha)
        };
        Mat4::from_scale_rotation_translation(scale, rotation, translation)
    }
}

/// Bind pose + sampled clip for one animated glTF node.
#[derive(Clone, Debug)]
pub struct RoomNodeAnim {
    pub node_name: String,
    bind_world_doc: Mat4,
    parent_world_doc: Mat4,
    bind_translation: Vec3,
    bind_rotation: Quat,
    bind_scale: Vec3,
    pub clip: RoomNodeAnimClip,
}

impl RoomNodeAnim {
    /// Right-multiply baked node vertices: `animated_doc = delta * baked_doc`.
    pub fn delta_doc_at(&self, t: f32) -> Mat4 {
        let animated = self.parent_world_doc
            * self.clip.local_matrix_at(
                t,
                self.bind_translation,
                self.bind_rotation,
                self.bind_scale,
            );
        animated * self.bind_world_doc.inverse()
    }
}

/// All named glTF animation clips parsed from a room GLB.
#[derive(Clone, Debug, Default)]
pub struct RoomGltfAnimLibrary {
    pub clips: FxHashMap<String, Vec<RoomNodeAnim>>,
}

impl RoomGltfAnimLibrary {
    pub fn clip_duration(&self, clip_name: &str) -> Option<f32> {
        self.clips.get(clip_name).and_then(|nodes| {
            nodes
                .iter()
                .map(|n| n.clip.duration_secs)
                .max_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal))
        })
    }

    pub fn has_clip(&self, clip_name: &str) -> bool {
        self.clips.contains_key(clip_name)
    }
}

/// GPU-side clip → primitive bindings for room env draws.
#[derive(Clone, Debug, Default)]
pub struct RoomGltfAnimGpu {
    pub library: RoomGltfAnimLibrary,
    /// clip name → [(primitive index, node anim index in clip)]
    pub clip_prim_bindings: FxHashMap<String, Vec<(usize, usize)>>,
}

impl RoomGltfAnimGpu {
    pub fn from_room_cpu(
        library: &RoomGltfAnimLibrary,
        prims: &[RoomEnvPrimitiveCpu],
        asset_label: &str,
    ) -> Self {
        let mut node_prim_indices: FxHashMap<String, Vec<usize>> = FxHashMap::default();
        for (i, prim) in prims.iter().enumerate() {
            if let Some(name) = prim.gltf_node_name.as_deref() {
                node_prim_indices
                    .entry(name.to_string())
                    .or_default()
                    .push(i);
            }
        }

        let mut clip_prim_bindings = FxHashMap::default();
        for (clip_name, node_anims) in &library.clips {
            let mut bindings = Vec::new();
            for (anim_idx, anim) in node_anims.iter().enumerate() {
                match node_prim_indices.get(&anim.node_name) {
                    Some(prim_indices) => {
                        for &pi in prim_indices {
                            bindings.push((pi, anim_idx));
                        }
                    }
                    None => log::warn!(
                        "{asset_label} anim {clip_name}: no mesh primitive for node {}",
                        anim.node_name
                    ),
                }
            }
            if bindings.is_empty() {
                log::warn!("{asset_label} anim {clip_name}: no GPU primitive bindings");
            } else {
                clip_prim_bindings.insert(clip_name.clone(), bindings);
            }
        }

        Self {
            library: library.clone(),
            clip_prim_bindings,
        }
    }

    pub fn resolve_prim_deltas(&self, samples: &[(String, f32)]) -> FxHashMap<usize, Mat4> {
        let mut out = FxHashMap::default();
        for (clip_name, t) in samples {
            let Some(bindings) = self.clip_prim_bindings.get(clip_name) else {
                continue;
            };
            let Some(node_anims) = self.library.clips.get(clip_name) else {
                continue;
            };
            for &(pi, anim_idx) in bindings {
                if let Some(anim) = node_anims.get(anim_idx) {
                    out.insert(pi, anim.delta_doc_at(*t));
                }
            }
        }
        out
    }

    pub fn has_clip(&self, clip_name: &str) -> bool {
        self.clip_prim_bindings.contains_key(clip_name)
    }
}

/// Parse all glTF animations from a room blob after the scene walk captures bind poses.
pub fn parse_gltf_anim_library(
    data: &[u8],
    node_binds: &FxHashMap<String, RoomNodeBindPose>,
    asset_label: &str,
) -> RoomGltfAnimLibrary {
    let Ok((document, buffers_vec, _images)) = gltf::import_slice(data) else {
        log::warn!("{asset_label}: glTF import failed while parsing animations");
        return RoomGltfAnimLibrary::default();
    };
    let buffers: Vec<Vec<u8>> = buffers_vec.into_iter().map(|b| b.0).collect();

    let mut node_names: Vec<Option<String>> = vec![None; document.nodes().len()];
    for node in document.nodes() {
        node_names[node.index()] = node.name().map(str::to_string);
    }

    let mut clips = FxHashMap::default();
    for anim in document.animations() {
        let clip_name = anim
            .name()
            .map(str::to_string)
            .unwrap_or_else(|| format!("animation_{}", anim.index()));

        let mut per_node: FxHashMap<usize, NodeChannelHarvest> = FxHashMap::default();
        for channel in anim.channels() {
            let node_idx = channel.target().node().index();
            let harvest = per_node.entry(node_idx).or_default();
            let sampler = channel.sampler();
            if harvest.times.is_none() {
                harvest.times = read_accessor_f32(sampler.input(), &buffers).ok();
            }
            match channel.target().property() {
                gltf::animation::Property::Translation => {
                    if let Ok(v) = read_accessor_vec3(sampler.output(), &buffers) {
                        harvest.translations = v;
                    }
                }
                gltf::animation::Property::Rotation => {
                    if let Ok(qs) = read_accessor_vec4(sampler.output(), &buffers) {
                        harvest.rotations = qs
                            .into_iter()
                            .map(|q| Quat::from_xyzw(q[0], q[1], q[2], q[3]).normalize())
                            .collect();
                    }
                }
                gltf::animation::Property::Scale => {
                    if let Ok(v) = read_accessor_vec3(sampler.output(), &buffers) {
                        harvest.scales = v;
                    }
                }
                _ => {}
            }
        }

        let mut node_anims = Vec::new();
        for (node_idx, harvest) in per_node {
            let Some(node_name) = node_names
                .get(node_idx)
                .and_then(|n| n.as_deref())
                .filter(|n| !n.is_empty())
            else {
                log::warn!(
                    "{asset_label} anim {clip_name}: skipping unnamed node index {node_idx}"
                );
                continue;
            };
            let Some(bind) = node_binds.get(node_name) else {
                log::warn!(
                    "{asset_label} anim {clip_name}: no bind pose for node {node_name}"
                );
                continue;
            };
            let Some(times) = harvest.times else {
                log::warn!(
                    "{asset_label} anim {clip_name}: node {node_name} has no key times"
                );
                continue;
            };
            if times.is_empty() {
                log::warn!(
                    "{asset_label} anim {clip_name}: node {node_name} has empty key times"
                );
                continue;
            }
            if !harvest.translations.is_empty() && harvest.translations.len() != times.len() {
                log::warn!(
                    "{asset_label} anim {clip_name}: node {node_name} translation key count mismatch"
                );
                continue;
            }
            if !harvest.rotations.is_empty() && harvest.rotations.len() != times.len() {
                log::warn!(
                    "{asset_label} anim {clip_name}: node {node_name} rotation key count mismatch"
                );
                continue;
            }
            if !harvest.scales.is_empty() && harvest.scales.len() != times.len() {
                log::warn!(
                    "{asset_label} anim {clip_name}: node {node_name} scale key count mismatch"
                );
                continue;
            }

            let duration_secs = times.last().copied().unwrap_or(0.0);
            let bind_local = bind
                .parent_world_doc
                .inverse()
                .mul_mat4(&bind.bind_world_doc);
            let (bind_scale, bind_rotation, bind_translation) =
                bind_local.to_scale_rotation_translation();
            node_anims.push(RoomNodeAnim {
                node_name: node_name.to_string(),
                bind_world_doc: bind.bind_world_doc,
                parent_world_doc: bind.parent_world_doc,
                bind_translation,
                bind_rotation,
                bind_scale,
                clip: RoomNodeAnimClip {
                    times,
                    translations: harvest.translations,
                    rotations: harvest.rotations,
                    scales: harvest.scales,
                    duration_secs,
                },
            });
        }

        if node_anims.is_empty() {
            log::warn!("{asset_label} anim {clip_name}: no usable node channels");
            continue;
        }
        log::info!(
            "{asset_label}: loaded glTF anim {clip_name} ({:.2}s, {} node(s))",
            node_anims
                .iter()
                .map(|n| n.clip.duration_secs)
                .fold(0.0f32, f32::max),
            node_anims.len()
        );
        clips.insert(clip_name, node_anims);
    }

    RoomGltfAnimLibrary { clips }
}

#[derive(Default)]
struct NodeChannelHarvest {
    times: Option<Vec<f32>>,
    translations: Vec<Vec3>,
    rotations: Vec<Quat>,
    scales: Vec<Vec3>,
}

/// Runtime playback state for one named glTF clip.
#[derive(Clone, Debug)]
pub struct GltfAnimPlayback {
    pub duration_secs: f32,
    started: Option<Instant>,
    elapsed_secs: f32,
    paused: bool,
    pub looping: bool,
}

impl GltfAnimPlayback {
    pub fn new(duration_secs: f32, looping: bool) -> Self {
        Self {
            duration_secs,
            started: Some(Instant::now()),
            elapsed_secs: 0.0,
            paused: false,
            looping,
        }
    }

    pub fn is_active(&self) -> bool {
        self.started.is_some() || self.paused
    }

<<<<<<< HEAD
    pub fn restart(&mut self, duration_secs: f32) {
        self.duration_secs = duration_secs;
        self.elapsed_secs = 0.0;
        self.started = Some(Instant::now());
        self.paused = false;
    }
=======
    let (raw_bind_scale, _, _) = bind_world_doc.to_scale_rotation_translation();
    let bind_scale = if raw_bind_scale.is_finite()
        && raw_bind_scale.x.abs() > 1e-5
        && raw_bind_scale.y.abs() > 1e-5
        && raw_bind_scale.z.abs() > 1e-5
    {
        raw_bind_scale
    } else {
        log::warn!(
            "shop.glb eyeball_travel: invalid bind scale {:?} — falling back to Vec3::ONE",
            raw_bind_scale
        );
        Vec3::ONE
    };
>>>>>>> 3ab1ff47 (Re-did pedestals in blender for Gameplay.glb, also added abalone coin bowl.)

    pub fn resume(&mut self) {
        if self.paused {
            self.started = Some(Instant::now());
            self.paused = false;
        }
    }

    pub fn pause(&mut self) {
        if self.paused {
            return;
        }
        let now = Instant::now();
        if let Some(started) = self.started.take() {
            self.elapsed_secs += now.saturating_duration_since(started).as_secs_f32();
        }
        self.paused = true;
    }

    pub fn playback_sec(&self) -> Option<f32> {
        let elapsed = if self.paused {
            self.elapsed_secs
        } else {
            let started = self.started?;
            self.elapsed_secs + Instant::now().saturating_duration_since(started).as_secs_f32()
        };
        if !self.looping && elapsed >= self.duration_secs {
            return None;
        }
        Some(if self.looping && self.duration_secs > 0.0 {
            elapsed % self.duration_secs
        } else {
            elapsed
        })
    }
}

/// Multiple simultaneous glTF clip playbacks (e.g. shop room props).
#[derive(Clone, Debug, Default)]
pub struct GltfAnimPlaybackSet {
    active: FxHashMap<String, GltfAnimPlayback>,
}

impl GltfAnimPlaybackSet {
    pub fn play(&mut self, clip_name: impl Into<String>, duration_secs: f32, looping: bool) {
        let clip_name = clip_name.into();
        match self.active.get_mut(&clip_name) {
            Some(pb) => {
                pb.duration_secs = duration_secs;
                pb.looping = looping;
                pb.resume();
            }
            None => {
                self.active
                    .insert(clip_name, GltfAnimPlayback::new(duration_secs, looping));
            }
        }
    }

    pub fn restart(&mut self, clip_name: &str, duration_secs: f32) -> bool {
        if let Some(pb) = self.active.get_mut(clip_name) {
            pb.restart(duration_secs);
            true
        } else {
            self.active.insert(
                clip_name.to_string(),
                GltfAnimPlayback::new(duration_secs, false),
            );
            true
        }
    }

    pub fn toggle_pause(&mut self, clip_name: &str) -> Option<bool> {
        let pb = self.active.get_mut(clip_name)?;
        if pb.paused {
            pb.resume();
            Some(false)
        } else {
            pb.pause();
            Some(true)
        }
    }

    pub fn stop(&mut self, clip_name: &str) {
        self.active.remove(clip_name);
    }

    pub fn is_playing(&self, clip_name: &str) -> bool {
        self.active.contains_key(clip_name)
    }

    /// Read active samples without removing finished clips.
    pub fn active_samples(&self) -> Vec<(String, f32)> {
        self.active
            .iter()
            .filter_map(|(clip, pb)| pb.playback_sec().map(|t| (clip.clone(), t)))
            .collect()
    }

    /// Drop finished non-looping clips.
    pub fn prune_finished(&mut self) {
        self.active.retain(|_, pb| pb.playback_sec().is_some());
    }

    /// Collect active samples and drop finished non-looping clips.
    pub fn collect_samples(&mut self) -> Vec<(String, f32)> {
        let out = self.active_samples();
        self.prune_finished();
        out
    }
}

#[cfg(test)]
mod tests {
    use glam::{Mat4, Vec3};

    fn eyeball_travel_anim() -> crate::render::room_gltf_anim::RoomNodeAnim {
        crate::asset_path::init();
        let file = crate::asset_path::get("3d/shop.glb").expect("shop.glb embedded");
        let cpu =
            crate::render::room_glb::load_shop_glb_from_bytes(&file.data).expect("load shop");
        cpu.gltf_anim_library
            .clips
            .get("eyeball_travel")
            .and_then(|nodes| nodes.iter().find(|n| n.node_name == "Eyeball").cloned())
            .expect("eyeball_travel clip on Eyeball")
    }

    #[test]
    fn shop_eyeball_travel_parses_and_moves() {
        let anim = eyeball_travel_anim();
        let probe = Vec3::new(0.5, 0.0, 0.0);
        let p0 = anim.delta_doc_at(0.0).transform_point3(probe);
        let p_mid = anim.delta_doc_at(8.0).transform_point3(probe);
        assert!(
            (p_mid - p0).length() > 0.05,
            "expected visible motion, p0={p0:?} p_mid={p_mid:?}"
        );
    }

    #[test]
    fn shop_eyeball_travel_delta_is_rigid() {
        let anim = eyeball_travel_anim();
        for t in [0.0, 4.0, 8.0, 16.0] {
            let delta = anim.delta_doc_at(t);
            assert!(
                delta.abs_diff_eq(Mat4::IDENTITY, 1e-3) || delta_is_rigid(delta, 0.05),
                "delta at t={t} should be rigid (near-identity or uniform scale + rotation)"
            );
        }
    }

    fn delta_is_rigid(delta: Mat4, eps: f32) -> bool {
        let x = delta.transform_vector3(Vec3::X).length();
        let y = delta.transform_vector3(Vec3::Y).length();
        let z = delta.transform_vector3(Vec3::Z).length();
        (x - y).abs() < eps && (y - z).abs() < eps && (0.9..=1.1).contains(&x)
    }
}

fn read_accessor_f32(accessor: gltf::Accessor<'_>, buffers: &[Vec<u8>]) -> anyhow::Result<Vec<f32>> {
    anyhow::ensure!(
        accessor.dimensions() == gltf::accessor::Dimensions::Scalar,
        "expected scalar accessor"
    );
    let data = accessor_data(accessor, buffers)?;
    Ok(data
        .chunks_exact(4)
        .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
        .collect())
}

fn read_accessor_vec3(accessor: gltf::Accessor<'_>, buffers: &[Vec<u8>]) -> anyhow::Result<Vec<Vec3>> {
    anyhow::ensure!(
        accessor.dimensions() == gltf::accessor::Dimensions::Vec3,
        "expected vec3 accessor"
    );
    let data = accessor_data(accessor, buffers)?;
    Ok(data
        .chunks_exact(12)
        .map(|c| {
            Vec3::new(
                f32::from_le_bytes([c[0], c[1], c[2], c[3]]),
                f32::from_le_bytes([c[4], c[5], c[6], c[7]]),
                f32::from_le_bytes([c[8], c[9], c[10], c[11]]),
            )
        })
        .collect())
}

fn read_accessor_vec4(accessor: gltf::Accessor<'_>, buffers: &[Vec<u8>]) -> anyhow::Result<Vec<[f32; 4]>> {
    anyhow::ensure!(
        accessor.dimensions() == gltf::accessor::Dimensions::Vec4,
        "expected vec4 accessor"
    );
    let data = accessor_data(accessor, buffers)?;
    Ok(data
        .chunks_exact(16)
        .map(|c| {
            [
                f32::from_le_bytes([c[0], c[1], c[2], c[3]]),
                f32::from_le_bytes([c[4], c[5], c[6], c[7]]),
                f32::from_le_bytes([c[8], c[9], c[10], c[11]]),
                f32::from_le_bytes([c[12], c[13], c[14], c[15]]),
            ]
        })
        .collect())
}

fn accessor_data<'a>(
    accessor: gltf::Accessor<'_>,
    buffers: &'a [Vec<u8>],
) -> anyhow::Result<&'a [u8]> {
    let view = accessor.view().context("accessor view")?;
    let buffer = buffers.get(view.buffer().index()).context("buffer index")?;
    let start = view.offset() + accessor.offset();
    let stride = view.stride().unwrap_or(accessor.size());
    let end = start + accessor.count() * stride;
    buffer
        .get(start..end)
        .context("accessor byte range out of bounds")
}

fn segment_index(times: &[f32], t: f32) -> (usize, usize, f32) {
    if times.len() <= 1 {
        return (0, 0, 0.0);
    }
    if t <= times[0] {
        return (0, 0, 0.0);
    }
    let last = times.len() - 1;
    if t >= times[last] {
        return (last, last, 0.0);
    }
    for i in 0..last {
        if t < times[i + 1] {
            let span = (times[i + 1] - times[i]).max(1e-8);
            let alpha = (t - times[i]) / span;
            return (i, i + 1, alpha);
        }
    }
    (last, last, 0.0)
}
