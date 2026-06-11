#!/usr/bin/env python3
"""
Bake Mahjuro assets/ into versioned ZIP packs + pack_manifest.json.

Default release profile: minify JSON; optional lossy tools if on PATH (oxipng, pngquant, ffmpeg).
See pack_rules.json for pack split (shared cross-scene, gameplay bulk, lazy scene_main_menu + music).
"""

from __future__ import annotations

import argparse
import fnmatch
import hashlib
import json
import os
import shutil
import subprocess
import sys
import tempfile
import zipfile
from dataclasses import dataclass
from pathlib import Path


REPO_ROOT = Path(__file__).resolve().parents[2]
RULES_PATH = Path(__file__).resolve().parent / "pack_rules.json"

# Room environment GLBs: runtime caps textures to 1024px and builds mips on load.
# Resize at bake when `gltf-transform` is on PATH (npm i -g @gltf-transform/cli).
ROOM_ENV_GLB_NAMES = frozenset(
    {
        "shop.glb",
        "hallway.glb",
        "archive.glb",
        "main_menu.glb",
        "staircase.glb",
        "gameplay.glb",
        "shadow_test_room.glb",
    }
)
ROOM_ENV_TEXTURE_MAX = 1024


@dataclass
class PackRule:
    id: str
    file: str
    load_tier: str
    path_prefixes: list[str]
    root_files: list[str]
    root_globs: list[str]


def parse_json_file(path: Path, *, label: str | None = None) -> object:
    text = path.read_text(encoding="utf-8")
    try:
        return json.loads(text)
    except json.JSONDecodeError as err:
        where = label or str(path)
        lines = text.splitlines()
        lineno = err.lineno
        colno = err.colno
        snippet: list[str] = []
        for i in range(max(0, lineno - 2), min(len(lines), lineno + 1)):
            marker = ">" if i == lineno - 1 else " "
            snippet.append(f"  {marker} {i + 1:4d} | {lines[i]}")
        if 1 <= lineno <= len(lines):
            caret_col = len(f"  > {lineno:4d} | ") + colno
            snippet.append(f"{' ' * caret_col}^")
        print(
            f"error: invalid JSON in {where}: {err.msg} (line {lineno}, column {colno})\n"
            + "\n".join(snippet),
            file=sys.stderr,
        )
        raise SystemExit(1) from err


def load_rules() -> list[PackRule]:
    raw = parse_json_file(RULES_PATH)
    out: list[PackRule] = []
    for p in raw["packs"]:
        out.append(
            PackRule(
                id=p["id"],
                file=p["file"],
                load_tier=p["load_tier"],
                path_prefixes=list(p.get("path_prefixes", [])),
                root_files=list(p.get("root_files", [])),
                root_globs=list(p.get("root_globs", [])),
            )
        )
    return out


def git_sha() -> str:
    try:
        r = subprocess.run(
            ["git", "rev-parse", "--short", "HEAD"],
            cwd=REPO_ROOT,
            capture_output=True,
            text=True,
            check=True,
        )
        return r.stdout.strip()
    except (subprocess.CalledProcessError, FileNotFoundError):
        return "unknown"


def cargo_version() -> str:
    """Read `[package].version` only (ignore other tables / workspace keys)."""
    toml = (REPO_ROOT / "Cargo.toml").read_text(encoding="utf-8")
    in_package = False
    for line in toml.splitlines():
        body = line.split("#", 1)[0].strip()
        if body.startswith("[") and body.endswith("]"):
            section = body[1:-1].strip()
            in_package = section == "package"
            continue
        if in_package and body.startswith("version"):
            key, _, val = body.partition("=")
            if key.strip() == "version":
                return val.strip().strip('"')
    return "0.0.0"


def assign_pack(rel: str, rules: list[PackRule]) -> str | None:
    """Return pack id for relative path (posix, under assets/).

    Precedence (first match wins):
    1. Packs appear in `pack_rules.json` order — an earlier pack wins over a later one.
    2. Within a pack: `root_files` (exact path), then `path_prefixes` (first matching prefix
       in that pack's list; put longer prefixes first if they overlap), then for paths with no
       `/`, `root_globs`.
    """
    rel_posix = rel.replace(os.sep, "/")
    for rule in rules:
        for rf in rule.root_files:
            if rel_posix == rf.replace(os.sep, "/"):
                return rule.id
    for rule in rules:
        for pref in rule.path_prefixes:
            pref_n = pref.replace(os.sep, "/")
            if rel_posix.startswith(pref_n):
                return rule.id
    if "/" not in rel_posix:
        for rule in rules:
            for pat in rule.root_globs:
                if fnmatch.fnmatch(rel_posix, pat):
                    return rule.id
    return None


