"""Build the two paste-ready Shadertoy passes from the single-pass alpine scene."""
import argparse
from pathlib import Path

import glsl_tools as gt


def passes():
    folder = gt.SKILL_DIR / "multipass/alpine-cache"
    scene = (gt.TEMPLATE_DIR / "alpine-valley.glsl").read_text()
    funcs = {fn.name: fn.text for fn in gt.parse(scene) if fn.kind == "fn"}
    height = funcs["alpineHeight"]
    domain = (folder / "domain.glsl").read_text()
    bake = "\n".join([domain, funcs["alpineValleyMask"], height, (folder / "buffer-a.glsl").read_text()])
    lookup = (folder / "lookup.glsl").read_text()
    image = scene.replace(height, height.replace("alpineHeight(", "alpineHeightExact(", 1) + "\n" + domain + lookup)
    if image == scene:
        raise ValueError("Could not replace the alpine height function")
    lib = gt.Library()
    return {name: gt.bundle_source(lib, src) for name, src in [("buffer-a", bake), ("image", image)]}


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("directory", type=Path)
    parser.add_argument("--check", action="store_true")
    args = parser.parse_args()
    for name, shader in passes().items():
        path = args.directory / (name + ".glsl")
        if args.check:
            if not path.exists() or path.read_text() != shader:
                parser.exit(1, f"Stale cached terrain pass: {path}\n")
        else:
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text(shader)
