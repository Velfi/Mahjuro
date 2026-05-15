"""Tests for the pure-Python helpers in generate_relic_art.py.

Run:
    python -m unittest scripts.tests.test_generate_relic_art

These tests deliberately skip anything that would hit the OpenAI API —
only the deterministic helpers (rarity parser, alpha/mask builders,
prompt builder) are exercised.
"""

import importlib.util
import sys
import tempfile
import unittest
from pathlib import Path

from PIL import Image


REPO_ROOT = Path(__file__).resolve().parent.parent.parent
SCRIPT = REPO_ROOT / "scripts" / "generate_relic_art.py"


def _load_module():
    spec = importlib.util.spec_from_file_location("generate_relic_art", SCRIPT)
    module = importlib.util.module_from_spec(spec)
    sys.modules["generate_relic_art"] = module
    spec.loader.exec_module(module)
    return module


class RarityParserTests(unittest.TestCase):
    """`load_slug_to_rarity` must resolve every slug in RELICS."""

    @classmethod
    def setUpClass(cls):
        cls.m = _load_module()

    def test_every_relic_has_rarity(self):
        missing = [r[0] for r in self.m.RELICS if r[0] not in self.m.SLUG_TO_RARITY]
        self.assertEqual(missing, [], f"Slugs missing from relic.rs: {missing}")

    def test_rarity_values_are_known_tiers(self):
        known = set(self.m.METAL_PROFILES)
        for slug, rarity in self.m.SLUG_TO_RARITY.items():
            self.assertIn(rarity, known, f"{slug} has unknown rarity {rarity!r}")

    def test_rarity_for_raises_on_unknown_slug(self):
        with self.assertRaises(SystemExit):
            self.m.rarity_for("nonexistent_slug_zzz")


class AlphaFromHeightTests(unittest.TestCase):
    """`alpha_from_height` must cut dark-on-dark object renders correctly."""

    @classmethod
    def setUpClass(cls):
        cls.m = _load_module()

    def _make_object(self, path: Path, size=(64, 64), color=(20, 20, 20, 255)):
        """Object render: dark opaque rectangle (mimics the dark-iron failure)."""
        Image.new("RGBA", size, color).save(path, format="PNG")

    def _make_height_circle(self, path: Path, size=(64, 64), radius=20):
        """Height map: gray circle on black background."""
        img = Image.new("L", size, 0)
        from PIL import ImageDraw
        draw = ImageDraw.Draw(img)
        cx, cy = size[0] // 2, size[1] // 2
        draw.ellipse((cx - radius, cy - radius, cx + radius, cy + radius), fill=180)
        img.save(path, format="PNG")

    def test_silhouette_transplanted_from_height(self):
        with tempfile.TemporaryDirectory() as td:
            obj = Path(td) / "obj.png"
            h = Path(td) / "h.png"
            self._make_object(obj)
            self._make_height_circle(h)

            self.assertTrue(self.m.alpha_from_height(obj, h))

            with Image.open(obj) as result:
                alpha = result.convert("RGBA").split()[-1]
                # Corner pixel (outside silhouette) should be fully transparent.
                self.assertEqual(alpha.getpixel((0, 0)), 0)
                # Center pixel (inside silhouette) should be fully opaque.
                self.assertEqual(alpha.getpixel((32, 32)), 255)

    def test_interior_stays_fully_opaque(self):
        """Mid-gray height values must yield alpha=255, not a half-transparent fill."""
        with tempfile.TemporaryDirectory() as td:
            obj = Path(td) / "obj.png"
            h = Path(td) / "h.png"
            self._make_object(obj)
            # Height: entire frame mid-gray (128) — fully inside silhouette.
            Image.new("L", (32, 32), 128).save(h, format="PNG")

            self.assertTrue(self.m.alpha_from_height(obj, h))

            with Image.open(obj) as result:
                alpha = result.convert("RGBA").split()[-1]
                lo, hi = alpha.getextrema()
            self.assertEqual(lo, 255, "mid-gray height should give fully opaque alpha")
            self.assertEqual(hi, 255)

    def test_missing_inputs_return_false(self):
        with tempfile.TemporaryDirectory() as td:
            self.assertFalse(
                self.m.alpha_from_height(Path(td) / "no.png", Path(td) / "none.png")
            )

    def test_size_mismatch_is_resized(self):
        """Height map is resized to object dimensions if sizes differ."""
        with tempfile.TemporaryDirectory() as td:
            obj = Path(td) / "obj.png"
            h = Path(td) / "h.png"
            self._make_object(obj, size=(64, 64))
            self._make_height_circle(h, size=(32, 32), radius=10)

            self.assertTrue(self.m.alpha_from_height(obj, h))
            with Image.open(obj) as result:
                self.assertEqual(result.size, (64, 64))


