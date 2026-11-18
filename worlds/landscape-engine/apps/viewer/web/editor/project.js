// The editor's document: a project's scene (docs/editor.md, `crates/solstice-project`), and how
// each of its entities is drawn by the viewer's instances, from the scene file's map of which
// entity made which.

import { invert, multiply, transformMatrix } from "./math.js";

/// Each kind of thing an entity can be, as the project's components name them.
export const KINDS = ["mesh", "actor", "light", "blocker", "wood", "clear", "path", "yard", "pond"];

/// What an entity is, for its icon and label: one of `KINDS`, or "folder".
export const kindOf = (entity) => KINDS.find((kind) => entity[kind]) ?? "folder";

/// Ids the map gives to what the land makes, which no entity has.
const LAND_IDS = new Set(["terrain", "circuit"]);

export class Project {
  /// The scene as read, edited in place.
  scene;
  /// Every entity by id.
  entities = new Map();
  /// Each entity's parent's id, or null at the top.
  parents = new Map();
  /// Each mapped entity's instances, and each instance's place relative to the entity.
  #drawn = new Map();
  /// Mapped ranges sorted by their first instance, for finding what an instance belongs to.
  #ranges = [];
  /// Instances of the land, which select nothing.
  #land = [];

  /// `scene` is the parsed `.scene` file, or null for a scene with no project: then each
  /// instance is an entity of its own, which can be moved but not saved.
  constructor(scene, viewer) {
    this.viewer = viewer;
    this.scene = scene ?? { format: 1, entities: syntheticEntities(viewer) };
    this.saveable = Boolean(scene);
    const visit = (list, parent) => {
      for (const entity of list) {
        this.entities.set(entity.id, entity);
        this.parents.set(entity.id, parent);
        visit(entity.children ?? [], entity.id);
      }
    };
    visit(this.scene.entities ?? (this.scene.entities = []), null);
    if (scene) this.#mapFromScene();
    else this.#mapOneToOne();
  }

