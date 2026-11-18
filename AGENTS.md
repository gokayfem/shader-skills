# AGENTS.md

Guidance for coding agents (Claude Code, Codex, Cursor, Copilot, Gemini) working on this repository. To use the
skill rather than develop it, see the README.

## Layout

```text
skills/shader-art-coding/      the skill (this is what gets installed)
  SKILL.md                     entry point: workflow, conventions, hard rules, reference routing
  glsl/                        building-block library, one module per file (see MODULE_ORDER in glsl_tools.py)
  multipass/                   source for the feedback simulation and cached terrain example
  viewer/                      reusable standalone WebGL2 viewer host
  templates/                   complete shaders written against the library (rendered in gallery/)
  references/                  guides and formula-code couples; library-index.md is generated
  scripts/glsl_tools.py        search | bundle | check | index  (stdlib Python only)
tests/                         GPU property tests and the WebGL2 harness
evals/                         `claude plugin eval` suite: skill vs no skill (paid to run; see evals/README.md)
gallery/                       rendered PNGs and paste-ready bundled shaders for each template (generated)
adapters/                      copy-in prompts for tools without skill support
.claude-plugin/                Claude Code plugin + marketplace manifests
```

## Commands

```bash
python3 skills/shader-art-coding/scripts/glsl_tools.py check          # lint + drift + glslang compile (if installed)
python3 skills/shader-art-coding/scripts/glsl_tools.py index          # regenerate references/library-index.md
python3 tests/build_gallery.py                                         # regenerate gallery/*.glsl bundles
python3 tests/build_harness.py && python3 tests/serve.py               # WebGL2 checks: open /build/harness.html
python3 tests/build_harness.py --save-gallery                          # same, and rewrite gallery/*.png
python3 tests/run_webgl.py                                             # headless WebGL2 checks (needs playwright)
python3 tests/check_viewer.py                                          # offline controls, 2-pass rendering and simulation
python3 tests/compare_alpine_cache.py                                  # multi-camera timings and image comparisons
python3 tests/check_evals.py                                           # validate evals/ without running it (free)
```

## Rules for library code (`glsl/`)

- GLSL ES 3.00, Shadertoy-compatible, original implementations. Credit the technique's source in `@see` and in
  CREDITS.md; never paste third-party shader code (Shadertoy's default license is CC BY-NC-SA).
- Every function has a `///` doc comment: one-line summary, `@use` with concrete subjects, and for SDFs one of
  `@exact`, `@bound` or `@approx`. `check` enforces this.
- A module may only call modules listed before it in `MODULE_ORDER`.
- Every new SDF gets a property test in `tests/properties.py` with inside and outside points. The harness checks the
  Lipschitz bound, exactness by surface projection, and signs on the GPU. A new non-SDF helper gets a property test
  when it has a checkable invariant (range, round trip, derivative, divergence).
- References may show library functions only verbatim. `check` fails on drift, so rename deliberate variants.
- After changing `glsl/`, regenerate the index and the gallery bundles. After changing templates, re-render the
  gallery PNGs with the harness and look at them.

## Definition of done

`glsl_tools.py check` passes. The WebGL2 harness reports all units compiled, all property tests passing and all
renders passing. `library-index.md` and `gallery/*.glsl` are regenerated, and any changed template renders look
right.
