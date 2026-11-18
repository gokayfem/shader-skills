// Tests of the web pages' own code that need no browser: the editor's project file writer,
// rotation maths and gizmo drags, and the viewer's automatic quality. Run with `node apps/viewer/web/editor/test.mjs`; CI runs it.

import { readFileSync } from "node:fs";
import { Commands } from "./commands.js";
import { Gizmo } from "./gizmo.js";
import * as m from "./math.js";
import { writeScene } from "./project.js";
import { AutoQuality } from "../auto-quality.js";

let failures = 0;
function check(what, ok, detail) {
  console.log(`${ok ? "ok  " : "FAIL"} ${what}${ok ? "" : `: ${JSON.stringify(detail)}`}`);
  if (!ok) failures += 1;
}

// ---- The file writer writes what the Rust crate writes, so saving changes nothing.
{
  const fixture = readFileSync(new URL("../../../../crates/solstice-project/tests/fixture.scene", import.meta.url), "utf8")
    .replace(/\r\n/g, "\n");
  const written = writeScene(JSON.parse(fixture));
  const at = [...written].findIndex((c, i) => c !== fixture[i]);
  check("a scene written by the editor is the file the bake wrote", written === fixture,
    at < 0 ? "lengths differ" : { at, editor: written.slice(at - 40, at + 40), bake: fixture.slice(at - 40, at + 40) });
  const edited = JSON.parse(fixture);
  edited.entities[1].children[2].transform.position = [-7.25, 0, 1e-7];
  check("numbers are written as f32s are, with .0 on whole ones",
    writeScene(edited).includes('"position": [-7.25, 0.0, 1e-7]'));
}

// ---- Rotations: degrees about X, Y and Z, applied as the bake applies them.
{
  let worst = 0;
  for (let k = 0; k < 2000; k++) {
    const a = [Math.random() * 170 - 85, Math.random() * 360 - 180, Math.random() * 360 - 180];
    const b = m.eulerFromColumns(m.rotationColumns(a));
    worst = Math.max(worst, ...a.map((v, i) => Math.abs(v - b[i])));
  }
  check("angles survive a trip through a matrix", worst < 1e-9, worst);
  check("a turn about Y alone stays about Y alone",
    JSON.stringify(m.tidyAngles(m.eulerFromColumns(m.rotationColumns([0, 137.5, 0])))) === "[0,137.5,0]");
  // glam's Quat::from_euler(EulerRot::YXZ, y, x, z) is Ry Rx Rz: X turned 90 about Y points along -Z.
  const [x] = m.rotationColumns([0, 90, 0]);
  check("yaw turns X towards -Z, as glam's does", Math.abs(x[2] + 1) < 1e-12, x);
  const matrix = m.transformMatrix({ position: [1, 2, 3], rotation: [10, 20, 30], scale: 2 });
  const identity = m.multiply(m.invert(matrix), matrix);
  check("a matrix times its inverse is the identity",
    identity.every((v, i) => Math.abs(v - (i % 5 === 0 ? 1 : 0)) < 1e-9));
}

// ---- The gizmo, driven by rays as a mouse would drive it.
{
  // A camera at (0, 10, 10) looking at the origin, built as glam's look_to_rh and
  // perspective_infinite_reverse_rh build the viewer's.
  const position = [0, 10, 10];
  const forward = m.normalize(m.sub([0, 0, 0], position));
  const f = 1 / Math.tan((55 * Math.PI) / 360);
  const side = m.normalize(m.cross(forward, [0, 1, 0]));
  const up = m.cross(side, forward);
  const view = [side[0], up[0], -forward[0], 0, side[1], up[1], -forward[1], 0, side[2], up[2], -forward[2], 0,
    -m.dot(side, position), -m.dot(up, position), m.dot(forward, position), 1];
  const projection = [f / (16 / 9), 0, 0, 0, 0, f, 0, 0, 0, 0, 0, -1, 0, 0, 0.05, 0];
  const camera = {
    view, projection, viewProjection: m.multiply(projection, view), position, forward,
    yaw: Math.atan2(-forward[0], -forward[2]), pitch: Math.asin(forward[1]),
    tanHalfFov: 1 / f, cssWidth: 1600, cssHeight: 900,
  };
  const ray = (p) => m.rayThrough(view, projection, p.x / 1600, p.y / 900);
  const screen = (p) => m.projectToScreen(camera.viewProjection, p, 1600, 900);
  const start = { position: [0, 0, 0], rotation: [0, 90, 0], scale: 1 };
  const centre = screen([0, 0, 0]);
  const gizmo = new Gizmo();
  // How long the handles are at this distance.
  const size = 10 * Math.SQRT2 * camera.tanHalfFov * 2 * 130 / 900;

  const middle = screen([size / 2, 0, 0]);
  const arrow = gizmo.hit(start, camera, middle.x, middle.y);
  check("the X arrow is under its middle", arrow?.kind === "axis" && arrow.axis === 0, arrow);
  // Grabbed at its middle, and dragged 2.3 m farther along.
  const target = screen([size / 2 + 2.3, 0, 0]);
  const moved = gizmo.drag(arrow, start, camera, ray(middle), middle)(ray(target), true, target);
  check("moving along X snaps x to the grid and leaves y and z",
    JSON.stringify(moved.transform.position) === "[2.5,0,0]", moved.transform);
  const free = gizmo.drag(arrow, start, camera, ray(middle), middle)(ray(target), false, target);
  check("and without snapping goes where the mouse does", Math.abs(free.transform.position[0] - 2.3) < 0.01, free.transform);

  gizmo.mode = "rotate";
  const h = size * Math.SQRT1_2;
  const [from, to] = [screen([h, 0, h]), screen([h, 0, -h])];
  const ring = gizmo.hit(start, camera, from.x, from.y);
  check("the Y ring is under a point of it", ring?.kind === "ring" && ring.axis === 1, ring);
  const turned = gizmo.drag(ring, start, camera, ray(from), from)(ray(to), true, to);
  check("a quarter turn round it adds 90 degrees of yaw",
    JSON.stringify(turned.transform.rotation) === "[0,180,0]", turned.transform);

  gizmo.mode = "scale";
  const knob = screen([size, 0, 0]);
  const handle = gizmo.hit(start, camera, knob.x, knob.y);
  check("a scale handle is under its knob", handle?.kind === "scale", handle);
  const far = { x: centre.x + (knob.x - centre.x) * 2, y: centre.y + (knob.y - centre.y) * 2 };
  check("dragging twice as far from the centre doubles the scale",
    gizmo.drag(handle, start, camera, ray(knob), knob)(ray(far), true, far).transform.scale === 2);
}

