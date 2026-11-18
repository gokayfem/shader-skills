#!/usr/bin/env python3
"""Unpacks Quaternius's Universal Animation Library (CC0) into target/ual, out of git.

The library is sold on a name-your-own-price page, so nothing can fetch it by URL. Get the two
Standard zips by hand, free if you like, and put them in one folder:

    https://quaternius.itch.io/universal-animation-library
    https://quaternius.itch.io/universal-animation-library-2

    python -I scripts/fetch-ual.py --zips FOLDER [--out DIR]

`--out` defaults to the repository's target/ual, wherever this is run from.

Each zip is checked against its known digest first. Only the glTF clip files and the licences
come out: the rest is for other engines.
"""

import argparse
import hashlib
import pathlib
import sys
import zipfile

# The zip, its digest, and the folder its files go in.
ZIPS = [
    (
        "Universal Animation Library[Standard].zip",
        "cc73fc4e495b82958207316596317a3f40b9fa38065bde1027937452da537724",
        "ual1",
    ),
    (
        "Universal Animation Library 2[Standard].zip",
        "4008ea208a604773a2b2177d965f0f5d3195498b5bf838c3f5785d68e95f2a68",
        "ual2",
    ),
]
# Which members to keep, by the end of their paths. Root motion is left out: actors are moved by
# the game, and the clips play on the spot.
WANTED = ("/Unreal-Godot/UAL1_Standard.glb", "/Unreal-Godot/UAL2_Standard.glb", "/License.txt")


def digest(path):
    hasher = hashlib.sha256()
    with open(path, "rb") as file:
        while chunk := file.read(1 << 20):
            hasher.update(chunk)
    return hasher.hexdigest()


def main():
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    repository = pathlib.Path(__file__).resolve().parent.parent
    parser.add_argument("--zips", type=pathlib.Path, required=True)
    parser.add_argument("--out", type=pathlib.Path, default=repository / "target" / "ual")
    options = parser.parse_args()

    for name, expected, folder in ZIPS:
        path = options.zips / name
        if not path.is_file():
            sys.exit(f"{path} is missing: see the top of this script for where to get it")
        actual = digest(path)
        if actual != expected:
            sys.exit(f"{path} is not the known file: its digest is {actual}")
        out = options.out / folder
        out.mkdir(parents=True, exist_ok=True)
        with zipfile.ZipFile(path) as archive:
            for member in archive.infolist():
                if member.filename.endswith(WANTED):
                    # By its last part alone, so that nothing can land outside `out`.
                    target = out / pathlib.PurePosixPath(member.filename).name
                    target.write_bytes(archive.read(member))
                    print(f"wrote {target} ({member.file_size} bytes)")


if __name__ == "__main__":
    main()
