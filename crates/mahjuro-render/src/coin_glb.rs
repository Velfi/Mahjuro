//! [`coin.glb`](../../../assets/3d/coin.glb) — authored cash coin mesh for yen piles
//! and flying-coin animations.
//!
//! Only the `coin_punched` node is decoded (the sibling `subtractor` mesh is a
//! boolean helper and is skipped). At runtime the renderer uploads each glTF
//! material slot and draws instances through the glTF PBR path in `tile_3d.wgsl`.

/// glTF node name for the drawable coin mesh in `coin.glb`.
pub const COIN_GLB_NODE: &str = "coin_punched";

#[cfg(test)]
mod tests {
    use crate::tile_glb::{load_glb_tile_from_node_name, normalize_mesh};

    use super::COIN_GLB_NODE;

    #[test]
    fn coin_glb_loads_punched_mesh_only() {
        let bytes = include_bytes!("../../../assets/3d/coin.glb");
        let mut tile =
            load_glb_tile_from_node_name(bytes, Some(COIN_GLB_NODE)).expect("coin.glb");
        normalize_mesh(&mut tile);
        assert!(
            tile.primitives.iter().map(|p| p.vertices.len()).sum::<usize>() > 100,
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
