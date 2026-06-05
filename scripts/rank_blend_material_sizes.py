#!/usr/bin/env python3
"""Rank .blend files by size and report top-N largest materials per file.

This script must be run with Blender's Python, for example:

    blender -b --factory-startup -P scripts/rank_blend_material_sizes.py
    blender -b --factory-startup -P scripts/rank_blend_material_sizes.py -- --top 5
    blender -b --factory-startup -P scripts/rank_blend_material_sizes.py -- assets/3d/source/Shop.blend
"""

from __future__ import annotations

import argparse
import os
import sys
from dataclasses import dataclass
from pathlib import Path

try:
    import bpy  # type: ignore
except ImportError as exc:  # pragma: no cover - only hits outside Blender
    raise SystemExit(
        "This script must be run by Blender, e.g. "
        "'blender -b --factory-startup -P scripts/rank_blend_material_sizes.py -- ...'."
    ) from exc


@dataclass
class MaterialStat:
    name: str
    texture_bytes: int
    encoded_bytes: int
    image_count: int
    node_count: int


def script_args() -> list[str]:
    if "--" not in sys.argv:
        return []
    return sys.argv[sys.argv.index("--") + 1 :]


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "paths",
        nargs="*",
        help="Blend files or directories to analyze. Defaults to assets/3d/source.",
    )
    parser.add_argument(
        "--top",
        type=int,
        default=3,
        help="Top N materials to print per blend file (default: 3).",
    )
    parser.add_argument(
        "--glob",
        default="*.blend",
        help="Filename pattern when expanding directories (default: *.blend).",
    )
    return parser


def repo_root() -> Path:
    return Path(__file__).resolve().parent.parent


def default_search_root() -> Path:
    candidate = repo_root() / "assets" / "3d" / "source"
    if candidate.exists():
        return candidate
    return Path.cwd()


def discover_blend_files(paths: list[str], pattern: str) -> list[Path]:
    discovered: list[Path] = []
    seen: set[Path] = set()

    def add(path: Path) -> None:
        resolved = path.resolve()
        if resolved not in seen and resolved.exists() and resolved.is_file():
            seen.add(resolved)
            discovered.append(resolved)

    if not paths:
        for path in sorted(default_search_root().rglob(pattern)):
            add(path)
        return discovered

    for raw in paths:
        path = Path(raw)
        if not path.is_absolute():
            path = (repo_root() / path).resolve()
        if path.is_dir():
            for match in sorted(path.rglob(pattern)):
                add(match)
        else:
            add(path)
    return discovered


def iter_images_in_node_tree(node_tree, visited: set[int]):
    if node_tree is None:
        return
    pointer = node_tree.as_pointer()
    if pointer in visited:
        return
    visited.add(pointer)

    for node in node_tree.nodes:
        if node.type == "TEX_IMAGE" and getattr(node, "image", None):
            yield node.image
        if node.type == "GROUP" and getattr(node, "node_tree", None):
            yield from iter_images_in_node_tree(node.node_tree, visited)


def image_texture_bytes(image) -> int:
    width = int(image.size[0]) if image.size else 0
    height = int(image.size[1]) if image.size else 0
    if width <= 0 or height <= 0:
        return 0

    channels = max(1, int(getattr(image, "channels", 4) or 4))
    bytes_per_channel = 4 if bool(getattr(image, "is_float", False)) else 1
    return width * height * channels * bytes_per_channel


def image_encoded_bytes(image) -> int:
    packed = getattr(image, "packed_file", None)
    if packed is not None and getattr(packed, "size", 0):
        return int(packed.size)

    filepath = getattr(image, "filepath", "")
    if not filepath:
        return 0

    try:
        abs_path = bpy.path.abspath(filepath, library=image.library)
    except Exception:
        abs_path = bpy.path.abspath(filepath)

    if abs_path and os.path.exists(abs_path):
        return os.path.getsize(abs_path)
    return 0


def material_stat(material) -> MaterialStat:
    images = {}
    node_count = 0
    if material.node_tree is not None:
        node_count = len(material.node_tree.nodes)
        for image in iter_images_in_node_tree(material.node_tree, set()):
            images[image.as_pointer()] = image

    texture_bytes = 0
    encoded_bytes = 0
    for image in images.values():
        texture_bytes += image_texture_bytes(image)
        encoded_bytes += image_encoded_bytes(image)

    return MaterialStat(
        name=material.name,
        texture_bytes=texture_bytes,
        encoded_bytes=encoded_bytes,
        image_count=len(images),
        node_count=node_count,
    )


def analyze_file(path: Path, top_n: int) -> tuple[list[MaterialStat], str | None]:
    try:
        bpy.ops.wm.open_mainfile(filepath=str(path), load_ui=False)
    except Exception as exc:  # pragma: no cover - Blender operator errors are runtime-specific
        return [], str(exc)

    stats = [material_stat(material) for material in bpy.data.materials]
    stats.sort(
        key=lambda item: (
            item.texture_bytes,
            item.encoded_bytes,
            item.image_count,
            item.node_count,
            item.name.lower(),
        ),
        reverse=True,
    )
    return stats[: max(1, top_n)], None


def mib(size_bytes: int) -> float:
    return size_bytes / (1024 * 1024)


def print_report(paths: list[Path], top_n: int) -> int:
    if not paths:
        print("No .blend files found.")
        return 1

    ranked = sorted(paths, key=lambda p: p.stat().st_size, reverse=True)

    print(f"Found {len(ranked)} blend file(s). Ranked by on-disk size:\n")
    for index, path in enumerate(ranked, start=1):
        size_mib = mib(path.stat().st_size)
        print(f"{index}. {path} ({size_mib:.2f} MiB)")
    print("")

    for path in ranked:
        stats, error = analyze_file(path, top_n)
        size_mib = mib(path.stat().st_size)
        print(f"=== {path} ({size_mib:.2f} MiB) ===")
        if error:
            print(f"  ERROR: {error}")
            print("")
            continue

        if not stats:
            print("  No materials found.")
            print("")
            continue

        for rank, stat in enumerate(stats, start=1):
            print(
                f"  {rank}. {stat.name} | tex={mib(stat.texture_bytes):.2f} MiB | "
                f"encoded={mib(stat.encoded_bytes):.2f} MiB | "
                f"images={stat.image_count} | nodes={stat.node_count}"
            )
        print("")

    return 0


def main() -> int:
    parser = build_parser()
    args = parser.parse_args(script_args())
    files = discover_blend_files(args.paths, args.glob)
    return print_report(files, args.top)


if __name__ == "__main__":
    raise SystemExit(main())
