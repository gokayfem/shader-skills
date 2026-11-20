# ChatGPT Custom GPT

1. Create a GPT and paste `adapters/universal-system-prompt.md` (the part below the line) into **Instructions**.
2. From `skills/shader-art-coding/`, provide `SKILL.md`, `references/example-selection.md`, `references/subject-routing.md` and `references/library-index.md`. Include the subject references, template or multipass source and GLSL modules for the work you expect. When folder access is available, provide the entire skill so every route can be followed.
3. Instruct the assistant to open the selected example, reuse library functions with their dependencies, and describe which checks it could actually perform. For work on the full WebGPU worlds, supply the relevant source from the checkout using `worlds/README.md` and `worlds/catalog.md` to locate it.
4. With code execution available, provide the skill folder as an archive preserving its directory structure. Extract it and run `python3 scripts/glsl_tools.py bundle shader.glsl -o out.glsl` and `python3 scripts/glsl_tools.py check out.glsl` from the extracted skill directory.
