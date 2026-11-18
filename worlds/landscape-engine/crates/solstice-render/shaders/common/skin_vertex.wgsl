// Poses a skinned vertex. Its skin is read from the page pool: see common/pool.wgsl for what a
// shader that includes this must define.

#include "common/pool.wgsl"

// Where a vertex of a cluster goes in the current pose, in the mesh's own space. Rigid instances
// stay as they are.
fn posed_position(instance: Instance, cluster: Cluster, local: u32, p: vec3f) -> vec3f {
    if !instance_skinned(instance) {
        return p;
    }
    return skin_point(skin_rows(instance.bones, vertex_skin(cluster, local), false), p);
}
