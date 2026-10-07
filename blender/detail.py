"""
Rebuilds the scenery models of blender/models.blend with more shape (`export.py` runs it before baking):

    blender -b blender/models.blend --python blender/detail.py [-- --save]

Trees get a flared, rooty trunk with branches into a canopy of lumpy blobs; pines three scalloped,
drooping tiers; mushrooms a dome with a rolled rim, gills, a frilled skirt, a waisted stem and raised
spots; clouds a dozen soft lobes with a flattened base; islands a wobbly grass cap that drips over the
edge, a craggy rock tapering to a point, bushes and pebbles; cones a rounded base, a rounded tip and two
raised stripes; the star is puffy. The mechanical props (hub, bumper, finish, door, hammer, hex) get
bevelled edges.

Everything the game relies on stays: each model's footprint and height (the colliders are in
core/fb_maps), the root node, the material names (`Kind::of_model`, the looks' repaints by material) and
the mesh names it looks for (`MushCap…` takes the map's tint, so the gills are `MushGills`). The models
are rebuilt from scratch each run, from fixed numbers and seeded noise: running it again changes nothing.
"""

import math
import sys

import bmesh
import bpy
from mathutils import Matrix, Vector, noise

TAU = math.tau


# ---------------------------------------------------------------- helpers


def n3(p, seed=0.0, freq=1.0):
    """Perlin noise in [-1, 1] at p (a Vector), shifted by a seed."""
    q = Vector(p) * freq + Vector((seed * 17.31, seed * 5.13, seed * 11.71))
    return noise.noise(q, noise_basis="PERLIN_ORIGINAL")


def smoothstep(a, b, x):
    t = min(1.0, max(0.0, (x - a) / (b - a)))
    return t * t * (3.0 - 2.0 * t)


def lathe(bm, profile, seg, shape=None, matrix=None):
    """A surface of revolution about +Z added to `bm`: `profile` is [(r, z)…] in the order that keeps the
    outside on the left (bottom to top for an outer wall), r = 0 a pole. `shape(theta, r, z, t)` -> (r, z)
    or (x, y, z) moves each vertex (t: 0…1 along the profile)."""
    rings = []
    last = len(profile) - 1
    for i, (r, z) in enumerate(profile):
        t = i / last if last else 0.0
        if r < 1e-6:
            co = Vector((0.0, 0.0, z))
            if shape:
                out = shape(0.0, 0.0, z, t)
                co = Vector(out) if len(out) == 3 else Vector((0.0, 0.0, out[1]))
            rings.append([bm.verts.new(matrix @ co if matrix else co)])
            continue
        ring = []
        for j in range(seg):
            th = j / seg * TAU
            rr, zz = r, z
            if shape:
                out = shape(th, r, z, t)
                if len(out) == 3:
                    co = Vector(out)
                    ring.append(bm.verts.new(matrix @ co if matrix else co))
                    continue
                rr, zz = out
            co = Vector((rr * math.cos(th), rr * math.sin(th), zz))
            ring.append(bm.verts.new(matrix @ co if matrix else co))
        rings.append(ring)
    faces = []
    for a, b in zip(rings, rings[1:]):
        for j in range(seg):
            if len(a) == 1:
                quad = (a[0], b[(j + 1) % seg], b[j])
            elif len(b) == 1:
                quad = (a[j], a[(j + 1) % seg], b[0])
            else:
                quad = (a[j], a[(j + 1) % seg], b[(j + 1) % seg], b[j])
            faces.append(bm.faces.new(quad))
    return faces


def blob(bm, centre, radius, subdiv, amp, freq, seed, squash=(1.0, 1.0, 1.0), floor=None):
    """A lumpy ball added to `bm`: an icosphere pushed out along its normals by two octaves of noise.
    `floor` = (z, keep): below z the shape is pressed flat (keep: share of the depth left)."""
    made = bmesh.ops.create_icosphere(bm, subdivisions=subdiv, radius=1.0)
    c = Vector(centre)
    for v in made["verts"]:
        d = v.co.normalized()
        w = Vector((d.x * squash[0], d.y * squash[1], d.z * squash[2])) * radius
        p = c + w
        k = 1.0 + amp * n3(p, seed, freq) + amp * 0.35 * n3(p, seed + 3.0, freq * 2.7)
        p = c + w * k
        if floor is not None and p.z < floor[0]:
            p.z = floor[0] + (p.z - floor[0]) * floor[1]
        v.co = p


