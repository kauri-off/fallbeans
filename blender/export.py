"""
Exports every model of blender/fallguys_assets.blend to glTF binaries, headless:

    blender -b blender/fallguys_assets.blend --python blender/export.py -- <out dir>

Each collection named A_<model> becomes <out dir>/<model>.glb (A_bean -> bean.glb), with the
collection's objects, their names and hierarchy (the game animates bean parts by node name),
modifiers applied, +Y up, no cameras, lights or animations. Run through `bun run assets --export`,
which then validates and checks the files before they replace public/models.
"""

import os
import sys

import bpy

argv = sys.argv[sys.argv.index("--") + 1 :] if "--" in sys.argv else []
out_dir = os.path.abspath(argv[0] if argv else os.path.join(os.path.dirname(bpy.data.filepath), "..", ".build", "models"))
os.makedirs(out_dir, exist_ok=True)

if bpy.context.object and bpy.context.object.mode != "OBJECT":
    bpy.ops.object.mode_set(mode="OBJECT")

exported = []
for coll in bpy.data.collections:
    if not coll.name.startswith("A_"):
        continue
    name = coll.name[2:]
    bpy.ops.object.select_all(action="DESELECT")
    objs = [o for o in coll.all_objects if o.type in {"MESH", "EMPTY"}]
    for o in objs:
        o.hide_set(False)
        o.select_set(True)
    if not objs:
        print(f"[export] {name}: empty collection, skipped")
        continue
    bpy.context.view_layer.objects.active = objs[0]
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
