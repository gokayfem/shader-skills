# Claude (claude.ai Projects)

Prefer the skill: zip the `skills/shader-art-coding/` folder and upload it under **Settings > Capabilities > Skills**.
Claude then loads `SKILL.md`, the references and the library on demand, and can run `scripts/glsl_tools.py` in its
code environment to bundle and check shaders.

Without skills, use a Project:

1. Paste `adapters/universal-system-prompt.md` (the part below the line) into the project instructions.
2. From `skills/shader-art-coding/`, provide `SKILL.md`, `references/example-selection.md`, `references/subject-routing.md` and `references/library-index.md`. Include the subject references, template or multipass source and GLSL modules for the work you expect. When folder access is available, provide the entire skill so every route can be followed.
3. Instruct the assistant to open the selected example, reuse library functions with their dependencies, and describe which checks it could actually perform. For work on the full WebGPU worlds, supply the relevant source from the checkout using `worlds/README.md` and `worlds/catalog.md` to locate it.

For Claude Code, install the plugin or copy the skill folder (see the repository README).
