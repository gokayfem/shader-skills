// The transform gizmo: move, rotate and scale handles drawn over the viewport, as Unreal's and
// Unity's are, a constant size on screen. It is drawn on a 2D canvas over the scene, and turns a
// drag on a handle into a new transform; the editor makes that a command.

import {
  add, closestAlongLine, cross, dot, eulerFromColumns, length, multiply, normalize,
  projectToScreen, rayPlane, rotateColumns, rotationColumns, scale, sub, tidy, tidyAngles,
} from "./math.js";

const COLOURS = ["#e5484d", "#46a758", "#3e63dd"];
const HOVER = "#ffd60a";
/// How tall the gizmo stands on screen, in CSS pixels.
const SIZE_PX = 130;
const PICK_PX = 9;
const RING_SEGMENTS = 64;

export class Gizmo {
  /// "move", "rotate" or "scale".
  mode = "move";
  /// "world" or "local": which way the handles point.
  space = "world";
  snap = { on: true, move: 0.5, angle: 15, scale: 0.1 };
  /// The handle under the mouse, or being dragged.
  hover = null;

  /// The handles' frame for `transform`, given the camera: where its centre is, which way its
  /// axes point, and how long they are in metres so they look `SIZE_PX` tall.
  #frame(transform, camera) {
    const origin = transform.position;
    const axes = this.space === "local" && this.mode !== "scale"
      ? rotationColumns(transform.rotation)
      : [[1, 0, 0], [0, 1, 0], [0, 0, 1]];
    const distance = Math.max(0.01, dot(sub(origin, camera.position), camera.forward));
    const size = distance * camera.tanHalfFov * 2 * SIZE_PX / camera.cssHeight;
    return { origin, axes, size };
  }

  /// Handles as screen shapes: each `{ handle, kind, points, colour }`.
  #shapes(transform, camera) {
    const { origin, axes, size } = this.#frame(transform, camera);
    const toScreen = (p) => projectToScreen(camera.viewProjection, p, camera.cssWidth, camera.cssHeight);
    const shapes = [];
    if (this.mode === "rotate") {
      axes.forEach((axis, k) => {
        const u = axes[(k + 1) % 3];
        const v = axes[(k + 2) % 3];
        const points = [];
        for (let i = 0; i <= RING_SEGMENTS; i++) {
          const a = (i / RING_SEGMENTS) * Math.PI * 2;
          const p = add(origin, add(scale(u, Math.cos(a) * size), scale(v, Math.sin(a) * size)));
          points.push({ ...toScreen(p), facing: dot(sub(p, origin), camera.forward) < 0 });
        }
        shapes.push({ handle: { kind: "ring", axis: k }, kind: "ring", points, colour: COLOURS[k] });
      });
      return shapes;
    }
    axes.forEach((axis, k) => {
      const tip = add(origin, scale(axis, size));
      shapes.push({
        handle: { kind: this.mode === "move" ? "axis" : "scale", axis: k },
        kind: this.mode === "move" ? "arrow" : "stick",
        points: [toScreen(origin), toScreen(tip)],
        colour: COLOURS[k],
      });
    });
    if (this.mode === "move") {
      // A square in each plane, between the two axes it holds.
      axes.forEach((_, k) => {
        const u = axes[(k + 1) % 3];
        const v = axes[(k + 2) % 3];
        const corner = (a, b) => toScreen(add(origin, add(scale(u, a * size), scale(v, b * size))));
        shapes.push({
          handle: { kind: "plane", axis: k },
          kind: "quad",
          points: [corner(0.25, 0.25), corner(0.45, 0.25), corner(0.45, 0.45), corner(0.25, 0.45)],
          colour: COLOURS[k],
        });
      });
    }
    shapes.push({
      handle: { kind: this.mode === "move" ? "free" : "uniform", axis: -1 },
      kind: "centre",
      points: [toScreen(origin)],
      colour: "#f2f2f2",
    });
    return shapes;
  }

  draw(context, transform, camera) {
    context.save();
    context.lineCap = "round";
    for (const shape of this.#shapes(transform, camera)) {
      const hot = this.hover && this.hover.kind === shape.handle.kind && this.hover.axis === shape.handle.axis;
      const colour = hot ? HOVER : shape.colour;
      context.strokeStyle = colour;
      context.fillStyle = colour;
      const [a, b] = shape.points;
      if (shape.points.some((p) => !p.front)) continue;
      if (shape.kind === "ring") {
        // The back half of a ring is fainter, as both editors draw it.
        for (let i = 1; i < shape.points.length; i++) {
          const p = shape.points[i - 1];
          const q = shape.points[i];
          context.globalAlpha = p.facing ? 1 : 0.35;
          context.lineWidth = hot ? 4 : 2.5;
          context.beginPath();
          context.moveTo(p.x, p.y);
          context.lineTo(q.x, q.y);
          context.stroke();
        }
        context.globalAlpha = 1;
      } else if (shape.kind === "arrow" || shape.kind === "stick") {
        context.lineWidth = hot ? 4 : 2.5;
        context.beginPath();
        context.moveTo(a.x, a.y);
        context.lineTo(b.x, b.y);
        context.stroke();
        const dx = b.x - a.x;
        const dy = b.y - a.y;
        const l = Math.hypot(dx, dy) || 1;
        const [ux, uy] = [dx / l, dy / l];
        context.beginPath();
        if (shape.kind === "arrow") {
          context.moveTo(b.x + ux * 12, b.y + uy * 12);
          context.lineTo(b.x - uy * 5, b.y + ux * 5);
          context.lineTo(b.x + uy * 5, b.y - ux * 5);
        } else {
          context.rect(b.x - 5, b.y - 5, 10, 10);
        }
        context.fill();
      } else if (shape.kind === "quad") {
        context.globalAlpha = hot ? 0.75 : 0.4;
        context.beginPath();
        shape.points.forEach((p, i) => (i ? context.lineTo(p.x, p.y) : context.moveTo(p.x, p.y)));
        context.closePath();
        context.fill();
        context.globalAlpha = 1;
      } else if (shape.kind === "centre") {
        context.beginPath();
        context.arc(a.x, a.y, hot ? 6 : 4.5, 0, Math.PI * 2);
        context.fill();
      }
    }
    context.restore();
  }

  /// The handle under a point of the canvas, in CSS pixels, or null.
  hit(transform, camera, x, y) {
    let best = null;
    let bestDistance = PICK_PX;
    const consider = (handle, distance) => {
      if (distance < bestDistance) {
        bestDistance = distance;
        best = handle;
      }
    };
    for (const shape of this.#shapes(transform, camera)) {
      if (shape.points.some((p) => !p.front)) continue;
      if (shape.kind === "centre") {
        consider(shape.handle, Math.hypot(shape.points[0].x - x, shape.points[0].y - y) - 3);
      } else if (shape.kind === "quad") {
        if (insidePolygon(shape.points, x, y)) consider(shape.handle, 1);
      } else {
        for (let i = 1; i < shape.points.length; i++) {
          // A ring is picked by its front half first, where the eye goes.
          const penalty = shape.points[i].facing === false ? 2 : 0;
          consider(shape.handle, segmentDistance(shape.points[i - 1], shape.points[i], x, y) + penalty);
        }
      }
    }
    return best;
  }

  /// Starts dragging `handle` on an entity whose transform is `start`, from a mouse ray. Gives
  /// back a function from each later ray (whether to snap, and the mouse) to the change so far:
  /// `{ transform, change }`, where `transform` is the dragged entity's and `change` is the same
  /// change for any other selected with it: `{ move: [x, y, z] }`, `{ axis, angle }` (radians
  /// about a world axis) or `{ ratio }`.
  drag(handle, start, camera, ray0, mouse0) {
    const { origin, axes } = this.#frame(start, camera);
    const columns0 = rotationColumns(start.rotation);
    const snap = (on) => on && this.snap.on;
    if (handle.kind === "axis") {
      const axis = axes[handle.axis];
      const t0 = closestAlongLine(origin, axis, ray0);
      return (ray, snapping) => {
        const t = closestAlongLine(origin, axis, ray);
        if (t === null || t0 === null) return { transform: start, change: { move: [0, 0, 0] } };
        const position = this.#snapMove(add(start.position, scale(axis, t - t0)), start.position, axis, snap(snapping));
        return { transform: { ...start, position }, change: { move: sub(position, start.position) } };
      };
    }
    if (handle.kind === "plane" || handle.kind === "free") {
      // The free handle moves across the screen's own plane.
      const normal = handle.kind === "plane" ? axes[handle.axis] : camera.forward;
      const p0 = rayPlane(ray0, origin, normal);
      return (ray, snapping) => {
        const p = rayPlane(ray, origin, normal);
        if (!p || !p0) return { transform: start, change: { move: [0, 0, 0] } };
        const position = this.#snapMove(add(start.position, sub(p, p0)), start.position, null, snap(snapping));
        return { transform: { ...start, position }, change: { move: sub(position, start.position) } };
      };
    }
    if (handle.kind === "ring") {
      const axis = axes[handle.axis];
      const p0 = rayPlane(ray0, origin, axis) ?? add(origin, axes[(handle.axis + 1) % 3]);
      const v0 = normalize(sub(p0, origin));
      return (ray, snapping) => {
        const p = rayPlane(ray, origin, axis);
        if (!p) return { transform: start, change: { axis, angle: 0 } };
        const v = normalize(sub(p, origin));
        let angle = Math.atan2(dot(cross(v0, v), axis), dot(v0, v));
        if (snap(snapping)) {
          const step = this.snap.angle * Math.PI / 180;
          angle = Math.round(angle / step) * step;
        }
        const rotation = tidyAngles(eulerFromColumns(rotateColumns(columns0, axis, angle)));
        return { transform: { ...start, rotation }, change: { axis, angle } };
      };
    }
    // Scale, by any handle: by how much farther from the centre on screen the mouse is.
    const centre = projectToScreen(camera.viewProjection, origin, camera.cssWidth, camera.cssHeight);
    const d0 = Math.max(4, Math.hypot(mouse0.x - centre.x, mouse0.y - centre.y));
    return (ray, snapping, mouse) => {
      const ratio = Math.hypot(mouse.x - centre.x, mouse.y - centre.y) / d0;
      let s = Math.max(0.01, start.scale * ratio);
      s = snap(snapping) ? Math.max(this.snap.scale, tidy(s, this.snap.scale)) : tidy(s, 0.001);
      return { transform: { ...start, scale: s }, change: { ratio: s / start.scale } };
    };
  }

  /// Snaps a moved position to the grid: along `axis` only, if given, as moving along one axis
  /// shouldn't shift the others.
  #snapMove(position, from, axis, on) {
    if (!on) return position.map((v) => tidy(v, 0.001));
    const step = this.snap.move;
    const isWorldAxis = axis && axis.filter((v) => Math.abs(v) > 1e-6).length === 1;
    return position.map((v, k) => {
      if (axis && isWorldAxis && Math.abs(axis[k]) < 1e-6) return from[k];
      if (axis && !isWorldAxis) {
        // Along a turned axis, the distance moved snaps, not the coordinates.
        const moved = dot(sub(position, from), axis);
        return tidy(from[k] + axis[k] * Math.round(moved / step) * step, 0.001);
      }
      return tidy(Math.round(v / step) * step, 0.001);
    });
  }
}

