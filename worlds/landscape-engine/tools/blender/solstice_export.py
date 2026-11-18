"""Solstice's export tool for rigged characters (docs/gameplay.md, The export tool).

Prepares the rigged character in the open .blend file for the engine, has Blender's own glTF
writer write it with one fixed set of options, then checks the file with Solstice's importer
(`bake --check-character`), failing loudly if anything is wrong. It works for any character with
one armature, not only MakeHuman's.

    blender --background --factory-startup --python-exit-code 1 CHARACTER.blend \
        --python tools/blender/solstice_export.py -- \
        OUT.glb --roles characters/rigs/game_engine.roles [--texture-size 1024] [--as-is]

`--python-exit-code 1` matters: without it Blender exits 0 even when the script fails. The tool
also fails on any error itself, and writes OUT.glb only once the file has passed its checks; a
file that fails them is left as OUT.failed.glb, to look at.

What it does, in order:
1. Hidden geometry. Shape keys are applied as they are mixed. Mask modifiers, which MakeHuman
   uses to hide helper geometry and the body under clothes, are carried out for real: the
   vertices they hide are deleted. Disabled modifiers are dropped, other modifiers but the
   armature are applied. Every vertex group that isn't a deforming bone is dropped.
2. Weights. Tiny weights dropped, at most four bones a vertex, renormalised. Leaf bones that
   deform nothing are removed, unless the role file names them.
3. Space. Transforms applied and the armature in its rest pose; the glTF writer turns Blender's
   Z up to glTF's Y up, in metres.
4. Materials. Each is baked with Cycles to the textures the engine reads: base colour (with
   alpha, where the material cuts out), normal, and roughness with metallic. Each material then
   comes out right, whatever its node graph.
5. Export, with fixed options, so the same .blend always gives the same file on one machine.
6. Checks, by reading the file back.

`--as-is` skips steps 1 to 4: it exports the character as Blender's writer would by default, to
show what the steps are for. Its checks fail.
"""

import argparse
import os
import shlex
import subprocess
import sys
import traceback

import bmesh
import bpy
import numpy

REPO = os.path.abspath(os.path.join(os.path.dirname(__file__), "..", ".."))
# Below this, a weight is noise: dropped before the four strongest are kept.
TINY_WEIGHT = 0.01
# Where a cut-out material's alpha falls below this, the surface isn't there.
ALPHA_CUTOFF = 0.5
# A material is cut out only if more than this share of the texels it covers are see-through.
MIN_HOLE_SHARE = 0.001
# Texels baked beyond each UV island's edge, so filtering never reaches the cleared background.
BAKE_MARGIN = 8


class ExportFailed(Exception):
    pass


def fail(message):
    raise ExportFailed(message)


def parse_args():
    argv = sys.argv[sys.argv.index("--") + 1:] if "--" in sys.argv else []
    parser = argparse.ArgumentParser(prog="solstice_export.py")
    parser.add_argument("output", help="the .glb to write")
    parser.add_argument("--roles", help="the rig's role file, whose bones must all be kept")
    parser.add_argument("--texture-size", type=int, default=1024)
    parser.add_argument("--as-is", action="store_true", help="export without preparing anything")
    parser.add_argument("--checker", default=None,
                        help="the command that checks the file (default: cargo run -p bake)")
    return parser.parse_args(argv)


def read_role_bones(path):
    bones = []
    if path:
        for line in open(path, encoding="utf-8"):
            words = line.split("#")[0].split()
            bones.extend(words[1:])
    return set(bones)


def select_only(objects, active=None):
    if bpy.context.object and bpy.context.object.mode != "OBJECT":
        bpy.ops.object.mode_set(mode="OBJECT")
    for o in bpy.context.view_layer.objects:
        o.select_set(False)
    for o in objects:
        o.select_set(True)
    bpy.context.view_layer.objects.active = active or (objects[0] if objects else None)


