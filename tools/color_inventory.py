#!/usr/bin/env python3
"""Inventory every ad-hoc color literal in src/ and bucket by closest
theme token.

Walks `src/**/*.rs` and `crates/*/src/**/*.rs`, finds `[r, g, b, a]` /
`[r, g, b]` / `(r, g, b, a)` / `(r, g, b)` literals where every component is
in [0, 1], measures Euclidean distance to each token in
`crates/mahjuro-render/src/theme.rs` (plus shared aliases in
`crates/mahjuro-types/src/theme_tokens.rs`), and emits a markdown report
grouped by suggested replacement.

Usage:
    python3 tools/color_inventory.py
    python3 tools/color_inventory.py --out docs/color-inventory.md
    python3 tools/color_inventory.py --threshold 0.06    # tighter matching
"""

from __future__ import annotations

import argparse
import math
import re
from collections import defaultdict
from dataclasses import dataclass, field
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[1]
THEME_RS = REPO_ROOT / "crates" / "mahjuro-render" / "src" / "theme.rs"
THEME_TOKENS_RS = REPO_ROOT / "crates" / "mahjuro-types" / "src" / "theme_tokens.rs"
TILE_RS = REPO_ROOT / "crates" / "mahjuro-core" / "src" / "core" / "tile.rs"

# Files we deliberately skip:
#   theme.rs / theme_tokens.rs — define the tokens; not "ad-hoc" by definition.
#   tile.rs                    — suit keyword colors are documented in
#                                COLOR_THEME.md and live here on purpose.
SKIP_FILES = {THEME_RS, THEME_TOKENS_RS, TILE_RS}


# ───────────────────────── token loading ──────────────────────────────


@dataclass
class Token:
    name: str
    rgb: tuple[float, float, float]
    a: float

    @property
    def hex(self) -> str:
        r, g, b = self.rgb
        return "#{:02X}{:02X}{:02X}".format(
            round(r * 255), round(g * 255), round(b * 255)
        )


CONST_RE = re.compile(
    r"pub const (?P<name>[A-Z_]+): \[f32; 4\] = "
    r"\[\s*(?P<r>[\d.]+)\s*,\s*(?P<g>[\d.]+)\s*,"
    r"\s*(?P<b>[\d.]+)\s*,\s*(?P<a>[\d.]+)\s*\]\s*;",
)


def load_tokens() -> list[Token]:
    by_name: dict[str, Token] = {}
    for path in (THEME_RS, THEME_TOKENS_RS):
        text = path.read_text(encoding="utf-8")
        for m in CONST_RE.finditer(text):
            name = m.group("name")
            if name == "CLEAR":
                continue
            by_name[name] = Token(
                name=name,
                rgb=(float(m.group("r")), float(m.group("g")), float(m.group("b"))),
                a=float(m.group("a")),
            )
    return list(by_name.values())


# ───────────────────────── literal scanning ───────────────────────────


F = r"(?:\d+\.\d+|\d+\.|\.\d+|\d+)"
COLOR4_RE = re.compile(
    rf"\[\s*(?P<r>{F})\s*,\s*(?P<g>{F})\s*,\s*(?P<b>{F})\s*,\s*(?P<a>{F})\s*\]"
)
COLOR3_RE = re.compile(
    rf"\[\s*(?P<r>{F})\s*,\s*(?P<g>{F})\s*,\s*(?P<b>{F})\s*\]"
)
TUPLE4_RE = re.compile(
    rf"\(\s*(?P<r>{F})\s*,\s*(?P<g>{F})\s*,\s*(?P<b>{F})\s*,\s*(?P<a>{F})\s*\)"
)
TUPLE3_RE = re.compile(
    rf"\(\s*(?P<r>{F})\s*,\s*(?P<g>{F})\s*,\s*(?P<b>{F})\s*\)"
)


@dataclass
class Literal:
    file: Path
    line: int
    rgb: tuple[float, float, float]
    a: float
    raw: str
    context: str
    surrounding_comment: str = ""

    @property
    def hex(self) -> str:
        r, g, b = self.rgb
        return "#{:02X}{:02X}{:02X}".format(
            round(r * 255), round(g * 255), round(b * 255)
        )


def in_range(*vals: float) -> bool:
    return all(0.0 <= v <= 1.0 for v in vals)


def is_trivial(rgb: tuple[float, float, float], a: float) -> bool:
    """Skip pure black, pure white, fully opaque/transparent identity values
    and obvious non-color uses like `[1.0, 1.0, 1.0, 1.0]` (default white
    vertex color, used hundreds of times for "no tint")."""
    r, g, b = rgb
    if r == g == b == 0.0:
        return True
    if r == g == b == 1.0:
        return True
    return False


