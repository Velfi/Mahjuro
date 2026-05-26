//! glTF node TRS animation sampling for embedded room meshes (shop `eyeball_travel`, …).

use anyhow::Context as _;
use glam::{Mat4, Quat, Vec3};

/// TRS channels for one animated glTF node (`translation` / `rotation`; scale stays at bind).
#[derive(Clone, Debug)]
pub struct RoomNodeAnimClip {
    pub times: Vec<f32>,
    pub translations: Vec<Vec3>,
    pub rotations: Vec<Quat>,
    pub duration_secs: f32,
}

impl RoomNodeAnimClip {
    /// Local TRS in glTF document space at `t` (seconds; before first key uses key 0).
    pub fn local_matrix_at(&self, t: f32, scale: Vec3) -> Mat4 {
        let (i0, i1, alpha) = segment_index(&self.times, t);
        let translation = if self.translations.is_empty() {
            Vec3::ZERO
        } else {
            self.translations[i0].lerp(self.translations[i1], alpha)
        };
        let rotation = if self.rotations.is_empty() {
            Quat::IDENTITY
        } else {
            self.rotations[i0].slerp(self.rotations[i1], alpha)
        };
        Mat4::from_scale_rotation_translation(scale, rotation, translation)
    }
}

/// Bind pose + clip for [`shop.glb`](../../assets/3d/shop.glb) `Eyeball` / `eyeball_travel`.
#[derive(Clone, Debug)]
pub struct ShopEyeballTravelAnim {
    pub bind_world_doc: Mat4,
    pub parent_world_doc: Mat4,
    /// Node scale at bind (animation channels do not key scale).
    bind_scale: Vec3,
    pub clip: RoomNodeAnimClip,
}

impl ShopEyeballTravelAnim {
    /// Right-multiply baked Eyeball vertices: `animated_doc = delta * baked_doc`.
    pub fn delta_doc_at(&self, t: f32) -> Mat4 {
        let animated = self.parent_world_doc * self.clip.local_matrix_at(t, self.bind_scale);
        animated * self.bind_world_doc.inverse()
    }
}

/// Parse `eyeball_travel` from a shop glTF blob after [`crate::render::room_glb::RoomGlbCpu`] walk.
pub fn parse_shop_eyeball_travel(
    data: &[u8],
    bind_world_doc: Mat4,
    parent_world_doc: Mat4,
) -> anyhow::Result<ShopEyeballTravelAnim> {
    let (document, buffers_vec, _images) =
        gltf::import_slice(data).context("gltf::import_slice(shop.glb anim)")?;
    let buffers: Vec<Vec<u8>> = buffers_vec.into_iter().map(|b| b.0).collect();

    let mut eyeball_node: Option<usize> = None;
    for node in document.nodes() {
        if node.name() == Some("Eyeball") {
            eyeball_node = Some(node.index());
            break;
        }
    }
    let eyeball_node = eyeball_node.context("shop.glb: Eyeball node not found")?;

    let anim = document
        .animations()
        .find(|a| a.name() == Some("eyeball_travel"))
        .context("shop.glb: eyeball_travel animation not found")?;

    let mut times: Option<Vec<f32>> = None;
    let mut translations: Vec<Vec3> = Vec::new();
    let mut rotations: Vec<Quat> = Vec::new();

    for channel in anim.channels() {
        if channel.target().node().index() != eyeball_node {
            continue;
        }
        let sampler = channel.sampler();
        if times.is_none() {
            times = Some(read_accessor_f32(sampler.input(), &buffers)?);
        }
        match channel.target().property() {
            gltf::animation::Property::Translation => {
                translations = read_accessor_vec3(sampler.output(), &buffers)?;
            }
            gltf::animation::Property::Rotation => {
                rotations = read_accessor_vec4(sampler.output(), &buffers)?
                    .into_iter()
                    .map(|q| Quat::from_xyzw(q[0], q[1], q[2], q[3]).normalize())
                    .collect();
            }
            gltf::animation::Property::Scale => {}
            _ => {}
        }
    }

    let times = times.context("eyeball_travel: no key times")?;
    let duration_secs = times.last().copied().unwrap_or(0.0);
    anyhow::ensure!(!times.is_empty(), "eyeball_travel: empty key times");
    anyhow::ensure!(
        translations.len() == times.len() || translations.is_empty(),
        "eyeball_travel: translation key count mismatch"
    );
    anyhow::ensure!(
        rotations.len() == times.len() || rotations.is_empty(),
        "eyeball_travel: rotation key count mismatch"
    );

    let (_, _, bind_scale) = bind_world_doc.to_scale_rotation_translation();

    Ok(ShopEyeballTravelAnim {
        bind_world_doc,
        parent_world_doc,
        bind_scale,
        clip: RoomNodeAnimClip {
            times,
            translations,
            rotations,
            duration_secs,
        },
    })
}

#[cfg(test)]
mod tests {
    use glam::Vec3;

    #[test]
    fn shop_eyeball_travel_parses_and_moves() {
        crate::asset_path::init();
        let file = crate::asset_path::get("3d/shop.glb").expect("shop.glb embedded");
        let cpu =
            crate::render::room_glb::load_shop_glb_from_bytes(&file.data).expect("load shop");
        let anim = cpu
            .shop_eyeball_travel
            .as_ref()
            .expect("eyeball_travel clip");
        let probe = Vec3::new(0.5, 0.0, 0.0);
        let p0 = anim.delta_doc_at(0.0).transform_point3(probe);
        let p_mid = anim.delta_doc_at(8.0).transform_point3(probe);
        assert!(
            (p_mid - p0).length() > 0.05,
            "expected visible motion, p0={p0:?} p_mid={p_mid:?}"
        );
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
