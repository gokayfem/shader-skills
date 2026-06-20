# Shader Skills

Platform-neutral procedural shader intelligence for LLM coding agents.

This repository packages the same shader-craft knowledge for multiple assistants:

- Codex skill: `skills/codex/shader-art-coding`
- Universal prompt: `prompts/universal-system-prompt.md`
- Claude project instructions: `CLAUDE.md` and `.claude/CLAUDE.md`
- Cursor rules: `.cursor/rules/shader-art-coding.mdc`
- GitHub Copilot instructions: `.github/copilot-instructions.md`
- ChatGPT/GPT builder instructions: `prompts/chatgpt-custom-gpt-instructions.md`
- Gemini/Gems instructions: `prompts/gemini-gems-instructions.md`

The references are written as formal mathematical construction manuals: implicit surfaces, SDF operators, heightfields, scalar masks, material fields, atmospheric coverage, and debug views.

## Debug Modes

Use `iDebugMode` in serious shaders:

- `0`: final render
- `1`: scalar field / SDF contours
- `2`: normals
- `3`: material IDs
- `4`: step count / depth
- `5`: masks / density
- `6`: lighting terms

## Recommended Agent Workflow

Give your assistant one of the adapter prompts, then ask it to:

1. Read the universal instructions.
2. Pick relevant references only.
3. Produce shader code using formal equation-code construction.
4. Include debug modes for formula verification.
5. Iterate on formulas, not guesses.

Example:

```text
Use the shader-art-coding instructions in this repo. Create a Shadertoy GLSL shader using formal SDF and heightfield construction, and explain the debug modes.
```

## Repository Layout

```text
prompts/                         Platform-neutral and platform-specific prompts
skills/codex/shader-art-coding/  Installed Codex skill package
```

## License

MIT