class BuildEditMaskTests(unittest.TestCase):
    """`build_edit_mask` must emit a valid RGBA mask when the reference has a cutout alpha."""

    @classmethod
    def setUpClass(cls):
        cls.m = _load_module()

    def _make_reference_with_cutout(self, path: Path, size=(64, 64)):
        """Reference: opaque circle, transparent surround."""
        img = Image.new("RGBA", size, (0, 0, 0, 0))
        from PIL import ImageDraw
        draw = ImageDraw.Draw(img)
        cx, cy = size[0] // 2, size[1] // 2
        draw.ellipse((cx - 20, cy - 20, cx + 20, cy + 20), fill=(180, 60, 60, 255))
        img.save(path, format="PNG")

    def test_emits_binary_silhouette_with_matching_size(self):
        with tempfile.TemporaryDirectory() as td:
            ref = Path(td) / "ref.png"
            mask = Path(td) / "mask.png"
            self._make_reference_with_cutout(ref, size=(48, 48))

            self.assertTrue(self.m.build_edit_mask(ref, mask))
            with Image.open(mask) as m:
                # Gemini receives the mask as a second image; the helper
                # writes an L-mode binary silhouette (white = subject,
                # black = background).
                self.assertEqual(m.mode, "L")
                self.assertEqual(m.size, (48, 48))
                self.assertEqual(m.getpixel((24, 24)), 255)
                self.assertEqual(m.getpixel((0, 0)), 0)

    def test_fully_opaque_reference_refused(self):
        """A reference with no transparent surround has no silhouette to lock."""
        with tempfile.TemporaryDirectory() as td:
            ref = Path(td) / "ref.png"
            mask = Path(td) / "mask.png"
            Image.new("RGBA", (32, 32), (100, 100, 100, 255)).save(ref, format="PNG")
            self.assertFalse(self.m.build_edit_mask(ref, mask))
            self.assertFalse(mask.exists())

    def test_missing_reference_returns_false(self):
        with tempfile.TemporaryDirectory() as td:
            self.assertFalse(
                self.m.build_edit_mask(Path(td) / "missing.png", Path(td) / "m.png")
            )

    def test_mask_silhouette_matches_reference_alpha(self):
        """Subject pixels become white; background pixels become black."""
        with tempfile.TemporaryDirectory() as td:
            ref = Path(td) / "ref.png"
            mask = Path(td) / "mask.png"
            self._make_reference_with_cutout(ref, size=(32, 32))

            self.assertTrue(self.m.build_edit_mask(ref, mask))
            with Image.open(mask) as m:
                self.assertEqual(m.mode, "L")
                self.assertEqual(m.getpixel((16, 16)), 255)
                self.assertEqual(m.getpixel((0, 0)), 0)


class WriteMaskFromHeightTests(unittest.TestCase):
    """`write_mask_from_height` must derive a binary silhouette from the height map."""

    @classmethod
    def setUpClass(cls):
        cls.m = _load_module()

    def test_produces_binary_silhouette(self):
        with tempfile.TemporaryDirectory() as td:
            h = Path(td) / "h.png"
            mask = Path(td) / "m.png"
            # Height: left half gray, right half black.
            img = Image.new("L", (32, 16), 0)
            for x in range(16):
                for y in range(16):
                    img.putpixel((x, y), 180)
            img.save(h, format="PNG")

            self.assertTrue(self.m.write_mask_from_height(h, mask))
            with Image.open(mask) as m:
                self.assertEqual(m.mode, "L")
                self.assertEqual(m.getpixel((5, 8)), 255)
                self.assertEqual(m.getpixel((25, 8)), 0)

    def test_missing_height_returns_false(self):
        with tempfile.TemporaryDirectory() as td:
            self.assertFalse(
                self.m.write_mask_from_height(Path(td) / "missing.png", Path(td) / "m.png")
            )


class SpecularFromHeightTests(unittest.TestCase):
    """`write_specular_from_height` maps metal bright and enamel matte."""

    @classmethod
    def setUpClass(cls):
        cls.m = _load_module()

    def test_metal_brighter_than_enamel(self):
        self.assertGreater(
            self.m.specular_from_height_luma(220),
            self.m.specular_from_height_luma(120),
        )
        self.assertEqual(self.m.specular_from_height_luma(0), 0)
        self.assertEqual(self.m.specular_from_height_luma(255), 255)

    def test_writes_grayscale_png(self):
        with tempfile.TemporaryDirectory() as td:
            h = Path(td) / "h.png"
            spec = Path(td) / "s.png"
            img = Image.new("L", (16, 16), 0)
            for x in range(8):
                for y in range(16):
                    img.putpixel((x, y), 180)
            img.save(h, format="PNG")

            self.assertTrue(self.m.write_specular_from_height(h, spec))
            with Image.open(spec) as out:
                self.assertEqual(out.mode, "L")
                self.assertGreater(out.getpixel((4, 8)), out.getpixel((12, 8)))


class ObjectPromptTests(unittest.TestCase):
    """Text-only object prompts must not mention a relief guide that doesn't exist."""

    @classmethod
    def setUpClass(cls):
        cls.m = _load_module()

    def test_text_mode_omits_relief_guide(self):
        prompt = self.m.build_object_prompt(
            "Test", "A thing.", "Warm palette.", "Common", from_reference=False,
        )
        self.assertNotIn("Relief guide usage", prompt)
        self.assertNotIn("relief guide", prompt.lower())

    def test_reference_mode_includes_relief_guide(self):
        prompt = self.m.build_object_prompt(
            "Test", "A thing.", "Warm palette.", "Common", from_reference=True,
        )
        self.assertIn("Relief guide usage", prompt)

    def test_every_metal_tier_composes(self):
        for tier in ("Common", "Uncommon", "Rare", "Legendary"):
            prompt = self.m.build_object_prompt(
                "Test", "A thing.", "Warm palette.", tier,
            )
            self.assertIn(tier, prompt)


if __name__ == "__main__":
    unittest.main()
