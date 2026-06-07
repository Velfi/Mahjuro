"""Tests for pure helpers in generate_boss_icons.py.

Run:
    python -m unittest scripts.tests.test_generate_boss_icons
"""

import importlib.util
import sys
import tempfile
import unittest
from pathlib import Path

from PIL import Image

REPO_ROOT = Path(__file__).resolve().parent.parent.parent
SCRIPT = REPO_ROOT / "scripts" / "generate_boss_icons.py"


def _load_module():
    spec = importlib.util.spec_from_file_location(
        "generate_boss_icons", SCRIPT
    )
    module = importlib.util.module_from_spec(spec)
    sys.modules["generate_boss_icons"] = module
    spec.loader.exec_module(module)
    return module


class BossDataTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.m = _load_module()

    def test_boss_json_order_matches_rust_all(self):
        json_ids = [b.slug for b in self.m.ORDEALS]
        rust_ids = self.m.load_boss_kind_order()
        self.assertEqual(json_ids, rust_ids)

    def test_every_boss_has_visual(self):
        for boss in self.m.ORDEALS:
            self.assertIn(boss.slug, self.m.BOSS_VISUALS)

    def test_layout_is_full_five_by_five_grid(self):
        self.assertEqual(len(self.m.LAYOUT), 25)
        self.assertEqual(len(self.m.LAYOUT) % self.m.COLUMNS, 0)

    def test_style_base_is_vector_inventory_art(self):
        self.assertIn("vector-style polygon shading", self.m.STYLE_BASE)
        self.assertIn("64×64", self.m.STYLE_BASE)
        self.assertIn("solid matte #000000", self.m.STYLE_BASE.lower())


class BossAtlasPackTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.m = _load_module()

    def test_pack_atlas_writes_png_and_toml(self):
        with tempfile.TemporaryDirectory() as tmp:
            proc = Path(tmp) / "processed"
            out = Path(tmp) / "out"
            proc.mkdir()
            for boss in self.m.ORDEALS:
                img = Image.new("RGBA", (128, 128), (232, 177, 74, 255))
                img.save(proc / f"ordeal_{boss.slug}.png")
            atlas_path = self.m.pack_atlas(proc, out)
            self.assertTrue(atlas_path.exists())
            self.assertTrue((out / "atlas.toml").exists())
            with Image.open(atlas_path) as atlas:
                self.assertEqual(atlas.size, (2560, 2560))


if __name__ == "__main__":
    unittest.main()
