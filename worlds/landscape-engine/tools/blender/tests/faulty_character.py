"""A small rigged character with every fault the export tool fixes, built on purpose.

    blender --background --factory-startup --python-exit-code 1 \
        --python tools/blender/tests/faulty_character.py -- OUT.blend

The sheet, a standing grid of 9 by 9 vertices:
- a mask modifier hiding its column of vertices nearest x = 0.5;
- a vertex group that isn't a bone, and one for a bone that doesn't deform;
- a shape key, mixed in at a half;
- at its top, five bone weights a vertex, two of them tiny;
- a blended material, `cloth`, solid over the left of its UVs and see-through on the right,
  in two slots of the sheet.

The patch, a small grid beside it:
- `cloth` again, shared with the sheet;
- `buckle`, metal, its metallic fed through a node, so only baking it gets it right;
- `solid`, its alpha wired in but 1 everywhere, as MakeHuman wires it into most materials.

And the rig: a leaf bone that deforms nothing, one the role file names that deforms nothing
either, and two whose only weights are tiny.

Its faults are in Blender's terms, not MakeHuman's, so it needs no plugin.
"""

import sys

import bpy

out = sys.argv[sys.argv.index("--") + 1]
for thing in list(bpy.data.objects):
    bpy.data.objects.remove(thing)

# A chain of bones up the Z axis, with short leaves off the spine.
armature = bpy.data.armatures.new("rig")
rig = bpy.data.objects.new("rig", armature)
bpy.context.scene.collection.objects.link(rig)
bpy.context.view_layer.objects.active = rig
bpy.ops.object.mode_set(mode="EDIT")
bones = {}
for name, head, tail, parent in [
    ("Root", (0, 0, 0), (0, 0, 0.1), None),
    ("pelvis", (0, 0, 0.1), (0, 0, 0.5), "Root"),
    ("spine", (0, 0, 0.5), (0, 0, 1.0), "pelvis"),
    ("a", (0, 0, 1.0), (0.1, 0, 1.0), "spine"),
    ("b", (0, 0, 1.0), (-0.1, 0, 1.0), "spine"),
    ("c", (0, 0, 1.0), (0, 0.1, 1.0), "spine"),
    ("control", (0, 0, 1.0), (0, 0.2, 1.0), "spine"),
    ("idle_tip", (0, 0, 1.0), (0, 0, 1.2), "spine"),
    ("named_tip", (0, 0, 1.0), (0, -0.1, 1.0), "spine"),
]:
    bone = armature.edit_bones.new(name)
    bone.head, bone.tail = head, tail
    if parent:
        bone.parent = bones[parent]
    bones[name] = bone
bones["control"].use_deform = False
bpy.ops.object.mode_set(mode="OBJECT")


def material(name, colour, roughness=0.5):
    m = bpy.data.materials.new(name)
    m.surface_render_method = "BLENDED"
    principled = next(n for n in m.node_tree.nodes if n.type == "BSDF_PRINCIPLED")
    principled.inputs["Base Color"].default_value = colour
    principled.inputs["Roughness"].default_value = roughness
    return m, principled


def skinned_grid(name, subdivisions, size, location):
    bpy.ops.mesh.primitive_grid_add(x_subdivisions=subdivisions, y_subdivisions=subdivisions, size=size)
    grid = bpy.context.object
    grid.name = name
    grid.rotation_euler = (1.5707963, 0, 0)
    grid.location = location
    bpy.ops.object.transform_apply(location=True, rotation=True, scale=True)
    grid.parent = rig
    grid.modifiers.new("Armature", "ARMATURE").object = rig
    return grid


# The sheet: 1 m wide and tall.
sheet = skinned_grid("sheet", 8, 1.0, (0, 0, 0.5))
groups = {n: sheet.vertex_groups.new(name=n) for n in ["pelvis", "spine", "a", "b", "c", "control"]}
shown = sheet.vertex_groups.new(name="shown")         # what the mask keeps
extra = sheet.vertex_groups.new(name="helper-stuff")  # not a bone
for vert in sheet.data.vertices:
    z = vert.co.z
    groups["pelvis"].add([vert.index], 1.0 - z, "REPLACE")
    groups["spine"].add([vert.index], z, "REPLACE")
    if z > 0.7:
        groups["a"].add([vert.index], 0.3, "REPLACE")
        groups["b"].add([vert.index], 0.004, "REPLACE")
        groups["c"].add([vert.index], 0.003, "REPLACE")
        groups["control"].add([vert.index], 0.5, "REPLACE")  # a bone that doesn't deform
        extra.add([vert.index], 1.0, "REPLACE")
    if vert.co.x < 0.4:
        shown.add([vert.index], 1.0, "REPLACE")
mask = sheet.modifiers.new("Hide strip", "MASK")
mask.vertex_group = "shown"
sheet.shape_key_add(name="Basis")
bulge = sheet.shape_key_add(name="bulge")
for point in bulge.data:
    point.co.y += 0.1
bulge.value = 0.5

# `cloth`: solid where its U is below 0.7, see-through beyond.
cloth, principled = material("cloth", (0.6, 0.3, 0.2, 1.0))
tree = cloth.node_tree
coords = tree.nodes.new("ShaderNodeTexCoord")
split = tree.nodes.new("ShaderNodeSeparateXYZ")
less = tree.nodes.new("ShaderNodeMath")
less.operation = "LESS_THAN"
less.inputs[1].default_value = 0.7
tree.links.new(coords.outputs["UV"], split.inputs["Vector"])
tree.links.new(split.outputs["X"], less.inputs[0])
tree.links.new(less.outputs["Value"], principled.inputs["Alpha"])
# In two slots: the bottom row of faces uses the second.
sheet.data.materials.append(cloth)
sheet.data.materials.append(cloth)
for face in sheet.data.polygons:
    if face.center.z < 0.2:
        face.material_index = 1

# The patch: 20 cm across, beside the sheet, all on the spine.
patch = skinned_grid("patch", 2, 0.2, (1.0, 0, 0.5))
patch.vertex_groups.new(name="spine").add([v.index for v in patch.data.vertices], 1.0, "REPLACE")
buckle, principled = material("buckle", (0.8, 0.7, 0.2, 1.0), roughness=0.3)
metal = buckle.node_tree.nodes.new("ShaderNodeValue")
metal.outputs[0].default_value = 1.0
buckle.node_tree.links.new(metal.outputs[0], principled.inputs["Metallic"])
solid, principled = material("solid", (0.2, 0.4, 0.6, 1.0))
whole = solid.node_tree.nodes.new("ShaderNodeValue")
whole.outputs[0].default_value = 1.0
solid.node_tree.links.new(whole.outputs[0], principled.inputs["Alpha"])
for m in (cloth, buckle, solid):
    patch.data.materials.append(m)
for index, face in enumerate(patch.data.polygons):
    face.material_index = index % 3

bpy.ops.wm.save_as_mainfile(filepath=out)