function segmentDistance(a, b, x, y) {
  const dx = b.x - a.x;
  const dy = b.y - a.y;
  const l2 = dx * dx + dy * dy;
  const t = l2 ? Math.max(0, Math.min(1, ((x - a.x) * dx + (y - a.y) * dy) / l2)) : 0;
  return Math.hypot(a.x + dx * t - x, a.y + dy * t - y);
}

function insidePolygon(points, x, y) {
  let inside = false;
  for (let i = 0, j = points.length - 1; i < points.length; j = i++) {
    const [a, b] = [points[i], points[j]];
    if ((a.y > y) !== (b.y > y) && x < ((b.x - a.x) * (y - a.y)) / (b.y - a.y) + a.x) inside = !inside;
  }
  return inside;
}

/// The camera as the gizmo needs it, from the viewer's matrices and the canvas's CSS size.
export function cameraFrom(viewer, cssWidth, cssHeight) {
  const both = Array.from(viewer.view_projection());
  const view = both.slice(0, 16);
  const projection = both.slice(16);
  const at = Array.from(viewer.camera());
  const [yaw, pitch] = [at[3], at[4]];
  const forward = [-Math.sin(yaw) * Math.cos(pitch), Math.sin(pitch), -Math.cos(yaw) * Math.cos(pitch)];
  return {
    view,
    projection,
    viewProjection: multiply(projection, view),
    position: at.slice(0, 3),
    yaw,
    pitch,
    forward,
    // projection[5] is 1 / tan(fov_y / 2).
    tanHalfFov: 1 / projection[5],
    cssWidth,
    cssHeight,
  };
}

export { length };
