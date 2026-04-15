# Shop Spawn Points — Blender GLTF Workflow

## How item placement works today

The shop scene does not load a GLTF environment. Instead, all objects (counter
slab, dishes, lamps, relics, packs, etc.) are procedurally drawn each frame by
`src/scenes/shop.rs` using `Object3dKind` variants defined in
`src/render/draw_cmd.rs`. Their positions come from `ShopPositions` in
`src/ui/scene_layout.rs`, which stores **normalized screen fractions** (`nx`,
`ny` in 0–1) and **lift in millimetres** above the felt.

When we redesign the shop in Blender, we will export a GLB for the static
environment geometry (counter, walls, shelves, etc.) and replace the hardcoded
`ShopPositions` values with positions read directly from named **empty objects**
(spawn points) inside that GLB.

---

## Coordinate spaces

| Space | Origin | Axes | Used for |
|-------|--------|------|----------|
| **Blender world** | Scene origin | X right, Y depth, Z up | Authoring in Blender |
| **GLTF world** | Same origin | X right, Y up, Z toward viewer (Y/Z swapped from Blender) | Inside the `.glb` file |
| **Game world** | Scene origin | X right, Y depth (into screen), Z up | `pixel_to_world()` output |

Blender's GLB exporter emits a root node with a −90° X rotation to convert
Z-up → Y-up. `tile_glb.rs::find_node_transform()` already accumulates this
transform when loading mesh vertices, so it bakes out automatically. Spawn point
positions read from the node graph must go through the **same accumulated
parent transform** so they land in game-world space.

---

## How to add spawn points in Blender

1. **Add → Empty → Plain Axes** at the exact position and orientation you want
   the item to appear.
2. Name it with the convention below (e.g. `spawn.relic.0`).
3. Optionally add **Custom Properties** on the object for extra metadata
   (`slot_type`, `capacity`, etc.) — these export as GLTF `extras`.
4. Export as GLB. Empties export as mesh-less nodes and carry their full
   world-space transform.

The empty's **origin** is the item anchor point. Its **+Z axis** (after the
coordinate conversion) defines "up" for the placed object, so rotating the
empty rotates the item.

---

## Spawn point naming convention

All spawn point names use dot-separated lowercase segments:
`spawn.<category>[.<index>]`

### For-sale slots (counter kiosk)

| Name | Quantity | Purpose |
|------|----------|---------|
| `spawn.relic.0` – `spawn.relic.2` | up to 3 | For-sale relic positions on the counter |
| `spawn.pack.0` | 1 | Tile pack box |
| `spawn.talisman.0` – `spawn.talisman.3` | up to 4 | For-sale talismans |
| `spawn.ribbon.0` – `spawn.ribbon.3` | up to 4 | For-sale zodiac ribbons |

### Owned-item shelf (bottom row)

| Name | Quantity | Purpose |
|------|----------|---------|
| `spawn.relic_dish` | 1 | Center of the owned-relic dish |
| `spawn.talisman_tray` | 1 | Center of the owned-talisman tray |
| `spawn.ribbon_tray` | 1 | Center of the owned-ribbon tray |
| `spawn.coin_dish` | 1 | Center of the coin/gold dish |
| `spawn.sell_tray` | 1 | Sell-return tray |

### Props and UI anchors

| Name | Quantity | Purpose |
|------|----------|---------|
| `spawn.lamp` | 1 | Overhead shop lamp |
| `spawn.restock_prop` | 1 | Restock / reroll action prop |
| `spawn.leave_prop` | 1 | Leave action prop |
| `spawn.book` | 1 | Yaku Journal book |

### Celebration positions (cutscene anchors)

| Name | Quantity | Purpose |
|------|----------|---------|
| `spawn.celeb_pack` | 1 | Pack box position during pack-open closeup |
| `spawn.celeb_reveal_row` | 1 | Left edge of tile reveal row (items spread right from here) |
| `spawn.celeb_ribbon` | 1 | Ribbon position during zodiac celebration |

---

## Reading spawn points in Rust

Add a pass alongside `load_glb_tile_from_bytes` in `src/render/tile_glb.rs`:

```rust
pub struct SpawnPoint {
    pub name: String,
    /// World-space position in game coordinates (X right, Y depth, Z up).
    pub position: glam::Vec3,
    /// World-space orientation.
    pub rotation: glam::Quat,
}

pub fn collect_spawn_points(
    document: &gltf::Document,
    buffers: &[gltf::buffer::Data],
) -> Vec<SpawnPoint> {
    let scene = document
        .default_scene()
        .or_else(|| document.scenes().next());
    let Some(scene) = scene else { return vec![] };

    let mut out = Vec::new();
    for root in scene.nodes() {
        walk_spawns(&root, glam::Mat4::IDENTITY, &mut out);
    }
    out
}

fn walk_spawns(node: &gltf::Node, parent: glam::Mat4, out: &mut Vec<SpawnPoint>) {
    let local = glam::Mat4::from_cols_array_2d(&node.transform().matrix());
    let world = parent * local;

    if node.mesh().is_none() {
        if let Some(name) = node.name() {
            if name.starts_with("spawn.") {
                let (scale, rotation, position) = world.to_scale_rotation_translation();
                out.push(SpawnPoint { name: name.to_string(), position, rotation });
            }
        }
    }

    for child in node.children() {
        walk_spawns(&child, world, out);
    }
}
```

Then in shop startup, replace `ShopPositions::default()` with a function that
maps each `SpawnPoint` by name into the equivalent `nx`/`ny`/`lift_mm` fields,
converting from game-world coordinates back to screen fractions using the
inverse of `pixel_to_world`.

---

## Checklist for a new shop GLB

- [ ] Static geometry in one or more named meshes (counter, walls, shelves…)
- [ ] One empty per spawn point, named per the table above
- [ ] Scene origin at the felt surface centre
- [ ] Export units: **metres** (Blender default); the renderer's `mm_to_world`
      scale handles the rest
- [ ] Verify by printing `collect_spawn_points` output on first load and
      checking positions visually against the old hardcoded layout