def character_objects():
    """The one armature, and every mesh it deforms."""
    armatures = [o for o in bpy.context.scene.objects if o.type == "ARMATURE"]
    if len(armatures) != 1:
        fail(f"a character has one armature, and this file has {len(armatures)}")
    rig = armatures[0]
    meshes = []
    for o in bpy.context.scene.objects:
        if o.type != "MESH":
            continue
        if any(m.type == "ARMATURE" and m.object == rig for m in o.modifiers):
            meshes.append(o)
        elif o.parent == rig:
            # A mesh carried by a bone, rather than deformed by the skin, isn't supported yet.
            fail(f"mesh `{o.name}` hangs from the rig but no armature modifier deforms it")
    if not meshes:
        fail("the armature deforms no mesh")
    return rig, sorted(meshes, key=lambda o: o.name)


def deforming_bones(rig):
    return {b.name for b in rig.data.bones if b.use_deform}


# 1. Hidden geometry -----------------------------------------------------------------------------

def apply_shape_keys(mesh):
    if mesh.data.shape_keys:
        select_only([mesh])
        bpy.ops.object.shape_key_remove(all=True, apply_mix=True)


def carry_out_masks(mesh):
    """Deletes what each live mask modifier hides, and removes the modifier."""
    for modifier in list(mesh.modifiers):
        if modifier.type != "MASK":
            continue
        if not modifier.show_viewport:
            mesh.modifiers.remove(modifier)  # it hides nothing
            continue
        if modifier.mode != "VERTEX_GROUP":
            fail(f"`{mesh.name}`: mask `{modifier.name}` hides by {modifier.mode}, not a vertex group")
        if modifier.use_smooth:
            fail(f"`{mesh.name}`: mask `{modifier.name}` cuts smoothly, which this tool can't carry out")
        group = mesh.vertex_groups.get(modifier.vertex_group)
        if group is None:
            fail(f"`{mesh.name}`: mask `{modifier.name}` names a missing group `{modifier.vertex_group}`")
        bm = bmesh.new()
        bm.from_mesh(mesh.data)
        layer = bm.verts.layers.deform.verify()
        threshold = modifier.threshold
        doomed = []
        for vert in bm.verts:
            inside = vert[layer].get(group.index, 0.0) > threshold
            if inside == modifier.invert_vertex_group:
                doomed.append(vert)
        bmesh.ops.delete(bm, geom=doomed, context="VERTS")
        bm.to_mesh(mesh.data)
        bm.free()
        mesh.modifiers.remove(modifier)


def apply_other_modifiers(mesh):
    """Applies each live modifier but the armature, as the viewport shows it, and drops the
    disabled ones."""
    for modifier in list(mesh.modifiers):
        if modifier.type == "ARMATURE":
            continue
        if not modifier.show_viewport:
            mesh.modifiers.remove(modifier)
            continue
        select_only([mesh])
        bpy.ops.object.modifier_apply(modifier=modifier.name)


def drop_non_bone_groups(mesh, bone_names):
    for group in list(mesh.vertex_groups):
        if group.name not in bone_names:
            mesh.vertex_groups.remove(group)


# 2. Weights ------------------------------------------------------------------------------------

def tidy_weights(mesh):
    for group in mesh.vertex_groups:
        group.lock_weight = False  # locked groups are skipped by cleaning and normalising
    select_only([mesh])
    # A vertex whose only weights are tiny keeps its strongest, rather than following nothing.
    bpy.ops.object.vertex_group_clean(group_select_mode="ALL", limit=TINY_WEIGHT, keep_single=True)
    bpy.ops.object.vertex_group_limit_total(group_select_mode="ALL", limit=4)
    bpy.ops.object.vertex_group_normalize_all(group_select_mode="ALL", lock_active=False)
    loose = sum(1 for v in mesh.data.vertices if not any(g.weight > 0 for g in v.groups))
    if loose:
        fail(f"`{mesh.name}`: {loose} vertices follow no bone")


