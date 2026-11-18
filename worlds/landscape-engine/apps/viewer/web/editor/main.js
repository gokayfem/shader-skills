// The editor's page (docs/editor.md): opens a scene and its project, and lets people, and agents
// through `window.solstice`, select, move, turn and scale what is in it, with undo.

import { openScene, requestAdapter } from "../loader.js";
import { Commands, CommandError } from "./commands.js";
import { Gizmo, cameraFrom } from "./gizmo.js";
import {
  add, dot, multiply, normalize, projectToScreen, rayThrough, rotateColumns, rotationColumns,
  eulerFromColumns, scale as scaleVector, sub, tidy, tidyAngles,
} from "./math.js";
import { Details, Outliner } from "./panels.js";
import { Project, kindOf, writeScene } from "./project.js";
import { Baker, SETTINGS, blankScene, checkSetting } from "./scenes.js";

const params = new URLSearchParams(location.search);
const el = (id) => document.getElementById(id);
const canvas = el("view");
const overlay = el("overlay-canvas");
const context = overlay.getContext("2d");

/// The scene's name: from ?scene= for one served beside the page, ?draft= for one kept in this
/// browser, or "untitled" for a new one (?new).
let sceneName = params.get("draft") ?? (params.has("new") ? "untitled" : params.get("scene") ?? "cove");
if (!/^[\w-]+$/.test(sceneName)) throw new Error("invalid scene name");
const sceneBase = params.get("scenes") ?? "assets/";
const API_VERSION = 1;

// ---- The output log, which agents read through `log.read`.
const log = {
  entries: [],
  next: 1,
  add(level, text) {
    const entry = { seq: this.next++, time: Math.round(performance.now()), level, text };
    this.entries.push(entry);
    if (this.entries.length > 1000) this.entries.shift();
    const line = document.createElement("div");
    line.className = `log-${level}`;
    line.textContent = text;
    el("log").append(line);
    while (el("log").childElementCount > 200) el("log").firstChild.remove();
    el("log").scrollTop = el("log").scrollHeight;
    if (level === "error") console.error(text);
  },
};

function fail(text) {
  el("loading").hidden = false;
  el("loading-text").textContent = text;
  el("loading-text").className = "error";
  log.add("error", text);
}

/// The viewer's module, once loaded, and the budgets every scene is opened with.
let wasm = null;
const budgets = {
  geometryBudget: (Number(params.get("geometry-mb")) || 128) * 1048576,
  textureBudget: (Number(params.get("texture-mb")) || 64) * 1048576,
};
const baker = new Baker(params.get("content") ?? "content/");

const fitCanvas = () => {
  const ratio = Math.min(window.devicePixelRatio || 1, 2);
  const box = canvas.getBoundingClientRect();
  return [Math.max(1, Math.round(box.width * ratio)), Math.max(1, Math.round(box.height * ratio)), ratio];
};

/// Opens a baked scene on the canvas: from `url`, or from `bytes` the bake just gave.
async function openViewer(source) {
  [canvas.width, canvas.height] = fitCanvas();
  return openScene(wasm, canvas, {
    ...source,
    ...budgets,
    onStage: (text) => { el("loading-text").textContent = text; },
    onDownload: (received) => {
      if (received !== null) el("loading-text").textContent = `Downloading scene: ${(received / 1e6).toFixed(0)} MB`;
    },
  });
}

async function main() {
  if (!navigator.gpu || !(await requestAdapter())) {
    fail("This browser has no WebGPU adapter for this GPU. Try a recent Chrome or Edge.");
    return;
  }
  wasm = await import("../editor-wasm/solstice_viewer.js");
  await wasm.default();

  // A new scene, or one kept in this browser, is baked here; one served beside the page comes
  // baked, with its project beside it.
  let scene = null;
  let opened;
  if (params.has("new") || params.has("draft")) {
    scene = params.has("draft") ? JSON.parse(await readDraft(sceneName) ?? "null") : blankScene();
    if (!scene) {
      fail(`There is no scene called ${sceneName} kept in this browser.`);
      return;
    }
    el("loading-text").textContent = "Baking the scene…";
    const baked = await baker.bake(writeScene(scene), `${sceneName}.scene`);
    opened = { ...await openViewer({ bytes: baked.bytes }), source: { bytes: baked.bytes } };
    log.add("info", `Baked ${sceneName} in ${(baked.ms / 1000).toFixed(1)} s.`);
  } else {
    const url = `${sceneBase}${sceneName}.solstice`;
    opened = { ...await openViewer({ url }), source: { url } };
    const projectUrl = params.get("project") ?? `${sceneBase}${sceneName}.scene`;
    try {
      const response = await fetch(projectUrl);
      if (response.ok) scene = JSON.parse(await response.text());
    } catch { /* none */ }
    if (!scene) {
      log.add("warn", `No project at ${projectUrl}: each instance is listed alone, and can be moved but not saved.`);
    }
  }
  const project = new Project(scene, opened.viewer);
  if (scene && opened.viewer.source_ids() === "" && project.entities.size) {
    log.add("warn", "This scene was baked without the map of its entities (bake it with --project): nothing can be picked or moved.");
  }
  editor.source = opened.source;
  editor.start(opened.viewer, opened.streamer, opened.file, project, fitCanvas);
  // A new scene is kept in this browser from the start, and a reload opens it again.
  if (params.has("new")) {
    keepAsDraft(editor);
    history.replaceState(null, "", `?draft=${sceneName}`);
  }
}

