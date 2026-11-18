"""Regression coverage for dependency insertion around quality-preset preprocessor blocks."""
import sys
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "skills/shader-art-coding/scripts"))
import glsl_tools as gt


class ConditionalPreamble(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.lib = gt.Library()

    def test_open_conditional_does_not_hide_dependencies(self):
        source = """#define QUALITY 2
#if QUALITY == 3
const int AA = 4;
#else
const int AA = 1;
#endif
float scene(vec2 p) { return gradientNoise(p); }
"""
        result = gt.bundle_source(self.lib, source)
        self.assertLess(result.index("#define QUALITY"), result.index("float gradientNoise("))
        self.assertLess(result.index("float gradientNoise("), result.index("#if QUALITY"))
        self.assertIn("const int AA = 1;", result)

    def test_balanced_macro_block_stays_before_library(self):
        source = """#define QUALITY 2
#if QUALITY == 3
#define AA 4
#else
#define AA 1
#endif
float scene(vec2 p) { return gradientNoise(p); }
"""
        result = gt.bundle_source(self.lib, source)
        self.assertLess(result.index("#endif"), result.index("float gradientNoise("))

    def test_indented_open_conditional_is_safe(self):
        source = "  #if QUALITY == 3\nconst int AA = 4;\n  #endif\nfloat scene(vec2 p) { return gradientNoise(p); }"
        result = gt.bundle_source(self.lib, source)
        self.assertLess(result.index("float gradientNoise("), result.index("#if QUALITY"))

    def test_annotated_variant_changes_only_its_define(self):
        variants = gt.template_variants("// @variant QUALITY=3\n#define QUALITY 2 // budget\n#define DEBUG_MODE 0\n")
        self.assertEqual(variants[0][0], "quality-3")
        self.assertIn("#define QUALITY 3 // budget", variants[0][1])
        self.assertIn("#define DEBUG_MODE 0", variants[0][1])

    def test_missing_variant_define_is_rejected(self):
        with self.assertRaises(ValueError):
            gt.template_variants("// @variant QUALITY=3\n#define DEBUG_MODE 0\n")


class ShaderLint(unittest.TestCase):
    def test_smoothstep_edges(self):
        for edges in ("1., .25", "2e-2, 0.02", "-0.2, -1.0"):
            self.assertTrue(gt.lint_shader(f"smoothstep({edges}, x)", "test", False))
        source = "smoothstep(-0.5, 0.0, x) + smoothstep(lo, hi, x); // smoothstep(1., 0., x)"
        self.assertFalse(gt.lint_shader(source, "test", False))


if __name__ == "__main__":
    unittest.main()
