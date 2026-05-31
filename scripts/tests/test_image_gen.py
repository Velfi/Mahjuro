from __future__ import annotations

import sys
import unittest
from pathlib import Path
from types import SimpleNamespace

SCRIPTS = Path(__file__).resolve().parents[1]
if str(SCRIPTS) not in sys.path:
    sys.path.insert(0, str(SCRIPTS))

from _image_gen import _candidate_image_parts, _describe_gemini_response  # noqa: E402


class ImageGenHelpersTest(unittest.TestCase):
    def test_candidate_image_parts_handles_missing_content(self) -> None:
        candidate = SimpleNamespace(content=None)
        self.assertEqual(_candidate_image_parts(candidate), [])

    def test_candidate_image_parts_handles_missing_parts(self) -> None:
        candidate = SimpleNamespace(content=SimpleNamespace(parts=None))
        self.assertEqual(_candidate_image_parts(candidate), [])

    def test_candidate_image_parts_returns_inline_data(self) -> None:
        part = SimpleNamespace(inline_data=SimpleNamespace(data=b"png"))
        candidate = SimpleNamespace(content=SimpleNamespace(parts=[part]))
        self.assertEqual(_candidate_image_parts(candidate), [part])

    def test_describe_gemini_response_includes_finish_reason(self) -> None:
        response = SimpleNamespace(prompt_feedback=None)
        candidate = SimpleNamespace(
            content=None,
            finish_reason="SAFETY",
            safety_ratings=[],
        )
        detail = _describe_gemini_response(response, candidate)
        self.assertIn("finish_reason='SAFETY'", detail)
        self.assertIn("candidate content missing", detail)


if __name__ == "__main__":
    unittest.main()