/// Everything the editor holds, and the commands that change it.
const editor = {
  viewer: null,
  project: null,
  selection: new Set(),
  hidden: new Set(),
  gizmo: new Gizmo(),
  commands: null,
  frames: 0,
  fps: 0,
  frameMs: 0,
  /// Picks waiting for the GPU's answer.
  picks: [],
  /// Calls waiting for frames: `{ frames, resolve }`.
  waits: [],
  dirty: false,

  call(name, args) {
    return this.commands.call(name, args, "user");
  },

  /// Where entities were before a drag or a scrub began showing changes, by id: the change is
  /// made a step in the history from there when it ends.
  previews: new Map(),

  /// Shows a transform while a field is scrubbed or a handle dragged, without a step in the
  /// history.
  preview(id, transform) {
    if (!this.previews.has(id)) this.previews.set(id, this.project.snapshot(id));
    this.project.setTransform(id, nextTransform(this, id, transform), false);
    this.renderOverlay();
  },

  /// Puts back what previews moved, as they were.
  endPreviews() {
    for (const [id, snapshot] of this.previews) this.project.restore(id, snapshot, true);
    this.previews.clear();
  },

  start(viewer, streamer, file, project, resize) {
    this.viewer = viewer;
    this.streamer = streamer;
    this.file = file;
    this.project = project;
    this.resize = resize;
    this.commands = new Commands(this);
    defineCommands(this.commands, this);
    this.commands.history.onChange = () => {
      this.dirty = true;
      this.refresh();
      scheduleAutosave(this);
      el("undo").disabled = this.commands.history.list().done.length === 0;
      el("redo").disabled = this.commands.history.list().undone.length === 0;
    };
    this.outliner = new Outliner(el("outliner"), this);
    this.details = new Details(el("details"), this);
    this.refresh();
    bindInput(this);
    bindToolbar(this);
    exposeApi(this);
    offerDraft(this);
    el("loading").hidden = true;
    log.add("info", `Opened ${sceneName}: ${project.entities.size} entities, ${viewer.instance_count()} instances.`);
    requestAnimationFrame((now) => this.tick(now, now));
  },

  /// Puts a newly baked scene in place of the one shown, keeping the camera, the selection and
  /// what is hidden.
  async swap(bytes, scene) {
    const camera = Array.from(this.viewer.camera());
    const old = this.viewer;
    // One canvas holds one viewer: the old one goes first, and no frame is drawn, nor command
    // run, until the new one is there (`whenReady`).
    let ready;
    this.ready = new Promise((resolve) => { ready = resolve; });
    this.viewer = null;
    // A drag or a scrub in progress belonged to the scene being left.
    this.previews.clear();
    // Picks waiting on the old viewer will never be answered by it.
    for (const pick of this.picks) pick.resolve({ id: null, instance: null, point: null });
    this.picks = [];
    this.pickStale = false;
    old.free();
    let opened;
    try {
      opened = await openViewer({ bytes });
      this.source = { bytes };
    } catch (error) {
      // The new scene can't be shown: show the last one again, so the editor goes on working.
      opened = await openViewer(this.source);
      scene = this.project.scene;
      this.viewer = opened.viewer;
      this.streamer = opened.streamer;
      this.file = opened.file;
      this.viewer.set_camera(...camera);
      this.project = new Project(scene, this.viewer);
      ready();
      this.refresh();
      throw error;
    }
    this.viewer = opened.viewer;
    this.streamer = opened.streamer;
    this.file = opened.file;
    this.viewer.set_camera(...camera);
    this.project = new Project(scene, this.viewer);
    this.selection = new Set([...this.selection].filter((id) => this.project.entities.has(id)));
    for (const id of this.hidden) {
      for (const instance of this.project.instancesOf(id)) this.viewer.set_instance_hidden(instance, true);
    }
    ready();
    this.refresh();
  },

  /// Resolves once a viewer is shown: at once, but for while a new scene is put in place.
  whenReady() {
    return this.viewer ? Promise.resolve() : this.ready;
  },

  /// The bake: what is in flight, whether another is wanted after it, and how the last went.
  bake: { running: null, again: false, timer: null, lastMs: null, lastError: null, generation: 0 },

  /// Bakes the scene again soon, after a change to what only a bake makes, such as the terrain.
  /// Changes close together bake once.
  rebakeSoon() {
    clearTimeout(this.bake.timer);
    this.bake.timer = setTimeout(() => {
      this.bake.timer = null;
      this.rebake();
    }, 400);
    el("hint").textContent = "Baking soon…";
  },

  /// Bakes the scene as it is now and shows it. Resolves when what was asked for is shown.
  async rebake() {
    clearTimeout(this.bake.timer);
    this.bake.timer = null;
    if (this.bake.running) {
      this.bake.again = true;
      await this.bake.running;
      return this.bake.running;
    }
    const generation = this.bake.generation;
    const run = (async () => {
      do {
        this.bake.again = false;
        el("hint").textContent = "Baking…";
        const text = writeScene(this.project.scene);
        try {
          const baked = await baker.bake(text, `${sceneName}.scene`);
          // New, while it baked: this scene is gone.
          if (generation !== this.bake.generation) return;
          // Changed while it baked: what was baked is out of date, so bake again.
          if (this.bake.again || writeScene(this.project.scene) !== text) {
            this.bake.again = true;
            continue;
          }
          await this.swap(baked.bytes, this.project.scene);
          this.bake.lastMs = baked.ms;
          this.bake.lastError = null;
          el("hint").textContent = `Baked in ${(baked.ms / 1000).toFixed(1)} s`;
          log.add("info", `Baked ${sceneName} in ${(baked.ms / 1000).toFixed(1)} s.`);
        } catch (error) {
          this.bake.lastError = error.message ?? String(error);
          el("hint").textContent = "The bake failed: see the log";
          log.add("error", `The bake failed: ${this.bake.lastError}`);
        }
      } while (this.bake.again);
    })();
    this.bake.running = run;
    try {
      await run;
    } finally {
      if (this.bake.running === run) this.bake.running = null;
    }
    return null;
  },

  refresh() {
    this.outliner.render();
    this.details.render();
    el("selected").textContent = this.selection.size
      ? [...this.selection].map((id) => this.project.entities.get(id)?.name).join(", ")
      : "Nothing selected";
    el("save").disabled = !this.project.saveable;
    document.title = `${this.dirty ? "• " : ""}${sceneName} — Solstice Editor`;
  },

  tick(now, previous) {
    if (!this.viewer) {
      requestAnimationFrame((next) => this.tick(next, now));
      return;
    }
    const delta = Math.min((now - previous) / 1000, 0.1);
    this.streamer.pump();
    const fly = this.input.flying ? this.input.axes() : [0, 0, 0];
    this.viewer.frame(delta, fly[0], fly[1], fly[2], this.input.lookYaw, this.input.lookPitch, this.input.fast());
    this.input.lookYaw = 0;
    this.input.lookPitch = 0;
    this.frames += 1;
    this.frameMs = this.frameMs * 0.9 + (now - previous) * 0.1;
    this.fps = this.frameMs > 0 ? 1000 / this.frameMs : 0;
    if (this.frames % 15 === 0) {
      el("fps").textContent = `${this.fps.toFixed(0)} fps · ${this.frameMs.toFixed(1)} ms`;
      const errors = this.viewer.gpu_error_count();
      if (errors !== this.gpuErrors) {
        if (this.gpuErrors !== undefined) log.add("error", `GPU errors: ${errors}`);
        this.gpuErrors = errors;
      }
    }
    const pick = this.viewer.take_pick();
    if (pick.length) {
      // An answer to a pick given up on (its tab was hidden) belongs to nobody waiting now.
      if (this.pickStale) this.pickStale = false;
      else if (this.picks.length) {
        const { resolve, fx, fy } = this.picks.shift();
        resolve(pickResult(this, pick, fx, fy));
      }
      if (this.picks.length) this.viewer.request_pick(this.picks[0].fx, this.picks[0].fy);
    }
    this.waits = this.waits.filter((w) => {
      w.frames -= 1;
      if (w.frames > 0) return true;
      w.resolve();
      return false;
    });
    this.renderOverlay();
    requestAnimationFrame((next) => this.tick(next, now));
  },

  camera() {
    return cameraFrom(this.viewer, overlay.clientWidth, overlay.clientHeight);
  },

  /// The selected entity the gizmo sits on: the last selected that has a transform.
  primary() {
    return [...this.selection].reverse().find((id) => this.project.shownTransform(id)) ?? null;
  },

  renderOverlay() {
    const ratio = window.devicePixelRatio || 1;
    const [w, h] = [overlay.clientWidth, overlay.clientHeight];
    if (overlay.width !== Math.round(w * ratio) || overlay.height !== Math.round(h * ratio)) {
      overlay.width = Math.round(w * ratio);
      overlay.height = Math.round(h * ratio);
    }
    context.setTransform(ratio, 0, 0, ratio, 0, 0);
    context.clearRect(0, 0, w, h);
    const camera = this.camera();
    // Things with nowhere to be drawn, such as lights and blockers, show as markers.
    for (const marker of this.markers(camera)) {
      context.fillStyle = this.selection.has(marker.id) ? "#ff9f1a" : "rgba(235, 235, 235, 0.8)";
      context.beginPath();
      context.arc(marker.x, marker.y, 5, 0, Math.PI * 2);
      context.fill();
    }
    for (const id of this.selection) {
      const bounds = this.project.bounds(id);
      if (!bounds) continue;
      const centre = projectToScreen(camera.viewProjection, bounds.centre, w, h);
      if (!centre.front) continue;
      const distance = Math.max(0.01, dot(sub(bounds.centre, camera.position), camera.forward));
      const radius = (bounds.radius / (distance * camera.tanHalfFov)) * (h / 2);
      context.strokeStyle = "#ff9f1a";
      context.lineWidth = 1.5;
      context.setLineDash([6, 4]);
      context.beginPath();
      context.arc(centre.x, centre.y, Math.min(radius, Math.max(w, h)), 0, Math.PI * 2);
      context.stroke();
      context.setLineDash([]);
    }
    const primary = this.primary();
    if (primary) this.gizmo.draw(context, this.project.shownTransform(primary), camera);
  },

  /// Where entities that draw nothing are on screen.
  markers(camera) {
    const out = [];
    for (const [id, entity] of this.project.entities) {
      if (!entity.transform || this.project.drawn(id) || this.hidden.has(id)) continue;
      const p = projectToScreen(camera.viewProjection, this.project.shownTransform(id).position,
        camera.cssWidth, camera.cssHeight);
      if (p.front) out.push({ id, x: p.x, y: p.y });
    }
    return out;
  },

  /// Asks the GPU what is under a point of the viewport, as fractions of its size.
  pick(fx, fy) {
    return withinTime(new Promise((resolve) => {
      this.picks.push({ fx, fy, resolve });
      if (this.picks.length === 1 && !this.pickStale) this.viewer.request_pick(fx, fy);
    }), () => {
      this.picks = [];
      this.pickStale = true;
    });
  },

  /// Resolves after `frames` frames. Frames stop while the tab is hidden, so it gives up after
  /// a while rather than leave an agent waiting for ever.
  waitFrames(frames) {
    const wait = { frames };
    return withinTime(new Promise((resolve) => {
      wait.resolve = resolve;
      this.waits.push(wait);
    }), () => { this.waits = this.waits.filter((w) => w !== wait); }, 2000 + frames * 100);
  },
};

