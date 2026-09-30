"""
Exports every model of blender/fallguys_assets.blend to glTF binaries, headless:

    blender -b blender/fallguys_assets.blend --python blender/export.py -- <out dir>

Each collection named A_<model> becomes <out dir>/<model>.glb (A_bean -> bean.glb), with the
collection's objects, their names and hierarchy (the game animates bean parts by node name),
modifiers applied, +Y up, no cameras, lights or animations. Run through `bun run assets --export`,
which then validates and checks the files before they replace public/models.

Ambient occlusion is baked for every model (Cycles, the model alone: what its own parts hide from
the sky) into <out dir>/<model>_ao.png, over a UV layout made for it (smart projection of the whole
model into one atlas, the only UV map exported: TEXCOORD_0). `bun run assets` compresses it and
wires it as the occlusion texture of all the model's materials. The .blend is never saved: the
modifiers are applied and the UVs replaced on this session's copy only.
"""

import math
import os
import sys

import bpy

argv = sys.argv[sys.argv.index("--") + 1 :] if "--" in sys.argv else []
out_dir = os.path.abspath(argv[0] if argv else os.path.join(os.path.dirname(bpy.data.filepath), "..", ".build", "models"))
os.makedirs(out_dir, exist_ok=True)

# AO: how far occluders count (Blender units, at most AO_REACH of the model's size), rays per texel,
# texels per unit of sqrt(surface area).
AO_DISTANCE = 0.5
AO_REACH = 0.2
AO_SAMPLES = 512
AO_DENSITY = 180
AO_MIN, AO_MAX = 128, 512
AO_MARGIN = 4

if bpy.context.object and bpy.context.object.mode != "OBJECT":
    bpy.ops.object.mode_set(mode="OBJECT")

scene = bpy.context.scene
view_layer = bpy.context.view_layer
scene.render.engine = "CYCLES"
scene.cycles.samples = AO_SAMPLES
scene.cycles.use_denoising = False
# A GPU if there is one (much faster bakes); the CPU otherwise.
try:
    prefs = bpy.context.preferences.addons["cycles"].preferences
    for kind in ("OPTIX", "CUDA", "HIP", "ONEAPI", "METAL"):
        try:
            prefs.compute_device_type = kind
        except TypeError:
            continue
        prefs.get_devices()
        gpus = [d for d in prefs.devices if d.type == kind]
        if gpus:
            for d in prefs.devices:
                d.use = d.type == kind
            scene.cycles.device = "GPU"
            print(f"[export] baking on {kind}: {', '.join(d.name for d in gpus)}")
            break
    else:
        scene.cycles.device = "CPU"
except Exception as e:  # noqa: BLE001
    scene.cycles.device = "CPU"
    print(f"[export] baking on the CPU ({e})")
if scene.world is None:
    scene.world = bpy.data.worlds.new("World")

from mathutils import Vector  # noqa: E402


def select_only(objs):
    bpy.ops.object.select_all(action="DESELECT")
    for o in objs:
        o.hide_set(False)
        o.select_set(True)
    view_layer.objects.active = objs[0]


def apply_modifiers(meshes):
    """Modifiers (bevels, geometry nodes, solidify) applied: the UVs and the bake see the exported shape."""
    dg = bpy.context.evaluated_depsgraph_get()
    for o in meshes:
        me = bpy.data.meshes.new_from_object(o.evaluated_get(dg), preserve_all_data_layers=True, depsgraph=dg)
        o.modifiers.clear()
        o.data = me


def world_area(meshes):
    area = 0.0
    for o in meshes:
        m = o.matrix_world
        for p in o.data.polygons:
            v = [m @ o.data.vertices[i].co for i in p.vertices]
            for k in range(1, len(v) - 1):
                area += (v[k] - v[0]).cross(v[k + 1] - v[0]).length / 2
    return area


