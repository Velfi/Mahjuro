/// Cheap deterministic hash of `(label, width, height)` used as the cache
/// key for rasterised tablet decals — when this changes, the renderer
/// re-rasterises the engraved label texture and uploads it. FNV-1a 64.
pub(crate) fn tablet_label_hash(label: &str, w: u32, h: u32) -> u64 {
    let mut hash: u64 = 0xcbf29ce484222325;
    for b in label.as_bytes() {
        hash ^= *b as u64;
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash ^= w as u64;
    hash = hash.wrapping_mul(0x100000001b3);
    hash ^= h as u64;
    hash.wrapping_mul(0x100000001b3)
}