def should_skip(rel: str) -> bool:
    base = Path(rel).name
    if base == ".DS_Store":
        return True
    if base.endswith(".blend") or base.endswith(".blend1"):
        return True
    # Authoring tree + gltf-transform scratch (not loaded by the game).
    if rel.startswith("3d/source/") or rel.startswith("3d/_gltf_sidecars/"):
        return True
    # Loose maps next to GLBs (orphans) — never ship in packs.
    if rel.startswith("3d/") and Path(rel).suffix.lower() in {
        ".jpg",
        ".jpeg",
        ".png",
        ".bin",
    }:
        return True
    # Source relic art — runtime loads pre-baked RLC1 under data/relic_baked/.
    if rel.startswith("textures/relics/"):
        return True
    return False


def zip_write_params(rel: str) -> tuple[int, int | None]:
    """Use STORED for already-compressed blobs; DEFLATE for JSON and similar."""
    ext = Path(rel).suffix.lower()
    if ext in {".png", ".jpg", ".jpeg", ".webp", ".ogg", ".mp3", ".glb"}:
        return zipfile.ZIP_STORED, None
    return zipfile.ZIP_DEFLATED, 6


def maybe_resize_room_env_glb(src: Path, tmp_out: Path) -> bool:
    """Resize embedded textures in room GLBs. Returns True if handled."""
    if src.name.lower() not in ROOM_ENV_GLB_NAMES:
        return False
    transform = shutil.which("gltf-transform")
    if transform is None:
        size_mb = src.stat().st_size / (1024 * 1024)
        if size_mb > 32:
            print(
                f"bake_assets: warning: {src.name} is {size_mb:.0f} MB — "
                f"install @gltf-transform/cli and re-bake to resize textures to "
                f"{ROOM_ENV_TEXTURE_MAX}px (faster startup)",
                file=sys.stderr,
            )
        return False
    # gltf-transform writes JSON when the output path lacks a .glb/.gltf suffix.
    out_glb = tmp_out if tmp_out.suffix.lower() == ".glb" else tmp_out.with_suffix(".glb")
    r = subprocess.run(
        [
            transform,
            "resize",
            str(src),
            str(out_glb),
            "--width",
            str(ROOM_ENV_TEXTURE_MAX),
            "--height",
            str(ROOM_ENV_TEXTURE_MAX),
        ],
        capture_output=True,
        text=True,
    )
    if r.returncode == 0 and out_glb.is_file() and out_glb.stat().st_size > 1024:
        if out_glb != tmp_out:
            shutil.copy2(out_glb, tmp_out)
            out_glb.unlink(missing_ok=True)
        return True
    print(
        f"bake_assets: gltf-transform resize failed for {src.name}: {r.stderr or r.stdout}",
        file=sys.stderr,
    )
    return False


def process_file(src: Path, rel: str, tmp_out: Path, lossy: bool) -> None:
    """Write processed bytes to tmp_out (single file)."""
    suf = src.suffix.lower()
    if suf == ".glb" and maybe_resize_room_env_glb(src, tmp_out):
        return
    if suf == ".json":
        data = parse_json_file(src, label=rel)
        tmp_out.write_bytes(json.dumps(data, separators=(",", ":"), ensure_ascii=False).encode("utf-8"))
        return

    if lossy and suf == ".png" and shutil.which("pngquant"):
        r = subprocess.run(
            [
                "pngquant",
                "--force",
                "--output",
                str(tmp_out),
                "--quality",
                "65-90",
                str(src),
            ],
            capture_output=True,
        )
        if r.returncode == 0 and tmp_out.is_file():
            if shutil.which("oxipng"):
                subprocess.run(
                    ["oxipng", "-o", "4", "--strip", str(tmp_out)],
                    capture_output=True,
                    check=False,
                )
            return
        # fall through to copy

    if lossy and suf in {".ogg", ".mp3"} and shutil.which("ffmpeg"):
        # ffmpeg picks the muxer from the output suffix; force .ogg / .mp3 even if tmp name is odd.
        audio_out = tmp_out.with_suffix(suf)
        r = subprocess.run(
            [
                "ffmpeg",
                "-y",
                "-i",
                str(src),
                "-c:a",
                "libvorbis" if suf == ".ogg" else "libmp3lame",
                "-q:a",
                "5",
                str(audio_out),
            ],
            capture_output=True,
        )
        if r.returncode == 0 and audio_out.is_file():
            if audio_out != tmp_out:
                audio_out.replace(tmp_out)
            return

    shutil.copy2(src, tmp_out)
    if lossy and suf == ".png" and shutil.which("oxipng"):
        subprocess.run(
            ["oxipng", "-o", "4", "--strip", str(tmp_out)],
            capture_output=True,
            check=False,
        )


