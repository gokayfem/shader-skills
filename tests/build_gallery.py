#!/usr/bin/env python3
"""Write gallery/<template>.glsl: each template bundled with its library dependencies, ready to paste into Shadertoy.

  python3 tests/build_gallery.py           regenerate
  python3 tests/build_gallery.py --check   fail if any bundle is stale (CI)
"""

from __future__ import annotations

import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "skills" / "shader-art-coding" / "scripts"))

import build_viewer
import glsl_tools as gt  # noqa: E402


def main() -> int:
    check = "--check" in sys.argv
    lib = gt.Library()
    out_dir = ROOT / "gallery"
    out_dir.mkdir(exist_ok=True)
    stale = []
    bundles = {tpl.name: gt.bundle_source(lib, tpl.read_text(encoding="utf-8"))
               for tpl in sorted(gt.TEMPLATE_DIR.glob("*.glsl"))}
    for scene, (_, shaders) in build_viewer.multipass_sources(lib).items():
        for name, source in shaders.items():
            bundles[f"{scene}/{name}.glsl"] = source
    for name, text in bundles.items():
        target = out_dir / name
        if check:
            if not target.exists() or target.read_text(encoding="utf-8") != text:
                stale.append(name)
        else:
            target.parent.mkdir(exist_ok=True)
            target.write_text(text, encoding="utf-8")
            print(f"wrote gallery/{name} ({text.count(chr(10))} lines)")
    build_viewer.build(out_dir / "studies.html", check)
    if stale:
        print("stale gallery bundles (run tests/build_gallery.py):", ", ".join(stale))
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
