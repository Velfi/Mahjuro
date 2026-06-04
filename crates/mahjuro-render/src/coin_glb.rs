//! [`coin.glb`](../../../assets/3d/coin.glb) — authored cash coin mesh for yen piles
//! and flying-coin animations.
//!
//! Only the `coin_punched` node is decoded (the sibling `subtractor` mesh is a
//! boolean helper and is skipped). At runtime the renderer uploads each glTF
//! material slot and draws instances through the glTF PBR path in `tile_3d.wgsl`.
//!
//! Load pipeline: reorient to engine axes, center at the origin, keep Blender
//! authored vertex scale. Scene code still passes [`Object3d::extents`] for pile
//! layout; draw time maps those extents onto the authored mesh AABB (see
//! [`layout_scale_for_extents`]) so detail/rim proportions come from Blender
//! without re-normalizing the glTF to a unit cube.

use std::sync::OnceLock;

use glam::Vec3;

/// glTF node name for the drawable coin mesh in `coin.glb`.
pub const COIN_GLB_NODE: &str = "coin_punched";

static COIN_GLB_HALF_EXTENTS: OnceLock<[f32; 3]> = OnceLock::new();

/// Called once after `coin.glb` is reoriented and centered at load.
pub fn init_coin_glb_half_extents(half: [f32; 3]) {
    let _ = COIN_GLB_HALF_EXTENTS.set(half);
}

pub fn coin_glb_half_extents() -> Option<[f32; 3]> {
    COIN_GLB_HALF_EXTENTS.get().copied()
}

/// Map scene layout extents onto the authored mesh size (per-axis).
pub fn layout_scale_for_extents(extents: [f32; 3]) -> Vec3 {
    let half = coin_glb_half_extents().unwrap_or([0.5, 0.5, 0.5]);
    Vec3::new(
        extents[0] / (half[0] * 2.0).max(1e-6),
        extents[1] / (half[1] * 2.0).max(1e-6),
        extents[2] / (half[2] * 2.0).max(1e-6),
    )
}

#[cfg(test)]
mod tests {
    use crate::tile_glb::{
        center_mesh_at_origin, load_glb_tile_from_node_name, normalize_mesh,
        reorient_mesh_to_engine_axes,
    };

    use super::{COIN_GLB_NODE, init_coin_glb_half_extents, layout_scale_for_extents};

    fn mesh_aabb(tile: &crate::tile_glb::LoadedTile) -> ([f32; 3], [f32; 3]) {
        let mut min = [f32::MAX; 3];
        let mut max = [f32::MIN; 3];
        for prim in &tile.primitives {
            for v in &prim.vertices {
                for i in 0..3 {
                    min[i] = min[i].min(v.position[i]);
                    max[i] = max[i].max(v.position[i]);
                }
            }
        }
        let center = [
            (min[0] + max[0]) * 0.5,
            (min[1] + max[1]) * 0.5,
            (min[2] + max[2]) * 0.5,
        ];
        let extent = [max[0] - min[0], max[1] - min[1], max[2] - min[2]];
        (center, extent)
    }

    #[test]
    fn coin_glb_authored_bounds_after_reorient_and_center() {
        let bytes = include_bytes!("../../../assets/3d/coin.glb");
        let mut tile = load_glb_tile_from_node_name(bytes, Some(COIN_GLB_NODE)).expect("coin.glb");
        reorient_mesh_to_engine_axes(&mut tile);
        center_mesh_at_origin(&mut tile);
        let (center, extent) = mesh_aabb(&tile);
        eprintln!("coin reorient+center center={center:?} extent={extent:?}");
        for c in center {
            assert!(c.abs() < 1e-3, "coin mesh should be centered at origin");
        }
        assert!(extent[0].max(extent[1]).max(extent[2]) > 1e-4);
    }

    #[test]
    fn layout_scale_maps_extents_onto_authored_mesh() {
        init_coin_glb_half_extents([0.08, 0.0055, 0.08]);
        let scale = layout_scale_for_extents([1.6, 0.11, 1.6]);
        assert!((scale.x - 10.0).abs() < 1e-3);
        assert!((scale.y - 10.0).abs() < 1e-3);
        assert!((scale.z - 10.0).abs() < 1e-3);
    }

    #[test]
    fn coin_glb_loads_punched_mesh_only() {
        let bytes = include_bytes!("../../../assets/3d/coin.glb");
        let mut tile = load_glb_tile_from_node_name(bytes, Some(COIN_GLB_NODE)).expect("coin.glb");
        normalize_mesh(&mut tile);
        assert!(
            tile.primitives
                .iter()
                .map(|p| p.vertices.len())
                .sum::<usize>()
                > 100,
            "coin_punched should have substantial geometry"
        );
        assert!(
            tile.primitives
                .iter()
                .any(|p| p.albedo_rgba.is_some() || p.metallic_factor > 0.0),
            "coin should carry glTF PBR material data"
        );
    }
}