// ---- Commands check their arguments, never throw, and keep the history.
{
  const state = { value: 0 };
  const commands = new Commands(state);
  commands.define("value.set", {
    description: "Sets the value.",
    args: { to: { type: "number", required: true, description: "The value." } },
    run: ({ to }, context) => {
      const before = context.value;
      context.value = to;
      return { result: to, step: { label: "Set", undo: () => { context.value = before; }, redo: () => { context.value = to; } } };
    },
  });
  check("a call runs", (await commands.call("value.set", { to: 3 }, "agent")).result === 3 && state.value === 3);
  const unknown = await commands.call("value.sett", {});
  check("an unknown command is an error, not a throw", unknown.ok === false && unknown.error.code === "unknown_command");
  const typo = await commands.call("value.set", { too: 3 });
  check("an unknown argument is named", typo.ok === false && /too/.test(typo.error.message), typo);
  const wrong = await commands.call("value.set", { to: "3" });
  check("a wrong type is refused", wrong.ok === false && wrong.error.code === "bad_arguments");
  check("the history says who did what", JSON.stringify(commands.history.list().done) === '[{"label":"Set","by":"agent"}]');
  commands.history.undo();
  check("undo puts it back", state.value === 0);
  const help = commands.help("value.set");
  check("help gives JSON Schema", help.arguments.required[0] === "to" && help.arguments.properties.to.type === "number");
}

// ---- Automatic quality steps down while slow, faster when far too slow, never while loading.
{
  // Reports until the next step, at a steady frame rate.
  const reportsToStep = (auto, fps) => {
    for (let n = 1; n <= 100; n++) if (auto.report(fps, false)) return n;
    return Infinity;
  };
  const retina = new AutoQuality(2);
  check("a high-density display ends on Low at one pixel per CSS pixel",
    JSON.stringify(retina.steps.at(-1)) === '{"quality":"low","pixelRatio":1}', retina.steps);
  check("a display of ratio 1 has no step for fewer pixels", new AutoQuality(1).steps.length === 3);
  check("a fast frame rate never steps", reportsToStep(new AutoQuality(2), 60) === Infinity);
  check("a little slow: three reports after the four at start", reportsToStep(new AutoQuality(2), 25) === 7);
  check("far too slow: two after the four at start", reportsToStep(retina, 10) === 6);
  check("and then two after the one for new targets", reportsToStep(retina, 10) === 3);
  reportsToStep(retina, 10);
  check("the last step is fewer pixels", retina.step.pixelRatio === 1 && retina.label === "low, fewer pixels");
  check("nothing after the last step", reportsToStep(retina, 1) === Infinity);
  const loading = new AutoQuality(2);
  for (let n = 0; n < 20; n++) loading.report(5, true);
  check("loading never steps down", loading.level === 0);
  const hitch = new AutoQuality(1);
  for (const fps of [60, 60, 60, 60, 10, 60, 10, 60, 25, 25, 60]) hitch.report(fps, false);
  check("a slow report between fast ones is a hitch", hitch.level === 0);
}

console.log(failures ? `${failures} failed` : "all passed");
process.exit(failures ? 1 : 0);
