// Which instances sway in the wind, and how far it can carry them. The bending itself is in
// common/sway_vertex.wgsl. See docs/weather-effects.md.

// False in the shaders of a scene with nothing that sways, so that it pays nothing for the wind:
// see `shaders::expand_for`.
const SWAYING: bool = true;
// The farthest a blade's bend carries a point, as a share of the point's height: an arc turned
// through GRASS_MAX_ANGLE takes it 0.578 of its height sideways and 0.277 down, 0.641 in all,
// rounded up.
const GRASS_REACH: f32 = 0.65;

// Trees sway by their height and reach from the trunk, never further than this share of the
// height plus this much, in metres: what culling allows them.
const TREE_REACH: f32 = 0.1;
const TREE_REACH_BASE: f32 = 0.6;

fn instance_sways(i: Instance) -> bool {
    return SWAYING && (i.material & INSTANCE_SWAYS) != 0u;
}

fn instance_tree(i: Instance) -> bool {
    return (i.material & INSTANCE_TREE) != 0u;
}

fn instance_flutters(i: Instance) -> bool {
    return (i.material & INSTANCE_FLUTTERS) != 0u;
}

// How much a bounding sphere of a swaying instance must grow, in the mesh's units, to hold
// what the wind can do to it: its top, `center_y + radius` above the instance's ground, can
// move that far. Nothing else ever moves, and in a view that doesn't sway nothing does.
fn sway_reach(i: Instance, center_y: f32, radius: f32, view_sways: bool) -> f32 {
    if !view_sways || !instance_sways(i) {
        return 0.0;
    }
    let top = max(center_y + radius, 0.0);
    if instance_tree(i) {
        return TREE_REACH * top + TREE_REACH_BASE;
    }
    return GRASS_REACH * top;
}