/// Gives up on what needs frames drawn when none come, as in a hidden tab, with an error that
/// says so; `cancel` forgets the wait.
function withinTime(promise, cancel, ms = 5000) {
  let timer;
  const timeout = new Promise((_, reject) => {
    timer = setTimeout(() => {
      cancel();
      reject(new CommandError("not_drawing",
        "the viewport drew no frames in time: is its tab hidden or in the background? Bring it to the front"));
    }, ms);
  });
  return Promise.race([promise, timeout]).finally(() => clearTimeout(timer));
}

/// What a pick found: the entity, the instance and the point hit, from the GPU's answer.
function pickResult(editor, [instance, depth], fx, fy) {
  if (instance < 0) return { id: null, instance: null, point: null };
  const camera = editor.camera();
  const ray = rayThrough(camera.view, camera.projection, fx, fy);
  // Reversed depth with no far plane: depth = near / distance along the view.
  const near = camera.projection[14];
  const along = near / depth;
  const point = add(ray.origin, scaleVector(ray.direction, along / Math.max(1e-6, dot(ray.direction, camera.forward))));
  return {
    id: editor.project.entityAt(instance),
    instance,
    point: point.map((v) => tidy(v, 0.001)),
  };
}

// ---- Changing transforms, as one step for any number of entities.

/// Applies new transforms (by id) and gives back the step that undoes them. What any preview
/// showed is put back first, so the step goes from where things really were.
function transformStep(editor, changes, label) {
  const before = new Map();
  for (const id of changes.keys()) {
    before.set(id, editor.previews.get(id) ?? editor.project.snapshot(id));
  }
  editor.endPreviews();
  for (const [id, transform] of changes) editor.project.setTransform(id, transform, true);
  const after = new Map([...changes.keys()].map((id) => [id, editor.project.snapshot(id)]));
  const apply = (map) => {
    for (const [id, snapshot] of map) editor.project.restore(id, snapshot, true);
    editor.refresh();
  };
  return { label, undo: () => apply(before), redo: () => apply(after) };
}

/// An entity's shown transform made into the project's, given a new one from the editor: a
/// model seated on the ground keeps its seat until its height is changed.
function nextTransform(editor, id, shown) {
  // Compared with where it was before any preview began.
  const original = editor.previews.get(id) ?? editor.project.snapshot(id);
  const was = editor.project.shownFrom(original);
  // Only what changed is tidied; what didn't keeps its value exactly as the file had it.
  const keep = (value, before, step) => (Math.abs(value - before) < 1e-9 ? before : tidy(value, step));
  const rotation = shown.rotation ?? [0, 0, 0];
  const turned = rotation.some((v, k) => Math.abs(v - was.rotation[k]) > 1e-9);
  const out = {
    position: shown.position.map((v, k) => keep(v, was.position[k], 0.0001)),
    rotation: turned ? tidyAngles(rotation) : was.rotation,
    scale: keep(shown.scale ?? 1, was.scale, 0.0001),
  };
  if (!(out.scale >= 0.0001) || !Number.isFinite(out.scale)) {
    throw new CommandError("bad_arguments", "scale must be at least 0.0001");
  }
  if (!out.position.every((v) => Number.isFinite(v) && Math.abs(v) < 1e7)) {
    throw new CommandError("bad_arguments", "positions must be within 10 000 km of the origin");
  }
  // A model seated on the ground stays seated until it is moved; then it keeps the height it
  // was shown at, as both editors' models do, and the project says that height.
  const ground = original.transform.ground;
  const moved = out.position.some((v, k) => v !== was.position[k]);
  if (ground !== undefined && !moved) out.ground = ground;
  return out;
}

/// The same change a gizmo drag made to its entity, made to another selected with it.
function applyChange(shown, change) {
  if (change.move) return { ...shown, position: add(shown.position, change.move) };
  if (change.axis) {
    const rotation = tidyAngles(eulerFromColumns(rotateColumns(rotationColumns(shown.rotation), change.axis, change.angle)));
    return { ...shown, rotation };
  }
  return { ...shown, scale: shown.scale * change.ratio };
}

// ---- Commands: what the panels, the keyboard and agents can do.

function needEntity(editor, id) {
  const entity = editor.project.entities.get(id);
  if (!entity) throw new CommandError("unknown_id", `no entity \`${id}\`; scene.objects lists them`);
  return entity;
}

function needTransform(editor, id) {
  needEntity(editor, id);
  const shown = editor.project.shownTransform(id);
  if (!shown) throw new CommandError("no_transform", `\`${id}\` is a folder or has no place to move`);
  return shown;
}

function describe(editor, id) {
  const entity = editor.project.entities.get(id);
  const shown = editor.project.shownTransform(id);
  return {
    id,
    name: entity.name,
    kind: kindOf(entity),
    parent: editor.project.parents.get(id) ?? null,
    ...(shown ? {
      position: shown.position.map((v) => tidy(v, 0.0001)),
      rotation: shown.rotation,
      scale: shown.scale,
    } : {}),
    ...(entity.transform?.ground !== undefined ? { on_ground: entity.transform.ground } : {}),
    hidden: editor.hidden.has(id),
    selected: editor.selection.has(id),
    instances: editor.project.instancesOf(id).length,
  };
}

function setSelection(editor, ids) {
  editor.selection = new Set(ids);
  for (const id of ids) editor.outliner.reveal(id);
  editor.refresh();
  if (ids.length) editor.outliner.scrollTo(ids[ids.length - 1]);
}

function setHidden(editor, id, hidden) {
  const ids = [id, ...descendants(editor, id)];
  for (const each of ids) {
    if (hidden) editor.hidden.add(each); else editor.hidden.delete(each);
    for (const instance of editor.project.instancesOf(each)) editor.viewer.set_instance_hidden(instance, hidden);
  }
}

function descendants(editor, id) {
  const out = [];
  const visit = (entity) => (entity.children ?? []).forEach((c) => { out.push(c.id); visit(c); });
  visit(editor.project.entities.get(id));
  return out;
}

function frameCamera(editor, ids) {
  const list = ids.length ? ids : [...editor.selection];
  const spheres = list.map((id) => editor.project.bounds(id)).filter(Boolean);
  if (!spheres.length) throw new CommandError("nothing_to_frame", "nothing selected to frame");
  const centre = [0, 1, 2].map((k) => spheres.reduce((s, b) => s + b.centre[k], 0) / spheres.length);
  const radius = Math.max(0.5, ...spheres.map((b) => Math.hypot(...sub(b.centre, centre)) + b.radius));
  const camera = editor.camera();
  const distance = radius / Math.sin(Math.atan(camera.tanHalfFov)) * 1.1;
  const position = sub(centre, scaleVector(camera.forward, distance));
  editor.viewer.set_camera(...position, camera.yaw, camera.pitch);
  editor.input.pivot = centre;
  return { position: position.map((v) => tidy(v, 0.001)), looking_at: centre.map((v) => tidy(v, 0.001)) };
}