  #mapFromScene() {
    const ids = this.viewer.source_ids().split("\n").filter(Boolean);
    const ranges = this.viewer.source_ranges();
    const placements = this.viewer.source_placements();
    ids.forEach((id, k) => {
      const [first, end] = [ranges[k * 2], ranges[k * 2 + 1]];
      if (LAND_IDS.has(id)) {
        this.#land.push([first, end]);
        return;
      }
      const entity = this.entities.get(id);
      if (!entity?.transform) return;
      const placement = Array.from(placements.slice(k * 16, k * 16 + 16));
      // A model seated on the ground was placed at a height the bake worked out, which the
      // project leaves out: the editor shows it, and saves it once the model is moved.
      if (entity.transform.ground !== undefined) {
        this.bakedHeights.set(id, placement[13]);
      }
      this.#attach(id, first, end);
    });
  }

  #mapOneToOne() {
    for (const entity of this.scene.entities) {
      this.#attach(entity.id, entity.instance, entity.instance + 1);
    }
  }

  /// Heights the bake seated models at, by id.
  bakedHeights = new Map();

  #attach(id, first, end) {
    const base = invert(this.matrixOf(id));
    if (!base) return;
    const instances = [];
    for (let instance = first; instance < end; instance++) {
      const world = Array.from(this.viewer.instance_transform(instance));
      if (world.length === 16) instances.push({ instance, local: multiply(base, world) });
    }
    this.#drawn.set(id, instances);
    this.#ranges.push([first, end, id]);
    this.#ranges.sort((a, b) => a[0] - b[0]);
  }

  /// The world matrix of an entity's transform as drawn, with any seated height filled in.
  matrixOf(id) {
    const transform = this.entities.get(id)?.transform;
    if (!transform) return null;
    const position = [...transform.position];
    if (transform.ground !== undefined && this.bakedHeights.has(id)) {
      position[1] = this.bakedHeights.get(id);
    }
    return transformMatrix({ ...transform, position });
  }

  /// The transform the editor shows and edits: the project's, with any seated height filled in.
  shownTransform(id) {
    const transform = this.entities.get(id)?.transform;
    if (!transform) return null;
    const shown = structuredClone(transform);
    if (shown.ground !== undefined && this.bakedHeights.has(id)) {
      shown.position[1] = this.bakedHeights.get(id);
    }
    shown.rotation ??= [0, 0, 0];
    shown.scale ??= 1;
    return shown;
  }

  /// Sets an entity's transform and moves what draws it. `jump` places it without motion, as
  /// undo and typed values should; a drag moves it as motion.
  setTransform(id, transform, jump = true) {
    const entity = this.entities.get(id);
    if (!entity) return false;
    entity.transform = canonicalTransform(transform);
    if (entity.transform.ground === undefined) this.bakedHeights.delete(id);
    this.#place(id, jump);
    return true;
  }

  /// All that says where an entity is, to put back exactly with `restore`.
  snapshot(id) {
    return {
      transform: structuredClone(this.entities.get(id)?.transform),
      baked: this.bakedHeights.get(id),
    };
  }

  restore(id, snapshot, jump = true) {
    const entity = this.entities.get(id);
    if (!entity) return;
    entity.transform = structuredClone(snapshot.transform);
    if (snapshot.baked === undefined) this.bakedHeights.delete(id);
    else this.bakedHeights.set(id, snapshot.baked);
    this.#place(id, jump);
  }

  /// The transform a snapshot shows, as `shownTransform` gives it.
  shownFrom(snapshot) {
    const shown = structuredClone(snapshot.transform);
    if (shown.ground !== undefined && snapshot.baked !== undefined) shown.position[1] = snapshot.baked;
    shown.rotation ??= [0, 0, 0];
    shown.scale ??= 1;
    return shown;
  }

  #place(id, jump) {
    const world = this.matrixOf(id);
    for (const { instance, local } of this.#drawn.get(id) ?? []) {
      this.viewer.set_instance_transform(instance, multiply(world, local), jump);
    }
  }

  /// Instances that draw an entity.
  instancesOf(id) {
    return (this.#drawn.get(id) ?? []).map((d) => d.instance);
  }

  /// Whether an entity and all it holds can be seen in the viewport (as `instancesOf` does).
  drawn(id) {
    return this.#drawn.has(id);
  }

  /// The entity that drew an instance, or null for the land, or anything unmapped.
  entityAt(instance) {
    let lo = 0;
    let hi = this.#ranges.length - 1;
    while (lo <= hi) {
      const mid = (lo + hi) >> 1;
      const [first, end, id] = this.#ranges[mid];
      if (instance < first) hi = mid - 1;
      else if (instance >= end) lo = mid + 1;
      else return id;
    }
    return null;
  }

  /// A sphere around everything an entity draws, or its position if it draws nothing.
  bounds(id) {
    const instances = this.instancesOf(id);
    const spheres = instances.map((i) => Array.from(this.viewer.instance_bounds(i)))
      .filter((s) => s.length === 4);
    if (spheres.length === 0) {
      const shown = this.shownTransform(id);
      return shown ? { centre: shown.position, radius: 1 } : null;
    }
    const centre = [0, 1, 2].map((k) => spheres.reduce((sum, s) => sum + s[k], 0) / spheres.length);
    const radius = Math.max(...spheres.map((s) =>
      Math.hypot(s[0] - centre[0], s[1] - centre[1], s[2] - centre[2]) + s[3]));
    return { centre, radius };
  }

  /// The ids of an entity's line of parents, nearest first.
  ancestors(id) {
    const out = [];
    for (let p = this.parents.get(id); p; p = this.parents.get(p)) out.push(p);
    return out;
  }

  /// The scene as its file holds it.
  toJson() {
    return writeScene(this.scene);
  }
}

/// For a scene with no project, one entity per instance, named by its number.
function syntheticEntities(viewer) {
  const count = viewer.instance_count();
  const entities = [];
  for (let instance = 0; instance < count; instance++) {
    const world = Array.from(viewer.instance_transform(instance));
    if (world.length !== 16) continue;
    entities.push({
      id: `instance-${instance}`,
      name: `Instance ${instance}`,
      instance,
      transform: { position: [world[12], world[13], world[14]] },
    });
  }
  return entities;
}