def is_basis_vector(rgb: tuple[float, float, float]) -> bool:
    """Skip cardinal-axis vectors like `[1.0, 0.0, 0.0]` / `[0, 1, 0]` /
    `[0, 0, 1]` and their negatives. These are almost always world-space
    `up:`/`normal:`/`axis:` directions, not colors. Pure red/green/blue
    "primaries" do exist as legitimate test colors, but they're vanishingly
    rare in this codebase compared to up-vectors, and any real one would
    show up in the reviewer's eye when scanning the inventory anyway."""
    nonzero = [v for v in rgb if v != 0.0]
    if len(nonzero) != 1:
        return False
    return abs(nonzero[0]) == 1.0


def parse_file(path: Path) -> list[Literal]:
    out: list[Literal] = []
    try:
        text = path.read_text(encoding="utf-8")
    except (UnicodeDecodeError, OSError):
        return out

    lines = text.splitlines()

    def line_starts_with_comment(line: str, match_col: int) -> bool:
        """Skip literals that live inside `//` or `///` comments (the regex
        catches example values in docstrings)."""
        before = line[:match_col]
        return "//" in before or before.lstrip().startswith("//")

    def push(
        match: re.Match[str], rgba: tuple[float, float, float, float], offset: int
    ) -> None:
        line_no = text.count("\n", 0, match.start()) + 1
        line = lines[line_no - 1] if line_no - 1 < len(lines) else ""
        prev = lines[line_no - 2] if line_no - 2 >= 0 else ""
        # Column within the line where the match starts.
        line_start = text.rfind("\n", 0, match.start()) + 1
        match_col = match.start() - line_start
        if line_starts_with_comment(line, match_col):
            return
        # Skip glam vector constructors and other call-site shapes that
        # happen to look like `(f, f, f)` but aren't colors. Field names
        # like `up:`, `normal:`, `dir:`, `axis:` are almost always
        # world-space directions, not RGBA values.
        before = line[max(0, match_col - 40) : match_col]
        if any(
            tok in before
            for tok in (
                "Vec3::new",
                "Vec4::new",
                "Vec2::new",
                "vec3(",
                "vec4(",
                "Rect ",
                "rect:",
                "extents:",
                "pos:",
                "normal:",
                "up:",
                "dir:",
                "axis:",
                "rotation:",
                "scale:",
                "size:",
                "offset:",
                "position:",
                "feather:",
                "Placement::at(",
                "transform_rect(",
            )
        ):
            return
        # Skip files whose entire purpose is screen-space placement, not
        # color: every literal in `src/ui/scene_layout/*.rs` is a
        # `Placement::at(x, y, z)` triple where x/y/z are normalized
        # screen coordinates that happen to fit `[0..=1]`.
        if "ui/scene_layout/" in path.as_posix():
            return
        comment = ""
        cm = re.search(r"//\s*(.*)$", line)
        if cm and cm.start() > match.start() - offset:
            comment = cm.group(1).strip()
        elif prev.strip().startswith("//"):
            comment = prev.strip().lstrip("/").strip()
        r, g, b, a = rgba
        if not in_range(r, g, b, a):
            return
        if is_trivial((r, g, b), a):
            return
        if is_basis_vector((r, g, b)):
            return
        out.append(
            Literal(
                file=path,
                line=line_no,
                rgb=(r, g, b),
                a=a,
                raw=match.group(0),
                context=line.strip(),
                surrounding_comment=comment,
            )
        )

    seen_spans: set[tuple[int, int]] = set()

    def already_taken(start: int, end: int) -> bool:
        for s, e in seen_spans:
            if not (end <= s or start >= e):
                return True
        return False

    for m in COLOR4_RE.finditer(text):
        if already_taken(m.start(), m.end()):
            continue
        rgba = (
            float(m.group("r")),
            float(m.group("g")),
            float(m.group("b")),
            float(m.group("a")),
        )
        push(m, rgba, m.start())
        seen_spans.add((m.start(), m.end()))

    for m in TUPLE4_RE.finditer(text):
        if already_taken(m.start(), m.end()):
            continue
        rgba = (
            float(m.group("r")),
            float(m.group("g")),
            float(m.group("b")),
            float(m.group("a")),
        )
        push(m, rgba, m.start())
        seen_spans.add((m.start(), m.end()))

    for m in COLOR3_RE.finditer(text):
        if already_taken(m.start(), m.end()):
            continue
        rgba = (
            float(m.group("r")),
            float(m.group("g")),
            float(m.group("b")),
            1.0,
        )
        push(m, rgba, m.start())
        seen_spans.add((m.start(), m.end()))

    for m in TUPLE3_RE.finditer(text):
        if already_taken(m.start(), m.end()):
            continue
        rgba = (
            float(m.group("r")),
            float(m.group("g")),
            float(m.group("b")),
            1.0,
        )
        push(m, rgba, m.start())
        seen_spans.add((m.start(), m.end()))

    return out


