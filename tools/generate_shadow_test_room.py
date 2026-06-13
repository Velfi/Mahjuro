"""Generate the Shadow & AO lab GLB comparison fixture.

Run with Blender:

    blender -b --python tools/generate_shadow_test_room.py
"""

from __future__ import annotations

import math
from pathlib import Path

import bpy
from mathutils import Vector


ROOT = Path(__file__).resolve().parents[1]
BLEND_OUT = ROOT / "assets/3d/source/shadow_test_room.blend"
GLB_OUT = ROOT / "assets/3d/shadow_test_room.glb"


def reset_scene() -> None:
    bpy.ops.object.select_all(action="SELECT")
    bpy.ops.object.delete()


def material(name: str, color: tuple[float, float, float, float], roughness: float = 0.9) -> bpy.types.Material:
    mat = bpy.data.materials.new(name)
    mat.use_nodes = True
    bsdf = mat.node_tree.nodes.get("Principled BSDF")
    if bsdf is not None:
        bsdf.inputs["Base Color"].default_value = color
        bsdf.inputs["Roughness"].default_value = roughness
        bsdf.inputs["Metallic"].default_value = 0.0
    return mat


def emissive_material(name: str, color: tuple[float, float, float, float], strength: float) -> bpy.types.Material:
    mat = bpy.data.materials.new(name)
    mat.use_nodes = True
    bsdf = mat.node_tree.nodes.get("Principled BSDF")
    if bsdf is not None:
        bsdf.inputs["Base Color"].default_value = color
        bsdf.inputs["Emission Color"].default_value = color
        bsdf.inputs["Emission Strength"].default_value = strength
        bsdf.inputs["Roughness"].default_value = 0.35
    return mat


def cube(name: str, loc: tuple[float, float, float], scale: tuple[float, float, float], mat: bpy.types.Material) -> bpy.types.Object:
    bpy.ops.mesh.primitive_cube_add(size=1.0, location=loc)
    obj = bpy.context.object
    obj.name = name
    obj.data.name = f"{name}_mesh"
    obj.dimensions = scale
    bpy.ops.object.transform_apply(location=False, rotation=False, scale=True)
    obj.data.materials.append(mat)
    return obj


def look_at(obj: bpy.types.Object, target: tuple[float, float, float]) -> None:
    direction = Vector(target) - obj.location
    obj.rotation_euler = direction.to_track_quat("-Z", "Y").to_euler()


def build_scene() -> None:
    reset_scene()

    wall_mat = material("warm_clay_walls", (0.76, 0.68, 0.56, 1.0))
    roof_mat = material("matte_charcoal_occluders", (0.23, 0.24, 0.25, 1.0))
    floor_mat = material("matte_test_floor", (0.62, 0.61, 0.56, 1.0))
    target_mat = material("white_shadow_receivers", (0.96, 0.94, 0.86, 1.0))
    blocker_mat = material("blue_shadow_caster_blocks", (0.18, 0.36, 0.86, 1.0))
    contact_mat = material("green_contact_samples", (0.30, 0.58, 0.42, 1.0))
    bulb_mat = emissive_material("visible_warm_light_bulb", (1.0, 0.77, 0.35, 1.0), 6.0)

    # Open gallery: the camera sees the floor receivers, the back-wall receiver,
    # and the underside of the roof occluder without the fixture filling the frame.
    cube("floor_receiver_stage", (0.0, -0.45, -0.06), (7.2, 5.6, 0.12), floor_mat)
    cube("back_wall_receiver", (0.0, 2.1, 1.12), (6.8, 0.22, 2.24), wall_mat)
    cube("left_wall_reference", (-3.4, -0.20, 0.78), (0.22, 4.6, 1.56), wall_mat)
    cube("right_wall_reference", (3.4, -0.20, 0.78), (0.22, 4.6, 1.56), wall_mat)
    cube("back_wall_shadow_receiver_panel", (0.70, 1.97, 1.08), (2.65, 0.05, 1.42), target_mat)

    # Roof occlusion comparison: left target is under a thick blocker, right target
    # is open to the same lamp. Both stay visible from the embedded camera.
    cube("thick_light_blocking_roof", (-1.55, -0.10, 2.10), (2.45, 2.35, 0.26), roof_mat)
    cube("shadowed_receiver_under_roof", (-1.55, -0.78, 0.04), (1.35, 1.05, 0.08), target_mat)
    cube("lit_receiver_open_apron", (1.55, -1.95, 0.04), (1.35, 1.05, 0.08), target_mat)

    # Floor and wall casters: thin blockers should project readable bands onto the
    # bright receivers, making bias/resolution changes easier to judge.
    cube("floor_shadow_fin", (1.55, -0.62, 0.42), (0.16, 0.74, 0.84), blocker_mat)
    cube("wall_shadow_bar", (0.85, 1.30, 1.22), (1.90, 0.18, 0.22), blocker_mat)
    cube("wall_shadow_post", (-0.45, 1.18, 0.90), (0.20, 0.20, 1.45), blocker_mat)

    # Contact/gap samples: a grounded block, a raised block, and a corner step give
    # the AO/contact-shadow pass something legible to compare.
    cube("grounded_contact_block", (2.05, 0.42, 0.20), (0.58, 0.58, 0.40), contact_mat)
    cube("raised_gap_block", (2.78, 0.42, 0.38), (0.58, 0.58, 0.40), contact_mat)
    cube("corner_contact_step_low", (-1.05, 1.58, 0.12), (0.90, 0.42, 0.24), contact_mat)
    cube("corner_contact_step_high", (-0.25, 1.58, 0.30), (0.70, 0.42, 0.60), contact_mat)

    bpy.ops.mesh.primitive_uv_sphere_add(segments=24, ring_count=12, radius=0.12, location=(0.0, -0.55, 3.50))
    bulb = bpy.context.object
    bulb.name = "visible_light_position"
    bulb.data.name = "visible_light_position_mesh"
    bulb.data.materials.append(bulb_mat)

    bpy.ops.object.light_add(type="POINT", location=(0.0, -0.55, 3.50))
    light = bpy.context.object
    light.name = "light_shadow_ao_comparison"
    light.data.name = "light_shadow_ao_comparison_data"
    light.data.color = (1.0, 0.86, 0.62)
    light.data.energy = 760.0
    light.data.shadow_soft_size = 0.12

    bpy.ops.object.camera_add(location=(5.8, -8.6, 3.65))
    cam = bpy.context.object
    cam.name = "default"
    cam.data.name = "default_camera"
    cam.data.angle = math.radians(51.0)
    cam.data.clip_end = 1000.0
    look_at(cam, (0.0, -0.35, 0.90))
    bpy.context.scene.camera = cam

    bpy.context.scene.render.resolution_x = 1920
    bpy.context.scene.render.resolution_y = 1080
    bpy.context.scene.world.color = (0.015, 0.015, 0.018)


def export() -> None:
    BLEND_OUT.parent.mkdir(parents=True, exist_ok=True)
    GLB_OUT.parent.mkdir(parents=True, exist_ok=True)
    bpy.ops.wm.save_as_mainfile(filepath=str(BLEND_OUT))
    bpy.ops.export_scene.gltf(
        filepath=str(GLB_OUT),
        export_format="GLB",
        export_cameras=True,
        export_lights=True,
        export_materials="EXPORT",
        export_apply=False,
    )


if __name__ == "__main__":
    build_scene()
    export()
