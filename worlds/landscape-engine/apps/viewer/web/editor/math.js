// Small vector and matrix helpers for the editor. Matrices are 16 numbers by column, as glam and
// WGSL keep them, so they pass to and from the viewer unchanged.

export const DEG = Math.PI / 180;

export const add = (a, b) => [a[0] + b[0], a[1] + b[1], a[2] + b[2]];
export const sub = (a, b) => [a[0] - b[0], a[1] - b[1], a[2] - b[2]];
export const scale = (a, s) => [a[0] * s, a[1] * s, a[2] * s];
export const dot = (a, b) => a[0] * b[0] + a[1] * b[1] + a[2] * b[2];
export const cross = (a, b) => [
  a[1] * b[2] - a[2] * b[1],
  a[2] * b[0] - a[0] * b[2],
  a[0] * b[1] - a[1] * b[0],
];
export const length = (a) => Math.hypot(a[0], a[1], a[2]);
export const normalize = (a) => {
  const l = length(a) || 1;
  return [a[0] / l, a[1] / l, a[2] / l];
};

export function multiply(a, b) {
  const out = new Array(16);
  for (let c = 0; c < 4; c++) {
    for (let r = 0; r < 4; r++) {
      out[c * 4 + r] = a[r] * b[c * 4] + a[4 + r] * b[c * 4 + 1] + a[8 + r] * b[c * 4 + 2]
        + a[12 + r] * b[c * 4 + 3];
    }
  }
  return out;
}

export function invert(m) {
  const [a00, a01, a02, a03, a10, a11, a12, a13, a20, a21, a22, a23, a30, a31, a32, a33] = m;
  const b00 = a00 * a11 - a01 * a10;
  const b01 = a00 * a12 - a02 * a10;
  const b02 = a00 * a13 - a03 * a10;
  const b03 = a01 * a12 - a02 * a11;
  const b04 = a01 * a13 - a03 * a11;
  const b05 = a02 * a13 - a03 * a12;
  const b06 = a20 * a31 - a21 * a30;
  const b07 = a20 * a32 - a22 * a30;
  const b08 = a20 * a33 - a23 * a30;
  const b09 = a21 * a32 - a22 * a31;
  const b10 = a21 * a33 - a23 * a31;
  const b11 = a22 * a33 - a23 * a32;
  const det = b00 * b11 - b01 * b10 + b02 * b09 + b03 * b08 - b04 * b07 + b05 * b06;
  if (!det) return null;
  const d = 1 / det;
  return [
    (a11 * b11 - a12 * b10 + a13 * b09) * d,
    (a02 * b10 - a01 * b11 - a03 * b09) * d,
    (a31 * b05 - a32 * b04 + a33 * b03) * d,
    (a22 * b04 - a21 * b05 - a23 * b03) * d,
    (a12 * b08 - a10 * b11 - a13 * b07) * d,
    (a00 * b11 - a02 * b08 + a03 * b07) * d,
    (a32 * b02 - a30 * b05 - a33 * b01) * d,
    (a20 * b05 - a22 * b02 + a23 * b01) * d,
    (a10 * b10 - a11 * b08 + a13 * b06) * d,
    (a01 * b08 - a00 * b10 - a03 * b06) * d,
    (a30 * b04 - a31 * b02 + a33 * b00) * d,
    (a21 * b02 - a20 * b04 - a23 * b00) * d,
    (a11 * b07 - a10 * b09 - a12 * b06) * d,
    (a00 * b09 - a01 * b07 + a02 * b06) * d,
    (a31 * b01 - a30 * b03 - a32 * b00) * d,
    (a20 * b03 - a21 * b01 + a22 * b00) * d,
  ];
}

export const transformPoint = (m, p) => {
  const w = m[3] * p[0] + m[7] * p[1] + m[11] * p[2] + m[15];
  return [
    (m[0] * p[0] + m[4] * p[1] + m[8] * p[2] + m[12]) / w,
    (m[1] * p[0] + m[5] * p[1] + m[9] * p[2] + m[13]) / w,
    (m[2] * p[0] + m[6] * p[1] + m[10] * p[2] + m[14]) / w,
  ];
};

/// A rotation's 3x3 as columns, from degrees about X, Y and Z applied as the bake applies them:
/// yaw about Y, then pitch about the turned X, then roll about the turned Z (R = Ry Rx Rz).
export function rotationColumns([x, y, z]) {
  const [sx, cx] = [Math.sin(x * DEG), Math.cos(x * DEG)];
  const [sy, cy] = [Math.sin(y * DEG), Math.cos(y * DEG)];
  const [sz, cz] = [Math.sin(z * DEG), Math.cos(z * DEG)];
  // Rows of Ry * Rx * Rz.
  const m00 = cy * cz + sy * sx * sz;
  const m01 = -cy * sz + sy * sx * cz;
  const m02 = sy * cx;
  const m10 = cx * sz;
  const m11 = cx * cz;
  const m12 = -sx;
  const m20 = -sy * cz + cy * sx * sz;
  const m21 = sy * sz + cy * sx * cz;
  const m22 = cy * cx;
  return [[m00, m10, m20], [m01, m11, m21], [m02, m12, m22]];
}

