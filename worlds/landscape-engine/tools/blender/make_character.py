"""Makes a character and exports it for the engine, with Blender run headless.

    python tools/blender/make_character.py CHARACTER.py OUT.glb [--roles RIG.roles]
        [--blender PATH] [--texture-size N] [--as-is]

CHARACTER.py is a Blender script that makes the character and saves it to the .blend it is
given (see characters/keeper.py). The .blend is kept beside OUT.glb. Then the export tool
(tools/blender/solstice_export.py) prepares it, writes OUT.glb and checks it.

Blender is found from --blender, the BLENDER environment variable, or D:/Blender/blender.exe.
"""

import argparse
import os
import subprocess
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
DEFAULT_ROLES = os.path.join(HERE, "..", "..", "characters", "rigs", "game_engine.roles")


def blender_path(given):
    for candidate in (given, os.environ.get("BLENDER"), "D:/Blender/blender.exe"):
        if candidate and os.path.exists(candidate):
            return candidate
    sys.exit("make_character.py: no Blender found: pass --blender or set BLENDER")


def run(command):
    print("+", " ".join(command), flush=True)
    result = subprocess.run(command)
    if result.returncode != 0:
        sys.exit(result.returncode)


def main():
    parser = argparse.ArgumentParser(prog="make_character.py")
    parser.add_argument("character")
    parser.add_argument("output")
    parser.add_argument("--roles", default=os.path.normpath(DEFAULT_ROLES))
    parser.add_argument("--blender")
    parser.add_argument("--texture-size", default="1024")
    parser.add_argument("--as-is", action="store_true")
    args = parser.parse_args()

    blender = blender_path(args.blender)
    output = os.path.abspath(args.output)
    blend = os.path.splitext(output)[0] + ".blend"
    os.makedirs(os.path.dirname(output), exist_ok=True)
    # The character needs MPFB, which needs the user's preferences: no factory startup here.
    # A failing script makes Blender fail too, rather than quit as if all were well.
    run([blender, "--background", "--python-exit-code", "1",
         "--python", os.path.abspath(args.character), "--", blend])
    export = [blender, "--background", "--factory-startup", "--python-exit-code", "1", blend,
              "--python", os.path.join(HERE, "solstice_export.py"), "--",
              output, "--roles", os.path.abspath(args.roles), "--texture-size", args.texture_size]
    if args.as_is:
        export.append("--as-is")
    run(export)


if __name__ == "__main__":
    main()