/// A transform with defaults left out, as the project's files hold it, and its fields in order.
function canonicalTransform({ position, rotation, scale, ground }) {
  const out = { position: position.map(Number) };
  if (rotation && rotation.some((a) => a !== 0)) out.rotation = rotation.map(Number);
  if (scale !== undefined && scale !== 1) out.scale = Number(scale);
  if (ground !== undefined) out.ground = Number(ground);
  return out;
}

// The order the Rust crate writes each kind of object's fields in, so that the editor's file is
// the one the bake would write: saving without an edit changes nothing.
const TRANSFORM = ["position", "rotation", "scale", "ground"];
const ENTITY_ORDER = {
  keys: ["id", "name", "note", "transform", ...KINDS, "children"],
  nested: {
    transform: { keys: TRANSFORM },
    mesh: { keys: ["asset", "part", "collide", "skinned"] },
    actor: { keys: ["asset", "properties"], nested: { properties: { keys: null } } },
    light: { keys: ["range", "intensity"] },
    blocker: { keys: ["size"] },
    wood: { keys: ["radius", "spacing"] },
    clear: { keys: ["radius"] },
    path: { keys: ["from", "to", "width"] },
    yard: { keys: ["from", "to"] },
    pond: { keys: ["min", "max", "level", "water"] },
  },
};
ENTITY_ORDER.nested.children = { list: ENTITY_ORDER };
const SCENE_ORDER = {
  keys: ["format", "note", "texture_size", "camera", "terrain", "land", "meadow", "water", "sea",
    "game", "entities"],
  nested: {
    camera: { keys: ["position", "yaw_radians", "pitch_radians"] },
    terrain: { keys: ["size", "cells", "height", "shore_x"] },
    land: { keys: ["sand", "dirt"] },
    water: { keys: ["level", "color"] },
    sea: { keys: ["level", "ocean"] },
    game: { keys: ["input", "track", "circuit"] },
    entities: { list: ENTITY_ORDER },
  },
};

/// A number as Rust writes an f32 or an integer: integral floats keep a `.0`.
function number(value, integer) {
  if (integer || !Number.isFinite(value)) return String(value);
  // The shortest text that reads back as the same f32, as serde_json writes it.
  const f = Math.fround(value);
  let text = String(f);
  for (let digits = 1; digits <= 9; digits++) {
    const candidate = f.toPrecision(digits);
    if (Math.fround(Number(candidate)) === f) {
      text = String(Number(candidate));
      break;
    }
  }
  // Very small or large numbers are written with exponents, which serde_json and JavaScript
  // place a little differently; the editor's own numbers are tidied well clear of them.
  if (/e/.test(text)) return text.replace("+", "");
  return text.includes(".") ? text : `${text}.0`;
}

// Fields held as whole numbers.
const INTEGERS = new Set(["format", "texture_size", "cells"]);

/// Writes a value as the project's files are written: two-space indents, fields in `order`,
/// lists of numbers on one line, and other lists one item to a line.
export function writeJson(value, order, indent = "", key = "") {
  const inner = `${indent}  `;
  if (Array.isArray(value)) {
    if (value.length === 0) return "[]";
    if (value.every((v) => typeof v === "number")) {
      return `[${value.map((v) => number(v, false)).join(", ")}]`;
    }
    const items = value.map((v) => inner + writeJson(v, order?.list ?? order, inner));
    return `[\n${items.join(",\n")}\n${indent}]`;
  }
  if (value && typeof value === "object") {
    const keys = order?.keys
      ? [...order.keys.filter((k) => k in value), ...Object.keys(value).filter((k) => !order.keys.includes(k))]
      : Object.keys(value);
    const fields = keys.filter((k) => value[k] !== undefined && !(Array.isArray(value[k])
      && value[k].length === 0 && (k === "children" || k === "entities")));
    if (fields.length === 0) return "{}";
    const lines = fields.map((k) =>
      `${inner}${JSON.stringify(k)}: ${writeJson(value[k], order?.nested?.[k], inner, k)}`);
    return `{\n${lines.join(",\n")}\n${indent}}`;
  }
  if (typeof value === "number") return number(value, INTEGERS.has(key));
  return JSON.stringify(value);
}

export const writeScene = (scene) => `${writeJson(scene, SCENE_ORDER)}\n`;