def atomic_zip_path(out_dir: Path, final_name: str) -> Path:
    """Per-process staging path in `out_dir` (same filesystem → atomic os.replace).

    Includes the pid so two concurrent bakes (two `cargo` processes running
    build.rs at once) stage to distinct files; each then atomically replaces the
    final, last-writer-wins, and neither ever exposes a half-written pack.
    """
    return out_dir / f".{final_name}.{os.getpid()}.tmp"


def sha256_file(path: Path) -> str:
    h = hashlib.sha256()
    with path.open("rb") as f:
        for chunk in iter(lambda: f.read(1 << 20), b""):
            h.update(chunk)
    return h.hexdigest()


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument(
        "--assets",
        type=Path,
        default=REPO_ROOT / "assets",
        help="Source assets directory",
    )
    ap.add_argument(
        "--out",
        type=Path,
        required=True,
        help="Output directory for pack_manifest.json and zips",
    )
    ap.add_argument(
        "--lossy",
        action=argparse.BooleanOptionalAction,
        default=True,
        help="Run lossy optimizers when tools are available (default: on)",
    )
    args = ap.parse_args()

    assets_dir: Path = args.assets
    out_dir: Path = args.out
    out_dir.mkdir(parents=True, exist_ok=True)

    rules = load_rules()

    assignments: dict[str, list[tuple[str, Path]]] = {r.id: [] for r in rules}

    for dirpath, _dirnames, filenames in os.walk(assets_dir):
        dpath = Path(dirpath)
        for fn in filenames:
            full = dpath / fn
            rel = full.relative_to(assets_dir).as_posix()
            if should_skip(rel):
                continue
            pid = assign_pack(rel, rules)
            if pid is None:
                print(f"error: no pack for {rel}", file=sys.stderr)
                return 1
            assignments[pid].append((rel, full))

    # verify disjoint
    seen: set[str] = set()
    for _pid, items in assignments.items():
        for rel, _ in items:
            if rel in seen:
                print(f"error: duplicate assignment {rel}", file=sys.stderr)
                return 1
            seen.add(rel)

    game_version = cargo_version()
    sha = git_sha()
    manifest_packs: list[dict] = []

    with tempfile.TemporaryDirectory() as td:
        tdir = Path(td)
        for rule in rules:
            items = assignments[rule.id]
            zip_path = out_dir / rule.file
            path_prefixes = sorted(rule.path_prefixes, key=len, reverse=True)
            entry = {
                "id": rule.id,
                "file": rule.file,
                "load_tier": rule.load_tier,
                "path_prefixes": path_prefixes,
                "root_files": rule.root_files,
                "root_globs": rule.root_globs,
            }

            # Build into a temp file in `out_dir`, then atomically replace the
            # final zip. The game (and concurrent bakes — two `cargo` processes
            # can run build.rs at once) read these packs from `out_dir`; an
            # in-place `ZipFile(zip_path, "w")` truncates and streams hundreds of
            # MB, so a reader opening mid-write sees a torn archive (missing or
            # corrupt entries → runtime "asset missing"). os.replace on the same
            # filesystem is atomic, so readers only ever observe a complete pack.
            staged_zip = atomic_zip_path(out_dir, rule.file)
            try:
                with zipfile.ZipFile(staged_zip, "w") as zf:
                    for rel, src in sorted(items, key=lambda x: x[0]):
                        tmp = tdir / rel.replace("/", "__")
                        process_file(src, rel, tmp, lossy=args.lossy)
                        ct, cl = zip_write_params(rel)
                        if cl is None:
                            zf.write(tmp, arcname=rel, compress_type=ct)
                        else:
                            zf.write(tmp, arcname=rel, compress_type=ct, compresslevel=cl)
                        tmp.unlink(missing_ok=True)

                entry["sha256"] = sha256_file(staged_zip)
                entry["size_bytes"] = staged_zip.stat().st_size
                os.replace(staged_zip, zip_path)
            except BaseException:
                staged_zip.unlink(missing_ok=True)
                raise
            manifest_packs.append(entry)

    manifest = {
        "schema_version": 1,
        "game_version": game_version,
        "git_sha": sha,
        "bake_profile": "release" if args.lossy else "lossless",
        "packs": manifest_packs,
    }
    # Manifest last + atomically: readers gate everything on it, so it must only
    # appear once all packs it references are in place.
    manifest_tmp = atomic_zip_path(out_dir, "pack_manifest.json")
    try:
        manifest_tmp.write_text(
            json.dumps(manifest, indent=2) + "\n", encoding="utf-8"
        )
        os.replace(manifest_tmp, out_dir / "pack_manifest.json")
    except BaseException:
        manifest_tmp.unlink(missing_ok=True)
        raise
    print(f"Wrote packs to {out_dir} (game_version={game_version} sha={sha})")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