def iter_source_rs_files() -> list[Path]:
    paths: list[Path] = []
    src = REPO_ROOT / "src"
    if src.is_dir():
        paths.extend(src.rglob("*.rs"))
    crates = REPO_ROOT / "crates"
    if crates.is_dir():
        for crate in sorted(crates.iterdir()):
            crate_src = crate / "src"
            if crate_src.is_dir():
                paths.extend(crate_src.rglob("*.rs"))
    return sorted(set(paths))


def scan_src() -> list[Literal]:
    out: list[Literal] = []
    for path in iter_source_rs_files():
        if path in SKIP_FILES:
            continue
        out.extend(parse_file(path))
    return out


# ───────────────────── matching against tokens ────────────────────────


def distance(a: tuple[float, float, float], b: tuple[float, float, float]) -> float:
    return math.sqrt(sum((x - y) ** 2 for x, y in zip(a, b)))


@dataclass
class Match:
    literal: Literal
    token: Token
    distance: float


def closest(token_list: list[Token], lit: Literal) -> Match:
    best = min(token_list, key=lambda t: distance(t.rgb, lit.rgb))
    return Match(lit, best, distance(best.rgb, lit.rgb))


# ───────────────────────── report writing ─────────────────────────────


EXACT = 0.025
NEAR = 0.08


def bucket_label(d: float) -> str:
    if d <= EXACT:
        return "exact"
    if d <= NEAR:
        return "near"
    return "ad-hoc"


def rel_path(p: Path) -> str:
    try:
        return str(p.relative_to(REPO_ROOT))
    except ValueError:
        return str(p)


def short_context(s: str, n: int = 60) -> str:
    s = s.replace("|", "\\|")
    if len(s) <= n:
        return s
    return s[: n - 1] + "…"


def short_comment(s: str, n: int = 50) -> str:
    s = s.replace("|", "\\|")
    if not s:
        return ""
    if len(s) <= n:
        return s
    return s[: n - 1] + "…"