/// Degrees about X, Y and Z (as `rotationColumns` takes them) of a rotation given as columns.
export function eulerFromColumns(columns) {
  const [[m00, m10, m20], [m01, m11, m21], [m02, m12, m22]] = columns;
  const x = Math.asin(Math.max(-1, Math.min(1, -m12)));
  let y;
  let z;
  if (Math.abs(m12) < 0.99999) {
    y = Math.atan2(m02, m22);
    z = Math.atan2(m10, m11);
  } else {
    // Looking straight up or down: yaw and roll turn about the same axis, so roll takes none.
    y = Math.atan2(-m20, m00);
    z = 0;
  }
  return [x / DEG, y / DEG, z / DEG];
}

/// The matrix of a project transform: `{ position, rotation, scale }`.
export function transformMatrix({ position, rotation = [0, 0, 0], scale: s = 1 }) {
  const [c0, c1, c2] = rotationColumns(rotation);
  return [
    c0[0] * s, c0[1] * s, c0[2] * s, 0,
    c1[0] * s, c1[1] * s, c1[2] * s, 0,
    c2[0] * s, c2[1] * s, c2[2] * s, 0,
    position[0], position[1], position[2], 1,
  ];
}

/// Rotates columns about a unit axis by `angle` radians: the axis-angle matrix times them.
export function rotateColumns(columns, axis, angle) {
  const [x, y, z] = axis;
  const c = Math.cos(angle);
  const s = Math.sin(angle);
  const t = 1 - c;
  const r = [
    [t * x * x + c, t * x * y + s * z, t * x * z - s * y],
    [t * x * y - s * z, t * y * y + c, t * y * z + s * x],
    [t * x * z + s * y, t * y * z - s * x, t * z * z + c],
  ];
  return columns.map((v) => [
    r[0][0] * v[0] + r[1][0] * v[1] + r[2][0] * v[2],
    r[0][1] * v[0] + r[1][1] * v[1] + r[2][1] * v[2],
    r[0][2] * v[0] + r[1][2] * v[1] + r[2][2] * v[2],
  ]);
}

/// Rounds a number for the project file, so edits leave tidy numbers: `step` apart, and no -0.
export function tidy(value, step) {
  const rounded = Math.round(value / step) * step;
  // Clears the binary noise a multiple of a step like 0.001 carries.
  const out = Number(rounded.toFixed(Math.max(0, Math.ceil(-Math.log10(step)) + 1)));
  return Object.is(out, -0) ? 0 : out;
}

/// Turns a rotation's degrees into the range (-180, 180], tidied.
export function tidyAngles(angles) {
  return angles.map((a) => {
    let d = ((a % 360) + 360) % 360;
    if (d > 180) d -= 360;
    return tidy(d, 0.001);
  });
}

/// A ray from the camera through a point of the canvas, given as fractions of its size.
export function rayThrough(view, projection, fx, fy) {
  const inverse = invert(multiply(projection, view));
  const ndcX = fx * 2 - 1;
  const ndcY = 1 - fy * 2;
  // Reversed depth: 1 is the near plane, and smaller is farther.
  const near = transformPoint(inverse, [ndcX, ndcY, 1]);
  const far = transformPoint(inverse, [ndcX, ndcY, 0.001]);
  return { origin: near, direction: normalize(sub(far, near)) };
}

/// Where a point of the world lands on the canvas, in pixels, and whether it is in front.
export function projectToScreen(viewProjection, point, width, height) {
  const m = viewProjection;
  const x = m[0] * point[0] + m[4] * point[1] + m[8] * point[2] + m[12];
  const y = m[1] * point[0] + m[5] * point[1] + m[9] * point[2] + m[13];
  const w = m[3] * point[0] + m[7] * point[1] + m[11] * point[2] + m[15];
  return { x: (x / w * 0.5 + 0.5) * width, y: (0.5 - y / w * 0.5) * height, front: w > 1e-4 };
}

/// The nearest point on a line (through `origin`, along unit `axis`) to a ray: how far along the
/// line it is, or null where the two are nearly parallel.
export function closestAlongLine(origin, axis, ray) {
  const w = sub(origin, ray.origin);
  const b = dot(axis, ray.direction);
  const d = dot(axis, w);
  const e = dot(ray.direction, w);
  const denominator = 1 - b * b;
  if (denominator < 1e-6) return null;
  return (b * e - d) / denominator;
}

/// Where a ray meets a plane through `point` with unit `normal`, or null if it doesn't.
export function rayPlane(ray, point, normal) {
  const denominator = dot(ray.direction, normal);
  if (Math.abs(denominator) < 1e-6) return null;
  const t = dot(sub(point, ray.origin), normal) / denominator;
  return t < 0 ? null : add(ray.origin, scale(ray.direction, t));
}