def tube(bm, p0, p1, r0, r1, seg=10, rings=4, seed=0.0):
    """A tapered branch from p0 to p1, closed at both ends."""
    p0, p1 = Vector(p0), Vector(p1)
    axis = p1 - p0
    rot = axis.normalized().to_track_quat("Z", "Y").to_matrix().to_4x4()
    m = Matrix.Translation(p0) @ rot
    length = axis.length
    prof = [(0.0, 0.0)]
    for i in range(rings + 1):
        t = i / rings
        prof.append((r0 + (r1 - r0) * t, length * t))
    prof.append((0.0, length + r1 * 0.6))
    lathe(bm, prof, seg, matrix=m)


def squircle(th, p=6.0):
    """Radius of a rounded square of half-width 1 at angle th."""
    c, s = abs(math.cos(th)), abs(math.sin(th))
    return 1.0 / (c**p + s**p) ** (1.0 / p)


def fresh_mesh(name):
    old = bpy.data.meshes.get(name)
    if old is not None and old.users == 0:
        bpy.data.meshes.remove(old)
    return bpy.data.meshes.new(name)


def finish(bm, name, material, coll, root, sharp=None):
    """The bmesh as object `name` (its mesh named the same) of `material`, smooth (edges sharper than
    `sharp` degrees stay hard), under `root` in `coll`."""
    bmesh.ops.remove_doubles(bm, verts=bm.verts, dist=1e-5)
    bm.normal_update()
    me = fresh_mesh(name)
    bm.to_mesh(me)
    bm.free()
    me.materials.append(bpy.data.materials[material])
    me.polygons.foreach_set("use_smooth", [True] * len(me.polygons))
    if sharp is not None:
        me.set_sharp_from_angle(angle=math.radians(sharp))
    me.update()
    o = bpy.data.objects.new(name, me)
    coll.objects.link(o)
    o.parent = root
    return o


def rebuild(model, build):
    """Empties collection A_<model> but for its root and fills it again."""
    coll = bpy.data.collections.get(f"A_{model}")
    if coll is None:
        print(f"[detail] no collection A_{model}")
        return
    roots = [o for o in coll.objects if o.parent is None]
    if len(roots) != 1 or roots[0].type != "EMPTY":
        print(f"[detail] A_{model}: no single root empty, left alone")
        return
    root = roots[0]
    gone = [o for o in coll.all_objects if o is not root]
    meshes = {o.data for o in gone if o.type == "MESH"}
    for o in gone:
        bpy.data.objects.remove(o, do_unlink=True)
    for me in meshes:
        if me.users == 0:
            bpy.data.meshes.remove(me)
    build(coll, root)
    tris = sum(sum(len(p.vertices) - 2 for p in o.data.polygons) for o in coll.all_objects if o.type == "MESH")
    print(f"[detail] {model}: {tris} triangles")


# ---------------------------------------------------------------- the models


def tree(coll, root):
    bm = bmesh.new()

    def bark(th, r, z, t):
        root_k = max(0.0, 1.0 - z / 0.6) ** 2
        lobes = max(0.0, math.cos(5 * th + 0.4)) ** 3
        r = r + 0.17 * root_k * lobes
        p = Vector((math.cos(th), math.sin(th), z * 1.5))
        return r * (1.0 + 0.05 * n3(p, 1.0, 1.7)), z

    prof = [
        (0.0, 0.0),
        (0.5, 0.0),
        (0.47, 0.05),
        (0.4, 0.15),
        (0.35, 0.32),
        (0.33, 0.6),
        (0.31, 1.0),
        (0.29, 1.6),
        (0.27, 2.2),
        (0.24, 2.8),
        (0.2, 3.4),
        (0.14, 3.9),
        (0.0, 4.0),
    ]
    lathe(bm, prof, 20, bark)
    tube(bm, (0.0, 0.0, 2.3), (0.85, 0.3, 3.25), 0.13, 0.07, seed=2.0)
    tube(bm, (0.0, 0.0, 2.55), (-0.8, -0.2, 3.35), 0.12, 0.07, seed=3.0)
    tube(bm, (0.0, 0.0, 2.9), (-0.15, 0.7, 4.0), 0.1, 0.06, seed=4.0)
    finish(bm, "TreeTrunk", "Trunk", coll, root)

    bm = bmesh.new()
    lobes = [
        ((0.0, 0.0, 3.8), 1.3),
        ((0.85, 0.3, 3.3), 0.92),
        ((-0.82, -0.2, 3.38), 0.95),
        ((0.1, -0.72, 4.25), 0.88),
        ((-0.2, 0.75, 4.2), 0.85),
        ((0.62, -0.55, 3.55), 0.72),
        ((0.3, 0.45, 4.62), 0.62),
    ]
    for i, (c, r) in enumerate(lobes):
        blob(bm, c, r, 4, 0.1, 1.5, 10.0 + i, squash=(1.0, 1.0, 0.92))
    finish(bm, "TreeLeaves", "Leaves", coll, root)


