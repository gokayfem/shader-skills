"""Makes a character's animation clips, by hand-written motion, and exports them as a glTF.

    blender --background --factory-startup --python-exit-code 1 CHARACTER.blend \
        --python tools/blender/make_clips.py -- OUT.glb

CHARACTER.blend is the file `make_character.py` keeps beside its .glb, on MPFB 2's game-engine
rig. The output is an armature with one animation per clip and no mesh: idle, walk and run,
turning in place of what the Universal Animation Library would give, on the same rig the
character is bound to, so no retargeting is needed. Each is a loop, and stays where it is:
gameplay walks the character.

Every motion is written as a turn about an axis in the character's own space (it faces -Y and is
Z up here), so it doesn't depend on how the rig's bones are rolled.
"""

import math
import sys

import bpy
from mathutils import Quaternion, Vector

# One cycle of each clip, in frames at 30 Hz, and how much of it it swings.
CLIPS = {
    "Idle": {"frames": 90, "stride": 0.0},
    "Walk": {"frames": 34, "stride": 1.0},
    "Run": {"frames": 22, "stride": 1.6},
}

SIDE = Vector((1, 0, 0))
FRONT_AXIS = Vector((0, 1, 0))
UP = Vector((0, 0, 1))


def rig():
    armatures = [o for o in bpy.context.scene.objects if o.type == "ARMATURE"]
    if len(armatures) != 1:
        raise SystemExit("make_clips: a character has one armature")
    return armatures[0]


def set_rotation(bone, axis, angle):
    """Turns a bone about an axis given in the character's own space."""
    local = bone.bone.matrix_local.to_3x3().inverted() @ axis
    bone.rotation_mode = "QUATERNION"
    bone.rotation_quaternion = Quaternion(local.normalized(), angle)


def combine_rotation(bone, turns):
    """Sets a bone to several turns about axes in the character's own space, one after another."""
    frame = bone.bone.matrix_local.to_3x3()
    total = Quaternion()
    for axis, angle in turns:
        local = frame.inverted() @ axis
        total = total @ Quaternion(local.normalized(), angle)
    bone.rotation_mode = "QUATERNION"
    bone.rotation_quaternion = total


def pose_at(armature, name, phase):
    pose = armature.pose.bones
    # The rig's rest pose has the arms straight out: every clip lowers them first.
    stride = CLIPS[name]["stride"]
    swing = math.sin(phase * math.tau)
    legs = swing * 0.42 * stride
    knee = max(0.0, math.sin(phase * math.tau - 0.6)) * 0.95 * stride
    other_knee = max(0.0, math.sin(phase * math.tau - 0.6 + math.pi)) * 0.95 * stride
    breath = math.sin(phase * math.tau) * 0.02

    for side, sign, leg in (("l", 1.0, legs), ("r", -1.0, -legs)):
        knee_bend = knee if side == "l" else other_knee
        set_rotation(pose[f"thigh_{side}"], SIDE, -leg - knee_bend * 0.35)
        set_rotation(pose[f"calf_{side}"], SIDE, knee_bend)
        set_rotation(pose[f"foot_{side}"], SIDE, -knee_bend * 0.45 + leg * 0.2)

        down = sign * 1.26
        arm_swing = -leg * 0.75
        combine_rotation(pose[f"upperarm_{side}"], [(FRONT_AXIS, down), (SIDE, arm_swing)])
        set_rotation(pose[f"lowerarm_{side}"], SIDE, -0.15 - max(0.0, -arm_swing) * 0.6)

    bob = abs(swing) * 0.012 * stride
    sway = swing * 0.012 * stride
    pelvis = pose["pelvis"]
    pelvis.location = pelvis.bone.matrix_local.to_3x3().inverted() @ Vector((sway, 0, -bob))
    set_rotation(pelvis, UP, swing * 0.06 * stride)
    set_rotation(pose["spine_02"], UP, -swing * 0.06 * stride)
    set_rotation(pose["spine_03"], SIDE, breath)
    if stride == 0.0:
        set_rotation(pose["head"], UP, math.sin(phase * math.tau) * 0.05)
    else:
        set_rotation(pose["head"], SIDE, -0.02)


def make_action(armature, name):
    frames = CLIPS[name]["frames"]
    action = bpy.data.actions.new(name)
    action.use_fake_user = True
    armature.animation_data_create()
    armature.animation_data.action = action
    bones = [
        "pelvis",
        "spine_02",
        "spine_03",
        "head",
        "thigh_l",
        "thigh_r",
        "calf_l",
        "calf_r",
        "foot_l",
        "foot_r",
        "upperarm_l",
        "upperarm_r",
        "lowerarm_l",
        "lowerarm_r",
    ]
    # One key per frame, and the first again at the end, so the loop closes.
    for frame in range(frames + 1):
        bpy.context.scene.frame_set(frame + 1)
        pose_at(armature, name, frame / frames)
        for bone in bones:
            pose_bone = armature.pose.bones[bone]
            pose_bone.keyframe_insert("rotation_quaternion", frame=frame + 1)
            if bone == "pelvis":
                pose_bone.keyframe_insert("location", frame=frame + 1)
    return action


def main():
    out = sys.argv[sys.argv.index("--") + 1]
    armature = rig()
    bpy.context.scene.render.fps = 30
    bpy.context.scene.frame_start = 1
    for name in CLIPS:
        action = make_action(armature, name)
        track = armature.animation_data.nla_tracks.new()
        track.name = name
        track.strips.new(name, 1, action)
        armature.animation_data.action = None
    for obj in bpy.context.scene.objects:
        obj.select_set(obj is armature)
    bpy.context.view_layer.objects.active = armature
    bpy.ops.export_scene.gltf(
        filepath=out,
        export_format="GLB",
        use_selection=True,
        export_yup=True,
        export_apply=False,
        export_animations=True,
        export_animation_mode="NLA_TRACKS",
        export_force_sampling=True,
        export_frame_step=1,
        export_def_bones=False,
        export_rest_position_armature=True,
        export_skins=False,
        export_morph=False,
        export_materials="NONE",
        export_cameras=False,
        export_lights=False,
    )
    print(f"make_clips: wrote {out}")


if __name__ == "__main__":
    main()