function defineCommands(commands, editor) {
  const id = { type: "string", required: true, description: "The entity's id, as scene.objects gives it." };
  commands.define("scene.info", {
    description: "The scene's name, how many entities and instances it has, whether it can be saved, and the API's version.",
    run: () => ({
      scene: sceneName,
      api_version: API_VERSION,
      entities: editor.project.entities.size,
      instances: editor.viewer.instance_count(),
      saveable: editor.project.saveable,
      unsaved_changes: editor.dirty,
      selection: [...editor.selection],
    }),
  });
  commands.define("scene.objects", {
    description: "The scene's entities, parents before children: id, name, kind, parent, position, rotation (degrees), scale, and how many instances draw it. Paged.",
    args: {
      filter: { type: "string", description: "Only those whose name or id holds this text (any case)." },
      kind: { type: "string", description: "Only this kind: mesh, actor, light, blocker, wood, clear, path, yard, pond or folder." },
      offset: { type: "number", description: "Skip this many (default 0)." },
      limit: { type: "number", description: "At most this many (default 100, at most 1000)." },
    },
    example: { filter: "statue" },
    run: ({ filter, kind, offset = 0, limit = 100 }) => {
      const query = filter?.toLowerCase();
      const all = [...editor.project.entities.keys()].filter((each) => {
        const entity = editor.project.entities.get(each);
        return (!query || entity.name.toLowerCase().includes(query) || each.toLowerCase().includes(query))
          && (!kind || kindOf(entity) === kind);
      });
      const page = all.slice(Math.max(0, offset), Math.max(0, offset) + Math.min(1000, Math.max(1, limit)));
      return { total: all.length, objects: page.map((each) => describe(editor, each)) };
    },
  });
  commands.define("object.get", {
    description: "One entity: everything scene.objects gives, its note, and its components as the project holds them.",
    args: { id },
    run: ({ id: which }) => {
      const entity = needEntity(editor, which);
      const { children, ...rest } = entity;
      return { ...describe(editor, which), note: entity.note ?? "", entity: rest, children: (children ?? []).map((c) => c.id) };
    },
  });
  commands.define("object.set", {
    description: "Sets an entity's fields: position, rotation (degrees about X, Y, Z), scale, name, note, or hidden (in the editor only).",
    changes: true,
    args: {
      id,
      position: { type: "vec3", description: "Where it is, in metres." },
      rotation: { type: "vec3", description: "Degrees about X, Y and Z: yaw about Y, then pitch, then roll." },
      scale: { type: "number", description: "Uniform scale, above 0." },
      name: { type: "string", description: "Its name in the outliner." },
      note: { type: "string", description: "What it is for." },
      hidden: { type: "boolean", description: "Hidden in the editor's viewport. Not saved." },
    },
    example: { id: "a91fXq2b", position: [-7, 0, 2] },
    run: (args) => {
      const entity = needEntity(editor, args.id);
      if (args.scale !== undefined && !(args.scale > 0)) throw new CommandError("bad_arguments", "scale must be above 0");
      const steps = [];
      if (args.position || args.rotation || args.scale !== undefined) {
        const shown = needTransform(editor, args.id);
        const next = nextTransform(editor, args.id, {
          position: args.position ?? shown.position,
          rotation: args.rotation ?? shown.rotation,
          scale: args.scale ?? shown.scale,
        });
        steps.push(transformStep(editor, new Map([[args.id, next]]), "Transform"));
      }
      for (const field of ["name", "note"]) {
        if (args[field] === undefined) continue;
        const before = entity[field];
        const set = (value) => {
          if (value === "" && field === "note") delete entity.note; else entity[field] = value;
          editor.refresh();
        };
        set(args[field]);
        steps.push({ label: `Rename`, undo: () => set(before ?? ""), redo: () => set(args[field]) });
      }
      if (args.hidden !== undefined) {
        setHidden(editor, args.id, args.hidden);
        editor.refresh();
      }
      const result = describe(editor, args.id);
      if (!steps.length) return result;
      const label = steps.length === 1 ? `${steps[0].label} ${entity.name}` : `Edit ${entity.name}`;
      return {
        result,
        step: { label, undo: () => [...steps].reverse().forEach((s) => s.undo()), redo: () => steps.forEach((s) => s.redo()) },
      };
    },
  });
  const transformCommand = (name, description, args, example, make) => commands.define(name, {
    description, changes: true, example,
    args: { ids: { type: "string[]", description: "The entities to change; the selection if left out." }, ...args },
    run: (call) => {
      const ids = call.ids ?? [...editor.selection];
      if (!ids.length) throw new CommandError("nothing_selected", `${name} needs ids, or something selected`);
      const changes = new Map();
      for (const each of ids) changes.set(each, nextTransform(editor, each, make(needTransform(editor, each), call)));
      const step = transformStep(editor, changes, `${name.split(".")[1][0].toUpperCase()}${name.split(".")[1].slice(1)} ${ids.length === 1 ? editor.project.entities.get(ids[0]).name : `${ids.length} things`}`);
      return { result: ids.map((each) => describe(editor, each)), step };
    },
  });
  transformCommand("object.move", "Moves entities by an offset, or to a position, in metres.", {
    by: { type: "vec3", description: "How far to move, in metres along X, Y and Z." },
    to: { type: "vec3", description: "Where to put them." },
  }, { ids: ["a91fXq2b"], by: [0, 0, 2] }, (shown, { by, to }) => {
    if (!by === !to) throw new CommandError("bad_arguments", "object.move takes `by` or `to`");
    return { ...shown, position: to ?? add(shown.position, by) };
  });
  transformCommand("object.rotate", "Turns entities: `by` degrees about the world's X, Y and Z axes (applied in that order), or `to` a rotation.", {
    by: { type: "vec3", description: "Degrees to turn about the world's X, then Y, then Z axis." },
    to: { type: "vec3", description: "The rotation to set: degrees about X, Y and Z, as object.set takes it." },
  }, { by: [0, 90, 0] }, (shown, { by, to }) => {
    if (!by === !to) throw new CommandError("bad_arguments", "object.rotate takes `by` or `to`");
    if (to) return { ...shown, rotation: to };
    let columns = rotationColumns(shown.rotation);
    [[1, 0, 0], [0, 1, 0], [0, 0, 1]].forEach((axis, k) => {
      if (by[k]) columns = rotateColumns(columns, axis, by[k] * Math.PI / 180);
    });
    return { ...shown, rotation: eulerFromColumns(columns) };
  });
  transformCommand("object.scale", "Scales entities evenly: `by` a factor, or `to` a scale.", {
    by: { type: "number", description: "A factor, above 0." },
    to: { type: "number", description: "The scale to set, above 0." },
  }, { by: 1.5 }, (shown, { by, to }) => {
    const value = to ?? (by === undefined ? undefined : shown.scale * by);
    if (by !== undefined && to !== undefined) throw new CommandError("bad_arguments", "object.scale takes `by` or `to`");
    if (!(value > 0)) throw new CommandError("bad_arguments", "object.scale needs `by` or `to`, above 0");
    return { ...shown, scale: value };
  });
  commands.define("selection.get", {
    description: "What is selected, as scene.objects describes it.",
    run: () => [...editor.selection].map((each) => describe(editor, each)),
  });
  commands.define("selection.set", {
    description: "Selects entities by id (an empty list clears the selection), as clicking them does.",
    args: { ids: { type: "string[]", required: true, description: "The entities to select." } },
    example: { ids: ["a91fXq2b"] },
    run: ({ ids }) => {
      ids.forEach((each) => needEntity(editor, each));
      setSelection(editor, ids);
      return [...editor.selection];
    },
  });
  commands.define("camera.get", {
    description: "The viewport's camera: position, yaw and pitch (radians; yaw 0 looks along -Z), field of view (degrees) and fly speed (m/s).",
    run: () => {
      const camera = editor.camera();
      return {
        position: camera.position.map((v) => tidy(v, 0.001)),
        yaw: tidy(camera.yaw, 0.0001),
        pitch: tidy(camera.pitch, 0.0001),
        fov_y: tidy(2 * Math.atan(camera.tanHalfFov) * 180 / Math.PI, 0.01),
        speed: editor.viewer.camera_speed(),
      };
    },
  });
  commands.define("camera.set", {
    description: "Moves the viewport's camera: a position, a yaw and pitch (radians), or a point to look at.",
    args: {
      position: { type: "vec3", description: "Where the camera is." },
      yaw: { type: "number", description: "Radians about Y; 0 looks along -Z." },
      pitch: { type: "number", description: "Radians above the horizon." },
      look_at: { type: "vec3", description: "A point to face, instead of yaw and pitch." },
      speed: { type: "number", description: "Fly speed, metres a second." },
    },
    example: { position: [40, 6, 20], look_at: [0, 2, 0] },
    run: ({ position, yaw, pitch, look_at: lookAt, speed }) => {
      const camera = editor.camera();
      const at = position ?? camera.position;
      let [y, p] = [yaw ?? camera.yaw, pitch ?? camera.pitch];
      if (lookAt) {
        const d = normalize(sub(lookAt, at));
        y = Math.atan2(-d[0], -d[2]);
        p = Math.asin(Math.max(-1, Math.min(1, d[1])));
      }
      editor.viewer.set_camera(...at, y, Math.max(-1.5, Math.min(1.5, p)));
      if (speed !== undefined) editor.viewer.set_camera_speed(speed);
      return commands.call("camera.get").then((r) => r.result);
    },
  });
  commands.define("camera.frame", {
    description: "Moves the camera to see entities whole (the selection if no ids), as F does.",
    args: { ids: { type: "string[]", description: "The entities to frame; the selection if left out." } },
    run: ({ ids = [] }) => {
      ids.forEach((each) => needEntity(editor, each));
      return frameCamera(editor, ids);
    },
  });
  commands.define("pick", {
    description: "What is under a point of the viewport, given as fractions of its width and height from the top left: the entity (null for the land or sky), the instance and the point hit. Selects nothing.",
    args: {
      x: { type: "number", required: true, description: "0 at the left edge, 1 at the right." },
      y: { type: "number", required: true, description: "0 at the top, 1 at the bottom." },
    },
    example: { x: 0.5, y: 0.5 },
    run: ({ x, y }) => editor.pick(Math.min(1, Math.max(0, x)), Math.min(1, Math.max(0, y))),
  });
  commands.define("history.undo", {
    description: "Undoes the last change, by anyone.",
    run: () => editor.commands.history.undo()?.label ?? null,
  });
  commands.define("history.redo", {
    description: "Redoes the last change undone.",
    run: () => editor.commands.history.redo()?.label ?? null,
  });
  commands.define("history.list", {
    description: "The changes that can be undone (oldest first) and redone (next first), each with who made it: user or agent.",
    run: () => editor.commands.history.list(),
  });
  commands.define("env.set", {
    description: "The time of day (hours, 0 to 24), the weather (by name, or `changing`), and the ponds (x0,z0,x1,z1,level; with ; between). Not saved yet.",
    args: {
      time: { type: "number", description: "Hours, 0 to 24." },
      weather: { type: "string", description: "changing, or a weather's name: env.weathers lists them." },
      ponds: { type: "string", description: "Ponds as x0,z0,x1,z1,level, with ; between them." },
    },
    example: { time: 18.5, weather: "fair" },
    run: ({ time, weather, ponds }) => {
      if (time !== undefined) editor.viewer.set_time(((time % 24) + 24) % 24);
      if (weather === "changing") editor.viewer.set_weather_changing(true, "fair");
      else if (weather !== undefined && !editor.viewer.set_weather(weather, 10)) {
        throw new CommandError("bad_arguments", `no weather \`${weather}\`; env.weathers lists them`);
      }
      if (ponds !== undefined && !editor.viewer.set_ponds(ponds)) {
        throw new CommandError("bad_arguments", "ponds couldn't be read: x0,z0,x1,z1,level with ; between");
      }
      return { time: tidy(editor.viewer.time_of_day(), 0.01), weather: editor.viewer.weather_summary() };
    },
  });
  commands.define("env.weathers", {
    description: "The names env.set takes for the weather.",
    run: () => editor.viewer.constructor.weather_names().split(",").map((w) => w.trim()).filter(Boolean),
  });
  commands.define("stats.frame", {
    description: "How the viewport is drawing: frames a second, milliseconds a frame, triangles, clusters, instances, geometry pages, megabytes fetched, GPU errors and any fallbacks for this GPU.",
    run: () => ({
      fps: tidy(editor.fps, 0.1),
      frame_ms: tidy(editor.frameMs, 0.01),
      frames: editor.frames,
      triangles: editor.viewer.triangles(),
      clusters: editor.viewer.clusters(),
      instances: editor.viewer.instances(),
      pages: `${editor.viewer.pages_installed()} of ${editor.viewer.pages_total()}`,
      fetched_mb: tidy(editor.file.bytesFetched / 1e6, 0.1),
      gpu_errors: editor.viewer.gpu_error_count(),
      fallbacks: editor.viewer.fallbacks(),
    }),
  });
  commands.define("log.read", {
    description: "The output log: the editor's messages, warnings and errors, each with a number; `since` gives only those after one.",
    args: {
      since: { type: "number", description: "Only entries numbered above this." },
      level: { type: "string", description: "Only info, warn or error." },
    },
    run: ({ since = 0, level }) => log.entries.filter((e) => e.seq > since && (!level || e.level === level)),
  });
  commands.define("wait", {
    description: "Resolves after some frames have been drawn, or once streaming has settled, so stats and screenshots show a finished frame.",
    args: {
      frames: { type: "number", description: "Frames to wait (default 2, at most 600)." },
      settled: { type: "boolean", description: "Wait until no geometry has arrived for half a second (at most 30 s)." },
    },
    run: async ({ frames = 2, settled = false }) => {
      await editor.waitFrames(Math.min(600, Math.max(1, Math.round(frames))));
      if (settled) {
        const until = performance.now() + 30000;
        let last = -1;
        while (performance.now() < until) {
          const pages = editor.viewer.pages_installed();
          if (pages === last) break;
          last = pages;
          await new Promise((resolve) => setTimeout(resolve, 500));
        }
      }
      return { frames: editor.frames };
    },
  });
  commands.define("scene.settings", {
    description: "The settings a scene has one of: its terrain, land bands, sea, texture size and starting camera, as the project holds them. Each is null if the scene has none.",
    run: () => {
      const scene = editor.project.scene;
      return {
        terrain: scene.terrain ?? null,
        land: scene.land ?? null,
        sea: scene.sea ?? null,
        texture_size: scene.texture_size ?? 1024,
        camera: scene.camera ?? null,
        fields: Object.fromEntries(Object.entries(SETTINGS).map(([name, setting]) =>
          [name, { description: setting.description, fields: setting.fields }])),
      };
    },
  });
  commands.define("scene.set", {
    description: "Changes the scene's settings: `terrain` { size, cells, height, shore_x }, `land` { sand, dirt }, `sea` { level, ocean }, each whole, or null to take it away. One undo step. The scene bakes again in the browser; bake.run waits for it.",
    changes: true,
    args: {
      terrain: { type: "object", description: "{ size, cells, height, shore_x }: metres across, cells along a side (8 to 1024; more takes longer), hill height in metres, and the x where the shore begins (the sea lies towards +X). Null for no ground." },
      land: { type: "object", description: "{ sand, dirt }: metres of sand from the water, then of dirt; grass beyond. Null for plain ground." },
      sea: { type: "object", description: "{ level, ocean }: the calm sea's height, and a built-in ocean (coastal). Null for no sea." },
    },
    example: { terrain: { size: 256, cells: 128, height: 6, shore_x: 40 } },
    run: (args) => {
      if (!editor.project.saveable) throw new CommandError("not_a_project", "this scene has no project to change");
      // Models and actors are read from files, which the bake in the browser can't read yet.
      const placed = [...editor.project.entities.values()].find((e) => e.mesh || e.actor);
      if (placed) {
        throw new CommandError("not_bakeable_here",
          `this scene places ${placed.name} from a file, which the browser can't bake yet: change it in the project and bake it with bake --project`);
      }
      if (!Object.keys(args).length) throw new CommandError("bad_arguments", "scene.set takes terrain, land or sea");
      for (const [name, value] of Object.entries(args)) {
        const problem = checkSetting(name, value);
        if (problem) throw new CommandError("bad_arguments", problem);
      }
      const scene = editor.project.scene;
      const before = Object.fromEntries(Object.keys(args).map((name) => [name, structuredClone(scene[name] ?? null)]));
      const apply = (values) => {
        for (const [name, value] of Object.entries(values)) {
          if (value === null) delete scene[name];
          else scene[name] = structuredClone(value);
        }
        editor.refresh();
        editor.rebakeSoon();
      };
      apply(args);
      return {
        result: { baking: true },
        step: { label: `Change ${Object.keys(args).join(" and ")}`, undo: () => apply(before), redo: () => apply(args) },
      };
    },
  });
  commands.define("bake.run", {
    description: "Bakes the scene in the browser now, or waits for the bake already asked for, and shows it. Gives how long it took and any error.",
    run: async () => {
      await editor.rebake();
      if (editor.bake.lastError) throw new CommandError("bake_failed", editor.bake.lastError);
      return { ms: Math.round(editor.bake.lastMs ?? 0), instances: editor.viewer.instance_count() };
    },
  });
  commands.define("bake.status", {
    description: "Whether a bake is running or waiting to, and how the last one went.",
    run: () => ({
      running: Boolean(editor.bake.running),
      waiting: Boolean(editor.bake.timer) && !editor.bake.running,
      last_ms: editor.bake.lastMs === null ? null : Math.round(editor.bake.lastMs),
      last_error: editor.bake.lastError,
    }),
  });
  commands.define("project.new", {
    description: "Starts a new scene from blank terrain: an island of sand, dirt and grass in a coastal sea, baked in the browser and kept there as it changes. The scene shown now is left; save it first if it has changes.",
    args: {
      name: { type: "string", description: "Its name: letters, digits, - and _ (default untitled)." },
      replace: { type: "boolean", description: "Replace a scene of that name kept in this browser. Without it, an existing one is refused." },
      discard: { type: "boolean", description: "Leave the scene shown now even though it has unsaved changes. Without it, they stop New." },
    },
    example: { name: "island" },
    run: async ({ name = "untitled", replace = false, discard = false }) => {
      if (!/^[\w-]+$/.test(name)) throw new CommandError("bad_arguments", "a name is letters, digits, - and _");
      // A scene kept in the browser is its own save; a served one with changes isn't saved.
      const unsaved = editor.dirty && !params.has("draft") && !location.search.includes("draft=");
      if (unsaved && !discard) {
        throw new CommandError("unsaved_changes", "the scene shown has unsaved changes: save it, or pass discard: true");
      }
      if (!replace && name !== sceneName && await readDraft(name)) {
        throw new CommandError("exists", `a scene called ${name} is kept in this browser already: pass replace: true, or choose another name`);
      }
      // Whatever was baking belongs to the scene being left.
      clearTimeout(editor.bake.timer);
      editor.bake.timer = null;
      editor.bake.generation += 1;
      await editor.bake.running;
      await flushAutosave(editor);
      const scene = blankScene();
      const baked = await baker.bake(writeScene(scene), `${name}.scene`);
      sceneName = name;
      editor.commands.history.clear();
      editor.selection.clear();
      editor.hidden.clear();
      await editor.swap(baked.bytes, scene);
      editor.viewer.set_camera(...scene.camera.position, scene.camera.yaw_radians, scene.camera.pitch_radians);
      keepAsDraft(editor);
      history.replaceState(null, "", `?draft=${name}`);
      log.add("info", `New scene ${name}, baked in ${(baked.ms / 1000).toFixed(1)} s.`);
      return { scene: name, ms: Math.round(baked.ms) };
    },
  });
  commands.define("project.get", {
    description: "The scene's project file as it would be saved, and whether it has changes not yet saved.",
    run: () => ({
      saveable: editor.project.saveable,
      unsaved_changes: editor.dirty,
      file: `${sceneName}.scene`,
      text: editor.project.saveable ? editor.project.toJson() : null,
    }),
  });
  commands.define("gizmo.set", {
    description: "The gizmo: its mode (move, rotate, scale), its space (world, local), and snapping (on, and its steps).",
    args: {
      mode: { type: "string", description: "move, rotate or scale." },
      space: { type: "string", description: "world or local." },
      snap: { type: "boolean", description: "Whether drags snap." },
      move_step: { type: "number", description: "Metres a move snaps to." },
      angle_step: { type: "number", description: "Degrees a turn snaps to." },
    },
    run: ({ mode, space, snap, move_step: move, angle_step: angle }) => {
      const gizmo = editor.gizmo;
      if (mode !== undefined) {
        if (!["move", "rotate", "scale"].includes(mode)) throw new CommandError("bad_arguments", "mode is move, rotate or scale");
        gizmo.mode = mode;
      }
      if (space !== undefined) {
        if (!["world", "local"].includes(space)) throw new CommandError("bad_arguments", "space is world or local");
        gizmo.space = space;
      }
      if (snap !== undefined) gizmo.snap.on = snap;
      if (move > 0) gizmo.snap.move = move;
      if (angle > 0) gizmo.snap.angle = angle;
      syncToolbar(editor);
      return { mode: gizmo.mode, space: gizmo.space, snap: { ...gizmo.snap } };
    },
  });
}

