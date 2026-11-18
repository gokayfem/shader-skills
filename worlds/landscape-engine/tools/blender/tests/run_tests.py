"""Tests of the export tool, run with Blender headless. Not in CI, which has no Blender.

    python tools/blender/tests/run_tests.py [--blender PATH] [--skip-keeper]

1. A character with every fault the tool fixes (faulty_character.py) comes out fixed, and passes
   the checks. The file is read back and its values checked: the masked faces gone, the shape
   key applied, idle bones removed except one the role file names, no tiny weights, each
   material's baked colour, metal and roughness, and cut out only where it is see-through.
2. The same character exported as it is fails the checks.
3. The keeper (characters/keeper.py), with MakeHuman's plugin: two exports from scratch are the
   same to the byte, pass the checks, and cut out only the brows, lashes and hair; exported as
   it is, it fails the checks.
"""

import argparse
import collections
import json
import os
import struct
import subprocess
import sys
import tempfile
import zlib

HERE = os.path.dirname(os.path.abspath(__file__))
TOOLS = os.path.dirname(HERE)
REPO = os.path.dirname(os.path.dirname(TOOLS))
sys.path.insert(0, TOOLS)
from make_character import blender_path  # noqa: E402

failures = []


def expect(condition, message):
    print(("  ok    " if condition else "  FAIL  ") + message, flush=True)
    if not condition:
        failures.append(message)


def run(command):
    result = subprocess.run(command, capture_output=True, text=True)
    return result.returncode, result.stdout + result.stderr


class Glb:
    """A .glb file, read back: its JSON, and its binary chunk."""

    def __init__(self, path):
        raw = open(path, "rb").read()
        length = struct.unpack_from("<I", raw, 12)[0]
        self.doc = json.loads(raw[20:20 + length])
        self.binary = raw[20 + length + 8:]

    def view(self, index):
        view = self.doc["bufferViews"][index]
        start = view.get("byteOffset", 0)
        return self.binary[start:start + view["byteLength"]]

    def floats(self, accessor_index):
        accessor = self.doc["accessors"][accessor_index]
        assert accessor["componentType"] == 5126, "floats only"
        width = {"SCALAR": 1, "VEC2": 2, "VEC3": 3, "VEC4": 4}[accessor["type"]]
        data = self.view(accessor["bufferView"])[accessor.get("byteOffset", 0):]
        values = struct.unpack_from(f"<{accessor['count'] * width}f", data)
        return [values[i:i + width] for i in range(0, len(values), width)]

    def mesh(self, node_name):
        node = next(n for n in self.doc["nodes"] if n.get("name") == node_name)
        return self.doc["meshes"][node["mesh"]]

    def image(self, texture_index):
        source = self.doc["textures"][texture_index]["source"]
        return read_png(self.view(self.doc["images"][source]["bufferView"]))

    def material(self, name):
        return next(m for m in self.doc["materials"] if m.get("name") == name)


