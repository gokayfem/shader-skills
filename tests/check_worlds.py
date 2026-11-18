"""Verify complete world snapshots and maintain their navigable source catalog."""
import argparse
import itertools
import json
import subprocess
from pathlib import Path
from urllib.parse import quote

ROOT = Path(__file__).resolve().parents[1]
WORLDS = ROOT / "worlds"


def git(*args):
    return subprocess.check_output(["git", *args], cwd=ROOT).decode().rstrip("\n")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--catalog", action="store_true", help="regenerate worlds/catalog.md")
    args = parser.parse_args()
    inventory = ["# World source catalog", "", "Paths are grouped by subsystem and directory."]
    for world, expected in json.loads((WORLDS / "snapshots.json").read_text()).items():
        prefix = f"worlds/{world}/"
        git("diff", "--quiet", "--", prefix)
        tree = git("write-tree", "--prefix=" + prefix)
        files = git("ls-tree", "-rz", "--name-only", tree).split("\0")[:-1]
        if tree != expected["tree"] or len(files) != expected["files"]:
            raise SystemExit(f"{world}: found {tree} and {len(files)} files, expected {expected}")
        print(f"{world}: {len(files)} files, tree {tree}")
        inventory.extend(["", f"## {world} ({len(files)} files)"])
        paths = sorted(map(Path, files), key=lambda p: (p.parent.as_posix(), p.name))
        for parent, group in itertools.groupby(paths, key=lambda p: p.parent):
            inventory.extend(["", f"### {world}/{parent}", ""])
            for path in group:
                inventory.append(f"- [{path.name}]({world}/{quote(path.as_posix())})")
    content = "\n".join(inventory) + "\n"
    catalog = WORLDS / "catalog.md"
    if args.catalog:
        catalog.write_text(content)
    elif catalog.read_text() != content:
        raise SystemExit("Regenerate the source catalog with --catalog")


if __name__ == "__main__":
    main()