def remove_idle_leaf_bones(rig, meshes, keep):
    """Removes bones that deform nothing, from the tips inwards, unless `keep` names them."""
    used = set()
    for mesh in meshes:
        names = {g.index: g.name for g in mesh.vertex_groups}
        for vert in mesh.data.vertices:
            for g in vert.groups:
                if g.weight > 0:
                    used.add(names[g.group])
    select_only([rig])
    bpy.ops.object.mode_set(mode="EDIT")
    removed = []
    changed = True
    while changed:
        changed = False
        for bone in list(rig.data.edit_bones):
            if not bone.children and bone.name not in used and bone.name not in keep:
                removed.append(bone.name)
                rig.data.edit_bones.remove(bone)
                changed = True
    bpy.ops.object.mode_set(mode="OBJECT")
    return removed


# 3. Space --------------------------------------------------------------------------------------

def settle_space(rig, meshes):
    """Applies every transform, and puts the armature in its rest pose, which the bake then
    sees too."""
    select_only([rig] + meshes, active=rig)
    bpy.ops.object.transform_apply(location=True, rotation=True, scale=True)
    for bone in rig.pose.bones:
        bone.matrix_basis.identity()
    rig.data.pose_position = "REST"


# 4. Materials ----------------------------------------------------------------------------------

def surface_principled(material):
    """The Principled BSDF that is the material's surface, if that is what it is."""
    tree = material.node_tree
    output = next((n for n in tree.nodes if n.type == "OUTPUT_MATERIAL" and n.is_active_output), None)
    if output is None or not output.inputs["Surface"].is_linked:
        return None
    node = output.inputs["Surface"].links[0].from_node
    return node if node.type == "BSDF_PRINCIPLED" else None


def source_of(principled, name):
    """What feeds a Principled input: the linked socket, or its constant value as a colour."""
    socket = principled.inputs[name]
    if socket.is_linked:
        return socket.links[0].from_socket
    value = socket.default_value
    if isinstance(value, float):
        return (value, value, value, 1.0)
    return tuple(value)


def new_image(name, size, colour):
    image = bpy.data.images.new(name, size, size, alpha=True, float_buffer=False)
    image.colorspace_settings.name = "sRGB" if colour else "Non-Color"
    return image


def set_bake_target(material, image):
    """Makes `image` the node the next bake writes into, for this material."""
    nodes = material.node_tree.nodes
    node = nodes.get("solstice bake target") or nodes.new("ShaderNodeTexImage")
    node.name = "solstice bake target"
    node.image = image
    for n in nodes:
        n.select = False
    node.select = True
    nodes.active = node


def bake(mesh, kind, targets, size, pass_filter=None, margin=BAKE_MARGIN):
    """Bakes `kind` into each material's image in `targets`. A bake writes every material of the
    mesh, so those not in `targets` write into a scratch image, thrown away after."""
    scratch = new_image("solstice scratch", size, False)
    for slot in mesh.material_slots:
        set_bake_target(slot.material, targets.get(slot.material, scratch))
    select_only([mesh])
    settings = bpy.context.scene.render.bake
    settings.use_clear = True
    settings.margin = margin
    settings.margin_type = "EXTEND"
    if kind == "NORMAL":
        settings.normal_space = "TANGENT"
    if pass_filter:
        bpy.ops.object.bake(type=kind, pass_filter=pass_filter)
    else:
        bpy.ops.object.bake(type=kind)
    bpy.data.images.remove(scratch)


def bake_as_emission(mesh, material, source, image, size, margin=BAKE_MARGIN):
    """Bakes what `source` gives, a socket or a constant colour, into `image` exactly, by showing
    it as the material's emission for a moment."""
    tree = material.node_tree
    output = next(n for n in tree.nodes if n.type == "OUTPUT_MATERIAL" and n.is_active_output)
    surface = output.inputs["Surface"].links[0].from_socket if output.inputs["Surface"].is_linked else None
    emission = tree.nodes.new("ShaderNodeEmission")
    if isinstance(source, tuple):
        emission.inputs["Color"].default_value = source
    else:
        tree.links.new(source, emission.inputs["Color"])
    tree.links.new(emission.outputs["Emission"], output.inputs["Surface"])
    bake(mesh, "EMIT", {material: image}, size, margin=margin)
    tree.nodes.remove(emission)
    if surface:
        tree.links.new(surface, output.inputs["Surface"])


