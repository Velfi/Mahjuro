# Talisman tablet art (shop + memorial)

Octagonal talisman tablets share one mesh (`src/render/talisman_mesh.rs`) and `MaterialKind::Chitin`. Each kind has its own **grayscale heightmap** and a separate **octagon mask** — heightmaps are full mid-gray plates with carved relief (no black void outside the silhouette).

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
python scripts/generate_talisman_art.py --set shop --force   # nine shop tablets
python scripts/generate_talisman_art.py --set memorial
python scripts/generate_talisman_art.py --masks-only         # procedural masks only
python scripts/generate_memorial_talisman_art.py --force     # memorial wrapper
```

Heightmaps align with `talisman_face_uv` (`v = 0` at +local Y, flat octagon edge at bottom of the image / −Y on the mesh). Masks are procedural octagons from `scripts/_talisman_art_common.py` (same angles as the mesh).

## Art direction

- **Shop:** crisp merchant engraving on a mid-gray plate; premium, readable at thumbnail size.
- **Memorial:** worn temple plaque / stone rubbing — solemn, not glossy shop foil.
- **Motifs:** one pictorial idea per kind (`SHOP_MOTIFS` / `MEMORIAL_MOTIFS` in `scripts/generate_talisman_art.py`). No Latin text on the face.
- **In-game tint:** JSON `accent` still tints the chitin foil; relief comes from the heightmap; silhouette discard uses the mask.

## Renderer

`build.rs` loads `talisman_height_views` + `talisman_mask_views` and memorial counterparts. `Object3dKind::{Talisman,MemorialTalisman}` rebind height → `albedo_tex`, mask → `relief_tex`. Chitin fragments outside the mask are discarded in `lit_mesh.wgsl` (threshold 8/255, same as relic enamel).

## Defeat screenshot from bot / career data

`screenshot --scene game_over_defeat` can hydrate stats from real runs instead of the baked fixture:

```bash
cargo run -p mahjuro-headless --bin mahjuro-screenshot --features screenshot --release -- --scene game_over_defeat --bot-play
cargo run -p mahjuro-headless --bin mahjuro-screenshot --features screenshot --release -- --scene game_over_defeat --from-run-history 2 --profile 0
cargo run -p mahjuro-headless --bin mahjuro-screenshot --features screenshot --release -- --scene game_over_defeat --seed-bot-runs 20 --from-run-history 15
```

`RunRecord::hydrate_game_over_run` restores the fields `GameOverScene` reads.
