//! Revolved candle flame mesh — digital-garden lathe profile.
//!
//! Vertex **position** stores `[cos_theta, y01, sin_theta]` for the plume vertex
//! shader; **uv** stores rest radial fraction for the cap ring.

use crate::lit_mesh::MeshCpu;
use crate::plume_sim::{FLAME_BASE, flame_envelope_width, flame_height_at, y01_from_height};
use crate::tile_glb::Vertex3dTex;

const CAP_STEPS: usize = 6;
const FLAME_HEIGHT: f32 = 1.0;

struct LatheDetail {
    radial: usize,
    height: usize,
}

const DETAIL: LatheDetail = LatheDetail {
    radial: 36,
    height: 20,
};

fn append_flame_cap_profile(profile: &mut Vec<[f32; 2]>) -> f32 {
    let merge_y01 = 0.09;
    let cap_r = flame_envelope_width(merge_y01).max(0.007);
    let cap_top_y = flame_height_at(merge_y01, FLAME_HEIGHT);

    for i in 0..=CAP_STEPS {
        let theta = (i as f32 / CAP_STEPS as f32) * std::f32::consts::FRAC_PI_2;
        profile.push([
            cap_r * theta.sin(),
            FLAME_BASE + (cap_top_y - FLAME_BASE) * (1.0 - theta.cos()),
        ]);
    }

    merge_y01
}

/// Build once at startup; instanced per candle in `flame.wgsl`.
pub fn build_candle_flame_volume_mesh() -> MeshCpu {
    let mut profile: Vec<[f32; 2]> = Vec::new();
    let merge_y01 = append_flame_cap_profile(&mut profile);
    let start_i = ((merge_y01 * DETAIL.height as f32).ceil() as usize).max(1);

    for i in start_i..=DETAIL.height {
        let y01 = i as f32 / DETAIL.height as f32;
        profile.push([
            flame_envelope_width(y01),
            flame_height_at(y01, FLAME_HEIGHT),
        ]);
    }

    let n_rad = DETAIL.radial;
    let n_rings = profile.len();
    let mut vertices: Vec<Vertex3dTex> = Vec::with_capacity(n_rings * (n_rad + 1));
    let mut indices: Vec<u32> = Vec::with_capacity((n_rings - 1) * n_rad * 6);

    for (ring, [width, y]) in profile.iter().enumerate() {
        let y01 = y01_from_height(*y, FLAME_HEIGHT);
        for ri in 0..=n_rad {
            let theta = (ri as f32 / n_rad as f32) * std::f32::consts::TAU;
            let (sin_t, cos_t) = theta.sin_cos();
            vertices.push(Vertex3dTex {
                position: [cos_t, y01, sin_t],
                normal: [0.0, 1.0, 0.0],
                uv: [ri as f32 / n_rad as f32, *width / width.max(1e-6)],
                tangent: Vertex3dTex::DEFAULT_TANGENT,
                uv_emr: [0.0, 0.0],
                color: [1.0, 1.0, 1.0, 1.0],
            });
            let _ = ring;
        }
    }

    let row = (n_rad + 1) as u32;
    for ring in 0..(n_rings - 1) {
        for ri in 0..n_rad {
            let i0 = ring as u32 * row + ri as u32;
            let i1 = i0 + 1;
            let i2 = i0 + row;
            let i3 = i2 + 1;
            indices.extend_from_slice(&[i0, i2, i1, i1, i2, i3]);
        }
    }

    MeshCpu {
        vertices,
        indices,
        default_material: crate::lit_mesh::MaterialParams::wick(),
    }
}