def pixels(image):
    data = numpy.empty(image.size[0] * image.size[1] * 4, dtype=numpy.float32)
    image.pixels.foreach_get(data)
    return data.reshape(-1, 4)


def cut_out_alpha(mesh, material, principled, size):
    """The material's alpha, per texel, if enough of what it covers is see-through to cut out;
    otherwise None. MakeHuman wires alpha into materials that are solid all over."""
    if principled is None or not principled.inputs["Alpha"].is_linked:
        if principled is not None and principled.inputs["Alpha"].default_value < 1.0:
            print(f"solstice_export: `{material.name}` is see-through all over, which isn't "
                  "supported: it comes out opaque")
        return None
    alpha = new_image(f"{material.name}_alpha", size, False)
    covered = new_image(f"{material.name}_covered", size, False)
    bake_as_emission(mesh, material, source_of(principled, "Alpha"), alpha, size)
    # Without a margin: only the texels the surface truly covers count.
    bake_as_emission(mesh, material, (1.0, 1.0, 1.0, 1.0), covered, size, margin=0)
    values = pixels(alpha)[:, 0]
    inside = pixels(covered)[:, 0] > 0.5
    bpy.data.images.remove(alpha)
    bpy.data.images.remove(covered)
    holes = numpy.count_nonzero(inside & (values < ALPHA_CUTOFF))
    # A few stray texels at seams aren't worth alpha testing a whole material for.
    if holes <= MIN_HOLE_SHARE * max(1, numpy.count_nonzero(inside)):
        return None
    return values


def unshare_materials(meshes):
    """Gives each mesh its own copy of any material another mesh uses too, before anything is
    baked: each mesh bakes into its own UV layout."""
    owner = {}
    for mesh in meshes:
        # One copy for the mesh, however many of its slots hold the material.
        copies = {}
        for slot in mesh.material_slots:
            material = slot.material
            if material is None:
                fail(f"`{mesh.name}` has an empty material slot")
            if owner.setdefault(material.name, mesh.name) != mesh.name:
                if material.name not in copies:
                    copies[material.name] = material.copy()
                    owner[copies[material.name].name] = mesh.name
                slot.material = copies[material.name]


