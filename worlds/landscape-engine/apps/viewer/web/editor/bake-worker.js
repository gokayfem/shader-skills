// The editor's bake, off the page's thread (docs/editor.md, "One bake"): takes a project's
// scene, gives back the scene file the viewer plays. Loaded only when the editor first bakes.
//
// In: `{ id, scene, name, content }`, where `scene` is the scene file's text and `content` the
// folder holding ready-made surfaces (`bake --surfaces`). Out: `{ id, bytes, ms }`, or
// `{ id, error }`.

import init, { bake_scene, provide_layer, surface_key } from "../bake-wasm/solstice_bake_web.js";

/// The surfaces a land is laid with, which take minutes to make and so come ready-made.
const LAND_SURFACES = ["grass", "sand", "dirt"];

let ready = null;
/// Surfaces handed over already, by key and size.
const provided = new Set();

async function provideSurfaces(content, size) {
  await Promise.all(LAND_SURFACES.map(async (name) => {
    const key = surface_key(name);
    if (provided.has(`${key}@${size}`)) return;
    const response = await fetch(`${content}${key}-${size}.solstice`).catch(() => null);
    // Without one, the bake makes the surface itself: right, but slow.
    if (!response?.ok) return;
    provide_layer(key, new Uint8Array(await response.arrayBuffer()));
    provided.add(`${key}@${size}`);
  }));
}

onmessage = async ({ data: { id, scene, name, content } }) => {
  try {
    // A start that failed, such as a fetch that didn't arrive, is tried again next time.
    ready ??= init().catch((error) => {
      ready = null;
      throw error;
    });
    await ready;
    const size = JSON.parse(scene).texture_size ?? 1024;
    await provideSurfaces(content, size);
    const start = performance.now();
    const bytes = bake_scene(scene, name);
    postMessage({ id, bytes, ms: performance.now() - start }, [bytes.buffer]);
  } catch (error) {
    postMessage({ id, error: String(error?.message ?? error) });
  }
};