def write_report(
    matches: list[Match], tokens: list[Token], out_path: Path, threshold: float
) -> None:
    by_token: dict[str, list[Match]] = defaultdict(list)
    ad_hoc: list[Match] = []
    near: list[Match] = []
    for m in matches:
        b = bucket_label(m.distance)
        if b == "exact":
            by_token[m.token.name].append(m)
        elif b == "near":
            near.append(m)
        else:
            ad_hoc.append(m)

    by_file: dict[str, int] = defaultdict(int)
    for m in matches:
        by_file[rel_path(m.literal.file)] += 1

    lines: list[str] = []
    lines.append("# Color literal inventory")
    lines.append("")
    lines.append(
        "Auto-generated by `python3 tools/color_inventory.py`. Re-run after "
        "any color refactor to see progress. **Do not hand-edit.**"
    )
    lines.append("")
    lines.append("## Summary")
    lines.append("")
    n = len(matches)
    n_exact = sum(len(v) for v in by_token.values())
    lines.append(f"- **Total ad-hoc literals scanned:** {n}")
    lines.append(
        f"- **Exact match to a theme token** (distance ≤ {EXACT:.3f}, "
        f"safe drop-in): **{n_exact}**"
    )
    lines.append(
        f"- **Near match** (distance ≤ {NEAR:.3f}, slight visual shift): "
        f"**{len(near)}**"
    )
    lines.append(
        f"- **No close match** (intentional unique color or large gap): "
        f"**{len(ad_hoc)}**"
    )
    lines.append("")
    lines.append(
        "Distance is Euclidean in linear-RGB space (channels in [0, 1]). "
        "Pure black / pure white are skipped (used as identity tints). "
        "`crates/mahjuro-render/src/theme.rs`, "
        "`crates/mahjuro-types/src/theme_tokens.rs`, and "
        "`crates/mahjuro-core/src/core/tile.rs` are skipped (they "
        "*are* the source of truth)."
    )
    lines.append("")

    lines.append("## Top files by literal count")
    lines.append("")
    lines.append("| File | Literals |")
    lines.append("|------|----------|")
    for path, count in sorted(by_file.items(), key=lambda kv: -kv[1])[:20]:
        lines.append(f"| `{path}` | {count} |")
    lines.append("")

    lines.append("## Exact matches (safe drop-ins)")
    lines.append("")
    lines.append(
        "Each row's literal is within "
        f"{EXACT:.3f} Euclidean distance of the named token. Replacing it "
        "should produce no perceptible visual change."
    )
    lines.append("")
    if not by_token:
        lines.append("_None._")
        lines.append("")
    else:
        for token_name in sorted(
            by_token.keys(),
            key=lambda n: (-len(by_token[n]), n),
        ):
            entries = sorted(by_token[token_name], key=lambda m: m.distance)
            tok = entries[0].token
            lines.append(
                f"### `{token_name}` ({tok.hex}) — {len(entries)} occurrence"
                f"{'s' if len(entries) != 1 else ''}"
            )
            lines.append("")
            lines.append("| File | Line | Literal hex | Δ | Comment / context |")
            lines.append("|------|-----:|-------------|--:|-------------------|")
            for m in entries:
                ctx = short_comment(m.literal.surrounding_comment) or short_context(
                    m.literal.context
                )
                lines.append(
                    f"| `{rel_path(m.literal.file)}` | {m.literal.line} | "
                    f"`{m.literal.hex}` | {m.distance:.3f} | {ctx} |"
                )
            lines.append("")

    lines.append("## Near matches (consider replacing)")
    lines.append("")
    lines.append(
        f"Within {NEAR:.3f} of a token. Replacing will shift the color slightly "
        "(typically <1 unit per channel of saturation or warmth). Worth "
        "checking case-by-case; many of these are deliberate variations."
    )
    lines.append("")
    if not near:
        lines.append("_None._")
        lines.append("")
    else:
        near_grouped: dict[str, list[Match]] = defaultdict(list)
        for m in near:
            near_grouped[m.token.name].append(m)
        for token_name in sorted(
            near_grouped.keys(),
            key=lambda n: (-len(near_grouped[n]), n),
        ):
            entries = sorted(near_grouped[token_name], key=lambda m: m.distance)
            tok = entries[0].token
            lines.append(
                f"### Near `{token_name}` ({tok.hex}) — {len(entries)} "
                f"occurrence{'s' if len(entries) != 1 else ''}"
            )
            lines.append("")
            lines.append("| File | Line | Literal hex | Δ | Comment / context |")
            lines.append("|------|-----:|-------------|--:|-------------------|")
            for m in entries:
                ctx = short_comment(m.literal.surrounding_comment) or short_context(
                    m.literal.context
                )
                lines.append(
                    f"| `{rel_path(m.literal.file)}` | {m.literal.line} | "
                    f"`{m.literal.hex}` | {m.distance:.3f} | {ctx} |"
                )
            lines.append("")

    lines.append("## No close match — intentional or orphan")
    lines.append("")
    lines.append(
        f"Distance > {NEAR:.3f} from any token. These are the colors that "
        "either deserve their own token (if reused), are intentionally "
        "outside the palette (e.g. tile-pack variations, particle bursts, "
        "debug-only chrome), or simply haven't been considered yet."
    )
    lines.append("")
    if not ad_hoc:
        lines.append("_None._")
        lines.append("")
    else:
        lines.append(
            "| File | Line | Literal hex | RGB | Closest token | Δ | Comment / context |"
        )
        lines.append(
            "|------|-----:|-------------|-----|---------------|--:|-------------------|"
        )
        for m in sorted(ad_hoc, key=lambda m: (rel_path(m.literal.file), m.literal.line)):
            r, g, b = m.literal.rgb
            rgb_str = f"{r:.2f}, {g:.2f}, {b:.2f}"
            ctx = short_comment(m.literal.surrounding_comment) or short_context(
                m.literal.context
            )
            lines.append(
                f"| `{rel_path(m.literal.file)}` | {m.literal.line} | "
                f"`{m.literal.hex}` | {rgb_str} | `{m.token.name}` | "
                f"{m.distance:.3f} | {ctx} |"
            )
        lines.append("")

    out_path.parent.mkdir(parents=True, exist_ok=True)
    out_path.write_text("\n".join(lines) + "\n", encoding="utf-8")


def main() -> None:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument(
        "--out",
        type=Path,
        default=REPO_ROOT / "docs" / "color-inventory.md",
        help="output markdown path (default: docs/color-inventory.md)",
    )
    ap.add_argument(
        "--threshold",
        type=float,
        default=NEAR,
        help=f"near-match threshold (default {NEAR})",
    )
    args = ap.parse_args()

    tokens = load_tokens()
    literals = scan_src()
    matches = [closest(tokens, lit) for lit in literals]

    write_report(matches, tokens, args.out, args.threshold)
    n_exact = sum(1 for m in matches if m.distance <= EXACT)
    n_near = sum(1 for m in matches if EXACT < m.distance <= NEAR)
    n_far = sum(1 for m in matches if m.distance > NEAR)
    print(f"wrote {args.out}")
    print(f"  total: {len(matches)}")
    print(f"  exact (Δ ≤ {EXACT:.3f}): {n_exact}")
    print(f"  near  (Δ ≤ {NEAR:.3f}): {n_near}")
    print(f"  ad-hoc      (Δ >  {NEAR:.3f}): {n_far}")


if __name__ == "__main__":
    main()
