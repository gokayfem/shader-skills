//! camera: look-at frames, primary rays, field of view, thin-lens depth of field.
//! Convention: uv = (2.0 * fragCoord - iResolution.xy) / iResolution.y, so uv.y spans [-1, 1] and aspect is kept.

/// Camera-to-world matrix looking from ro toward ta, with roll in radians. Columns are (right, up, forward).
/// @use every 3D shader
mat3 cameraMatrix(vec3 ro, vec3 ta, float roll) {
    vec3 w = normalize(ta - ro);
    vec3 up = vec3(sin(roll), cos(roll), 0.0);
    vec3 u = normalize(cross(w, up));
    vec3 v = cross(u, w);
    return mat3(u, v, w);
}

/// Focal length (the z of the view-space ray) for a vertical field of view in degrees.
/// @use matching a reference photo, wide (90) vs telephoto (20) framing
float fovToFocal(float fovDegrees) { return 1.0 / tan(radians(fovDegrees) * 0.5); }

/// Primary ray direction for screen point uv. focal is about 1.5-2.5 for natural framing (see fovToFocal).
/// @use every 3D shader
vec3 cameraRay(mat3 cam, vec2 uv, float focal) {
    return normalize(cam * vec3(uv, focal));
}

/// Thin-lens depth of field. Moves the ray origin over a lens disk of radius aperture, and re-aims it so that
/// points at distance focusDist stay sharp. xi is a pair of uniform random numbers in [0,1) (e.g. hash22 of the
/// pixel and frame). Average many samples (AA loop or multipass accumulation).
/// @use macro shots, product renders, cinematic focus pulls, bokeh
void thinLensRay(mat3 cam, vec2 uv, float focal, float aperture, float focusDist, vec2 xi,
                 inout vec3 ro, out vec3 rd) {
    vec3 dir = normalize(cam * vec3(uv, focal));
    float ft = focusDist / dot(dir, cam[2]);
    vec3 focusPoint = ro + dir * ft;
    float a = TAU * xi.x;
    float r = aperture * sqrt(xi.y);
    ro += cam * vec3(r * cos(a), r * sin(a), 0.0);
    rd = normalize(focusPoint - ro);
}

/// Orthographic ray: parallel rays from a plane of half-height `size` centered on ro.
/// @use isometric scenes, technical illustrations, flat diorama looks
void orthoRay(mat3 cam, vec2 uv, float size, inout vec3 ro, out vec3 rd) {
    ro += cam * vec3(uv * size, 0.0);
    rd = cam[2];
}

/// Project a world-space transverse error into pixels at positive camera-space depth, with vertical FOV focal length.
/// @use cluster refinement thresholds, selecting vegetation detail, silhouette error budgets
/// @see Pinhole projection, screen height times focal length divided by twice the forward depth.
float projectedErrorPx(float worldError, float depth, float focal, float imageHeight) {
    return max(worldError, 0.0) * max(focal, 0.0) * max(imageHeight, 0.0) / (2.0 * max(depth, 1e-5));
}
