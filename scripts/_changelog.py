"""Shared helpers for the changelog tooling.

Fragments live in `.changes/*.md`. Each fragment has YAML frontmatter with a
`category` field and a markdown body. This module parses them and renders a
CHANGELOG section.
"""

from __future__ import annotations

import pathlib
import re
from dataclasses import dataclass

REPO_ROOT = pathlib.Path(__file__).resolve().parent.parent
FRAGMENTS_DIR = REPO_ROOT / ".changes"
CHANGELOG_PATH = REPO_ROOT / "CHANGELOG.md"

# Order matters — it's the order sections appear under each version heading.
CATEGORIES = ("added", "changed", "fixed", "removed")
CATEGORY_TITLES = {
    "added": "Added",
    "changed": "Changed",
    "fixed": "Fixed",
    "removed": "Removed",
}

# Files that live in .changes/ but are not fragments.
NON_FRAGMENT_FILES = {"README.md", ".gitkeep"}


@dataclass
class Fragment:
    path: pathlib.Path
    category: str
    body: str  # Rendered as a single bullet; newlines in source become spaces.


_FRONTMATTER_RE = re.compile(r"^---\s*\n(.*?)\n---\s*\n?(.*)$", re.DOTALL)


def parse_fragment(path: pathlib.Path) -> Fragment:
    raw = path.read_text(encoding="utf-8")
    m = _FRONTMATTER_RE.match(raw)
    if not m:
        raise ValueError(
            f"{path.name}: missing YAML frontmatter (expected '---' delimited header)"
        )
    header, body = m.group(1), m.group(2)

    category: str | None = None
    for line in header.splitlines():
        line = line.strip()
        if not line or line.startswith("#"):
            continue
        if ":" not in line:
            continue
        key, _, value = line.partition(":")
        if key.strip() == "category":
            category = value.strip().strip("\"'").lower()

    if category is None:
        raise ValueError(f"{path.name}: frontmatter missing 'category'")
    if category not in CATEGORIES:
        raise ValueError(
            f"{path.name}: category '{category}' not one of {list(CATEGORIES)}"
        )

    body = body.strip()
    if not body:
        raise ValueError(f"{path.name}: body is empty")
    # Collapse internal whitespace so multi-line entries render as one bullet.
    body = re.sub(r"\s+", " ", body)

    return Fragment(path=path, category=category, body=body)


def load_fragments() -> list[Fragment]:
    if not FRAGMENTS_DIR.is_dir():
        return []
    fragments: list[Fragment] = []
    for path in sorted(FRAGMENTS_DIR.iterdir()):
        if not path.is_file():
            continue
        if path.name in NON_FRAGMENT_FILES:
            continue
        if not path.name.endswith(".md"):
            continue
        fragments.append(parse_fragment(path))
    return fragments


def render_section(heading: str, fragments: list[Fragment]) -> str:
    """Render a CHANGELOG section. `heading` is the full '## ...' line body."""
    lines = [f"## {heading}"]
    for category in CATEGORIES:
        entries = [f for f in fragments if f.category == category]
        if not entries:
            continue
        lines.append("")
        lines.append(f"### {CATEGORY_TITLES[category]}")
        for entry in entries:
            lines.append(f"- {entry.body}")
    lines.append("")
    return "\n".join(lines)
