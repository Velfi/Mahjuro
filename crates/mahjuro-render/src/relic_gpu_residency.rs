//! Relic texture GPU residency (LRU cap on Low memory).

use std::collections::{HashSet, VecDeque};

use mahjuro_core::core::relic::RelicId;

use crate::gpu_types::RelicGpuMeta;

pub(crate) fn bc7_mip_bytes(width: u32, height: u32) -> usize {
    ((width + 3) / 4) as usize * ((height + 3) / 4) as usize * 16
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
