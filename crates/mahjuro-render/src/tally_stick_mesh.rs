//! Authored tally stick meshes — the draws/discards counter fans that stand in
//! front of the Play (mirror) and Discard (river) actions.
//!
//! Local space: pivot at the narrow base, stick extends along `+Y` from
//! `y=0` to `y=1`, and thickness is along `Z`. A per-instance scale of
//! `(w, len, t)` maps directly to a stick of width `w`, length `len`, and
//! thickness `t` in world units.

use crate::lit_mesh::{MaterialParams, MeshCpu};
use crate::tile_glb::Vertex3dTex;

pub const PLAY_TALLY_STICK_GLB_PATH: &str = "3d/play_tally_stick.glb";
pub const DISCARD_TALLY_STICK_GLB_PATH: &str = "3d/discard_tally_stick.glb";

fn axis_order_from_extents(extents: [f32; 3]) -> (usize, usize, usize) {
    let mut axes = [0_usize, 1, 2];
    axes.sort_by(|&a, &b| {
        extents[b]
            .partial_cmp(&extents[a])
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    (axes[0], axes[1], axes[2])
}

fn remap_normal(
    n: [f32; 3],
    width_axis: usize,
    length_axis: usize,
    thickness_axis: usize,
) -> [f32; 3] {
    let mapped = glam::Vec3::new(n[width_axis], n[length_axis], n[thickness_axis]);
    mapped.normalize_or_zero().to_array()
}

fn tally_stick_bounds<'a>(vertices: impl Iterator<Item = &'a Vertex3dTex>) -> ([f32; 3], [f32; 3]) {
    let mut min = [f32::MAX; 3];
    let mut max = [f32::MIN; 3];
    for v in vertices {
        for axis in 0..3 {
            min[axis] = min[axis].min(v.position[axis]);
            max[axis] = max[axis].max(v.position[axis]);
        }
    }
    (min, max)
}

fn canonicalize_tally_stick_vertices<'a>(
    vertices: impl Iterator<Item = &'a mut Vertex3dTex>,
    min: [f32; 3],
    max: [f32; 3],
) {
    let extents = [
        (max[0] - min[0]).max(1.0e-6),
        (max[1] - min[1]).max(1.0e-6),
        (max[2] - min[2]).max(1.0e-6),
    ];
    let (length_axis, width_axis, thickness_axis) = axis_order_from_extents(extents);
    let center_width = (min[width_axis] + max[width_axis]) * 0.5;
    let center_thickness = (min[thickness_axis] + max[thickness_axis]) * 0.5;

    for v in vertices {
        let p = v.position;
        v.position = [
            (p[width_axis] - center_width) / extents[width_axis],
            (p[length_axis] - min[length_axis]) / extents[length_axis],
            (p[thickness_axis] - center_thickness) / extents[thickness_axis],
        ];
        v.normal = remap_normal(v.normal, width_axis, length_axis, thickness_axis);
        v.tangent = Vertex3dTex::DEFAULT_TANGENT;
    }
}

fn canonicalize_tally_stick_mesh(mesh: &mut MeshCpu) {
    let (min, max) = tally_stick_bounds(mesh.vertices.iter());
    canonicalize_tally_stick_vertices(mesh.vertices.iter_mut(), min, max);
}

fn canonicalize_tally_stick_tile(tile: &mut crate::tile_glb::LoadedTile) {
    let (min, max) = tally_stick_bounds(tile.primitives.iter().flat_map(|p| p.vertices.iter()));
    for prim in &mut tile.primitives {
        canonicalize_tally_stick_vertices(prim.vertices.iter_mut(), min, max);
    }
}

fn flatten_loaded_tally_stick_mesh(
    loaded: crate::tile_glb::LoadedTile,
    default_material: MaterialParams,
) -> MeshCpu {
    let mut vertices: Vec<Vertex3dTex> = Vec::new();
    let mut indices: Vec<u32> = Vec::new();

    for prim in loaded.primitives {
        let base = vertices.len() as u32;
        vertices.extend(prim.vertices);
        indices.extend(prim.indices.into_iter().map(|idx| base + idx));
    }

    let mut mesh = MeshCpu {
        vertices,
        indices,
        default_material,
    };
    canonicalize_tally_stick_mesh(&mut mesh);
    mesh
}

pub fn load_tally_stick_glb_tile(path: &str) -> crate::tile_glb::LoadedTile {
    let file = mahjuro_assets::asset_path::get(path)
        .unwrap_or_else(|| panic!("required tally stick GLB missing at {path}"));
    let mut loaded = crate::tile_glb::load_glb_tile_from_bytes_with_label(&file.data, path)
        .unwrap_or_else(|e| panic!("could not load required tally stick GLB {path}: {e:#}"));
    canonicalize_tally_stick_tile(&mut loaded);
    loaded
}

pub fn load_tally_stick_glb_mesh(path: &str, default_material: MaterialParams) -> MeshCpu {
    let loaded = load_tally_stick_glb_tile(path);
    flatten_loaded_tally_stick_mesh(loaded, default_material)
}
