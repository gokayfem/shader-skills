// The outliner (the scene's entities as a tree) and the details panel (the selected entity's
// fields), as plain DOM. Both only show the editor's state and send commands; neither changes
// anything itself.

import { kindOf } from "./project.js";
import { SETTINGS } from "./scenes.js";

const ICONS = {
  folder: "▤", mesh: "◆", actor: "●", light: "✦", blocker: "▭", wood: "♣", clear: "○",
  path: "┅", yard: "▦", pond: "≈",
};

const make = (tag, props = {}, children = []) => {
  const node = Object.assign(document.createElement(tag), props);
  for (const child of children) node.append(child);
  return node;
};

export class Outliner {
  #root;
  #search;
  #rows = new Map();
  #collapsed = new Set();

  /// `editor` gives `project`, `selection`, `call(name, args)` and `hidden` (a set of ids).
  constructor(element, editor) {
    this.editor = editor;
    this.#search = make("input", {
      type: "search", placeholder: "Search", className: "search", ariaLabel: "Search the outliner",
    });
    this.#search.addEventListener("input", () => this.render());
    this.#root = make("div", { className: "tree", role: "tree" });
    element.append(this.#search, this.#root);
  }

  /// Opens the folders holding `id`, so it can be seen.
  reveal(id) {
    for (const parent of this.editor.project.ancestors(id)) this.#collapsed.delete(parent);
  }

  render() {
    const { project, selection } = this.editor;
    const query = this.#search.value.trim().toLowerCase();
    const matches = (entity) => !query || entity.name.toLowerCase().includes(query)
      || entity.id.toLowerCase().includes(query)
      || (entity.children ?? []).some(matches);
    this.#root.replaceChildren();
    this.#rows.clear();
    const fragment = document.createDocumentFragment();
    const visit = (list, depth) => {
      for (const entity of list) {
        if (!matches(entity)) continue;
        const kind = kindOf(entity);
        const open = query || !this.#collapsed.has(entity.id);
        const hasChildren = (entity.children ?? []).length > 0;
        const row = make("div", {
          className: `row${selection.has(entity.id) ? " selected" : ""}`,
          role: "treeitem",
          ariaSelected: String(selection.has(entity.id)),
          title: entity.note || entity.name,
        });
        row.dataset.id = entity.id;
        row.style.paddingLeft = `${6 + depth * 14}px`;
        const twisty = make("span", {
          className: "twisty",
          textContent: hasChildren ? (open ? "▾" : "▸") : "",
        });
        twisty.addEventListener("click", (e) => {
          e.stopPropagation();
          if (this.#collapsed.has(entity.id)) this.#collapsed.delete(entity.id);
          else this.#collapsed.add(entity.id);
          this.render();
        });
        const icon = make("span", { className: `icon ${kind}`, textContent: ICONS[kind] ?? "·" });
        const label = make("span", { className: "label", textContent: entity.name });
        const hidden = this.editor.hidden.has(entity.id);
        const eye = make("button", {
          className: `eye${hidden ? " off" : ""}`,
          textContent: hidden ? "◌" : "◉",
          title: hidden ? "Show" : "Hide",
          ariaLabel: `${hidden ? "Show" : "Hide"} ${entity.name}`,
        });
        eye.addEventListener("click", (e) => {
          e.stopPropagation();
          this.editor.call("object.set", { id: entity.id, hidden: !hidden });
        });
        row.append(twisty, icon, label, eye);
        row.addEventListener("click", (e) => {
          const ids = e.ctrlKey || e.metaKey
            ? (selection.has(entity.id)
              ? [...selection].filter((id) => id !== entity.id)
              : [...selection, entity.id])
            : [entity.id];
          this.editor.call("selection.set", { ids });
        });
        row.addEventListener("dblclick", () => this.editor.call("camera.frame", { ids: [entity.id] }));
        fragment.append(row);
        this.#rows.set(entity.id, row);
        if (hasChildren && open) visit(entity.children, depth + 1);
      }
    };
    visit(project.scene.entities, 0);
    this.#root.append(fragment);
  }

  scrollTo(id) {
    this.#rows.get(id)?.scrollIntoView({ block: "nearest" });
  }
}

/// A number field whose label drags to change it, as both editors' are.
function numberField(label, value, step, onChange, onScrub) {
  const input = make("input", { type: "number", value: String(value), step: String(step) });
  input.addEventListener("change", () => {
    const v = Number(input.value);
    if (Number.isFinite(v)) onChange(v);
  });
  input.addEventListener("keydown", (e) => {
    if (e.key === "Enter") input.blur();
    e.stopPropagation();
  });
  const name = make("span", { className: `axis ${label.toLowerCase()}`, textContent: label });
  name.addEventListener("pointerdown", (e) => {
    const start = Number(input.value);
    const x0 = e.clientX;
    name.setPointerCapture(e.pointerId);
    let last = start;
    const move = (m) => {
      const fine = m.shiftKey ? 0.1 : 1;
      last = Number((start + Math.round((m.clientX - x0) * fine) * step).toFixed(6));
      input.value = String(last);
      onScrub(last);
    };
    const up = () => {
      name.removeEventListener("pointermove", move);
      name.removeEventListener("pointerup", up);
      if (last !== start) onChange(last, start);
    };
    name.addEventListener("pointermove", move);
    name.addEventListener("pointerup", up);
  });
  return make("label", { className: "number" }, [name, input]);
}

/// What each setting and field is called on the panel.
const LABELS = {
  terrain: "Terrain", land: "Land", sea: "Sea",
  size: "Size (m)", cells: "Cells", height: "Hills (m)", shore_x: "Shore x",
  sand: "Sand (m)", dirt: "Dirt (m)", level: "Level (m)", ocean: "Ocean",
};

