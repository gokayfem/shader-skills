// New scenes, and baking them in the browser (docs/editor.md, phase 2): the template a new scene
// starts from, the settings a scene has one of, and the worker that bakes a scene to the file the
// viewer plays.

/// A new scene: an island of sand, dirt and grass in a coastal sea, seen from offshore.
export function blankScene() {
  return {
    format: 1,
    note: "A new scene.",
    texture_size: 1024,
    // Out over the sea, which lies towards +X, looking back at the land (along -X).
    camera: { position: [150, 30, 0], yaw_radians: Math.PI / 2, pitch_radians: -0.25 },
    terrain: { size: 256, cells: 128, height: 3, shore_x: 64 },
    land: { sand: 10, dirt: 5 },
    sea: { level: -0.6, ocean: "coastal" },
    entities: [],
  };
}

/// The settings a scene has one of, each with its fields and their ranges. They are checked
/// here as the bake checks them, so a bad value is refused when it is set, not when it bakes.
export const SETTINGS = {
  terrain: {
    description: "The ground: metres across, cells along each side, how tall its hills are, and the x where the shore begins (the sea lies towards +X).",
    fields: {
      size: { min: 16, max: 4096, step: 1 },
      cells: { min: 8, max: 1024, step: 1, integer: true },
      height: { min: 0, max: 200, step: 0.1 },
      shore_x: { min: -4096, max: 4096, step: 1 },
    },
  },
  land: {
    description: "The terrain in bands: metres of sand in from the water, then of dirt; grass beyond.",
    fields: { sand: { min: 0, max: 500, step: 0.5 }, dirt: { min: 0, max: 500, step: 0.5 } },
  },
  sea: {
    description: "An open sea: its calm level, and a built-in ocean's name.",
    fields: { level: { min: -100, max: 100, step: 0.1 }, ocean: { names: ["coastal", "clear", "ocean", "murky"] } },
  },
};

/// Checks a scene setting's value: an object with the fields `SETTINGS` lists, or null to take
/// it away. Gives back an error's message, or null if it is good.
export function checkSetting(name, value) {
  const setting = SETTINGS[name];
  if (!setting) return `no setting \`${name}\`; scene.settings lists them`;
  if (value === null) return null;
  if (typeof value !== "object" || Array.isArray(value)) return `\`${name}\` is an object of its fields, or null`;
  for (const key of Object.keys(value)) {
    if (!(key in setting.fields)) return `\`${name}\` has no field \`${key}\`; it has ${Object.keys(setting.fields).join(", ")}`;
  }
  for (const [key, field] of Object.entries(setting.fields)) {
    const v = value[key];
    if (v === undefined) return `\`${name}\` needs \`${key}\``;
    if (field.names) {
      if (!field.names.includes(v)) return `\`${name}.${key}\` is one of ${field.names.join(", ")}`;
      continue;
    }
    if (typeof v !== "number" || !Number.isFinite(v) || v < field.min || v > field.max) {
      return `\`${name}.${key}\` is a number from ${field.min} to ${field.max}`;
    }
    if (field.integer && !Number.isInteger(v)) return `\`${name}.${key}\` is a whole number`;
  }
  return null;
}

/// Bakes scenes in a Web Worker, one at a time.
export class Baker {
  #worker = null;
  #next = 1;
  #waiting = new Map();
  /// Where the ready-made surfaces are.
  content;

  /// `content` is relative to the page; the worker is given it whole, as it would otherwise
  /// read it relative to its own script.
  constructor(content) {
    this.content = new URL(content, location.href).href;
  }

  /// Bakes a scene's file text. Resolves to `{ bytes, ms }`; rejects with the bake's error.
  bake(text, name) {
    if (!this.#worker) {
      this.#worker = new Worker(new URL("./bake-worker.js", import.meta.url), { type: "module" });
      this.#worker.onmessage = ({ data }) => {
        const waiting = this.#waiting.get(data.id);
        this.#waiting.delete(data.id);
        if (data.error) waiting?.reject(new Error(data.error));
        else waiting?.resolve({ bytes: data.bytes, ms: data.ms });
      };
      this.#worker.onerror = (event) => {
        for (const waiting of this.#waiting.values()) waiting.reject(new Error(event.message || "the bake's worker failed"));
        this.#waiting.clear();
        this.#worker = null;
      };
    }
    const id = this.#next++;
    return new Promise((resolve, reject) => {
      this.#waiting.set(id, { resolve, reject });
      this.#worker.postMessage({ id, scene: text, name, content: this.content });
    });
  }
}
