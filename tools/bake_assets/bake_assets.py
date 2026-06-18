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
import struct
import subprocess
import sys
import tempfile
import zipfile
from dataclasses import dataclass
from pathlib import Path


REPO_ROOT = Path(__file__).resolve().parents[2]
RULES_PATH = Path(__file__).resolve().parent / "pack_rules.json"

GLB_MAGIC = 0x46546C67
GLB_VERSION = 2
GLB_JSON = 0x4E4F534A
GLB_BIN = 0x004E4942

# 1x1 opaque white PNG. Packed GLBs keep valid texture slots, but shipped image
# payloads live in BTX1 under data/texture_baked/.
GLB_PLACEHOLDER_PNG = bytes.fromhex(
    "89504e470d0a1a0a0000000d49484452000000010000000108060000001f15c489"
    "0000000d49444154789c63f8ffffff7f0009fb03fd2a86e38a0000000049454e44ae426082"
)


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
    # Source relic art — runtime loads pre-baked RLC2 under data/relic_baked/.
    if rel.startswith("textures/relics/"):
        return True
    # Source texture PNGs — runtime loads BTX1 under data/texture_baked/.
    if rel.startswith("textures/") and Path(rel).suffix.lower() == ".png":
        if is_raw_runtime_texture(rel):
            return False
        return True
    return False


def is_raw_runtime_texture(rel: str) -> bool:
    return (
        rel == "textures/main_menu_logo.png"
        or rel == "textures/temptations/atlas.png"
        or rel.startswith("textures/ordeal_icons/")
        or (
            rel.startswith("textures/kenney_input-prompts/")
            and Path(rel).suffix.lower() == ".png"
        )
    )


def zip_write_params(rel: str) -> tuple[int, int | None]:
    """Use STORED for already-compressed blobs; DEFLATE for JSON and similar."""
    ext = Path(rel).suffix.lower()
    if ext in {".png", ".jpg", ".jpeg", ".webp", ".ogg", ".mp3", ".glb"}:
        return zipfile.ZIP_STORED, None
    return zipfile.ZIP_DEFLATED, 6


def parse_glb(data: bytes, *, label: str) -> tuple[dict, bytes]:
    if len(data) < 20:
        raise ValueError(f"{label}: too small for GLB")
    magic, version, total_len = struct.unpack_from("<III", data, 0)
    if magic != GLB_MAGIC or version != GLB_VERSION or total_len != len(data):
        raise ValueError(f"{label}: expected GLB v2")

    pos = 12
    json_obj: dict | None = None
    bin_chunk = b""
    while pos + 8 <= len(data):
        chunk_len, chunk_type = struct.unpack_from("<II", data, pos)
        pos += 8
        chunk = data[pos : pos + chunk_len]
        pos += chunk_len
        if chunk_type == GLB_JSON:
            json_obj = json.loads(chunk.rstrip(b" \t\r\n\0").decode("utf-8"))
        elif chunk_type == GLB_BIN:
            bin_chunk = chunk
    if json_obj is None:
        raise ValueError(f"{label}: missing JSON chunk")
    return json_obj, bin_chunk


def pad4(data: bytes, pad_byte: bytes) -> bytes:
    extra = (-len(data)) % 4
    if extra:
        return data + pad_byte * extra
    return data


def write_glb(json_obj: dict, bin_chunk: bytes, out: Path) -> None:
    json_bytes = json.dumps(json_obj, separators=(",", ":"), ensure_ascii=False).encode("utf-8")
    json_padded = pad4(json_bytes, b" ")
    bin_padded = pad4(bin_chunk, b"\0")
    total_len = 12 + 8 + len(json_padded) + 8 + len(bin_padded)
    out.write_bytes(
        struct.pack("<III", GLB_MAGIC, GLB_VERSION, total_len)
        + struct.pack("<II", len(json_padded), GLB_JSON)
        + json_padded
        + struct.pack("<II", len(bin_padded), GLB_BIN)
        + bin_padded
    )