/// What a setting starts as when it is turned on.
const STARTING = {
  terrain: { size: 256, cells: 128, height: 3, shore_x: 64 },
  land: { sand: 10, dirt: 5 },
  sea: { level: -0.6, ocean: "coastal" },
};

export class Details {
  #element;

  /// The scene's own settings, shown when nothing is selected: each a section that can be
  /// turned on or off, whose fields bake the scene again when changed.
  #renderScene() {
    const scene = this.editor.project.scene;
    this.#element.append(make("p", {
      className: "empty",
      textContent: "Nothing selected: these are the scene's own settings. A change bakes the scene again.",
    }));
    for (const [name, setting] of Object.entries(SETTINGS)) {
      const value = scene[name];
      const toggle = make("input", { type: "checkbox", checked: Boolean(value), title: `Has ${name}` });
      toggle.addEventListener("click", (e) => e.stopPropagation());
      toggle.addEventListener("change", () =>
        this.editor.call("scene.set", { [name]: toggle.checked ? (value ?? STARTING[name]) : null }));
      const summary = make("summary", {}, [toggle, ` ${LABELS[name]}`]);
      const rows = [];
      if (value) {
        for (const [key, field] of Object.entries(setting.fields)) {
          const set = (v) => this.editor.call("scene.set", { [name]: { ...value, [key]: v } });
          let input;
          if (field.names) {
            input = make("select", {}, field.names.map((n) => make("option", { value: n, textContent: n, selected: n === value[key] })));
            input.addEventListener("change", () => set(input.value));
          } else {
            input = numberField("·", value[key], field.step, (v) => {
              const clamped = Math.min(field.max, Math.max(field.min, field.integer ? Math.round(v) : v));
              if (clamped !== value[key]) set(clamped);
            }, () => {});
          }
          rows.push(make("div", { className: "field" }, [make("span", { className: "name", textContent: LABELS[key] ?? key }), input]));
        }
      }
      this.#element.append(make("details", { open: Boolean(value), title: setting.description }, [summary, ...rows]));
    }
  }

  constructor(element, editor) {
    this.#element = element;
    this.editor = editor;
  }

  render() {
    const { project, selection } = this.editor;
    const ids = [...selection];
    this.#element.replaceChildren();
    if (ids.length === 0) {
      if (project.saveable) this.#renderScene();
      else this.#element.append(make("p", { className: "empty", textContent: "Select something to see its details." }));
      return;
    }
    if (ids.length > 1) {
      this.#element.append(make("p", { className: "empty", textContent: `${ids.length} selected` }));
      return;
    }
    const id = ids[0];
    const entity = project.entities.get(id);
    if (!entity) return;
    const section = (title, rows) => make("details", { open: true }, [make("summary", { textContent: title }), ...rows]);
    const row = (label, value) => make("div", { className: "field" }, [
      make("span", { className: "name", textContent: label }), value,
    ]);
    const text = (value, onChange, readOnly = false) => {
      const input = make("input", { type: "text", value: value ?? "", readOnly });
      input.addEventListener("change", () => onChange(input.value));
      input.addEventListener("keydown", (e) => {
        if (e.key === "Enter") input.blur();
        e.stopPropagation();
      });
      return input;
    };
    const general = [
      row("Name", text(entity.name, (name) => this.editor.call("object.set", { id, name }))),
      row("Id", text(entity.id, () => {}, true)),
      row("Kind", make("span", { textContent: kindOf(entity) })),
    ];
    if (entity.note !== undefined || project.saveable) {
      const note = make("textarea", { value: entity.note ?? "", rows: 2 });
      note.addEventListener("change", () => this.editor.call("object.set", { id, note: note.value }));
      note.addEventListener("keydown", (e) => e.stopPropagation());
      general.push(row("Note", note));
    }
    this.#element.append(section("General", general));

    const shown = project.shownTransform(id);
    if (shown) {
      const vector = (label, key, step) => {
        const fields = [0, 1, 2].map((k) => numberField("XYZ"[k], shown[key][k], step,
          (value) => {
            const next = [...shown[key]];
            next[k] = value;
            this.editor.call("object.set", { id, [key]: next });
          },
          (value) => {
            const next = [...shown[key]];
            next[k] = value;
            this.editor.preview(id, { ...shown, [key]: next });
          }));
        return row(label, make("div", { className: "vector" }, fields));
      };
      const rows = [
        vector("Location", "position", 0.01),
        vector("Rotation", "rotation", 0.1),
        row("Scale", numberField("S", shown.scale, 0.01,
          (scale) => this.editor.call("object.set", { id, scale }),
          (scale) => this.editor.preview(id, { ...shown, scale }))),
      ];
      if (shown.ground !== undefined) {
        rows.push(row("On ground", make("span", {
          className: "hint",
          textContent: `${shown.ground} sunk; moving it keeps its height instead`,
        })));
      }
      this.#element.append(section("Transform", rows));
    }

    const kind = kindOf(entity);
    if (kind !== "folder") {
      const component = entity[kind];
      const rows = Object.entries(component).map(([key, value]) => row(key,
        make("span", {
          className: "value",
          textContent: typeof value === "object" ? JSON.stringify(value) : String(value),
        })));
      this.#element.append(section(kind[0].toUpperCase() + kind.slice(1), rows));
    }
    const drawn = project.instancesOf(id);
    if (drawn.length) {
      this.#element.append(section("Drawn by", [row("Instances", make("span", {
        textContent: drawn.length === 1 ? `#${drawn[0]}` : `#${drawn[0]} to #${drawn[drawn.length - 1]}`,
      }))]));
    }
  }
}
