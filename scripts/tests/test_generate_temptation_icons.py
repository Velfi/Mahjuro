"""Tests for pure helpers in generate_temptation_icons.py.

Run:
    python -m unittest scripts.tests.test_generate_temptation_icons
"""

import importlib.util
import sys
import tempfile
import unittest
from pathlib import Path

from PIL import Image


REPO_ROOT = Path(__file__).resolve().parent.parent.parent
SCRIPT = REPO_ROOT / "scripts" / "generate_temptation_icons.py"


def _load_module():
    spec = importlib.util.spec_from_file_location(
        "generate_temptation_icons", SCRIPT
    )
    module = importlib.util.module_from_spec(spec)
    sys.modules["generate_temptation_icons"] = module
    spec.loader.exec_module(module)
    return module


class TagDataTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.m = _load_module()

    def test_tags_match_rust_all_order(self):
        json_slugs = [t.slug for t in self.m.TAGS]
        rust_slugs = self.m.load_tag_kind_order()
        self.assertEqual(json_slugs, rust_slugs)

    def test_every_tag_has_visual(self):
        for tag in self.m.TAGS:
            self.assertIn(tag.slug, self.m.TAG_VISUALS)

    def test_layout_is_three_by_three(self):
        self.assertEqual(len(self.m.LAYOUT), 9)
        self.assertEqual(len(self.m.LAYOUT), len(self.m.TAGS))

    def test_style_base_is_vector_inventory_art(self):
        self.assertIn("vector-style polygon shading", self.m.STYLE_BASE)
        self.assertIn("64×64", self.m.STYLE_BASE)
        self.assertIn("solid matte #000000", self.m.STYLE_BASE.lower())
        self.assertIn("wordless pictogram", self.m.STYLE_BASE.lower())


class ImageGenHelperTests(unittest.TestCase):
    """Sanity-check the shared scripts/_image_gen.py helper surface."""

    @classmethod
    def setUpClass(cls):
        sys.path.insert(0, str(REPO_ROOT / "scripts"))
        import _image_gen

        cls.helper = _image_gen

    def test_size_alias_normalisation(self):
        self.assertEqual(self.helper._normalize_image_size("512"), "512px")
        self.assertEqual(self.helper._normalize_image_size("1K"), "1K")
        self.assertIsNone(self.helper._normalize_image_size(None))
        with self.assertRaises(ValueError):
            self.helper._normalize_image_size("nope")

    def test_parse_size_pixel_pair(self):
        ar, tier = self.helper.parse_size("1024x1536")
        self.assertEqual(ar, "2:3")
        self.assertEqual(tier, "2K")

    def test_parse_size_explicit_pair(self):
        self.assertEqual(self.helper.parse_size("9:16@2K"), ("9:16", "2K"))

    def test_parse_size_tier_only(self):
        self.assertEqual(self.helper.parse_size("1K"), ("1:1", "1K"))


class PromptTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.m = _load_module()

    def test_build_prompt_includes_name_and_rarity(self):
        tag = self.m.TAGS[0]
        prompt = self.m.build_prompt(tag)
        self.assertIn(tag.name, prompt)
        self.assertIn(self.m.TAG_VISUALS[tag.slug], prompt)

    def test_prompts_accent_subject_edges_only(self):
        tag = self.m.TAGS[0]
        prompt = self.m.build_prompt(tag)
        self.assertIn("along the subject edges", prompt.lower())


class ContentFitTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.m = _load_module()

    def test_content_bbox_finds_subject(self):
        img = Image.new("RGBA", (64, 64), (0, 0, 0, 0))
        img.paste(Image.new("RGBA", (20, 30), (232, 177, 74, 255)), (10, 5))
        bbox = self.m.content_bbox(img)
        self.assertEqual(bbox, (10, 5, 30, 35))

    def test_fit_content_centers_subject(self):
        img = Image.new("RGBA", (64, 64), (0, 0, 0, 0))
        img.paste(Image.new("RGBA", (16, 16), (232, 177, 74, 255)), (24, 24))
        fitted = self.m.fit_content_to_square(img, side=32, fill=0.8)
        self.assertEqual(fitted.size, (32, 32))
        self.assertGreater(fitted.getpixel((16, 16))[3], 0)


class IconPostprocessTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.m = _load_module()

    def test_postprocess_output_size(self):
        src = Image.new("RGB", (512, 512), (10, 8, 6))
        draw_center = Image.new("RGB", (200, 200), (232, 177, 74))
        src.paste(draw_center, (156, 156))
        out = self.m.icon_postprocess(src, cell_size=128)
        self.assertEqual(out.size, (128, 128))
        self.assertEqual(out.mode, "RGBA")

    def test_black_background_becomes_transparent(self):
        src = Image.new("RGB", (256, 256), (0, 0, 0))
        src.paste(Image.new("RGB", (96, 96), (232, 177, 74)), (80, 80))
        out = self.m.icon_postprocess(src)
        self.assertEqual(out.getpixel((0, 0))[3], 0)
        center = out.getpixel((64, 64))
        self.assertGreater(center[3], 0)

    def test_white_background_becomes_transparent(self):
        src = Image.new("RGB", (256, 256), (255, 255, 255))
        src.paste(Image.new("RGB", (96, 96), (232, 177, 74)), (80, 80))
        out = self.m.icon_postprocess(src)
        self.assertEqual(out.getpixel((0, 0))[3], 0)
        center = out.getpixel((64, 64))
        self.assertGreater(center[3], 0)

    def test_dark_shadow_facets_are_preserved(self):
        src = Image.new("RGB", (256, 256), (0, 0, 0))
        src.paste(Image.new("RGB", (120, 120), (232, 177, 74)), (68, 68))
        src.paste(Image.new("RGB", (40, 40), (28, 22, 17)), (88, 108))
        out = self.m.remove_corner_background(src)
        self.assertEqual(out.getpixel((0, 0))[3], 0)
        self.assertGreater(out.getpixel((108, 128))[3], 0)

    def test_light_highlights_are_preserved_on_white_backdrop(self):
        src = Image.new("RGB", (256, 256), (255, 255, 255))
        src.paste(Image.new("RGB", (120, 120), (232, 177, 74)), (68, 68))
        src.paste(Image.new("RGB", (30, 30), (244, 241, 232)), (98, 98))
        out = self.m.remove_corner_background(src)
        self.assertEqual(out.getpixel((0, 0))[3], 0)
        self.assertGreater(out.getpixel((113, 113))[3], 0)

    def test_palette_snap_uses_theme_colors(self):
        src = Image.new("RGBA", (32, 32), (0, 0, 0, 0))
        src.paste(Image.new("RGBA", (16, 16), (230, 180, 70, 255)), (8, 8))
        snapped = self.m.snap_to_palette(src)
        center = snapped.getpixel((16, 16))[:3]
        self.assertIn(center, self.m.THEME_PALETTE)


class AtlasPackTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.m = _load_module()

    def test_pack_atlas_writes_png_and_toml(self):
        with tempfile.TemporaryDirectory() as tmp:
            proc = Path(tmp) / "processed"
            out = Path(tmp) / "out"
            proc.mkdir()
            for slug in self.m.LAYOUT:
                img = Image.new("RGBA", (128, 128), (232, 177, 74, 255))
                img.save(proc / f"tag_{slug}.png")
            atlas_path = self.m.pack_atlas(proc, out)
            self.assertTrue(atlas_path.exists())
            self.assertTrue((out / "atlas.toml").exists())
            atlas = Image.open(atlas_path)
            self.assertEqual(atlas.size, (384, 384))


if __name__ == "__main__":
    unittest.main()
