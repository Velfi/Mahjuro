#!/usr/bin/env python3
"""Generate heightmap PNGs for each talisman kind.

Run:
    python3 tools/gen_talisman_heightmaps.py

Outputs four 256x256 grayscale PNGs into assets/textures/:
    talisman_jade.png
    talisman_pearl.png
    talisman_gilded.png
    talisman_polychrome.png

These are grayscale heightfields sampled by the lit_mesh.wgsl talisman
branch (kind 7). Mid-gray (128) is the neutral surface plane; brighter
pixels are raised, darker pixels are recessed. The shader reads finite
differences to perturb the surface normal so the carved motifs catch
candlelight.

The octagonal tablet silhouette is baked into each map — pixels outside
the octagon are pinned to mid-gray so the rim faces stay unperturbed.
"""

import math
import os
import struct
import zlib

# ---------------------------------------------------------------------------
# Config
# ---------------------------------------------------------------------------

SIZE = 256          # px, square
MID = 0.5           # neutral height (no perturbation)
OUT_DIR = os.path.join(os.path.dirname(__file__), "..", "assets", "textures")

# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------

def smoothstep(edge0: float, edge1: float, x: float) -> float:
    t = max(0.0, min(1.0, (x - edge0) / (edge1 - edge0)))
    return t * t * (3.0 - 2.0 * t)


def oct_dist(x: float, y: float) -> float:
    """Octagonal distance from origin (Chebyshev blended with L1)."""
    ax, ay = abs(x), abs(y)
    return ax * 0.9239 + ay * 0.3827 if ax > ay else ay * 0.9239 + ax * 0.3827


def clamp01(v: float) -> float:
    return max(0.0, min(1.0, v))


def make_grid():
    """Return (u, v) arrays in [-1, 1] and an octagonal mask in [0, 1]."""
    us = []
    vs = []
    masks = []
    for y in range(SIZE):
        for x in range(SIZE):
            u = (x / (SIZE - 1)) * 2.0 - 1.0
            v = (y / (SIZE - 1)) * 2.0 - 1.0
            d = oct_dist(u, v)
            mask = smoothstep(0.94, 0.88, d)
            us.append(u)
            vs.append(v)
            masks.append(mask)
    return us, vs, masks


def write_png(path: str, pixels: list[float]):
    """Write a SIZE x SIZE grayscale image as an 8-bit PNG (no PIL needed)."""
    def make_chunk(chunk_type: bytes, data: bytes) -> bytes:
        c = chunk_type + data
        return struct.pack(">I", len(data)) + c + struct.pack(">I", zlib.crc32(c) & 0xFFFFFFFF)

    sig = b"\x89PNG\r\n\x1a\n"
    ihdr = struct.pack(">IIBBBBB", SIZE, SIZE, 8, 0, 0, 0, 0)  # 8-bit grayscale
    raw = b""
    for y in range(SIZE):
        raw += b"\x00"  # filter: None
        for x in range(SIZE):
            raw += bytes([int(clamp01(pixels[y * SIZE + x]) * 255.0 + 0.5)])
    idat = zlib.compress(raw, 9)
    png = sig + make_chunk(b"IHDR", ihdr) + make_chunk(b"IDAT", idat) + make_chunk(b"IEND", b"")
    os.makedirs(os.path.dirname(path), exist_ok=True)
    with open(path, "wb") as f:
        f.write(png)
    print(f"  wrote {path}  ({os.path.getsize(path)} bytes)")


# ---------------------------------------------------------------------------
# Jade -- carved bi-disc pendant with cloud scrollwork
# ---------------------------------------------------------------------------

