# Changelog

Published versions and downloadable packages are available on
[GitHub Releases](https://github.com/gokayfem/shader-skills/releases).

## Unreleased

- Four single-pass studies cover butterfly structural color, transparent comb rows, Schwarzschild lensing and streamline engraving. A two-channel feedback simulation adds paintable reaction–diffusion.
- The offline viewer renders 18 scenes, exposes material and simulation controls, supports frame stepping and exports PNGs.
- Static alpine elevations and fixed-camera black-hole light paths can be retained across animation frames. The terrain comparison reports image error and frame time at three camera positions.
- The agate study and footprint-aware fBm bring filtered growth bands and terrain grain into the existing library. Shader checks catch unordered literal smoothstep edges.

## [2.0.0] — 2026-10-23

First tagged GitHub release, matching the existing 2.0.0 plugin and extension manifests. This release packages
the current library and accumulated improvements; it does not imply that all features were added on this date.

- An agent skill for procedural GLSL / Shadertoy work in Claude Code, Codex, Cursor, GitHub Copilot, Gemini CLI
  and other compatible tools, with manual adapters for chat interfaces.
- 186 GLSL functions across 20 modules, including signed distance functions, raymarching, noise, patterns,
  procedural architecture, terrain, water optics, shading, color, tonemapping, volumes and atmosphere.
- A Python search tool, dependency bundler and checker that produce standalone Shadertoy shaders.
- 11 complete examples with editable templates, bundled shaders and rendered gallery previews.
- A noise learning guide, procedural building and terrain workflows, and coastal water quality presets.
- A video-to-shader prompt with visual comparison and acceptance checks.
- A WebGL2 harness with 90 GPU property tests, plus compilation, render and bundler regression checks.
- Clearer installation instructions, contribution guides and issue templates.

Download `shader-art-coding-v2.0.0.zip` for the portable skill, or a source archive for the full repository.
The skill ZIP includes the project's license and technique credits. `SHA256SUMS` verifies the skill ZIP.

[2.0.0]: https://github.com/gokayfem/shader-skills/releases/tag/v2.0.0
