"""
Exports every model of blender/models.blend to glTF binaries, headless:

    blender -b blender/models.blend --python blender/export.py -- <out dir>

Each collection named A_<model> becomes <out dir>/<model>.glb (A_bean -> bean.glb), with the
collection's objects, their names and hierarchy (the game animates bean parts by node name),
modifiers applied, +Y up, no cameras, lights or animations. Run through `cargo xtask assets --export`,
which then loads every file in the client.

The scenery models are rebuilt with more shape first (`detail.py`, the same every run).

Ambient occlusion is baked for every model (Cycles, the model alone: what its own parts hide from
the sky) over a UV layout made for it (smart projection of the whole model into one atlas, the only
UV map exported: TEXCOORD_0) and embedded as a grey PNG, the occlusion texture of all the model's
materials. Parts the game moves on their own (`APART`: a bean's arms, hands and legs, a fan's blades)
are baked each by itself, the rest of the model without them: no shadow of an arm stays painted on the
body when the arm swings away. The .blend is never saved: the modifiers are applied and the UVs replaced
on this session's copy only.
"""

import json
import math
import os
import struct
import sys
import zlib

import bpy

sys.path.insert(0, os.path.dirname(os.path.abspath(bpy.data.filepath)))
sys.dont_write_bytecode = True  # (no __pycache__ in blender/)
import detail  # noqa: E402

detail.apply()

argv = sys.argv[sys.argv.index("--") + 1 :] if "--" in sys.argv else []
out_dir = os.path.abspath(argv[0] if argv else os.path.join(os.path.dirname(bpy.data.filepath), "..", "assets", "models"))
os.makedirs(out_dir, exist_ok=True)

# AO: how far occluders count (Blender units, at most AO_REACH of the model's size), rays per texel,
# texels per unit of sqrt(surface area).
AO_DISTANCE = 0.5
AO_REACH = 0.2
AO_SAMPLES = 512
AO_DENSITY = 180
AO_MIN, AO_MAX = 128, 512
AO_MARGIN = 8

# Parts of a model that move apart from the rest (by object name), each list baked by itself: what they
# hide of the rest, and the rest of them, changes as they move.
APART = {
    "bean": [["ArmL"], ["HandL"], ["ArmR"], ["HandR"], ["LegL", "ShoeL"], ["LegR", "ShoeR"]],
    "fan": [["FanHub", "FanBlade0", "FanBlade1", "FanBlade2", "FanBlade3"]],
    "flag": [["Pennant"]],
}

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


def bake_groups(name, meshes):
    """The model's meshes in the sets baked apart (`APART`): the rest first, then each moving part."""
    by_name = {o.name: o for o in meshes}
    apart = [[by_name[n] for n in group if n in by_name] for group in APART.get(name, [])]
    apart = [g for g in apart if g]
    moved = {o for g in apart for o in g}
    rest = [o for o in meshes if o not in moved]
    return ([rest] if rest else []) + apart


def bake_ao(name, meshes, res):
    import numpy as np

    scene.world.light_settings.distance = min(AO_DISTANCE, AO_REACH * model_size(meshes))
    # Each set bakes into an image of its own, and only the texels inside its UV islands are kept (less a
    # texel round their edges, where the bake may or may not have reached). The texels no set covers (-1)
    # get the margin afterwards.
    ao = np.full((res, res), -1.0, dtype=np.float32)
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
            m.node_tree.nodes.active = node
            temp.append((m, node))
    hidden = {o: o.hide_render for o in scene.objects}
    px = np.empty(res * res * 4, dtype=np.float32)
    for i, group in enumerate(bake_groups(name, meshes)):
        img = bpy.data.images.new(f"AO_{name}_{i}", res, res, alpha=False, float_buffer=True)
        img.colorspace_settings.name = "Non-Color"
        for _, node in temp:
            node.image = img
        # Only this set casts occlusion.
        for o in scene.objects:
            o.hide_render = o not in group
        select_only(group)
        bpy.ops.object.bake(type="AO", margin=0, use_clear=True, target="IMAGE_TEXTURES")
        img.pixels.foreach_get(px)
        bpy.data.images.remove(img)
        covered = uv_cover(group, res)
        ao[covered] = px[0::4].reshape(res, res)[covered]
    for m, node in temp:
        m.node_tree.nodes.remove(node)
    for o, h in hidden.items():
        o.hide_render = h
    return gray_png(pad_islands(ao, AO_MARGIN))