def gen_jade(us, vs, masks):
    """Concentric octagonal borders, central bi-disc ring, cloud scrolls."""
    pixels = []
    for i in range(len(us)):
        u, v, mask = us[i], vs[i], masks[i]
        d = oct_dist(u, v)
        r = math.hypot(u, v)
        angle = math.atan2(v, u)

        # -- Outer border: double raised rim near the edge.
        outer = smoothstep(0.82, 0.79, d) - smoothstep(0.76, 0.73, d)
        outer2 = smoothstep(0.88, 0.85, d) - smoothstep(0.83, 0.80, d)

        # -- Inner border frame.
        inner = smoothstep(0.58, 0.55, d) - smoothstep(0.52, 0.49, d)

        # -- Central bi-disc ring (raised torus).
        ring = smoothstep(0.32, 0.28, r) - smoothstep(0.22, 0.18, r)

        # -- Central hole depression (the bi-disc aperture).
        hole = smoothstep(0.15, 0.10, r)

        # -- Cloud scroll motif between inner border and ring.
        # Four symmetric cloud curls using polar modulation.
        cloud_r = r * 2.5
        cloud_angle = angle * 4.0
        curl = math.sin(cloud_angle + cloud_r * 3.0) * 0.5 + 0.5
        curl *= smoothstep(0.18, 0.28, r) * smoothstep(0.52, 0.38, r)
        curl *= 0.6

        h = MID
        h += outer * 0.22 + outer2 * 0.18
        h += inner * 0.16
        h += ring * 0.20
        h -= hole * 0.25
        h += curl * 0.12

        pixels.append(MID + (h - MID) * mask)
    return pixels


# ---------------------------------------------------------------------------
# Pearl -- concentric luster ripples with organic irregularity
# ---------------------------------------------------------------------------

def gen_pearl(us, vs, masks):
    """Smooth concentric ripples with slight angular wobble."""
    pixels = []
    for i in range(len(us)):
        u, v, mask = us[i], vs[i], masks[i]
        d = oct_dist(u, v)
        r = math.hypot(u, v)
        angle = math.atan2(v, u)

        # Slightly wobbly radius for organic feel.
        wobble = 1.0 + 0.06 * math.sin(angle * 5.0) + 0.04 * math.sin(angle * 7.0 + 1.3)
        rw = r * wobble

        # Concentric ripples, amplitude fading toward edge.
        ripple = math.sin(rw * 18.0) * 0.5 + 0.5
        ripple *= smoothstep(0.90, 0.20, r)

        # Gentle central dome.
        dome = max(0.0, 1.0 - r * 1.1) ** 1.5

        # Outer border.
        border = smoothstep(0.86, 0.83, d) - smoothstep(0.80, 0.77, d)

        h = MID
        h += ripple * 0.22
        h += dome * 0.12
        h += border * 0.18

        pixels.append(MID + (h - MID) * mask)
    return pixels


# ---------------------------------------------------------------------------
# Gilded -- hammered gold filigree lattice
# ---------------------------------------------------------------------------

def gen_gilded(us, vs, masks):
    """Diamond lattice with raised nodes and an ornate border."""
    pixels = []
    freq = 5.5
    for i in range(len(us)):
        u, v, mask = us[i], vs[i], masks[i]
        d = oct_dist(u, v)

        # Diamond lattice (45-deg rotated grid).
        ru = (u + v) * 0.7071
        rv = (u - v) * 0.7071
        gx = (math.sin(ru * freq * math.pi) * 0.5 + 0.5) ** 0.5
        gy = (math.sin(rv * freq * math.pi) * 0.5 + 0.5) ** 0.5
        lattice = (gx * gy) ** 0.6

        # Raised nodes at lattice intersections.
        nx = abs(math.cos(ru * freq * math.pi)) ** 10.0
        ny = abs(math.cos(rv * freq * math.pi)) ** 10.0
        nodes = nx * ny

        # Ornate double border.
        b1 = smoothstep(0.86, 0.83, d) - smoothstep(0.80, 0.77, d)
        b2 = smoothstep(0.75, 0.72, d) - smoothstep(0.69, 0.66, d)

        # Fade lattice near border to avoid collision.
        lattice_mask = smoothstep(0.68, 0.58, d)

        h = MID
        h += lattice * 0.18 * lattice_mask
        h += nodes * 0.22 * lattice_mask
        h += b1 * 0.22 + b2 * 0.16

        pixels.append(MID + (h - MID) * mask)
    return pixels


# ---------------------------------------------------------------------------
# Polychrome -- prismatic starburst with faceted rays
# ---------------------------------------------------------------------------