def collect_protected_buffer_views(gltf: dict, image_views: set[int]) -> set[int]:
    protected: set[int] = set()
    for accessor in gltf.get("accessors", []):
        if isinstance(accessor, dict):
            view = accessor.get("bufferView")
            if isinstance(view, int):
                protected.add(view)
            sparse = accessor.get("sparse")
            if isinstance(sparse, dict):
                for key in ("indices", "values"):
                    part = sparse.get(key)
                    if isinstance(part, dict) and isinstance(part.get("bufferView"), int):
                        protected.add(part["bufferView"])
    for skin in gltf.get("skins", []):
        if isinstance(skin, dict):
            inverse_bind_matrices = skin.get("inverseBindMatrices")
            if isinstance(inverse_bind_matrices, int):
                accessor = gltf.get("accessors", [])[inverse_bind_matrices]
                if isinstance(accessor, dict) and isinstance(accessor.get("bufferView"), int):
                    protected.add(accessor["bufferView"])
    return protected & image_views


def maybe_strip_glb_images(src: Path, tmp_out: Path) -> bool:
    """Replace embedded image payloads with one tiny PNG while preserving texture slots."""
    try:
        gltf, bin_chunk = parse_glb(src.read_bytes(), label=src.name)
    except (OSError, ValueError, json.JSONDecodeError) as err:
        print(f"bake_assets: warning: could not inspect {src.name}: {err}", file=sys.stderr)
        return False

    images = gltf.get("images")
    buffer_views = gltf.get("bufferViews")
    buffers = gltf.get("buffers")
    if not isinstance(images, list) or not isinstance(buffer_views, list) or not isinstance(buffers, list):
        return False

    image_views: set[int] = set()
    for image in images:
        if isinstance(image, dict) and isinstance(image.get("bufferView"), int):
            image_views.add(image["bufferView"])
    if not image_views:
        return False

    protected_image_views = collect_protected_buffer_views(gltf, image_views)
    new_bin = bytearray()

    def append_aligned(payload: bytes) -> int:
        while len(new_bin) % 4:
            new_bin.append(0)
        offset = len(new_bin)
        new_bin.extend(payload)
        return offset

    placeholder_offset: int | None = None
    for index, view in enumerate(buffer_views):
        if not isinstance(view, dict):
            continue
        buffer_index = view.get("buffer", 0)
        if buffer_index != 0:
            continue
        old_offset = int(view.get("byteOffset", 0))
        old_len = int(view.get("byteLength", 0))
        if old_len < 0 or old_offset < 0 or old_offset + old_len > len(bin_chunk):
            raise SystemExit(f"error: {src.name}: bufferView {index} is outside the BIN chunk")
        if index in image_views and index not in protected_image_views:
            if placeholder_offset is None:
                placeholder_offset = append_aligned(GLB_PLACEHOLDER_PNG)
            view["byteOffset"] = placeholder_offset
            view["byteLength"] = len(GLB_PLACEHOLDER_PNG)
            view.pop("byteStride", None)
            view.pop("target", None)
            continue
        payload = bin_chunk[old_offset : old_offset + old_len]
        view["byteOffset"] = append_aligned(payload)

    if placeholder_offset is None:
        placeholder_offset = append_aligned(GLB_PLACEHOLDER_PNG)

    shared_placeholder_view: int | None = None
    for image in images:
        if not isinstance(image, dict):
            continue
        image["mimeType"] = "image/png"
        image.pop("uri", None)
        view = image.get("bufferView")
        if isinstance(view, int) and view in protected_image_views:
            if shared_placeholder_view is None:
                shared_placeholder_view = len(buffer_views)
                buffer_views.append(
                    {
                        "buffer": 0,
                        "byteOffset": placeholder_offset,
                        "byteLength": len(GLB_PLACEHOLDER_PNG),
                    }
                )
            image["bufferView"] = shared_placeholder_view

    buffers[0]["byteLength"] = len(new_bin)
    buffers[0].pop("uri", None)
    write_glb(gltf, bytes(new_bin), tmp_out)
    return True


def process_file(src: Path, rel: str, tmp_out: Path, lossy: bool) -> None:
    """Write processed bytes to tmp_out (single file)."""
    suf = src.suffix.lower()
    if suf == ".glb" and maybe_strip_glb_images(src, tmp_out):
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
