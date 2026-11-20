# Gemini Gem

1. Create a Gem and paste `adapters/universal-system-prompt.md` (the part below the line) into its instructions.
2. From `skills/shader-art-coding/`, provide `SKILL.md`, `references/example-selection.md`, `references/subject-routing.md` and `references/library-index.md`. Include the subject references, template or multipass source and GLSL modules for the work you expect. When folder access is available, provide the entire skill so every route can be followed.
3. Instruct the assistant to open the selected example, reuse library functions with their dependencies, and describe which checks it could actually perform. For work on the full WebGPU worlds, supply the relevant source from the checkout using `worlds/README.md` and `worlds/catalog.md` to locate it.

For Gemini CLI, install the skill directly (see the repository README); the CLI reads `SKILL.md` folders.
