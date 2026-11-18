#!/usr/bin/env python3
"""Publishes baked scenes, uploading only the scenes that changed.

Scenes take minutes to bake and hundreds of megabytes to store, so they are baked on a PC
rather than by CI. Each scene has its own repository and GitHub Pages site,
SamG-Coder/solstice-scene-NAME, which serves it at samg-coder.github.io/solstice-scene-NAME/:
publishing one never redeploys the viewer or the other scenes. Each repository's scene.txt names
the release holding its scene and the scene's SHA-256.

This script compares each scene in assets/ with its repository's scene.txt. For each one that
changed, it creates a release in that repository holding the new file, and updates scene.txt,
which deploys it.

    python scripts/publish-scenes.py [--check] [--bake [SCENE...]]

--check only reports what would be published. --bake bakes scenes first: the ones named, or all.
Needs the GitHub CLI, signed in with permission to create releases and push to those repositories.
"""
import base64
import datetime
import hashlib
import json
import os
import subprocess
import sys

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
OWNER = "SamG-Coder"
# Each scene the site offers: its repository's name, its file, and the command that bakes it.
SCENES = {
    "temple": ("web.solstice", ["cargo", "run", "--release", "-p", "bake", "--", "--quality", "web",
                                "--output", "assets/web.solstice"]),
    "cove": ("cove.solstice", ["cargo", "run", "--release", "-p", "bake", "--", "--scene",
                               "scenes/cove.txt", "--texture-size", "1024",
                               "--output", "assets/cove.solstice"]),
    "forest": ("forest.solstice", ["cargo", "run", "--release", "-p", "bake", "--", "--forest",
                                   "--output", "assets/forest.solstice"]),
}
HEADER = (
    "# The scene this site serves: its file name, the release that holds it (OWNER/REPO@TAG,\n"
    "# or a tag of this repository) and its SHA-256. Written by scripts/publish-scenes.py in\n"
    "# SamG-Coder/solstice; pushing a change deploys the new scene.\n"
)


def run(command, **kwargs):
    print("$", " ".join(command), flush=True)
    return subprocess.run(command, cwd=ROOT, check=True, **kwargs)


def output(command):
    return subprocess.run(command, cwd=ROOT, check=True, capture_output=True, text=True).stdout.strip()


def sha256(path):
    digest = hashlib.sha256()
    with open(path, "rb") as f:
        for block in iter(lambda: f.read(1 << 20), b""):
            digest.update(block)
    return digest.hexdigest()


def published(repo):
    """The repository's scene.txt as (its blob SHA, (file, release, SHA-256))."""
    info = json.loads(output(["gh", "api", f"repos/{repo}/contents/scene.txt"]))
    text = base64.b64decode(info["content"]).decode()
    for line in text.splitlines():
        words = line.split("#")[0].split()
        if len(words) == 3:
            return info["sha"], tuple(words)
    sys.exit(f"{repo}: scene.txt names no scene")


def main():
    args = sys.argv[1:]
    to_bake = []
    if "--bake" in args:
        at = args.index("--bake")
        to_bake = [a for a in args[at + 1:] if not a.startswith("--")] or list(SCENES)
        args = args[:at] + [a for a in args[at + 1:] if a.startswith("--")]
    check_only = "--check" in args
    if any(a != "--check" for a in args):
        sys.exit(__doc__)

    unknown = [s for s in to_bake if s not in SCENES]
    if unknown:
        sys.exit(f"unknown scene: {', '.join(unknown)} (scenes: {', '.join(SCENES)})")
    if "cove" in to_bake:
        run([sys.executable, "scripts/fetch-scans.py", "scenes/cove.txt", "2k"])
    for name in to_bake:
        run(SCENES[name][1])

    commit = output(["git", "rev-parse", "--short", "HEAD"])
    baking = ["git", "status", "--porcelain", "--", "apps", "crates", "scenes", "Cargo.lock"]
    dirty = " (with uncommitted changes)" if output(baking) else ""
    tag = datetime.datetime.now().strftime("%Y-%m-%d-%H%M")
    changed = 0
    for name, (file, _) in SCENES.items():
        repo = f"{OWNER}/solstice-scene-{name}"
        path = os.path.join(ROOT, "assets", file)
        blob, (_, release, recorded) = published(repo)
        if not os.path.exists(path):
            print(f"{name}: not baked here, keeping {release}")
            continue
        digest = sha256(path)
        if digest == recorded:
            print(f"{name}: unchanged, in {release}")
            continue
        changed += 1
        print(f"{name}: changed")
        if check_only:
            continue
        notes = (f"{file}, baked from SamG-Coder/solstice@{commit}{dirty}.\n\n"
                 f"{os.path.getsize(path) / 1e6:.1f} MB, SHA-256 `{digest}`.")
        run(["gh", "release", "create", tag, "--repo", repo, "--title", f"{name} {tag}",
             "--notes", notes, path])
        content = HEADER + f"{file} {tag} {digest}\n"
        run(["gh", "api", "-X", "PUT", f"repos/{repo}/contents/scene.txt",
             "-f", f"message=Serve the {name} scene from {tag}",
             "-f", f"content={base64.b64encode(content.encode()).decode()}",
             "-f", f"sha={blob}"], stdout=subprocess.DEVNULL)
        print(f"{name}: published as {repo}@{tag}; its site is deploying")
    if changed == 0:
        print("nothing to publish")
    elif check_only:
        print(f"would publish {changed} scene(s)")


if __name__ == "__main__":
    main()