def pine(coll, root):
    bm = bmesh.new()

    def flare(th, r, z, t):
        k = max(0.0, 1.0 - z / 0.45) ** 2
        return r + 0.12 * k * max(0.0, math.cos(4 * th)) ** 3, z

    lathe(bm, [(0.0, 0.0), (0.32, 0.0), (0.27, 0.1), (0.24, 0.4), (0.21, 1.0), (0.18, 1.6), (0.0, 1.75)], 14, flare)
    finish(bm, "PineTrunk", "Trunk", coll, root)

    bm = bmesh.new()
    # (Rim height, tip height, rim radius, lobes): the old cones' places, so the snow caps of `decor.rs` fit.
    tiers = [(1.0, 2.75, 1.46, 9), (2.05, 3.62, 1.11, 8), (3.02, 4.42, 0.76, 7)]
    for i, (z0, z1, rr, lobes) in enumerate(tiers):
        h = z1 - z0
        prof = [
            (0.0, z0 + 0.36 * h),
            (0.25 * rr, z0 + 0.3 * h),
            (0.6 * rr, z0 + 0.12 * h),
            (0.9 * rr, z0 + 0.0),
            (0.98 * rr, z0 + 0.01),
            (1.0 * rr, z0 + 0.06),
            (0.95 * rr, z0 + 0.12),
            (0.72 * rr, z0 + 0.34 * h),
            (0.47 * rr, z0 + 0.58 * h),
            (0.24 * rr, z0 + 0.82 * h),
            (0.08 * rr, z1 - 0.03),
            (0.0, z1),
        ]
        phase = i * 0.7

        def skirt(th, r, z, t, rr=rr, lobes=lobes, phase=phase, seed=i):
            f = (r / rr) ** 2
            s = (0.5 + 0.5 * math.cos(lobes * th + phase)) ** 1.5
            wob = 0.03 * n3(Vector((math.cos(th), math.sin(th), z)), 20.0 + seed, 2.0)
            r2 = r * (1.0 + f * (0.11 * (2.0 * s - 1.0) + wob))
            z2 = z - 0.14 * rr * f * s + 0.05 * rr * f
            return r2, z2

        lathe(bm, prof, 72, skirt)
    finish(bm, "PineLayers", "Pine", coll, root)


def cap_point(t):
    """The mushroom cap's outline (r, z) at t (0: under the rim, 1: the top)."""
    pts = [
        (0.86, 1.28),
        (0.97, 1.24),
        (1.04, 1.3),
        (1.05, 1.38),
        (1.02, 1.52),
        (0.94, 1.66),
        (0.8, 1.79),
        (0.6, 1.9),
        (0.35, 1.97),
        (0.12, 1.995),
        (0.0, 2.0),
    ]
    x = t * (len(pts) - 1)
    i = min(int(x), len(pts) - 2)
    f = x - i
    return tuple(pts[i][k] + (pts[i + 1][k] - pts[i][k]) * f for k in range(2)), pts


