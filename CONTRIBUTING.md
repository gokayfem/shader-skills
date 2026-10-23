# Contributing to Shader Skills

Help improve the GLSL library, shader examples, agent instructions or documentation. Small, focused changes are
welcome. For a new technique or a large change, open an issue first to explain the intended use and approach.

## Report a bug

Use the [bug report form](https://github.com/gokayfem/shader-skills/issues/new?template=bug_report.yml).
Include the smallest shader or prompt that reproduces the problem, the expected and actual result, your coding
agent, and your browser/GPU when relevant. Screenshots and compiler output are especially useful for rendering
bugs. Remove API keys and private project data before posting.

## Suggest a technique or example

Use the [feature request form](https://github.com/gokayfem/shader-skills/issues/new?template=feature_request.yml).
Describe the visual result, who it helps and any relevant papers or author-maintained references. Shader code
must be an original implementation of the technique; reference images and third-party code retain their own licenses.

## Make a change

1. Fork and clone the repository, then create a branch for your change.
2. Read [AGENTS.md](AGENTS.md) for the layout, GLSL conventions, attribution and testing requirements.
3. Keep the change focused. Update examples and documentation when behavior changes.
4. Run the checks below and open a pull request with the result and relevant screenshots.

From the repository root:

```bash
python3 skills/shader-art-coding/scripts/glsl_tools.py index --check
python3 tests/build_gallery.py --check
python3 skills/shader-art-coding/scripts/glsl_tools.py check
python3 tests/check_evals.py
python3 -m unittest discover -s tests -p 'test_*.py'
```

Python 3 is required. Install `glslangValidator` for the compiler checks (`brew install glslang` on macOS or
`sudo apt install glslang-tools` on Ubuntu). The WebGL2 harness also requires Playwright and Chromium:

```bash
python3 -m venv .venv
source .venv/bin/activate
python3 -m pip install playwright==1.49.1
python3 -m playwright install chromium
python3 tests/run_webgl.py --size 320x180
```

On Linux, Playwright may also need system dependencies; use `python3 -m playwright install --with-deps chromium`.
For shader or template changes, regenerate the library index and gallery as described in [AGENTS.md](AGENTS.md),
then inspect the rendered images. The paid agent evaluations are optional and run manually; see
[evals/README.md](evals/README.md).

## Credits and license

The project uses the [MIT license](LICENSE). Credit the source of each technique in `@see` and
[CREDITS.md](CREDITS.md). Do not copy third-party shader implementations into the library.
