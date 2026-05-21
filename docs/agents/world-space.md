# 3D world space (HUD props)

**World space** is the shared right-handed **Z-up** 3D frame for gameplay and shop. The **table** (felt, wood, dishes) lies in the **XY** plane near **`z = 0`**. Meshes use packed [`WorldSurfaceAnchor`](../../src/render/draw_cmd.rs) values: `[pixel_x, pixel_y, lift]` where `lift` is height in world **+Z**. The renderer maps them with [`pixel_to_world`](../../src/render/world_space.rs). Lit-mesh model matrices use [`translate_rot_scale`](../../src/render/table_transform.rs) with world-space centers and rotations from the same helpers (no separate "Z-up wrapper" layer). Gameplay bottom-bar spacing and lift live in [`action_bar_layout.rs`](../../src/scenes/gameplay/action_bar_layout.rs).

For screen placement and [`Placement`](../../src/ui/placement.rs), see [`PlacementAnchor`](../../src/ui/placement.rs) (composes layout degrees into `Object3d` rotation). Note: [`PlacementAnchor`](../../src/render/world_space.rs) in `world_space.rs` is the score-reel / HUD helper — different from [`ui::placement::PlacementAnchor`](../../src/ui/placement.rs).

**Conventions:** **+X** right, **+Z** up; larger layout `py` moves along **+Y** (screen-down on the felt). Mesh orientation uses [`table_transform.rs`](../../src/render/table_transform.rs); each `DrawCmd` placement type matches one rotation recipe there; do not invent parallel Euler orders.