def mushroom(coll, root):
    bm = bmesh.new()
    prof = [(0.0, 0.0), (0.46, 0.0), (0.48, 0.04), (0.44, 0.14), (0.39, 0.35), (0.36, 0.65), (0.36, 0.95), (0.39, 1.2), (0.44, 1.36), (0.0, 1.42)]
    lathe(bm, prof, 32)
    finish(bm, "MushStem", "Stem", coll, root)

    # The skirt round the stem: a frilled collar.
    bm = bmesh.new()

    def frill(th, r, z, t):
        f = smoothstep(0.37, 0.52, r)
        return r * (1.0 + 0.04 * f * math.cos(14 * th)), z - 0.03 * f * (0.5 + 0.5 * math.cos(14 * th))

    lathe(bm, [(0.355, 0.94), (0.42, 0.9), (0.52, 0.83), (0.55, 0.83), (0.53, 0.86), (0.45, 0.95), (0.355, 1.02)], 112, frill)
    finish(bm, "MushSkirt", "Stem", coll, root)

    (_, pts) = cap_point(0.0)

    def wobble(th, r, z, t):
        return r * (1.0 + 0.025 * n3(Vector((math.cos(th) * 2, math.sin(th) * 2, 0.0)), 30.0)), z

    bm = bmesh.new()
    lathe(bm, pts, 64, wobble)
    finish(bm, "MushCap", "Cap", coll, root)

    # Gills under the cap (not tinted: not `MushCap…`).
    bm = bmesh.new()

    def gills(th, r, z, t):
        f = smoothstep(0.35, 0.8, r)
        r = r * (1.0 + 0.025 * n3(Vector((math.cos(th) * 2, math.sin(th) * 2, 0.0)), 30.0))
        return r, z + 0.018 * f * math.cos(40 * th)

    lathe(bm, [(0.0, 1.41), (0.3, 1.39), (0.6, 1.34), (0.8, 1.3), (0.87, 1.285)], 80, gills)
    finish(bm, "MushGills", "Stem", coll, root)

    # Spots: flattened lumps half sunk into the cap along its normal.
    bm = bmesh.new()
    spots = [(0.0, 0.62, 0.17), (1.1, 0.48, 0.13), (2.0, 0.66, 0.15), (2.9, 0.5, 0.12), (3.7, 0.7, 0.16), (4.6, 0.46, 0.12), (5.5, 0.64, 0.14), (0.6, 0.86, 0.11), (2.6, 0.88, 0.1), (4.4, 0.9, 0.1), (5.9, 0.82, 0.09)]
    for i, (th, t, r) in enumerate(spots):
        (pr, pz), _ = cap_point(t)
        (qr, qz), _ = cap_point(min(1.0, t + 0.02))
        tangent = Vector((qr - pr, qz - pz)).normalized()
        nr, nz = tangent.y, -tangent.x
        if nz < 0:
            nr, nz = -nr, -nz
        normal = Vector((nr * math.cos(th), nr * math.sin(th), nz)).normalized()
        at = Vector((pr * math.cos(th), pr * math.sin(th), pz)) - normal * 0.025
        made = bmesh.ops.create_icosphere(bm, subdivisions=2, radius=1.0)
        rot = normal.to_track_quat("Z", "Y").to_matrix()
        for v in made["verts"]:
            d = v.co.normalized()
            local = Vector((d.x * r, d.y * r, d.z * 0.055))
            v.co = at + rot @ local
    finish(bm, "MushSpots", "White", coll, root)


def cloud(coll, root):
    bm = bmesh.new()
    big = [((0.0, 0.0, 0.1), 2.0), ((1.9, 0.2, -0.2), 1.55), ((-1.95, -0.15, -0.3), 1.5), ((0.7, 0.5, 1.0), 1.25), ((-0.8, 0.3, 0.9), 1.2)]
    small = [
        ((3.0, -0.3, -0.6), 0.9),
        ((-3.0, 0.2, -0.7), 0.85),
        ((0.3, -1.2, -0.5), 1.0),
        ((-0.6, 1.3, -0.4), 1.0),
        ((1.4, -1.0, 0.4), 0.9),
        ((-1.5, 1.0, 0.3), 0.85),
        ((0.1, 0.2, 1.55), 0.55),
        ((2.4, 0.9, -0.4), 0.8),
        ((-2.3, -1.0, -0.5), 0.8),
    ]
    for i, (c, r) in enumerate(big):
        blob(bm, c, r, 4, 0.05, 0.9, 40.0 + i, floor=(-1.1, 0.3))
    for i, (c, r) in enumerate(small):
        blob(bm, c, r, 3, 0.05, 0.9, 50.0 + i, floor=(-1.1, 0.3))
    finish(bm, "Puffs", "Cloud", coll, root)