// ---- Input: the viewport's mouse and keyboard.

function bindInput(editor) {
  const keys = new Set();
  const input = {
    flying: false,
    lookYaw: 0,
    lookPitch: 0,
    pivot: null,
    axes: () => {
      const axis = (plus, minus) => (keys.has(plus) ? 1 : 0) - (keys.has(minus) ? 1 : 0);
      return [axis("KeyW", "KeyS"), axis("KeyD", "KeyA"), axis("KeyE", "KeyQ")];
    },
    fast: () => keys.has("ShiftLeft") || keys.has("ShiftRight"),
  };
  editor.input = input;
  const fraction = (e) => {
    const box = overlay.getBoundingClientRect();
    return { x: e.clientX - box.left, y: e.clientY - box.top, fx: (e.clientX - box.left) / box.width, fy: (e.clientY - box.top) / box.height };
  };
  let drag = null;

  overlay.addEventListener("contextmenu", (e) => e.preventDefault());
  overlay.addEventListener("pointerdown", (e) => {
    if (!editor.viewer) return;
    overlay.focus();
    const m = fraction(e);
    const camera = editor.camera();
    overlay.setPointerCapture(e.pointerId);
    if (e.button === 2) {
      input.flying = true;
      drag = { kind: "look", last: m };
      return;
    }
    if (e.button === 1 || (e.button === 0 && e.altKey)) {
      const pivot = input.pivot ?? add(camera.position, scaleVector(camera.forward, 10));
      drag = { kind: e.button === 1 ? "pan" : "orbit", last: m, pivot };
      return;
    }
    if (e.button !== 0) return;
    const primary = editor.primary();
    if (primary) {
      const shown = editor.project.shownTransform(primary);
      const handle = editor.gizmo.hit(shown, camera, m.x, m.y);
      if (handle) {
        const ray = rayThrough(camera.view, camera.projection, m.fx, m.fy);
        const others = [...editor.selection].filter((id) => id !== primary && editor.project.shownTransform(id));
        const starts = new Map([primary, ...others].map((id) => [id, editor.project.shownTransform(id)]));
        // Moved as motion while dragged, then static again, so cached shadows redraw once.
        for (const id of starts.keys()) {
          for (const instance of editor.project.instancesOf(id)) editor.viewer.set_instance_movable(instance, true);
        }
        drag = { kind: "gizmo", handle, primary, starts, update: editor.gizmo.drag(handle, shown, camera, ray, m), last: null };
        editor.gizmo.hover = handle;
        return;
      }
    }
    drag = { kind: "click", start: m, additive: e.ctrlKey || e.metaKey || e.shiftKey };
  });
  overlay.addEventListener("pointermove", (e) => {
    if (!editor.viewer) return;
    const m = fraction(e);
    if (!drag) {
      const primary = editor.primary();
      editor.gizmo.hover = primary
        ? editor.gizmo.hit(editor.project.shownTransform(primary), editor.camera(), m.x, m.y)
        : null;
      overlay.style.cursor = editor.gizmo.hover ? "grab" : "default";
      return;
    }
    if (drag.kind === "look") {
      input.lookYaw -= (m.x - drag.last.x) * 0.003;
      input.lookPitch -= (m.y - drag.last.y) * 0.003;
      drag.last = m;
    } else if (drag.kind === "orbit" || drag.kind === "pan") {
      const camera = editor.camera();
      const [dx, dy] = [m.x - drag.last.x, m.y - drag.last.y];
      drag.last = m;
      if (drag.kind === "orbit") {
        const distance = Math.hypot(...sub(camera.position, drag.pivot));
        const yaw = camera.yaw - dx * 0.006;
        const pitch = Math.max(-1.5, Math.min(1.5, camera.pitch - dy * 0.006));
        const forward = [-Math.sin(yaw) * Math.cos(pitch), Math.sin(pitch), -Math.cos(yaw) * Math.cos(pitch)];
        editor.viewer.set_camera(...sub(drag.pivot, scaleVector(forward, distance)), yaw, pitch);
      } else {
        const distance = Math.max(1, dot(sub(drag.pivot, camera.position), camera.forward));
        const perPixel = (distance * camera.tanHalfFov * 2) / camera.cssHeight;
        const right = [Math.cos(camera.yaw), 0, -Math.sin(camera.yaw)];
        const up = normalize(subCross(right, camera.forward));
        const move = add(scaleVector(right, -dx * perPixel), scaleVector(up, dy * perPixel));
        editor.viewer.set_camera(...add(camera.position, move), camera.yaw, camera.pitch);
        drag.pivot = add(drag.pivot, move);
        input.pivot = drag.pivot;
      }
    } else if (drag.kind === "gizmo") {
      const camera = editor.camera();
      const ray = rayThrough(camera.view, camera.projection, m.fx, m.fy);
      // Ctrl inverts snapping while held, as in both editors.
      const { change } = drag.update(ray, !(e.ctrlKey || e.metaKey), m);
      drag.last = change;
      for (const [id, start] of drag.starts) editor.preview(id, applyChange(start, change));
    }
  });
  const finish = async (e) => {
    const was = drag;
    drag = null;
    input.flying = false;
    if (!was || !editor.viewer) return;
    if (was.kind === "gizmo") {
      const changes = new Map();
      if (was.last) {
        for (const [id, start] of was.starts) changes.set(id, nextTransform(editor, id, applyChange(start, was.last)));
      }
      const unchanged = [...changes].every(([id, t]) =>
        JSON.stringify(t) === JSON.stringify(nextTransform(editor, id, was.starts.get(id))));
      if (changes.size && !unchanged) {
        const label = `${editor.gizmo.mode[0].toUpperCase()}${editor.gizmo.mode.slice(1)} ${changes.size === 1 ? editor.project.entities.get(was.primary).name : `${changes.size} things`}`;
        editor.commands.history.push({ ...transformStep(editor, changes, label), by: "user" });
      } else {
        editor.endPreviews();
      }
      for (const id of was.starts.keys()) {
        for (const instance of editor.project.instancesOf(id)) editor.viewer.set_instance_movable(instance, false);
      }
      return;
    }
    if (was.kind === "click" && e) {
      const m = fraction(e);
      if (Math.hypot(m.x - was.start.x, m.y - was.start.y) > 4) return;
      // Markers first: lights and the like, which draw nothing to pick.
      const marker = editor.markers(editor.camera()).find((k) => Math.hypot(k.x - m.x, k.y - m.y) < 8);
      const found = marker ? { id: marker.id } : await editor.pick(m.fx, m.fy);
      const ids = found.id
        ? (was.additive
          ? (editor.selection.has(found.id) ? [...editor.selection].filter((x) => x !== found.id) : [...editor.selection, found.id])
          : [found.id])
        : (was.additive ? [...editor.selection] : []);
      editor.call("selection.set", { ids });
      if (found.id) input.pivot = editor.project.bounds(found.id)?.centre ?? null;
    }
  };
  overlay.addEventListener("pointerup", finish);
  overlay.addEventListener("pointercancel", () => finish(null));
  overlay.addEventListener("wheel", (e) => {
    e.preventDefault();
    if (!editor.viewer) return;
    if (input.flying) {
      // As in Unreal: the wheel while looking sets how fast the camera flies.
      const speed = editor.viewer.camera_speed() * (e.deltaY < 0 ? 1.25 : 0.8);
      editor.viewer.set_camera_speed(Math.max(0.5, Math.min(500, speed)));
      el("hint").textContent = `Fly speed ${editor.viewer.camera_speed().toFixed(1)} m/s`;
      return;
    }
    const camera = editor.camera();
    const distance = input.pivot ? Math.max(1, Math.hypot(...sub(input.pivot, camera.position))) : 10;
    const step = Math.sign(-e.deltaY) * Math.max(0.25, distance * 0.12);
    editor.viewer.set_camera(...add(camera.position, scaleVector(camera.forward, step)), camera.yaw, camera.pitch);
  }, { passive: false });

  const typing = (e) => /^(INPUT|SELECT|TEXTAREA)$/.test(e.target.tagName);
  addEventListener("keydown", (e) => {
    if (typing(e)) return;
    keys.add(e.code);
    if (input.flying) return;
    const ctrl = e.ctrlKey || e.metaKey;
    // Nothing is undone under a drag: its previews stand on what is there now.
    if (ctrl && drag && /^Key[ZY]$/.test(e.code)) {
      e.preventDefault();
    } else if (ctrl && e.code === "KeyZ") {
      e.preventDefault();
      editor.call(e.shiftKey ? "history.redo" : "history.undo");
    } else if (ctrl && e.code === "KeyY") {
      e.preventDefault();
      editor.call("history.redo");
    } else if (ctrl && e.code === "KeyS") {
      e.preventDefault();
      save(editor);
    } else if (!ctrl && e.code === "KeyW") editor.call("gizmo.set", { mode: "move" });
    else if (!ctrl && e.code === "KeyE") editor.call("gizmo.set", { mode: "rotate" });
    else if (!ctrl && e.code === "KeyR") editor.call("gizmo.set", { mode: "scale" });
    else if (e.code === "Space" && !ctrl) {
      e.preventDefault();
      const modes = ["move", "rotate", "scale"];
      editor.call("gizmo.set", { mode: modes[(modes.indexOf(editor.gizmo.mode) + 1) % 3] });
    } else if (e.code === "KeyF" && !ctrl) editor.call("camera.frame", {});
    else if (e.code === "Escape") editor.call("selection.set", { ids: [] });
  });
  addEventListener("keyup", (e) => keys.delete(e.code));
  addEventListener("blur", () => keys.clear());

  new ResizeObserver(() => {
    const [width, height] = editor.resize();
    if (width !== canvas.width || height !== canvas.height) {
      [canvas.width, canvas.height] = [width, height];
      // A viewer being put in place is opened at the canvas's size then.
      editor.viewer?.resize(width, height);
    }
  }).observe(canvas);
}