def uv_cover(meshes, res):
    """The texels (rows bottom to top) whose centres lie in the meshes' UV triangles, less those on the
    edge of what they cover."""
    import numpy as np

    mask = np.zeros((res, res), dtype=bool)
    for o in meshes:
        me = o.data
        me.calc_loop_triangles()
        n = len(me.loop_triangles)
        if n == 0:
            continue
        loops = np.empty(n * 3, dtype=np.int32)
        me.loop_triangles.foreach_get("loops", loops)
        uv = np.empty(len(me.uv_layers.active.data) * 2, dtype=np.float32)
        me.uv_layers.active.data.foreach_get("uv", uv)
        tris = uv.reshape(-1, 2)[loops].reshape(n, 3, 2) * res
        for t in tris:
            x0, y0 = np.clip(np.floor(t.min(0)).astype(int), 0, res)
            x1, y1 = np.clip(np.ceil(t.max(0)).astype(int), 0, res)
            area = (t[1][0] - t[0][0]) * (t[2][1] - t[0][1]) - (t[1][1] - t[0][1]) * (t[2][0] - t[0][0])
            if x1 <= x0 or y1 <= y0 or abs(area) < 1e-6:
                continue
            xs, ys = np.meshgrid(np.arange(x0, x1) + 0.5, np.arange(y0, y1) + 0.5)
            e = [(b[0] - a[0]) * (ys - a[1]) - (b[1] - a[1]) * (xs - a[0]) for a, b in ((t[0], t[1]), (t[1], t[2]), (t[2], t[0]))]
            inside = ((e[0] >= 0) & (e[1] >= 0) & (e[2] >= 0)) | ((e[0] <= 0) & (e[1] <= 0) & (e[2] <= 0))
            mask[y0:y1, x0:x1] |= inside
    p = np.pad(mask, 1)
    return mask & p[:-2, 1:-1] & p[2:, 1:-1] & p[1:-1, :-2] & p[1:-1, 2:]


def pad_islands(ao, margin):
    """Texels of no island (< 0) take the mean of their baked neighbours, `margin` rings out (as Blender's
    EXTEND margin); the rest white."""
    import numpy as np

    ao = ao.copy()
    for _ in range(margin):
        done = ao >= 0.0
        padded = np.pad(np.where(done, ao, 0.0), 1)
        count = np.pad(done.astype(np.float32), 1)
        h, w = ao.shape
        total = np.zeros_like(ao)
        n = np.zeros_like(ao)
        for dy in (-1, 0, 1):
            for dx in (-1, 0, 1):
                if dy or dx:
                    total += padded[1 + dy : 1 + dy + h, 1 + dx : 1 + dx + w]
                    n += count[1 + dy : 1 + dy + h, 1 + dx : 1 + dx + w]
        grow = ~done & (n > 0)
        ao[grow] = total[grow] / n[grow]
    ao[ao < 0.0] = 1.0
    return ao


def gray_png(ao):
    """An AO map (rows bottom to top, as Blender keeps them) as an 8-bit grey PNG (rows top to bottom)."""
    import numpy as np

    h, w = ao.shape
    g = (np.clip(ao, 0.0, 1.0) * 255 + 0.5).astype(np.uint8)[::-1]
    raw = b"".join(b"\x00" + row.tobytes() for row in g)

    def chunk(kind, data):
        return struct.pack(">I", len(data)) + kind + data + struct.pack(">I", zlib.crc32(kind + data) & 0xFFFFFFFF)

    ihdr = struct.pack(">IIBBBBB", w, h, 8, 0, 0, 0, 0)
    return b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", ihdr) + chunk(b"IDAT", zlib.compress(raw, 9)) + chunk(b"IEND", b"")


def attach_ao(path, png):
    """The AO PNG as the only texture of the .glb, the occlusion texture (UV set 0) of every material."""
    with open(path, "rb") as f:
        data = f.read()
    json_len = struct.unpack_from("<I", data, 12)[0]
    doc = json.loads(data[20 : 20 + json_len])
    at = 20 + json_len
    bin_len = struct.unpack_from("<I", data, at)[0] if at < len(data) else 0
    body = bytearray(data[at + 8 : at + 8 + bin_len])
    body += b"\x00" * (-len(body) % 4)
    views = doc.setdefault("bufferViews", [])
    views.append({"buffer": 0, "byteOffset": len(body), "byteLength": len(png)})
    body += png + b"\x00" * (-len(png) % 4)
    doc.setdefault("buffers", [{}])[0]["byteLength"] = len(body)
    doc["images"] = [{"name": "AO", "mimeType": "image/png", "bufferView": len(views) - 1}]
    doc["textures"] = [{"source": 0}]
    doc.pop("samplers", None)
    for m in doc.get("materials", []):
        pbr = m.get("pbrMetallicRoughness", {})
        for k in [k for k in pbr if k.endswith("Texture")]:
            del pbr[k]
        for k in [k for k in m if k.endswith("Texture")]:
            del m[k]
        m["occlusionTexture"] = {"index": 0}
    text = json.dumps(doc, separators=(",", ":")).encode()
    text += b" " * (-len(text) % 4)
    out = struct.pack("<III", 0x46546C67, 2, 12 + 8 + len(text) + 8 + len(body))
    out += struct.pack("<II", len(text), 0x4E4F534A) + text
    out += struct.pack("<II", len(body), 0x004E4942) + bytes(body)
    with open(path, "wb") as f:
        f.write(out)


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
    ao = None
    if meshes:
        apply_modifiers(meshes)
        area = world_area(meshes)
        res = 2 ** round(math.log2(max(1.0, math.sqrt(area) * AO_DENSITY)))
        res = max(AO_MIN, min(AO_MAX, res))
        unwrap(meshes, res)
        ao = bake_ao(name, meshes, res)
        print(f"[export] {name}: AO {res}x{res}, reach {scene.world.light_settings.distance:.2f}, {len(ao) // 1024} KB")
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
    if ao is not None:
        attach_ao(path, ao)
    exported.append(name)
    print(f"[export] {name}: {len(objs)} objects -> {path}")

print(f"[export] done: {len(exported)} models in {out_dir}")