def gen_polychrome(us, vs, masks):
    """Radial starburst with angular facets and a central gem."""
    pixels = []
    n_rays = 8
    for i in range(len(us)):
        u, v, mask = us[i], vs[i], masks[i]
        d = oct_dist(u, v)
        r = math.hypot(u, v)
        angle = math.atan2(v, u)

        # Radial rays: sharp angular peaks.
        ray_angle = angle * n_rays
        rays = (math.sin(ray_angle) * 0.5 + 0.5) ** 2.0
        # Fade rays: visible from inner ring outward, tapering at edge.
        rays *= smoothstep(0.15, 0.30, r) * smoothstep(0.82, 0.50, r)

        # Central gem: raised faceted circle.
        gem_facets = abs(math.cos(angle * 6.0)) ** 0.3
        gem = max(0.0, 1.0 - r * 5.0) * gem_facets

        # Concentric ring pulse at mid-radius.
        pulse = max(0.0, 1.0 - abs(r * 3.5 - 1.4)) * 0.7

        # Five-pointed star overlay (rotated from rays).
        star_angle = angle * 5.0 + math.pi * 0.5
        star = abs(math.cos(star_angle)) ** 0.5
        star *= max(0.0, 1.0 - r * 1.8)

        # Border.
        border = smoothstep(0.86, 0.83, d) - smoothstep(0.80, 0.77, d)

        h = MID
        h += rays * 0.20
        h += gem * 0.20
        h += pulse * 0.12
        h += star * 0.14
        h += border * 0.18

        pixels.append(MID + (h - MID) * mask)
    return pixels


# ---------------------------------------------------------------------------
# Kiln -- cracked clay surface with a central flame motif
# ---------------------------------------------------------------------------

def gen_kiln(us, vs, masks):
    """Cracked kiln surface: radial cracks from centre, baked-clay texture."""
    pixels = []
    for i in range(len(us)):
        u, v, mask = us[i], vs[i], masks[i]
        d = oct_dist(u, v)
        r = math.hypot(u, v)
        angle = math.atan2(v, u)

        # -- Outer border: single raised rim.
        border = smoothstep(0.85, 0.82, d) - smoothstep(0.78, 0.75, d)

        # -- Radial cracks emanating from centre (8 spokes, slightly wobbly).
        spoke_count = 8
        spoke_angle = angle * spoke_count / (2.0 * math.pi)
        spoke_frac = abs(spoke_angle - round(spoke_angle))
        crack_width = smoothstep(0.06, 0.02, spoke_frac)
        crack_depth = crack_width * smoothstep(0.15, 0.70, r) * smoothstep(0.82, 0.60, r)

        # -- Central flame: elongated upward teardrop.
        flame_u = u * 1.6
        flame_v = (v + 0.15) * 1.3
        flame_r = math.hypot(flame_u, flame_v)
        flame_mask = smoothstep(0.45, 0.30, flame_r)
        # Taper the top: narrower above centre.
        if flame_v < 0:
            flame_mask *= smoothstep(0.35, 0.15, abs(flame_u) - flame_v * 0.3)
        flame = flame_mask * 0.22

        # -- Baked-clay grain: high-freq noise approximation.
        grain = math.sin(u * 31.7 + v * 17.3) * math.sin(v * 23.1 - u * 11.9)
        grain = grain * 0.04 * smoothstep(0.88, 0.40, d)

        h = MID
        h += border * 0.18
        h -= crack_depth * 0.16
        h += flame
        h += grain

        pixels.append(MID + (h - MID) * mask)
    return pixels


# ---------------------------------------------------------------------------
# Main
# ---------------------------------------------------------------------------

def main():
    print("Generating talisman heightmaps...")
    us, vs, masks = make_grid()

    generators = [
        ("talisman_jade.png",       gen_jade),
        ("talisman_pearl.png",      gen_pearl),
        ("talisman_gilded.png",     gen_gilded),
        ("talisman_polychrome.png", gen_polychrome),
        ("talisman_kiln.png",       gen_kiln),
    ]

    for filename, gen_fn in generators:
        pixels = gen_fn(us, vs, masks)
        write_png(os.path.join(OUT_DIR, filename), pixels)

    print("Done.")


if __name__ == "__main__":
    main()