/// The camera's up: right × forward, for panning.
function subCross(right, forward) {
  return [
    right[1] * forward[2] - right[2] * forward[1],
    right[2] * forward[0] - right[0] * forward[2],
    right[0] * forward[1] - right[1] * forward[0],
  ];
}

// ---- The toolbar.

function syncToolbar(editor) {
  for (const mode of ["move", "rotate", "scale"]) {
    el(`mode-${mode}`).setAttribute("aria-pressed", String(editor.gizmo.mode === mode));
  }
  el("space").textContent = editor.gizmo.space === "world" ? "World" : "Local";
  el("snap").setAttribute("aria-pressed", String(editor.gizmo.snap.on));
  el("snap-move").value = String(editor.gizmo.snap.move);
  el("snap-angle").value = String(editor.gizmo.snap.angle);
}

function bindToolbar(editor) {
  el("undo").addEventListener("click", () => editor.call("history.undo"));
  el("redo").addEventListener("click", () => editor.call("history.redo"));
  el("save").addEventListener("click", () => save(editor));
  el("new").addEventListener("click", async () => {
    const kept = location.search.includes("draft=");
    if (editor.dirty && !kept && !confirm("Leave this scene's unsaved changes for a new scene?")) return;
    el("hint").textContent = "Baking a new scene…";
    // A new scene takes the first free name: untitled, untitled-2...
    let name = "untitled";
    for (let n = 2; name !== sceneName && await readDraft(name); n += 1) name = `untitled-${n}`;
    const outcome = await editor.call("project.new", { name, discard: true });
    if (!outcome.ok) log.add("error", outcome.error.message);
  });
  for (const mode of ["move", "rotate", "scale"]) {
    el(`mode-${mode}`).addEventListener("click", () => editor.call("gizmo.set", { mode }));
  }
  el("space").addEventListener("click", () =>
    editor.call("gizmo.set", { space: editor.gizmo.space === "world" ? "local" : "world" }));
  el("snap").addEventListener("click", () => editor.call("gizmo.set", { snap: !editor.gizmo.snap.on }));
  el("snap-move").addEventListener("change", (e) => editor.call("gizmo.set", { move_step: Number(e.target.value) }));
  el("snap-angle").addEventListener("change", (e) => editor.call("gizmo.set", { angle_step: Number(e.target.value) }));
  el("frame").addEventListener("click", () => editor.call("camera.frame", {}));
  el("view-scene").href = `./?scene=${encodeURIComponent(sceneName)}`;
  el("undo").disabled = true;
  el("redo").disabled = true;
  syncToolbar(editor);
}

