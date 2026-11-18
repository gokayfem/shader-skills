#!/usr/bin/env python3
"""Works out which workspace crates a change touches, so checks run only for those.

A crate is affected when a file in its folder changed, or when it depends, directly or through
other crates, on one that did: changing the renderer re-checks the renderer and the viewer, but
not the gameplay crates. A change outside every crate's folder that could affect the build
(Cargo.lock, the workspace's Cargo.toml, CI, toolchain or build scripts, anything unrecognised)
affects everything, as does an unknown base. Documentation affects nothing.

    python scripts/affected.py [--base REV] [--head REV] [--all] [--github-output]

Without --head, the working tree counts too: staged, unstaged and untracked files, so a local
run covers what has not been committed yet. --base defaults to where HEAD left origin/main.
--all skips the choice and affects everything. The crates and their folders are read from the
working tree, so --head should be the checked-out revision, as it is in CI.

It prints one line: `all`, `none`, or the affected crates' names separated by spaces. With
--github-output it also writes `packages`, `all`, `any` and `web` (whether the viewer, and so
the browser build, is affected) to $GITHUB_OUTPUT.
"""
import argparse
import json
import os
import subprocess
import sys

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
# Paths that build nothing, as in the CI workflow's paths-ignore.
IGNORED_SUFFIXES = (".md",)
IGNORED_PREFIXES = ("docs/",)
IGNORED_FILES = {"LICENSE"}
# Folders outside the workspace's crates that no workspace check reads: the benchmarks have
# their own workspace. (Not `scenes/`: tests include its actors and scripts.)
UNCHECKED_PREFIXES = ("tools/",)
WEB_PACKAGE = "viewer"


def git(*args):
    return subprocess.run(["git", *args], cwd=ROOT, check=True, capture_output=True, text=True).stdout


def changed_files(base, head):
    """Files changed between `base` and `head`, or the working tree when `head` is None. A
    moved file counts at both its old and its new path: the crate it left changed too."""
    if head is None:
        files = git("diff", "--name-only", "--no-renames", base).splitlines()
        files += git("ls-files", "--others", "--exclude-standard").splitlines()
    else:
        files = git("diff", "--name-only", "--no-renames", base, head).splitlines()
    return sorted({f.strip().replace("\\", "/") for f in files if f.strip()})


def workspace():
    """Each workspace crate's folder, relative to the root, and the crates it depends on."""
    meta = json.loads(subprocess.run(
        ["cargo", "metadata", "--no-deps", "--format-version", "1"],
        cwd=ROOT, check=True, capture_output=True, text=True,
    ).stdout)
    names = {p["name"] for p in meta["packages"]}
    folders, deps = {}, {}
    for p in meta["packages"]:
        folder = os.path.relpath(os.path.dirname(p["manifest_path"]), meta["workspace_root"])
        folders[p["name"]] = folder.replace("\\", "/").rstrip("/") + "/"
        # Every kind counts: a dev-dependency's change can break a crate's tests.
        deps[p["name"]] = {d["name"] for d in p["dependencies"] if d["name"] in names}
    return folders, deps


def affected(files, folders, deps):
    """The crates the changed `files` affect, or None when they affect everything."""
    touched = set()
    for f in files:
        if f in IGNORED_FILES or f.endswith(IGNORED_SUFFIXES) or f.startswith(IGNORED_PREFIXES):
            continue
        if f.startswith(UNCHECKED_PREFIXES):
            continue
        owners = [name for name, folder in folders.items() if f.startswith(folder)]
        if not owners:
            # Cargo.lock, the workspace manifest, CI, the toolchain, build scripts: anything
            # outside the crates may change how all of them build.
            return None
        # The crate whose folder is deepest owns the file.
        touched.add(max(owners, key=lambda name: len(folders[name])))
    # Everything that depends on a touched crate, however indirectly.
    result = set(touched)
    grew = True
    while grew:
        grew = False
        for name, needs in deps.items():
            if name not in result and needs & result:
                result.add(name)
                grew = True
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--base", help="the revision to compare against (default: merge base with origin/main)")
    parser.add_argument("--head", help="the revision to compare (default: the working tree)")
    parser.add_argument("--all", action="store_true", help="affect everything, without looking at the change")
    parser.add_argument("--github-output", action="store_true", help="also write the result to $GITHUB_OUTPUT")
    args = parser.parse_args()

    base = args.base
    try:
        if args.all:
            raise subprocess.CalledProcessError(1, "--all")
        if base is None:
            base = git("merge-base", "HEAD", "origin/main").strip()
        elif set(base) == {"0"}:
            # A push that created the branch: there is nothing to compare against.
            raise subprocess.CalledProcessError(1, "base")
        git("rev-parse", "--verify", f"{base}^{{commit}}")
        files = changed_files(base, args.head)
        folders, deps = workspace()
        result = affected(files, folders, deps)
    except subprocess.CalledProcessError:
        result = None

    if result is None:
        line, packages, everything = "all", "", True
    elif not result:
        line, packages, everything = "none", "", False
    else:
        line = packages = " ".join(sorted(result))
        everything = False
    print(line)

    if args.github_output:
        web = everything or (result is not None and WEB_PACKAGE in result)
        with open(os.environ["GITHUB_OUTPUT"], "a", encoding="utf-8") as out:
            out.write(f"packages={packages}\n")
            out.write(f"all={'true' if everything else 'false'}\n")
            out.write(f"any={'true' if everything or packages else 'false'}\n")
            out.write(f"web={'true' if web else 'false'}\n")
    return 0


if __name__ == "__main__":
    sys.exit(main())
