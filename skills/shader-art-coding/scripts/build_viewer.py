"""Build an offline WebGL2 viewer for the templates and the two-pass examples."""
import argparse
import json
import re
from pathlib import Path

import glsl_tools as gt
from build_alpine_cache import passes


def multipass_sources(lib):
    folder = gt.SKILL_DIR / "multipass/reaction-diffusion"
    reaction = {name: gt.bundle_source(lib, (folder / (name + ".glsl")).read_text(), keep_docs=False)
                for name in ("image", "buffer-a")}
    room_dir = gt.SKILL_DIR / "multipass/room-radiosity"
    chamber = (gt.TEMPLATE_DIR / "pigment-chamber.glsl").read_text()
    geometry, image = chamber.split("void mainImage(", 1)
    common = geometry + (room_dir / "transport.glsl").read_text()
    room = {"image": gt.bundle_source(lib, "#define ROOM_RADIOSITY 1\n" + common + "void mainImage(" + image),
            "buffer-a": gt.bundle_source(lib, common + (room_dir / "buffer-a.glsl").read_text())}
    return {"room-radiosity": ("radiosity", room), "alpine-cache": ("cache", passes()), "reaction-diffusion": ("simulation", reaction)}


def build(output, check=False):
    lib = gt.Library()
    scenes = {}
    for path in sorted(gt.TEMPLATE_DIR.glob("*.glsl")):
        source = path.read_text()
        time = re.search(r"@render time=([\d.]+)", source)
        scenes[path.stem] = {"image": gt.bundle_source(lib, source, keep_docs=False),
                             "time": float(time[1]) if time else 0}
    # Both variants share the tested orbit solver and shading from the single-pass template.
    disk = scenes["lensed-disk"]
    disk["kind"] = "transfer"
    disk["buffer"] = disk["image"].replace("#define DISK_PASS 0", "#define DISK_PASS 1")
    disk["cachedImage"] = disk["image"].replace("#define DISK_PASS 0", "#define DISK_PASS 2")
    for name, (kind, stages) in multipass_sources(lib).items():
        scenes[name] = {"kind": kind, "image": stages["image"], "buffer": stages["buffer-a"]}
    for scene in scenes.values():
        scene["modes"] = sorted({int(m) for m in re.findall(r"#(?:el)?if\s+DEBUG_MODE\s*==\s*(\d+)", scene["image"])} | {0})
    assets = gt.SKILL_DIR / "viewer"
    data = {"prelude": gt.SHADERTOY_PRELUDE.replace("uniform sampler2D", "uniform highp sampler2D"), "main": gt.MAIN_COMPLETE, "scenes": scenes}
    html = (assets / "viewer.html").read_text().replace("/*VIEWER_JS*/", (assets / "viewer.js").read_text())
    html = html.replace("/*SCENE_DATA*/null", json.dumps(data, separators=(",", ":")).replace("</", "<\\/"))
    if check:
        if not output.is_file() or output.read_text() != html:
            raise SystemExit(f"Regenerate {output}")
    else:
        output.parent.mkdir(parents=True, exist_ok=True)
        output.write_text(html)


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("output", type=Path)
    parser.add_argument("--check", action="store_true")
    args = parser.parse_args()
    build(args.output, args.check)