// ---- Saving: to the browser as you go, and to a file when asked.

const draftFile = (name = sceneName) => `${name}.scene`;

async function draftDirectory() {
  const root = await navigator.storage?.getDirectory?.();
  return root ? root.getDirectoryHandle("drafts", { create: true }) : null;
}

/// A scene kept in this browser, as its file's text, or null.
async function readDraft(name) {
  try {
    const folder = await draftDirectory();
    return await (await (await folder?.getFileHandle(draftFile(name)))?.getFile())?.text() ?? null;
  } catch {
    return null;
  }
}

/// Writes any autosave still waiting, before the scene it belongs to is left.
async function flushAutosave(editor) {
  if (!autosaveTimer) return;
  clearTimeout(autosaveTimer);
  autosaveTimer = null;
  await writeDraft(editor);
}

/// Keeps the scene in this browser now, as autosave does after each change.
function keepAsDraft(editor) {
  editor.dirty = true;
  scheduleAutosave(editor);
}

let autosaveTimer = null;

async function writeDraft(editor) {
  try {
    const folder = await draftDirectory();
    if (!folder) return;
    const writable = await (await folder.getFileHandle(draftFile(), { create: true })).createWritable();
    await writable.write(editor.project.toJson());
    await writable.close();
    el("hint").textContent = "Draft kept in this browser";
  } catch (error) {
    log.add("warn", `Couldn't keep a draft in this browser: ${error.message ?? error}`);
  }
}
function scheduleAutosave(editor) {
  if (!editor.project.saveable) return;
  clearTimeout(autosaveTimer);
  autosaveTimer = setTimeout(() => {
    autosaveTimer = null;
    writeDraft(editor);
  }, 800);
}

