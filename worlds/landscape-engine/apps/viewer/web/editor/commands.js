// The editor's one list of commands (docs/editor.md, "The command API for agents"). The panels,
// the keyboard, the gizmo and agents all act through it, so each can do what the others can, and
// every change is undone the same way.
//
// A command is `{ description, args, run(args, context) }`. `args` is a small schema: each
// argument's type, whether it is needed, and what it means, which `help` reports. A command
// that changes the scene returns a step for the history: `{ label, undo(), redo() }`, with the
// change already made.

export class CommandError extends Error {
  constructor(code, message) {
    super(message);
    this.code = code;
  }
}

/// Checks a call's arguments against a command's schema, and gives them back tidied.
function check(name, schema, args) {
  if (args === null || typeof args !== "object" || Array.isArray(args)) {
    throw new CommandError("bad_arguments", `${name} takes an object of arguments`);
  }
  for (const key of Object.keys(args)) {
    if (!(key in schema)) {
      throw new CommandError("bad_arguments",
        `${name} has no argument \`${key}\`; it takes ${Object.keys(schema).join(", ") || "none"}`);
    }
  }
  const out = {};
  for (const [key, spec] of Object.entries(schema)) {
    const value = args[key];
    if (value === undefined) {
      if (spec.required) throw new CommandError("bad_arguments", `${name} needs \`${key}\``);
      continue;
    }
    const fail = () => {
      throw new CommandError("bad_arguments", `${name}: \`${key}\` should be ${spec.type}`);
    };
    switch (spec.type) {
      case "number": if (typeof value !== "number" || !Number.isFinite(value)) fail(); break;
      case "string": if (typeof value !== "string") fail(); break;
      case "boolean": if (typeof value !== "boolean") fail(); break;
      case "vec3":
        if (!Array.isArray(value) || value.length !== 3
          || !value.every((v) => typeof v === "number" && Number.isFinite(v))) fail();
        break;
      case "string[]":
        if (!Array.isArray(value) || !value.every((v) => typeof v === "string")) fail();
        break;
      case "object":
        if (value !== null && (typeof value !== "object" || Array.isArray(value))) fail();
        break;
      default: break;
    }
    out[key] = value;
  }
  return out;
}

/// What `help` reports of an argument schema: JSON Schema, which agents and WebMCP read.
function jsonSchema(schema) {
  const types = {
    number: { type: "number" },
    string: { type: "string" },
    boolean: { type: "boolean" },
    vec3: { type: "array", items: { type: "number" }, minItems: 3, maxItems: 3 },
    "string[]": { type: "array", items: { type: "string" } },
    object: { type: ["object", "null"] },
  };
  return {
    type: "object",
    properties: Object.fromEntries(Object.entries(schema).map(([key, spec]) =>
      [key, { ...types[spec.type], description: spec.description }])),
    required: Object.entries(schema).filter(([, spec]) => spec.required).map(([key]) => key),
    additionalProperties: false,
  };
}

/// Undo and redo, step by step, each step saying who took it.
export class History {
  #done = [];
  #undone = [];
  /// Called after every change to the history.
  onChange = () => {};
  static LIMIT = 500;

  push(step) {
    this.#done.push(step);
    if (this.#done.length > History.LIMIT) this.#done.shift();
    this.#undone = [];
    this.onChange();
  }

  undo() {
    const step = this.#done.pop();
    if (!step) return null;
    step.undo();
    this.#undone.push(step);
    this.onChange();
    return step;
  }

  redo() {
    const step = this.#undone.pop();
    if (!step) return null;
    step.redo();
    this.#done.push(step);
    this.onChange();
    return step;
  }

  /// Forgets every step, as a new scene does.
  clear() {
    this.#done = [];
    this.#undone = [];
    this.onChange();
  }

  /// What can be undone, oldest first, and what can be redone, next first.
  list() {
    const describe = (step) => ({ label: step.label, by: step.by });
    return { done: this.#done.map(describe), undone: [...this.#undone].reverse().map(describe) };
  }
}

export class Commands {
  #commands = new Map();
  history = new History();
  /// Called after each command that ran, with its name, its arguments and who called it.
  onRun = () => {};

  constructor(context) {
    this.context = context;
  }

  define(name, command) {
    this.#commands.set(name, command);
  }

  /// Runs a command. `by` is who asked: "user" from the page, "agent" from the API. Gives back
  /// `{ ok: true, result }` or `{ ok: false, error: { code, message } }`, never throwing, so a
  /// caller always has something to read.
  async call(name, args = {}, by = "user") {
    try {
      const command = this.#commands.get(name);
      if (!command) {
        throw new CommandError("unknown_command",
          `no command \`${name}\`; solstice.help() lists them`);
      }
      const checked = check(name, command.args ?? {}, args ?? {});
      // While a newly baked scene is put in place there is no viewer to act on: wait for it.
      await this.context.whenReady?.();
      const outcome = await command.run(checked, this.context, by);
      const { step, result } = outcome && "step" in outcome ? outcome : { step: null, result: outcome };
      if (step) this.history.push({ ...step, by });
      this.onRun(name, checked, by);
      return { ok: true, result: result ?? null };
    } catch (error) {
      if (!(error instanceof CommandError)) console.error(error);
      return {
        ok: false,
        error: {
          code: error.code ?? "failed",
          message: error.message ?? String(error),
        },
      };
    }
  }

  /// Every command, or one, with its description, its arguments as JSON Schema, and an example.
  help(name) {
    const describe = (n, c) => ({
      name: n,
      description: c.description,
      changes: Boolean(c.changes),
      arguments: jsonSchema(c.args ?? {}),
      example: c.example ?? null,
    });
    if (name !== undefined) {
      const command = this.#commands.get(name);
      return command ? describe(name, command) : null;
    }
    return [...this.#commands].map(([n, c]) => describe(n, c));
  }
}
