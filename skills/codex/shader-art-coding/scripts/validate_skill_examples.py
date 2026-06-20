#!/usr/bin/env python3
"""Validate GLSL snippets bundled with the shader-art-coding skill."""

from __future__ import annotations

import re
import sys
from pathlib import Path


FENCE_RE = re.compile(r"```glsl\n(.*?)\n```", re.DOTALL)


def balanced(source: str, open_ch: str, close_ch: str) -> bool:
    depth = 0
    for ch in source:
        if ch == open_ch:
            depth += 1
        elif ch == close_ch:
            depth -= 1
            if depth < 0:
                return False
    return depth == 0


def main() -> int:
    skill_dir = Path(__file__).resolve().parents[1]
    refs = skill_dir / "references"
    failures: list[str] = []
    snippet_count = 0
    complete_count = 0

    for path in sorted(refs.glob("*.md")):
        text = path.read_text(encoding="utf-8")
        for index, snippet in enumerate(FENCE_RE.findall(text), start=1):
            snippet_count += 1
            label = f"{path.name} snippet {index}"
            if ("TO" + "DO") in snippet:
                failures.append(f"{label}: contains placeholder text")
            for a, b, name in [("{", "}", "braces"), ("(", ")", "parentheses"), ("[", "]", "brackets")]:
                if not balanced(snippet, a, b):
                    failures.append(f"{label}: unbalanced {name}")
            if "void mainImage" in snippet:
                complete_count += 1
                for required in ["fragColor", "fragCoord", "iResolution"]:
                    if required not in snippet:
                        failures.append(f"{label}: mainImage missing {required}")
                for loop in re.findall(r"for\\s*\\((.*?)\\)", snippet):
                    if "int" not in loop or "<" not in loop:
                        failures.append(f"{label}: loop may not be statically bounded: {loop}")

    if snippet_count == 0:
        failures.append("no GLSL snippets found")
    if complete_count == 0:
        failures.append("no complete mainImage snippet found")

    if failures:
        print("Validation failed:")
        for failure in failures:
            print(f"- {failure}")
        return 1

    print(f"Validated {snippet_count} GLSL snippets; {complete_count} complete shader snippet(s).")
    return 0


if __name__ == "__main__":
    sys.exit(main())