def island(coll, root):
    # The grass cap: a wobbly, slightly domed disc whose edge rolls over and drips.
    bm = bmesh.new()

    def grass(th, r, z, t):
        rim = 1.0 + 0.04 * n3(Vector((math.cos(th) * 1.3, math.sin(th) * 1.3, 0.0)), 60.0)
        r2 = r * rim
        if z < 0.05 and r > 3.5:
            drip = max(0.0, math.sin(11 * th + 1.3 * math.sin(3 * th))) ** 3
            z = z - 0.22 * drip
        elif r < 4.05:
            z = z + 0.035 * n3(Vector((r2 * math.cos(th), r2 * math.sin(th), 0.0)), 61.0, 0.7) * (r / 4.0)
        return r2, z

    prof = [(0.0, -0.05), (3.9, -0.05), (4.12, -0.1), (4.27, 0.0), (4.3, 0.15), (4.22, 0.31), (4.0, 0.39), (3.0, 0.41), (1.5, 0.42), (0.0, 0.42)]
    lathe(bm, prof, 96, grass)
    finish(bm, "IslandGrass", "Grass", coll, root)

    # The rock: craggy, tapering to a bent point; flat-ish facets.
    bm = bmesh.new()

    def crag(th, r, z, t):
        p = Vector((math.cos(th) * 1.4, math.sin(th) * 1.4, z * 0.45))
        k = 1.0 + 0.13 * n3(p, 70.0) + 0.06 * n3(p, 71.0, 2.6)
        x = r * k * math.cos(th) + 0.35 * (-z / 6.9) ** 2
        y = r * k * math.sin(th) - 0.2 * (-z / 6.9) ** 2
        return x, y, z

    prof = [
        (0.0, -6.9),
        (0.35, -6.45),
        (0.8, -5.6),
        (1.4, -4.6),
        (1.9, -3.7),
        (2.6, -2.95),
        (2.85, -2.55),
        (3.3, -2.15),
        (3.5, -1.55),
        (3.75, -1.0),
        (4.0, -0.5),
        (4.1, -0.18),
        (4.02, 0.0),
        (0.0, 0.0),
    ]
    lathe(bm, prof, 28, crag)
    # Pebbles on the grass.
    for i, (c, r) in enumerate([((3.1, -1.6, 0.42), 0.28), ((-2.6, -2.3, 0.42), 0.22), ((-3.2, 1.4, 0.41), 0.18)]):
        blob(bm, c, r, 2, 0.15, 2.0, 80.0 + i, squash=(1.2, 1.0, 0.7))
    finish(bm, "IslandRock", "Rock", coll, root, sharp=38)

    bm = bmesh.new()
    bushes = [((1.6, 1.0, 0.75), 0.62), ((-1.9, 0.6, 0.7), 0.52), ((0.4, -2.0, 0.72), 0.56), ((3.0, 0.4, 0.62), 0.4), ((-1.2, -2.8, 0.6), 0.38)]
    for i, ((x, y, z), r) in enumerate(bushes):
        blob(bm, (x, y, z), r, 3, 0.1, 2.2, 90.0 + i, squash=(1.0, 1.0, 0.85), floor=(0.42, 0.2))
        blob(bm, (x + r * 0.55, y - r * 0.3, z - r * 0.25), r * 0.62, 3, 0.1, 2.2, 95.0 + i, floor=(0.42, 0.2))
    finish(bm, "IslandBush", "Leaves", coll, root)


def cone(coll, root):
    bm = bmesh.new()

    def square(th, r, z, t):
        return r * squircle(th + math.pi / 4), z

    lathe(bm, [(0.0, 0.0), (0.29, 0.0), (0.305, 0.012), (0.31, 0.04), (0.305, 0.068), (0.285, 0.082), (0.25, 0.086), (0.0, 0.086)], 48, square)
    finish(bm, "ConeBase", "Cone", coll, root)

    def body_r(z):
        return 0.245 + (0.05 - 0.245) * (z - 0.12) / (0.84 - 0.12)

    bm = bmesh.new()
    prof = [(0.0, 0.08), (0.275, 0.08), (0.27, 0.1), (0.255, 0.115), (body_r(0.12), 0.12)]
    prof += [(body_r(z), z) for z in (0.3, 0.5, 0.7, 0.8)]
    prof += [(0.046, 0.855), (0.034, 0.872), (0.016, 0.879), (0.0, 0.88)]
    lathe(bm, prof, 40)
    finish(bm, "ConeBody", "Cone", coll, root)

    bm = bmesh.new()
    for z0, z1 in ((0.34, 0.48), (0.6, 0.7)):
        lathe(
            bm,
            [(body_r(z0) - 0.004, z0), (body_r(z0) + 0.005, z0 + 0.008), (body_r(z1) + 0.005, z1 - 0.008), (body_r(z1) - 0.004, z1)],
            40,
        )
    finish(bm, "ConeStripe", "White", coll, root)


