# Changelog fragments

Each file in this directory is a single **unreleased** changelog entry.
At release time, `scripts/release.sh` compiles every fragment into a new
section at the top of `CHANGELOG.md` and deletes the fragment files.

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

## What happens at release

`scripts/release.sh <version>` will:

1. Compile `.changes/*.md` into a new `## <version> — YYYY-MM-DD` section
   at the top of `CHANGELOG.md`.
2. `git rm` the fragment files.
3. Bump `Cargo.toml`, commit, tag, and push.

The GitHub Actions release workflow then extracts the section for this
version and uses it as the release body.

## Tips

- Write entries from the **player's** perspective, not the implementer's.
  "Added hanami relic" > "Added HanamiEffect struct in relics.rs".
- One fragment per logical change. It's fine to have many small ones.
- If you realize a fragment was wrong, just edit or delete the file —
  nothing is compiled until release time.
