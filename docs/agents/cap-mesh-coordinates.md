# Cap-mesh coordinates (image → local → UV → shader)

**World space** (Z-up table, screen pixels) is documented in [world-space.md](world-space.md). This note covers **lit-mesh cap extrusion**: how PNG silhouettes become model-space geometry and how albedo/heightmap UVs stay paired with vertex normals in WGSL. Full shader reference: [lit mesh shader](lit-mesh-shader.md).

**Source of truth (Rust):** [`cap_extrude.rs`](../../crates/mahjuro-render/src/cap_extrude.rs) — use these helpers instead of ad-hoc `to_local` / `to_uv` closures.

## Two cap families

Both share the same silhouette pipeline (marching squares → Lyon fill → side walls) but differ in which local axis is thin:

| | Relic / ordeal pin | Talisman pendant |
|---|---|---|
| **Kind** | `CapExtrudeKind::RelicPinY` | `CapExtrudeKind::TalismanZ` |
| Cap normal | **+Y** | **+Z** |
| Silhouette plane | **XZ** | **XY** |
| Thickness | **±Y** | **±Z** |
| Image row 0 (top) | smaller local **Z** | larger local **+Y** |
| Y flip in `pixel_to_cap_local` | no | **yes** (`union_cy - py`) |

Builders: [`build_extruded_pin_mesh_from_solid`](../../crates/mahjuro-render/src/relic_dish.rs) and [`build_extruded_talisman_mesh_from_solid`](../../crates/mahjuro-render/src/relic_dish.rs).

## Image → cap local → albedo UV

1. **Union bbox** — all polygon outers share one centroid + extent so multi-island art keeps relative scale (`SilhouetteUnionBounds`).
2. **Cap local** — center on union centroid; wider axis maps to ±0.5. Talisman flips image Y when mapping to cap vertical so “display up” matches the PNG.
3. **Albedo UV** — `pixel_to_albedo_uv`: `[px/w, py/h]`. **Never flip V** in CPU code; GPU v=0 is the top row of the texture.

Parametric meshes that already have cap coords in ~[-0.5, +0.5] use the helpers below — **do not hand-roll** `x + 0.5` / `0.5 - y`.

## UV projection families (parametric meshes)

Two deliberate conventions — pick by how the texture was authored:

| Helper | When to use | +vertical in cap plane → texture v |
|---|---|---|
| `parametric_cap_uv(x, y)` | PNG art read upright (talismans, pack −Y face, ribbon, ofuda +Z) | **low v** |
| `parametric_cap_uv_mirror_u` | Pack +Y back (mirrored U) | low v |
| `planar_y_cap_uv_xz(x, z)` | +Y cap, heightmap projected from above (coin, mirror, bone/wood tablets) | **high v** (+Z) |
| `planar_y_cap_uv_xz_extents(x, z, half_w, half_d)` | +Y cap, non-square footprint (reliquary tray) | high v (+Z) |

Silhouette extrusion albedo always uses `pixel_to_albedo_uv` (no V flip). Only cap-local placement uses `pixel_to_cap_local` + optional Y flip per `CapExtrudeKind`.

**Meshes now routed through `cap_extrude`:** relic/talisman extrusion, pack card, ribbon, ofuda, coin, mirror, reliquary tray, bone tablet, wood tablet.

**Still bespoke (OK):** cabinet hex caps (polar θ), side walls / rims with `[0,0]` placeholder UVs, GLB-imported geometry.

## Height normals (CPU + WGSL)

For flat caps with heightmap relief, gradient normals must match the chitin branch in [`lit_mesh.wgsl`](../../shaders/lit_mesh.wgsl):

| Kind | Front-cap normal from `(dhdu, dhdv)` |
|---|---|
| Relic +Y | `(-dhdu, 1, -dhdv)` |
| Talisman +Z | `(-dhdu, +dhdv, 1)` |

**+dhdv on talisman** because texture v increases toward −local Y while cap “up” is +local Y.

Inspect orbit uses the same flat slab mesh as shelf view; carved relief comes from the chitin shader heightmap bump only.

## Placement (world Z-up)

Talisman pendants: [`talisman_face_camera_rotation`](../../crates/mahjuro-render/src/talisman_mesh.rs) = Rx(90°) so +local Z → world **−Y** (camera), +local Y → world **+Z** (up on table).

Relic pins on the dish use their own placement recipes in scene code; mesh local is still +Y cap / XZ silhouette.

## Footprint normalization

Both families scale front-cap area toward [`CAP_REFERENCE_AREA`](../../crates/mahjuro-render/src/cap_extrude.rs) (π·0.25). Relics: uniform XYZ scale. Talismans: **XY only** so slab thickness and carved +Z relief stay authored.

## When adding a new cap-extruded prop

1. Pick `CapExtrudeKind` (or extend the enum if a third axis convention is truly needed).
2. Route pixel → local/UV through `cap_extrude` helpers.
3. Use `wall_normal_from_cap_edge` for side walls.
4. If using heightmaps, pair UV with `height_normal_from_grad_front` and mirror the sign in WGSL if the cap axis differs.
5. Add a unit test in `cap_extrude.rs` or the mesh builder module for image-top → cap-up → low-v.