def unwrap(meshes, res):
    """One UV map, "AO", for the whole model: a smart projection packed into a single atlas."""
    for o in meshes:
        for uv in list(o.data.uv_layers):
            o.data.uv_layers.remove(uv)
        o.data.uv_layers.new(name="AO")
    select_only(meshes)
    bpy.ops.object.mode_set(mode="EDIT")
    bpy.ops.mesh.select_all(action="SELECT")
    bpy.ops.uv.smart_project(angle_limit=math.radians(66), island_margin=(AO_MARGIN * 1.5) / res, scale_to_bounds=False)
    bpy.ops.object.mode_set(mode="OBJECT")


def model_size(meshes):
    lo = Vector((math.inf,) * 3)
    hi = Vector((-math.inf,) * 3)
    for o in meshes:
        for v in o.data.vertices:
            p = o.matrix_world @ v.co
            lo = Vector(map(min, lo, p))
            hi = Vector(map(max, hi, p))
    return max(hi - lo)


def bake_ao(name, meshes, res):
    scene.world.light_settings.distance = min(AO_DISTANCE, AO_REACH * model_size(meshes))
    img = bpy.data.images.new(f"AO_{name}", res, res, alpha=False)
    img.colorspace_settings.name = "Non-Color"
    # Only this model casts occlusion.
    hidden = {}
    for o in scene.objects:
        hidden[o] = o.hide_render
        o.hide_render = o not in meshes
    # Every material of the model bakes into the image (Cycles bakes into the active image node).
    temp = []
    for o in meshes:
        if not o.material_slots:
            m = bpy.data.materials.new(f"AOBake_{o.name}")
            o.data.materials.append(m)
        for slot in o.material_slots:
            m = slot.material
            if m is None:
                m = slot.material = bpy.data.materials.new(f"AOBake_{o.name}")
            m.use_nodes = True
            node = m.node_tree.nodes.new("ShaderNodeTexImage")
            node.image = img
            m.node_tree.nodes.active = node
            temp.append((m, node))
    select_only(meshes)
    bpy.ops.object.bake(type="AO", margin=AO_MARGIN, margin_type="EXTEND", use_clear=True, target="IMAGE_TEXTURES")
    for m, node in temp:
        m.node_tree.nodes.remove(node)
    for o, h in hidden.items():
        o.hide_render = h
    path = os.path.join(out_dir, f"{name}_ao.png")
    img.filepath_raw = path
    img.file_format = "PNG"
    img.save()
    bpy.data.images.remove(img)
    return path


exported = []
for coll in bpy.data.collections:
    if not coll.name.startswith("A_"):
        continue
    name = coll.name[2:]
    objs = [o for o in coll.all_objects if o.type in {"MESH", "EMPTY"}]
    if not objs:
        print(f"[export] {name}: empty collection, skipped")
        continue
    meshes = [o for o in objs if o.type == "MESH"]
    if meshes:
        apply_modifiers(meshes)
        area = world_area(meshes)
        res = 2 ** round(math.log2(max(1.0, math.sqrt(area) * AO_DENSITY)))
        res = max(AO_MIN, min(AO_MAX, res))
        unwrap(meshes, res)
        ao = bake_ao(name, meshes, res)
        print(f"[export] {name}: AO {res}x{res}, reach {scene.world.light_settings.distance:.2f} -> {ao}")
    select_only(objs)
    path = os.path.join(out_dir, f"{name}.glb")
    bpy.ops.export_scene.gltf(
        filepath=path,
        export_format="GLB",
        use_selection=True,
        use_active_scene=True,
        export_apply=True,
        export_yup=True,
        export_animations=False,
        export_cameras=False,
        export_lights=False,
        export_extras=False,
        export_texcoords=True,
        export_normals=True,
        export_materials="EXPORT",
    )
    exported.append(name)
    print(f"[export] {name}: {len(objs)} objects -> {path}")

print(f"[export] done: {len(exported)} models in {out_dir}")
