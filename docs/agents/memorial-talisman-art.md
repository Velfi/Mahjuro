# Talisman carving art (shop + memorial)

Carved jade **relief** (not octagonal tablets). Each kind has a figurative subject from Chinese myth/nature, an organic asymmetric silhouette extruded from its mask, and a grayscale heightmap for deep relief carving.

## Files

| Set | Heightmap | Mask |
|-----|-----------|------|
| Shop | `assets/textures/talismans/talisman_{slug}.png` | `talisman_{slug}_mask.png` |
| Memorial | `assets/textures/talismans/memorial_{slug}.png` | `memorial_{slug}_mask.png` |

`slug` matches JSON `id` in `assets/data/talismans.json` or `memorial_talismans.json`. Runtime paths: `TalismanKind::{heightmap,mask}_asset_path`, `MemorialTalismanKind::{heightmap,mask}_asset_path`.

## Generate

```bash
pip install google-genai pillow
export GEMINI_API_KEY="..."
python scripts/generate_talisman_art.py                    # missing only, both sets
python scripts/generate_talisman_art.py --force            # all shop + memorial
python scripts/generate_talisman_art.py --set shop --force
python scripts/generate_talisman_art.py --set memorial
python scripts/generate_talisman_art.py --masks-only       # organic masks from heights
python scripts/generate_memorial_talisman_art.py --force   # memorial wrapper
```

Heightmaps: mid-gray jade with figurative high-relief carving. Masks: derived from heightmap via `--mask-method` (default `auto`):

- **Flat heightfields** (black void at border): luminance threshold
- **Sculpted renders** (gray studio background): border-connected plate-tone flood, then **interior plate-void punch** for piercings (wave gates, coin squares) that match studio gray but sit inside the jade; tiny carving speckles only are filled (`MAX_CARVING_PINHOLE_AREA`)
- **Fallback**: local **rembg/u2net** (`pip install rembg onnxruntime`) when flood-fill ratio looks wrong

```bash
python scripts/generate_talisman_art.py --masks-only --mask-method auto   # recommended
python scripts/generate_talisman_art.py --masks-only --mask-method rembg  # force ML matte
```

Postprocess exaggeration: **2.4** (shop) / **2.0** (memorial) via `--exaggerate-shop` / `--exaggerate-memorial`.

## Art direction

- **Heightmap style:** orthographic top-down displacement plate — mid-gray (#808080) ground edge-to-edge; carving contour meets the canvas margin (no mat, frame, or bezel). Postprocess strips AI matte/frame bands before exaggeration.
- **Shop:** moon rabbit, beetle on sycee, peacock, cicada on bamboo, coiled dragon (pinzu), pixiu, three honor dragons + east wind, lotus + kingfisher, twin mirror koi.
- **Memorial:** ox, crane in ice, leaping carp, magpie on pouch, money toad, butterfly, bent jian + taotie, deer + lingzhi, nine-tailed fox, ofuda strips, nesting swallows, xuanwu on terraces.
- **Motifs:** one unique figurative subject per kind (`SHOP_MOTIFS` / `MEMORIAL_MOTIFS`); prompts name the only scene to carve. Avoid repeating bats, coin stacks, phoenixes, lotus, or pixiu across kinds.
- **In-game tint:** JSON `accent` tints nacre sheen; relief from heightmap (normal perturbation + iridescence); silhouette discard uses mask.

## Renderer

`build_talisman_mesh_from_mask()` extrudes each mask like enamel relic badges (caps on ±Z). Per-kind meshes cached in `talisman_meshes` / `memorial_talisman_meshes` (shelf and inspect orbit share the same mesh). Octagonal prism remains fallback.

`Object3dKind::{Talisman,MemorialTalisman}` rebind height → `albedo_tex`, mask → `relief_tex`. Chitin fragments outside the mask are discarded in `lit_mesh.wgsl` (threshold 8/255). Shop tablets get lustrous mother-of-pearl; memorial tablets (`material_params.w >= 128`) get subdued stone-pearl.

## Defeat screenshot from bot / career data

`screenshot --scene defeat` can hydrate stats from real runs instead of the baked fixture:

```bash
cargo run -p mahjuro-headless --bin mahjuro-screenshot --features screenshot --release -- --scene defeat --bot-play
cargo run -p mahjuro-headless --bin mahjuro-screenshot --features screenshot --release -- --scene defeat --from-run-history 2 --profile 0
cargo run -p mahjuro-headless --bin mahjuro-screenshot --features screenshot --release -- --scene defeat --seed-bot-runs 20 --from-run-history 15
```

`RunRecord::hydrate_game_over_run` restores the fields `GameOverScene` reads.
