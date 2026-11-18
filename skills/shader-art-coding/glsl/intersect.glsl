//! intersect: analytic ray-primitive intersections. Rays are ro + t * rd with unit rd.
//! Use them for bounding volumes (clip the raymarch range to where the scene can be), exact floors and water planes,
//! glass spheres and eyeballs, and cheap reflections.
//! Formulas follow https://iquilezles.org/articles/intersectors/.

/// Ray vs plane dot(p, n) + h = 0 (unit n). Returns t, which is negative when the plane is behind the ray or parallel to it.
/// @use floors, water, mirrors, clipping marches to a ground plane
float iPlane(vec3 ro, vec3 rd, vec3 n, float h) {
    float denom = dot(rd, n);
    if (abs(denom) < 1e-8) return -1.0;
    return -(dot(ro, n) + h) / denom;
}

/// Ray vs sphere centered at c with radius r. Returns vec2(tNear, tFar), or vec2(-1) on a miss.
/// Uses the precision-friendly form that measures from the closest point on the ray.
/// @use bounding spheres, glass balls, eyeballs, planets, bubbles
vec2 iSphere(vec3 ro, vec3 rd, vec3 c, float r) {
    vec3 oc = ro - c;
    float b = dot(oc, rd);
    vec3 qc = oc - b * rd;
    float h = r * r - dot(qc, qc);
    if (h < 0.0) return vec2(-1.0);
    h = sqrt(h);
    return vec2(-b - h, -b + h);
}

/// Ray vs axis-aligned box centered at the origin with half-extents b (slab method).
/// Returns vec2(tNear, tFar), or vec2(-1) on a miss. Works when the ray starts inside (tNear < 0 < tFar).
/// @use bounding boxes for raymarching, glass cubes, rooms
vec2 iBox(vec3 ro, vec3 rd, vec3 b) {
    vec3 m = 1.0 / rd;
    vec3 n = m * ro;
    vec3 k = abs(m) * b;
    vec3 t1 = -n - k;
    vec3 t2 = -n + k;
    float tN = max(max(t1.x, t1.y), t1.z);
    float tF = min(min(t2.x, t2.y), t2.z);
    if (tN > tF || tF < 0.0) return vec2(-1.0);
    return vec2(tN, tF);
}

/// Ray interval for an axis-aligned ellipsoid at the origin with positive radii. Returns (-1,-1) on a miss.
/// @use transparent sea creatures, squashable bubbles, bounded volume integration with a known optical path
/// @see Analytic sphere intersection in scaled coordinates.
vec2 iEllipsoid(vec3 ro, vec3 rd, vec3 radii) {
    vec3 v = rd / radii;
    float speed = length(v);
    vec2 t = iSphere(ro / radii, v / speed, vec3(0.0), 1.0);
    if (t.x == -1.0 && t.y == -1.0) return vec2(-1.0);
    return t / speed;
}