def star(coll, root):
    """A puffy five-pointed star standing in the x/z plane, thickness along y."""
    bm = bmesh.new()
    centre = Vector((0.0, 0.0, 0.54))
    ro, ri, thick = 0.43, 0.23, 0.11
    seg, rings = 70, 6

    def outline(phi):
        s = (0.5 + 0.5 * math.cos(5 * (phi - math.pi / 2))) ** 1.25
        return ri + (ro - ri) * s

    def ring(side, k):
        rho = math.sin(math.pi / 2 * k / rings)
        y = side * thick * math.sqrt(max(0.0, 1.0 - rho * rho)) * (1.0 - 0.25 * rho)
        return [
            bm.verts.new(centre + Vector((rho * outline(p) * math.cos(p), y, rho * outline(p) * math.sin(p))))
            for p in (j / seg * TAU for j in range(seg))
        ]

    rim = ring(0, rings)
    for side in (-1, 1):
        tip = bm.verts.new(centre + Vector((0.0, side * thick, 0.0)))
        rows = [ring(side, k) for k in range(1, rings)] + [rim]
        prev = None
        for row in rows:
            for j in range(seg):
                a, b = row[j], row[(j + 1) % seg]
                if prev is None:
                    f = (tip, a, b) if side < 0 else (tip, b, a)
                else:
                    c, d = prev[(j + 1) % seg], prev[j]
                    f = (d, a, b, c) if side < 0 else (d, c, b, a)
                bm.faces.new(f)
            prev = row
    bmesh.ops.recalc_face_normals(bm, faces=bm.faces)
    finish(bm, "StarBody", "Star", coll, root)


# ---------------------------------------------------------------- bevels

# Object: bevel width (m). Hard-edged primitives of the mechanical props get rounded edges.
BEVELS = {
    "HubBase": 0.06,
    "HubPost": 0.05,
    "BumperStem": 0.06,
    "BumperRing": 0.03,
    "FinPost-1": 0.08,
    "FinPost1": 0.08,
    "DoorSlab": 0.08,
    "DoorRing": 0.025,
    "DoorDot": 0.025,
    "HammerPivot": 0.04,
    "HammerArm": 0.03,
    "HammerHead": 0.08,
    "HammerCapL": 0.04,
    "HammerCapR": 0.04,
    "HexTile": 0.035,
}


def bevel_props():
    for name, width in BEVELS.items():
        o = bpy.data.objects.get(name)
        if o is None or o.type != "MESH" or any(m.name == "DetailBevel" for m in o.modifiers):
            continue
        m = o.modifiers.new("DetailBevel", "BEVEL")
        m.width = width
        m.segments = 3
        m.limit_method = "ANGLE"
        m.angle_limit = math.radians(35)
        m.harden_normals = True
        o.data.polygons.foreach_set("use_smooth", [True] * len(o.data.polygons))
        o.data.set_sharp_from_angle(angle=math.radians(35))


MODELS = {
    "tree": tree,
    "pine": pine,
    "mushroom": mushroom,
    "cloud": cloud,
    "island": island,
    "cone": cone,
    "star": star,
}


def apply():
    if bpy.context.object and bpy.context.object.mode != "OBJECT":
        bpy.ops.object.mode_set(mode="OBJECT")
    for name, build in MODELS.items():
        rebuild(name, build)
    bevel_props()


if __name__ == "__main__":
    apply()
    argv = sys.argv[sys.argv.index("--") + 1 :] if "--" in sys.argv else []
    if "--save" in argv:
        # (No models.blend1 beside it.)
        bpy.context.preferences.filepaths.save_version = 0
        bpy.ops.wm.save_mainfile()
        print(f"[detail] saved {bpy.data.filepath}")
