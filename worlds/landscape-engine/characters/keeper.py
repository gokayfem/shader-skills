"""The fort's keeper: the cove's playable character (docs/gameplay.md, Content).

Made with MakeHuman's Blender plugin, MPFB 2, from its CC0 system assets. Everything about the
character is here: change it by editing this file, and remake it.

    python tools/blender/make_character.py characters/keeper.py target/characters/keeper.glb

which runs this in Blender to make `keeper.blend`, then exports it with the export tool.
"""

import glob
import os
import sys

import bpy

# The body, as MakeHuman's macro sliders set it: each from 0 to 1.
BODY = {
    "gender": 1.0,          # male
    "age": 0.6,             # about 40
    "muscle": 0.6,
    "weight": 0.55,
    "proportions": 0.5,
    "height": 0.55,
    "cupsize": 0.5,
    "firmness": 0.5,
    "race": {"asian": 0.0, "caucasian": 0.85, "african": 0.15},
}
RIG = "game_engine"
SKIN = "skins/middleage_caucasian_male/*.mhmat"
# Each asset, by its kind and where it is among MPFB's system assets.
ASSETS = [
    ("Eyes", "eyes/low-poly/*.mhclo"),
    ("Eyebrows", "eyebrows/eyebrow001/*.mhclo"),
    ("Eyelashes", "eyelashes/eyelashes01/*.mhclo"),
    ("Teeth", "teeth/teeth_base/*.mhclo"),
    ("Tongue", "tongue/tongue01/*.mhclo"),
    ("Hair", "hair/short02/*.mhclo"),
    ("Clothes", "clothes/male_worksuit01/*.mhclo"),
    ("Clothes", "clothes/shoes02/*.mhclo"),
]


def main():
    from bl_ext.blender_org.mpfb.services.humanservice import HumanService
    from bl_ext.blender_org.mpfb.services.locationservice import LocationService

    output = sys.argv[sys.argv.index("--") + 1]
    data = LocationService.get_user_data()

    def asset(pattern):
        found = sorted(glob.glob(os.path.join(data, pattern)))
        if not found:
            sys.exit(f"keeper.py: no MPFB asset matches {pattern}: install the system assets")
        return found[0]

    for thing in list(bpy.data.objects):
        bpy.data.objects.remove(thing)
    basemesh = HumanService.create_human(macro_detail_dict=BODY)
    basemesh.name = "keeper"
    HumanService.add_builtin_rig(basemesh, RIG)
    # MPFB's own choices, given rather than left to its defaults, which may change.
    HumanService.set_character_skin(asset(SKIN), basemesh, skin_type="ENHANCED_SSS",
                                    material_instances=True)
    for kind, pattern in ASSETS:
        HumanService.add_mhclo_asset(asset(pattern), basemesh, asset_type=kind, subdiv_levels=0,
                                     material_type="MAKESKIN")
    bpy.ops.wm.save_as_mainfile(filepath=os.path.abspath(output))


main()