def bake_materials(meshes, size):
    """Bakes each material of each mesh, and puts in its place a plain one the glTF writer
    writes exactly."""
    scene = bpy.context.scene
    scene.render.engine = "CYCLES"
    scene.cycles.device = "CPU"
    scene.cycles.samples = 4
    scene.cycles.seed = 0
    scene.cycles.use_denoising = False
    unshare_materials(meshes)
    for mesh in meshes:
        if not mesh.data.uv_layers:
            fail(f"`{mesh.name}` has no UVs to bake its materials into")
        # Cycles bakes into the active UV map; the glTF writer writes the render one.
        render = next(i for i, uv in enumerate(mesh.data.uv_layers) if uv.active_render)
        mesh.data.uv_layers.active_index = render
        # The same material in two slots is baked once.
        materials = list(dict.fromkeys(s.material for s in mesh.material_slots))
        principled = {m: surface_principled(m) for m in materials}
        base = {m: new_image(f"{m.name}_base", size, True) for m in materials}
        normal = {m: new_image(f"{m.name}_normal", size, False) for m in materials}
        rough = {m: new_image(f"{m.name}_roughness", size, False) for m in materials}
        metal = {}
        # Base colour as it is, untouched by alpha or metal, where it can be read off a
        # Principled surface; through Cycles' diffuse colour otherwise, as for node groups.
        plain = {m: base[m] for m in materials if principled[m] is None}
        if plain:
            bake(mesh, "DIFFUSE", plain, size, pass_filter={"COLOR"})
        for m in materials:
            if principled[m] is not None:
                bake_as_emission(mesh, m, source_of(principled[m], "Base Color"), base[m], size)
                if principled[m].inputs["Metallic"].is_linked:
                    metal[m] = new_image(f"{m.name}_metallic", size, False)
                    bake_as_emission(mesh, m, source_of(principled[m], "Metallic"), metal[m], size)
        bake(mesh, "NORMAL", normal, size)
        bake(mesh, "ROUGHNESS", rough, size)
        for material in materials:
            colour = pixels(base[material])
            alpha = cut_out_alpha(mesh, material, principled[material], size)
            colour[:, 3] = 1.0 if alpha is None else alpha
            base[material].pixels.foreach_set(colour.ravel())
            # glTF keeps roughness in green and metallic in blue.
            packed = numpy.zeros_like(colour)
            packed[:, 1] = pixels(rough[material])[:, 0]
            if material in metal:
                packed[:, 2] = pixels(metal[material])[:, 0]
                bpy.data.images.remove(metal[material])
            elif principled[material] is not None:
                packed[:, 2] = principled[material].inputs["Metallic"].default_value
            packed[:, 3] = 1.0
            rough[material].pixels.foreach_set(packed.ravel())
            rough[material].name = f"{material.name}_metallic_roughness"
            rebuild_material(material, base[material], normal[material], rough[material],
                             alpha is not None)


def rebuild_material(material, base, normal, metallic_roughness, cut_out):
    tree = material.node_tree
    tree.nodes.clear()
    output = tree.nodes.new("ShaderNodeOutputMaterial")
    principled = tree.nodes.new("ShaderNodeBsdfPrincipled")
    tree.links.new(principled.outputs["BSDF"], output.inputs["Surface"])
    colour = tree.nodes.new("ShaderNodeTexImage")
    colour.image = base
    tree.links.new(colour.outputs["Color"], principled.inputs["Base Color"])
    normal_image = tree.nodes.new("ShaderNodeTexImage")
    normal_image.image = normal
    normal_map = tree.nodes.new("ShaderNodeNormalMap")
    tree.links.new(normal_image.outputs["Color"], normal_map.inputs["Color"])
    tree.links.new(normal_map.outputs["Normal"], principled.inputs["Normal"])
    mr_image = tree.nodes.new("ShaderNodeTexImage")
    mr_image.image = metallic_roughness
    split = tree.nodes.new("ShaderNodeSeparateColor")
    tree.links.new(mr_image.outputs["Color"], split.inputs["Color"])
    tree.links.new(split.outputs["Green"], principled.inputs["Roughness"])
    tree.links.new(split.outputs["Blue"], principled.inputs["Metallic"])
    if cut_out:
        # The glTF writer reads `1 - (alpha < cutoff)` as alpha mode MASK with that cutoff.
        less = tree.nodes.new("ShaderNodeMath")
        less.operation = "LESS_THAN"
        less.inputs[1].default_value = ALPHA_CUTOFF
        tree.links.new(colour.outputs["Alpha"], less.inputs[0])
        invert = tree.nodes.new("ShaderNodeMath")
        invert.operation = "SUBTRACT"
        invert.inputs[0].default_value = 1.0
        tree.links.new(less.outputs["Value"], invert.inputs[1])
        tree.links.new(invert.outputs["Value"], principled.inputs["Alpha"])
    material.surface_render_method = "DITHERED"
    # Solid surfaces are drawn from the front only. Cut-out cards, such as hair, are seen from
    # both sides.
    material.use_backface_culling = not cut_out


# 6. Checks -------------------------------------------------------------------------------------

