# Unified HTML editor for ship game data JSON

## Why
[`tools/relic_flavor_editor.html`](../../tools/relic_flavor_editor.html) gives a good workflow for **`relics.json`** (`flavor_spans` only, save whole file). **`assets/data/bosses.json`** and **`assets/data/yaku.json`** are still plain JSON in an editor—easy to break commas or drift from field names. One shared tool (drag/drop, row pick by `id`, validated fields, same save UX) cuts friction for balance and copy tweaks without standing up a server.

## Scope
1. **Shared shell** — Reuse the relic editor’s patterns: file picker, drag/drop, dark panel layout, pretty-printed save via `showSaveFilePicker` with download fallback, stable output filename from the loaded file.
2. **Route by dataset** — After `JSON.parse`, branch on detected file (by user-chosen name or a post-load dropdown): `relics.json` vs `bosses.json` vs `yaku.json`. Reject or warn on unknown top-level shapes (not a top-level array of objects with `id`).
3. **Relics mode** — Keep the existing **contenteditable → `flavor_spans`** pipeline and per-relic select. Optionally add simple inputs later for `name` / `description` / `rarity` on the same row; not required for the first milestone.
4. **Generic table modes** — For bosses and yaku: `<select>` keyed by `id`, form fields driven by a small **manifest** (inline table or [`tools/data_editor_manifest.json`](../../tools/data_editor_manifest.json): field → `string` | `number` | `enum`). Live preview of the current object as JSON; save writes the **entire** array back (same contract as the relic editor).
5. **Escape hatch** — Optional “Raw JSON” for the current row or whole file when a new field lands before the manifest is updated.
6. **Housekeeping** — Either rename/replace `relic_flavor_editor.html` with the unified page or leave a short pointer comment at the top of the old file to the new tool.

Out of scope: JSON Schema generation from Rust types, editing `tools/bake_assets/pack_rules.json` or under `docs/`, CI validation gates, or a localhost backend.

## Touchpoints
- [`tools/relic_flavor_editor.html`](../../tools/relic_flavor_editor.html) — Source to merge or factor (flavor HTML serialization, `editorToSpans` / `spansToHtml`, save logic).
- [`assets/data/relics.json`](../../assets/data/relics.json) — Primary relic dataset; confirm no loader expects trailing/ordering quirks beyond valid JSON.
- [`assets/data/bosses.json`](../../assets/data/bosses.json) — `id`, `name`, `description`, `tier`, `min_ante` row editor fields.
- [`assets/data/yaku.json`](../../assets/data/yaku.json) — `id`, `name`, `mult_bonus`, `chip_bonus` row editor fields.
- Rust loaders ([`src/core/relic.rs`](../../src/core/relic.rs) and any `bosses` / `yaku` ingest) — No change required if JSON shape stays the same; doc here so schema changes stay in sync with serde expectations.

## Open questions
- **Manifest vs JSON Schema.** A hand-written manifest is fastest; schemas are nicer long-term if someone wants to generate forms from one source of truth.
- **Single HTML vs small ES module.** One file keeps “open from disk” trivial; splitting JS only pays off if the tool grows past ~800 lines.
