# Scene placement — screen, world, rotation, arrange

One mental model for moving 3D props: **where on the window**, **where in Z‑up space**, **how it’s oriented**, and **how arrange mode edits persist**.

## 1. Screen-relative position

Authors usually think in **layout pixels** `(px, py)` with `py` downward, plus **lift** above the felt.

| Mechanism | Use when |
|-----------|----------|
| **[`Placement`](../../src/ui/placement.rs)** (`nx`, `ny`, `lift_mm`) | Saved in JSON; resized with window; drives **arrange mode** and hierarchy picker. |
| **Offsets on a Cassowary anchor** | Same `Placement` fields, interpreted as **fractional offsets** from a rect (hand strip, score panel, etc.). |
| **[`ScreenAnchor`](../../src/render/screen_space_3d.rs)** | One-off responsive props: norm vs px per axis, pixel nudge, [`ScreenLift`](../../src/render/screen_space_3d.rs). Resolves → [`LayoutAnchorPx`](../../src/render/world_space.rs). |

**Pipeline:** screen fractions / pixels + lift → **[`Object3d::pos`](../../src/render/draw_cmd.rs)** as `[px, py, lift]` (packed “surface anchor” — see [world-space](world-space.md)).

## 2. World space

The renderer’s **Z‑up** frame: table in **XY**, **`world_z = lift`**. Mapping from layout pixels is **[`pixel_to_world`](../../src/render/world_space.rs)** when you need a true `Vec3` center (debug picking, lights, CPU helpers).

Draw paths mostly keep **`pos` in pixel-packed form**; the GPU path applies the same convention as `pixel_to_world`. Don’t mix conventions in one chain.

## 3. Rotation / orientation

| Layer | Representation |
|-------|----------------|
| **`Object3d.rotation`** | **`[f32; 3]`** euler **XYZ radians** ([`rot_euler_xyz_rad`](../../src/render/table_transform.rs)), mesh pose **before** placement-specific arrange spins. |
| **`Placement.rx_deg` / `ry_deg` / `rz_deg`** | **Degrees**, persisted and edited in arrange mode. The renderer **sums** them with live deltas, then applies **`Rz * Ry * Rx`** to the model’s rotation block (see `apply_arrange_override` in [`wgpu_renderer.rs`](../../src/render/wgpu_renderer.rs)). |

**Avoid double application:** if an object is tied to a `Placement`, build it with **[`PlacementAnchor::new`](../../src/ui/placement.rs)** and put **`anchor.object3d_rotation()`** + **`arrange_name: Some(...)`** on the `Object3d`. Do **not** bake `placement.rx_deg`… into `Object3d.rotation` yourself — the renderer folds committed placement rotation via `committed_arrange_rotations`.

For camera‑facing props, use **`camera_facing_euler_xyz_rad`** ([`draw_cmd`](../../src/render/draw_cmd.rs)) and compose extras with **`mat4_to_euler_xyz_rad`** when you need a single euler triple.

## 4. Arrange mode (move / rotate easily)

1. Add a **`Placement`** field to the scene’s `*Positions` struct and register it in **`ArrangeTarget::hierarchy`** ([`Node`](../../src/ui/placement.rs) tree).
2. Implement **`placement_mut` / `placement`** for the canonical dotted name (e.g. `gameplay.score_panel.plaque`).
3. At the draw site, use **`PlacementAnchor::new(...)`** so position uses `nx`/`ny`/`lift_mm` and **`arrange_name`** matches the hierarchy leaf.
4. **Groups** in the hierarchy let players nudge **many leaves at once** ([`apply_arrange`](../../src/ui/placement.rs)).
5. Non‑mesh effects (smoke, wind) use **`ArrangePreview::applied_to`** with the same hierarchy so preview matches commit.

Confirm in arrange mode runs **`apply_arrange_to_layout`** ([`main/arrange.rs`](../../src/main/arrange.rs)); deltas are normalized there and saved per scene layout file.

## Name collision note

**[`render::world_space::PlacementAnchor`](../../src/render/world_space.rs)** (anchor + `rot_y` + `scale`) is for score reel / cascade HUD–style widgets — **not** the same as **`ui::placement::PlacementAnchor`** (Placement → `Object3d`). Pick the type by domain; don’t rename casually.
