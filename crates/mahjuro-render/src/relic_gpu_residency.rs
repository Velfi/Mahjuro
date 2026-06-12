//! Relic texture GPU residency (LRU cap on Low memory).

use std::collections::{HashSet, VecDeque};

use mahjuro_core::core::relic::RelicId;

use crate::gpu_types::RelicGpuMeta;

pub(crate) fn bc7_mip_bytes(width: u32, height: u32) -> usize {
    ((width + 3) / 4) as usize * ((height + 3) / 4) as usize * 16
}

/// BC7 uses 4×4 blocks; wgpu rejects `Queue::write_texture` below that size.
pub(crate) fn bc7_mip_level_count(base_w: u32, base_h: u32) -> u32 {
    let mut count = 0u32;
    let mut w = base_w.max(4);
    let mut h = base_h.max(4);
    loop {
        count += 1;
        if w == 4 && h == 4 {
            break;
        }
        w = (w / 2).max(4);
        h = (h / 2).max(4);
    }
    count
}

pub(crate) fn bc7_chain_bytes(base_w: u32, base_h: u32, mip_count: u32) -> usize {
    let mut total = 0usize;
    let mut w = base_w.max(1);
    let mut h = base_h.max(1);
    for _ in 0..mip_count.max(1) {
        total += bc7_mip_bytes(w, h);
        w = (w / 2).max(1);
        h = (h / 2).max(1);
    }
    total
}

/// Bytes for the BC7 mip subset that is safe to upload (stops at 4×4).
pub(crate) fn bc7_upload_chain_bytes(base_w: u32, base_h: u32) -> usize {
    bc7_chain_bytes(base_w, base_h, bc7_mip_level_count(base_w, base_h))
}

pub(crate) fn rgba_mip_bytes(width: u32, height: u32, mip_count: u32) -> usize {
    let mut total = 0usize;
    let mut w = width.max(1);
    let mut h = height.max(1);
    for _ in 0..mip_count.max(1) {
        total += (w as usize) * (h as usize) * 4;
        w = (w / 2).max(1);
        h = (h / 2).max(1);
    }
    total
}

pub(crate) fn estimate_relic_texture_gpu_bytes(meta: &RelicGpuMeta) -> usize {
    meta.albedo_bytes + meta.relief_bytes + meta.mesh_bytes
}

pub(crate) fn total_relic_gpu_bytes(meta_map: &rustc_hash::FxHashMap<RelicId, RelicGpuMeta>) -> usize {
    meta_map.values().map(estimate_relic_texture_gpu_bytes).sum()
}

pub(crate) fn touch_relic_lru(lru: &mut VecDeque<RelicId>, id: RelicId) {
    if let Some(i) = lru.iter().position(|&r| r == id) {
        lru.remove(i);
    }
    lru.push_front(id);
}

pub(crate) fn trim_relic_lru(
    lru: &mut VecDeque<RelicId>,
    cap: usize,
    protected: &HashSet<RelicId>,
    evict: &mut impl FnMut(RelicId),
) {
    while lru.len() > cap {
        let Some(idx) = lru
            .iter()
            .enumerate()
            .rev()
            .find(|(_, id)| !protected.contains(id))
            .map(|(i, _)| i)
        else {
            break;
        };
        let id = lru.remove(idx).expect("relic lru index");
        evict(id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bc7_mip_level_count_stops_at_4x4() {
        assert_eq!(bc7_mip_level_count(1024, 1024), 9);
        assert_eq!(bc7_mip_level_count(512, 512), 8);
        assert_eq!(bc7_mip_level_count(4, 4), 1);
    }

    #[test]
    fn bc7_upload_chain_smaller_than_full_chain() {
        assert!(bc7_upload_chain_bytes(1024, 1024) < bc7_chain_bytes(1024, 1024, 11));
    }
}