def check_in_blender(rig, meshes):
    bones = deforming_bones(rig)
    for mesh in meshes:
        extra = [g.name for g in mesh.vertex_groups if g.name not in bones]
        if extra:
            fail(f"`{mesh.name}` still has vertex groups that aren't deforming bones: {extra[:5]}")
        others = [m.name for m in mesh.modifiers if m.type != "ARMATURE"]
        if others:
            fail(f"`{mesh.name}` still has modifiers other than its armature: {others}")
        if mesh.data.shape_keys:
            fail(f"`{mesh.name}` still has shape keys")


def check_file(path, roles, checker):
    command = (shlex.split(checker) if checker else
               ["cargo", "run", "--release", "--quiet", "-p", "bake", "--"])
    command += ["--check-character", path]
    if roles:
        command += ["--roles", roles]
    return subprocess.run(command, cwd=REPO).returncode == 0


def export(output):
    options = dict(
        filepath=output,
        export_format="GLB",
        use_selection=True,
        export_yup=True,
        export_apply=False,
        export_texcoords=True,
        export_normals=True,
        export_tangents=False,
        export_materials="EXPORT",
        export_image_format="AUTO",
        export_skins=True,
        export_all_influences=False,
        export_influence_nb=4,
        export_def_bones=False,
        export_rest_position_armature=True,
        export_morph=False,
        export_animations=False,
        export_cameras=False,
        export_lights=False,
        export_extras=False,
        export_vertex_color="NONE",
        export_attributes=False,
        export_shared_accessors=False,
        export_draco_mesh_compression_enable=False,
    )
    # Pinned wherever this Blender has the option, so a change of its defaults can't change the
    # file.
    known = bpy.ops.export_scene.gltf.get_rna_type().properties.keys()
    missing = sorted(k for k in options if k not in known)
    if missing:
        print(f"solstice_export: this Blender's glTF writer lacks the options {missing}")
    bpy.ops.export_scene.gltf(**{k: v for k, v in options.items() if k in known})


def main():
    args = parse_args()
    output = os.path.abspath(args.output)
    stem = os.path.splitext(output)[0]
    partial, failed = f"{stem}.partial.glb", f"{stem}.failed.glb"
    roles = os.path.abspath(args.roles) if args.roles else None
    for old in (output, partial, failed):
        if os.path.exists(old):
            os.remove(old)
    rig, meshes = character_objects()
    if not args.as_is:
        for mesh in meshes:
            if mesh.data.users > 1:
                mesh.data = mesh.data.copy()
            apply_shape_keys(mesh)
            carry_out_masks(mesh)
            apply_other_modifiers(mesh)
            drop_non_bone_groups(mesh, deforming_bones(rig))
            tidy_weights(mesh)
        removed = remove_idle_leaf_bones(rig, meshes, read_role_bones(args.roles))
        if removed:
            print(f"solstice_export: removed bones that deform nothing: {removed}")
            # Their groups, emptied of tiny weights, go with them.
            for mesh in meshes:
                drop_non_bone_groups(mesh, deforming_bones(rig))
        settle_space(rig, meshes)
        bake_materials(meshes, args.texture_size)
        check_in_blender(rig, meshes)

    select_only([rig] + meshes, active=rig)
    os.makedirs(os.path.dirname(output), exist_ok=True)
    if args.as_is:
        # Blender's own defaults, as anyone exporting the character by hand would get.
        bpy.ops.export_scene.gltf(filepath=partial, export_format="GLB", use_selection=True)
    else:
        export(partial)
    if not check_file(partial, roles, args.checker):
        os.replace(partial, failed)
        fail(f"the file failed its checks: left as {failed}")
    os.replace(partial, output)
    print(f"solstice_export: wrote {output}, and it passed its checks")


try:
    main()
except ExportFailed as error:
    print(f"solstice_export: {error}", file=sys.stderr)
    sys.exit(1)
except Exception:
    traceback.print_exc()
    print("solstice_export: failed", file=sys.stderr)
    sys.exit(1)