def read_png(data):
    """The texels of an 8-bit RGB or RGBA PNG, as tuples of four, row by row."""
    assert data[:8] == b"\x89PNG\r\n\x1a\n"
    offset, chunks, header = 8, [], None
    while offset < len(data):
        length, kind = struct.unpack_from(">I4s", data, offset)
        body = data[offset + 8:offset + 8 + length]
        if kind == b"IHDR":
            header = struct.unpack(">IIBBBBB", body)
        elif kind == b"IDAT":
            chunks.append(body)
        offset += 12 + length
    width, height, depth, colour, _, _, interlace = header
    assert depth == 8 and colour in (2, 6) and interlace == 0, header
    channels = 4 if colour == 6 else 3
    raw = zlib.decompress(b"".join(chunks))
    stride = width * channels
    rows, previous = [], bytearray(stride)
    for y in range(height):
        kind = raw[y * (stride + 1)]
        line = bytearray(raw[y * (stride + 1) + 1:(y + 1) * (stride + 1)])
        for x in range(stride):
            left = line[x - channels] if x >= channels else 0
            up = previous[x]
            corner = previous[x - channels] if x >= channels else 0
            if kind == 1:
                line[x] = (line[x] + left) & 255
            elif kind == 2:
                line[x] = (line[x] + up) & 255
            elif kind == 3:
                line[x] = (line[x] + (left + up) // 2) & 255
            elif kind == 4:
                p = left + up - corner
                pa, pb, pc = abs(p - left), abs(p - up), abs(p - corner)
                predictor = left if pa <= pb and pa <= pc else (up if pb <= pc else corner)
                line[x] = (line[x] + predictor) & 255
        rows.append(line)
        previous = line
    texels = []
    for line in rows:
        for x in range(width):
            texel = tuple(line[x * channels:(x + 1) * channels])
            texels.append(texel if channels == 4 else texel + (255,))
    return texels


def commonest(texels):
    """The texel value that covers most of an image, past the bake's cleared background."""
    counts = collections.Counter(t for t in texels if t[:3] != (0, 0, 0))
    return counts.most_common(1)[0][0]


def close(a, b, within=2):
    return all(abs(x - y) <= within for x, y in zip(a, b))


def faulty(blender, work):
    blend = os.path.join(work, "faulty.blend")
    code, log = run([blender, "--background", "--factory-startup", "--python-exit-code", "1",
                     "--python", os.path.join(HERE, "faulty_character.py"), "--", blend])
    expect(code == 0, "the faulty character is made")
    if code:
        print(log)
        return
    roles = os.path.join(HERE, "test.roles")
    export = [blender, "--background", "--factory-startup", "--python-exit-code", "1", blend,
              "--python", os.path.join(TOOLS, "solstice_export.py"), "--", None,
              "--roles", roles, "--texture-size", "64"]

    fixed = os.path.join(work, "fixed.glb")
    export[export.index(None)] = fixed
    code, log = run(export)
    expect(code == 0, "exported with its faults fixed, it passes the checks")
    if code:
        print(log)
        return
    expect("removed bones that deform nothing: ['b', 'c', 'control', 'idle_tip']" in log,
           "the idle leaf bones go: those whose only weights were tiny, and one that doesn't deform")
    glb = Glb(fixed)
    doc = glb.doc
    joints = [doc["nodes"][j]["name"] for j in doc["skins"][0]["joints"]]
    expect(sorted(joints) == ["Root", "a", "named_tip", "pelvis", "spine"],
           f"the bones a role names stay, even idle: {joints}")

    sheet = glb.mesh("sheet")["primitives"]
    # Blender's writer makes a part for each slot; both use the one material.
    expect(len({part["material"] for part in sheet}) == 1, "a material in two slots is one material")
    corners = {tuple(round(c, 4) for c in v)
               for part in sheet for v in glb.floats(part["attributes"]["POSITION"])}
    # A 9 by 9 grid, less the column of nine the mask hid.
    expect(len(corners) == 72, f"the masked vertices are gone: {len(corners)} left")
    primitive = sheet[0]
    positions = doc["accessors"][primitive["attributes"]["POSITION"]]
    expect("targets" not in primitive, "no morph targets are left")
    # The shape key moved the sheet 0.05 along Blender's +Y, which is glTF's -Z.
    expect(abs(positions["min"][2] + 0.05) < 1e-4 and abs(positions["max"][2] + 0.05) < 1e-4,
           f"the shape key is applied as it was mixed: z from {positions['min'][2]} to {positions['max'][2]}")
    expect("JOINTS_1" not in primitive["attributes"], "at most four weights a vertex")
    weights = glb.floats(primitive["attributes"]["WEIGHTS_0"])
    expect(all(abs(sum(w) - 1) < 1e-3 for w in weights), "every vertex's weights sum to one")
    tiny = sum(1 for w in weights for x in w if 0 < x < 0.01)
    expect(tiny == 0, f"no tiny weights are left: {tiny}")

    cloth = glb.material("cloth")
    expect(cloth.get("alphaMode") == "MASK", "the see-through material is cut out, not blended")
    texels = glb.image(cloth["pbrMetallicRoughness"]["baseColorTexture"]["index"])
    # Base colour 0.6, 0.3, 0.2 in linear light is 203, 149, 124 in sRGB.
    colour = commonest([t for t in texels if t[3] == 255])
    expect(close(colour[:3], (203, 149, 124)), f"its colour is baked as it was, not darkened: {colour}")
    # Across the middle of the 64 by 64 texture: solid at U 0.3, see-through at U 0.8.
    middle = 32 * 64
    solid_at, clear_at = texels[middle + 19][3], texels[middle + 51][3]
    expect(solid_at == 255 and clear_at < 128,
           f"it is see-through where its alpha says: {solid_at} at U 0.3, {clear_at} at U 0.8")
    mr = commonest(glb.image(cloth["pbrMetallicRoughness"]["metallicRoughnessTexture"]["index"]))
    expect(close(mr[1:3], (128, 0)), f"its roughness is 0.5, baked once though in two slots: {mr}")
    expect(any(m.get("name") == "cloth.001" for m in doc["materials"]),
           "the patch has its own copy of the cloth, baked into its own UVs")

    buckle = glb.material("buckle")
    colour = commonest(glb.image(buckle["pbrMetallicRoughness"]["baseColorTexture"]["index"]))
    expect(close(colour[:3], (231, 219, 124)), f"a metal's colour is baked, not black: {colour}")
    mr = commonest(glb.image(buckle["pbrMetallicRoughness"]["metallicRoughnessTexture"]["index"]))
    expect(close(mr[1:3], (77, 255)), f"its metallic, fed through a node, is baked: {mr}")
    solid = glb.material("solid")
    expect(solid.get("alphaMode", "OPAQUE") == "OPAQUE",
           "a material whose alpha is wired in but solid stays opaque")
    texels = glb.image(solid["pbrMetallicRoughness"]["baseColorTexture"]["index"])
    expect(all(t[3] == 255 for t in texels), "an opaque material's colour is opaque in every texel")
    expect(solid.get("doubleSided") is not True and cloth.get("doubleSided") is True,
           "solid materials are one-sided, cut-out ones two-sided")

    as_is = os.path.join(work, "as_is.glb")
    export[export.index(fixed)] = as_is
    export.append("--as-is")
    code, log = run(export)
    expect(code != 0 and "morph targets" in log and "is blended" in log,
           "exported as it is, it fails the checks")
    expect(not os.path.exists(as_is) and os.path.exists(os.path.join(work, "as_is.failed.glb")),
           "a file that fails its checks is left aside, not where the good one goes")


def keeper(blender, work):
    maker = os.path.join(TOOLS, "make_character.py")
    character = os.path.join(REPO, "characters", "keeper.py")
    outputs = []
    for name in ("first", "second"):
        out = os.path.join(work, name, "keeper.glb")
        code, log = run([sys.executable, maker, character, out, "--blender", blender])
        expect(code == 0, f"the keeper's {name} export passes the checks")
        if code:
            print(log[-3000:])
            return
        outputs.append(out)
    same = open(outputs[0], "rb").read() == open(outputs[1], "rb").read()
    expect(same, "two exports of the keeper are the same to the byte")
    cut_out = sorted(m["name"] for m in Glb(outputs[0]).doc["materials"] if m.get("alphaMode") == "MASK")
    expect(cut_out == ["keeper.eyebrow001", "keeper.eyelashes01", "keeper.short02"],
           f"only the brows, lashes and hair are cut out: {cut_out}")
    out = os.path.join(work, "as_is", "keeper.glb")
    code, log = run([sys.executable, maker, character, out, "--blender", blender, "--as-is"])
    expect(code != 0 and "not ready for the engine" in log,
           "the keeper exported as it is fails the checks")


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--blender")
    parser.add_argument("--skip-keeper", action="store_true",
                        help="skip the tests that need MakeHuman's plugin")
    args = parser.parse_args()
    blender = blender_path(args.blender)
    # Built once, so the checks the tool runs don't each wait on cargo.
    subprocess.run(["cargo", "build", "--release", "--quiet", "-p", "bake"], cwd=REPO, check=True)
    with tempfile.TemporaryDirectory(prefix="solstice-export-") as work:
        print("the faulty character:")
        faulty(blender, work)
        if not args.skip_keeper:
            print("the keeper:")
            keeper(blender, work)
    print(f"{len(failures)} failed" if failures else "all passed")
    sys.exit(1 if failures else 0)


if __name__ == "__main__":
    main()