/// Offers to bring back a draft left from an earlier visit, if it differs from the scene served.
async function offerDraft(editor) {
  if (!editor.project.saveable || params.has("draft") || params.has("new")) return;
  try {
    const folder = await draftDirectory();
    const text = await (await (await folder?.getFileHandle(draftFile()))?.getFile())?.text();
    if (!text || text === editor.project.toJson()) return;
    const draft = JSON.parse(text);
    el("draft").hidden = false;
    el("draft-restore").onclick = () => {
      el("draft").hidden = true;
      const changes = new Map();
      const texts = [];
      const visit = (list) => list.forEach((entity) => {
        const now = editor.project.entities.get(entity.id);
        if (!now) return;
        if (entity.transform && now.transform
          && JSON.stringify(entity.transform) !== JSON.stringify(now.transform)) {
          changes.set(entity.id, entity.transform);
        }
        for (const field of ["name", "note"]) {
          if ((entity[field] ?? "") !== (now[field] ?? "")) texts.push([now, field, now[field], entity[field]]);
        }
        visit(entity.children ?? []);
      });
      visit(draft.entities ?? []);
      const setText = (entity, field, value) => {
        if (value === undefined || (field === "note" && value === "")) delete entity[field];
        else entity[field] = value;
      };
      texts.forEach(([entity, field, , after]) => setText(entity, field, after));
      const moved = changes.size ? transformStep(editor, changes, "") : null;
      if (moved || texts.length) {
        editor.commands.history.push({
          label: `Restore draft (${changes.size + texts.length} changes)`,
          by: "user",
          undo: () => {
            moved?.undo();
            texts.forEach(([entity, field, before]) => setText(entity, field, before));
            editor.refresh();
          },
          redo: () => {
            moved?.redo();
            texts.forEach(([entity, field, , after]) => setText(entity, field, after));
            editor.refresh();
          },
        });
      }
      log.add("info", `Restored the draft: ${changes.size} moved, ${texts.length} renamed or noted.`);
    };
    el("draft-discard").onclick = async () => {
      el("draft").hidden = true;
      await folder.removeEntry(draftFile()).catch(() => {});
    };
  } catch { /* no draft */ }
}

let fileHandle = null;
async function save(editor) {
  if (!editor.project.saveable) return;
  const text = editor.project.toJson();
  try {
    if (window.showSaveFilePicker) {
      fileHandle ??= await window.showSaveFilePicker({
        suggestedName: `${sceneName}.scene`,
        types: [{ description: "Solstice scene", accept: { "application/json": [".scene"] } }],
      });
      const writable = await fileHandle.createWritable();
      await writable.write(text);
      await writable.close();
    } else {
      const link = Object.assign(document.createElement("a"), {
        href: URL.createObjectURL(new Blob([text], { type: "application/json" })),
        download: `${sceneName}.scene`,
      });
      link.click();
      URL.revokeObjectURL(link.href);
    }
    editor.dirty = false;
    editor.refresh();
    log.add("info", `Saved ${sceneName}.scene`);
  } catch (error) {
    if (error.name !== "AbortError") log.add("error", `Couldn't save: ${error.message ?? error}`);
  }
}

// ---- The API for agents.

function exposeApi(editor) {
  const api = {
    version: API_VERSION,
    /// Runs a command: `await solstice.call("object.move", { id, by: [0, 0, 2] })`.
    call: (name, args) => editor.commands.call(name, args, "agent"),
    /// Every command with its arguments as JSON Schema, or one by name.
    help: (name) => editor.commands.help(name),
    /// The commands' names.
    list: () => editor.commands.help().map((c) => c.name),
  };
  Object.defineProperty(window, "solstice", { value: Object.freeze(api), configurable: true });
  log.add("info", "Agents: window.solstice.help() lists the editor's commands.");
  registerWebMcp(editor);
}

/// Where the browser offers WebMCP (https://github.com/webmachinelearning/webmcp), each command
/// is a tool an agent sees by name, with the same description and arguments. Tool names can't
/// hold dots, so `object.move` is `object_move`.
function registerWebMcp(editor) {
  const context = navigator.modelContext;
  if (!context) return;
  const tools = editor.commands.help().map((command) => ({
    name: command.name.replace(/\./g, "_"),
    description: command.description,
    inputSchema: command.arguments,
    async execute(args) {
      const outcome = await editor.commands.call(command.name, args ?? {}, "agent");
      return { content: [{ type: "text", text: JSON.stringify(outcome) }], isError: !outcome.ok };
    },
  }));
  try {
    if (typeof context.provideContext === "function") context.provideContext({ tools });
    else for (const tool of tools) context.registerTool(tool);
    log.add("info", `WebMCP: the editor's ${tools.length} commands are tools.`);
  } catch (error) {
    log.add("warn", `WebMCP is here but refused the editor's tools: ${error.message ?? error}`);
  }
}

main().catch((error) => fail(`The editor couldn't start: ${error.message ?? error}`));
