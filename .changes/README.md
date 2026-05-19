# Changelog fragments

Each file in this directory is a single **unreleased** changelog entry.
At release time, `scripts/release.sh` compiles any fragments into a new section
at the top of `CHANGELOG.md` and deletes those fragment files.

## Adding an entry

Create a new file here with a short, descriptive slug as the filename,
ending in `.md`:

```
.changes/fix-dora-indicator.md
.changes/add-walk-mode.md
.changes/balance-second-wind.md
```

The file has YAML frontmatter and a one-paragraph body:

```markdown
---
category: fixed
---

Dora indicator no longer reveals after the final draw.
```

### Categories

- `added`     — new features, content, relics, tiles, modes
- `changed`   — tweaks to existing behavior, balance, UI
- `fixed`     — bug fixes
- `removed`   — removed features or content

## Previewing

```
python3 scripts/preview_changelog.py
```

Shows what the next `UNRELEASED` section would look like, without
touching any files.

## Releases with no player-facing work

If everything since the last tag is internal (CI, refactors, docs, etc.), you
do not need to add a fragment. `scripts/release.sh` will still append a version
section to `CHANGELOG.md` with a short placeholder line so the release workflow
has a body to publish.

## What happens at release

`scripts/release.sh <version>` will:

1. Compile `.changes/*.md` into a new `## <version> — YYYY-MM-DD` section
   at the top of `CHANGELOG.md` (or add a short placeholder line if there are
   no fragments).
2. Remove any fragment files that were compiled.
3. Bump `Cargo.toml`, commit, tag, and push.

The GitHub Actions release workflow then extracts the section for this
version and uses it as the release body.

## Tips

- Write entries from the **player's** perspective, not the implementer's.
  "Added hanami relic" > "Added HanamiEffect struct in relics.rs".
- One fragment per logical change. It's fine to have many small ones.
- If you realize a fragment was wrong, just edit or delete the file —
  nothing is compiled until release time.
