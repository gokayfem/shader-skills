"""Exercise the exact-helper lookup through the public CLI."""
import contextlib
import io
import sys
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "skills/shader-art-coding/scripts"))
import glsl_tools as gt


class ExampleLookup(unittest.TestCase):
    def test_locates_fresnel_call_and_diagnostic(self):
        output = io.StringIO()
        with contextlib.redirect_stdout(output):
            self.assertEqual(gt.main(["examples", "fresnelDielectric"]), 0)
        lines = output.getvalue().splitlines()
        self.assertIn("glsl/water.glsl", lines[0])
        path, line = lines[1].rsplit(":", 1)
        self.assertEqual(path, "templates/coastal-water.glsl")
        self.assertIn("fresnelDielectric(", (gt.SKILL_DIR / path).read_text().splitlines()[int(line) - 1])
        self.assertIn("5 Fresnel", lines[2])

    def test_finds_multipass_source(self):
        output = io.StringIO()
        with contextlib.redirect_stdout(output):
            self.assertEqual(gt.main(["examples", "hash32"]), 0)
        self.assertIn("multipass/reaction-diffusion/buffer-a.glsl:", output.getvalue())

    def test_rejects_partial_names(self):
        with contextlib.redirect_stderr(io.StringIO()):
            self.assertEqual(gt.main(["examples", "fresnelDielect"]), 1)
