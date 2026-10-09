//! Merging the parts nothing moves by cell and material, and shading them where they touch.
use super::*;

/// Times the ticks are tried at to see what they move (s).
const TRIES: [f32; 6] = [0.0, 0.37, 1.9, 5.3, 13.7, 41.1];

/// The scenery's entities its ticks move, turn, show or hide: each tick is tried at a few times, and what
/// they changed is put back.
fn ticked(world: &mut World, ticks: &[Tick]) -> HashSet<Entity> {
    let mut read = world.query_filtered::<(Entity, &Transform, &Visibility), With<Decor>>();
    let before: HashMap<Entity, (Transform, Visibility)> = read.iter(world).map(|(e, t, v)| (e, (*t, *v))).collect();
    let mut write = world.query_filtered::<(&'static mut Transform, &'static mut Visibility), With<Decor>>();
    let mut moved = HashSet::new();
    for t in TRIES {
        {
            let mut q = write.query_mut(world);
            let mut tx = Tx { q: &mut q };
            for f in ticks {
                f(t, &mut tx);
            }
        }
        for (e, tf, v) in read.iter(world) {
            if before.get(&e).is_some_and(|(t0, v0)| t0 != tf || v0 != v) {
                moved.insert(e);
            }
        }
    }
    for e in &moved {
        if let (Some((t, v)), Ok(mut e)) = (before.get(e), world.get_entity_mut(*e)) {
            e.insert((*t, *v));
        }
    }
    moved
}

/// The parts nothing moves are drawn merged, one mesh per cell and material (`meshes::merge`): the set pieces
/// are hundreds of small parts. Each merged mesh comes at two levels of detail (`Level`), and its vertices carry
/// how much the rest of their set piece hides them from the sky (`occlusion`), which the surface shader darkens
/// them by: no shadow map reaches this far, and without it the pieces look cut out of paper.
pub(super) fn merge_still(world: &mut World, ticks: &[Tick], parts: &[Part], grounds: &[(u32, Entity)], base: Entity) {
    use crate::render::meshes::{self, BANDS, BandPad, LodBand};
    let moving = ticked(world, ticks);
    // World matrices by entity (None: it, or something above it, moves or is hidden).
    let mut placed: HashMap<Entity, Option<Mat4>> = HashMap::new();
    fn place(
        world: &World,
        e: Entity,
        moving: &HashSet<Entity>,
        placed: &mut HashMap<Entity, Option<Mat4>>,
    ) -> Option<Mat4> {
        if let Some(m) = placed.get(&e) {
            return *m;
        }
        let local = world.get::<Transform>(e).map(Transform::to_matrix);
        let m = if moving.contains(&e) || world.get::<Visibility>(e) == Some(&Visibility::Hidden) {
            None
        } else {
            match world.get::<ChildOf>(e) {
                Some(up) => place(world, up.parent(), moving, placed).zip(local).map(|(p, l)| p * l),
                None => local,
            }
        };
        placed.insert(e, m);
        m
    }
    let mut materials: HashMap<UntypedAssetId, usize> = HashMap::new();
    let mut candidates = Vec::with_capacity(parts.len());
    let mut worlds = Vec::with_capacity(parts.len());
    let mut shades: HashMap<u32, Shade> = HashMap::new();
    for (i, part) in parts.iter().enumerate() {
        let m = place(world, part.e, &moving, &mut placed);
        // (A part something hangs from goes with what hangs from it: left alone.)
        let leaf = world.get::<Children>(part.e).is_none_or(|c| c.is_empty());
        // (See-through ones are sorted one by one.)
        let (id, opaque) = match &part.mat {
            Mat::S(h) => (
                h.id().untyped(),
                world
                    .resource::<Assets<SurfaceMaterial>>()
                    .get(h)
                    .is_some_and(|m| m.base.alpha_mode == AlphaMode::Opaque),
            ),
            Mat::G(h) => (
                h.id().untyped(),
                world
                    .resource::<Assets<StandardMaterial>>()
                    .get(h)
                    .is_some_and(|m| m.alpha_mode == AlphaMode::Opaque),
            ),
        };
        let n = materials.len();
        candidates.push(meshes::Candidate {
            at: m.map_or(Vec3::ZERO, |m| m.w_axis.truncate()),
            material: *materials.entry(id).or_insert(n),
            class: u64::from(matches!(part.mat, Mat::G(_))),
            still: leaf && opaque && m.is_some_and(|m| meshes::frame(&m).is_some()),
        });
        // (What stands still shades the rest of its piece, merged or not.)
        if let Some(m) = m
            && part.piece != 0
            && opaque
        {
            balls(part.shape, &m, i, &mut shades.entry(part.piece).or_default().balls);
        }
        worlds.push(m.unwrap_or(Mat4::IDENTITY));
    }
    for &(piece, e) in grounds {
        if let Some(m) = place(world, e, &moving, &mut placed) {
            shades.entry(piece).or_default().ground = Some(m.w_axis.y);
        }
    }
    let k = meshes::lod_k(
        world.get_resource::<crate::settings::Display>().map_or(70.0, |d| d.fov),
        world
            .get_resource::<crate::render::quality::Quality>()
            .map(|q| q.preset),
    );
    let mut cpu: HashMap<(Shape, Level), Mesh> = HashMap::new();
    let mut gone = 0;
    let mut drawn = 0;
    let groups = meshes::groups(&candidates);
    for group in &groups {
        let members: Vec<(usize, &Part, Mat4)> = group
            .iter()
            .filter_map(|&i| Some((i, parts.get(i)?, *worlds.get(i)?)))
            .collect();
        let Some(&(_, first, _)) = members.first() else {
            continue;
        };
        let mat = &first.mat;
        let (lo, hi) = members
            .iter()
            .fold((Vec3::INFINITY, Vec3::NEG_INFINITY), |(lo, hi), p| {
                let at = p.2.w_axis.truncate();
                (lo.min(at), hi.max(at))
            });
        let origin = (lo + hi) / 2.0;
        // (The levels switch at the distances of the group's biggest part, padded by how far its parts lie from
        // the centre, where the distance is measured: no part is drawn coarser than it would be by itself.)
        let pad = members
            .iter()
            .map(|p| p.2.w_axis.truncate().distance(origin))
            .fold(0.0, f32::max);
        let r = members.iter().map(|p| part_radius(&p.2)).fold(0.0, f32::max);
        let levels: Vec<(Level, Option<LodBand>)> = if members.iter().any(|p| has_levels(p.1.shape)) {
            vec![
                (
                    Level::Near,
                    Some(LodBand {
                        r,
                        first: 0,
                        last: NEAR_LAST,
                    }),
                ),
                (
                    Level::Far,
                    Some(LodBand {
                        r,
                        first: NEAR_LAST + 1,
                        last: BANDS - 1,
                    }),
                ),
            ]
        } else {
            vec![(Level::Far, None)]
        };
        for &(level, _) in &levels {
            for p in &members {
                cpu.entry((p.1.shape, level))
                    .or_insert_with(|| shape_mesh(p.1.shape, level));
            }
        }
        let frames = matches!(mat, Mat::S(_));
        let made: Option<Vec<(Mesh, Option<LodBand>)>> = levels
            .iter()
            .map(|&(level, band)| {
                let pieces: Vec<(&Mesh, Mat4)> = members
                    .iter()
                    .filter_map(|p| cpu.get(&(p.1.shape, level)).map(|m| (m, p.2)))
                    .collect();
                let mut mesh = meshes::merge(&pieces, origin, frames)?;
                if frames {
                    let spans: Vec<(usize, u32, usize)> = members
                        .iter()
                        .map(|p| {
                            let count = cpu.get(&(p.1.shape, level)).map_or(0, Mesh::count_vertices);
                            (p.0, p.1.piece, count)
                        })
                        .collect();
                    shade_merged(&mut mesh, origin, &spans, &shades);
                }
                Some((mesh, band))
            })
            .collect();
        let Some(made) = made else { continue };
        for (mesh, band) in made {
            let aabb = mesh.compute_aabb();
            let mesh = world.resource_mut::<Assets<Mesh>>().add(mesh);
            let mut e = world.spawn((
                Mesh3d(mesh),
                Transform::from_translation(origin),
                Visibility::default(),
                NotShadowCaster,
                ChildOf(base),
            ));
            match mat {
                Mat::S(h) => e.insert(MeshMaterial3d(h.clone())),
                Mat::G(h) => e.insert((MeshMaterial3d(h.clone()), NotShadowReceiver)),
            };
            if let Some(aabb) = aabb {
                e.insert(aabb);
            }
            if let Some(band) = band {
                e.insert((band, BandPad(pad), band.range_padded(k, pad)));
            }
            drawn += 1;
        }
        for p in &members {
            world.despawn(p.1.e);
        }
        gone += members.len();
    }
    debug!(
        "scenery: {gone} of {} parts merged into {drawn} meshes ({} groups)",
        parts.len(),
        groups.len()
    );
}

/// The last distance band (`meshes::BANDS`) the near level of the merged scenery is drawn in.
const NEAR_LAST: usize = 3;

/// How far a placed part (world matrix `m`) reaches from its origin, about (the unit shapes reach about 1).
fn part_radius(m: &Mat4) -> f32 {
    [m.x_axis, m.y_axis, m.z_axis]
        .iter()
        .map(|a| a.truncate().length())
        .fold(0.0, f32::max)
        * 1.12
}

/// What shades the parts of a set piece (`occlusion`): its still parts as balls (the part's index, centre,
/// radius), and the height of the grass it stands on, if it does.
#[derive(Default)]
pub(super) struct Shade {
    pub(super) balls: Vec<(usize, Vec3, f32)>,
    pub(super) ground: Option<f32>,
}

/// Balls standing in for a placed part (world matrix `m`, index `part`): one for a squat part, a row of them
/// along a long one.
pub(super) fn balls(s: Shape, m: &Mat4, part: usize, out: &mut Vec<(usize, Vec3, f32)>) {
    let (half, centre) = bulk(s);
    if half == Vec3::ZERO {
        return;
    }
    let axes = [m.x_axis.truncate(), m.y_axis.truncate(), m.z_axis.truncate()];
    let ext = [
        axes[0].length() * half.x,
        axes[1].length() * half.y,
        axes[2].length() * half.z,
    ];
    let c = m.transform_point3(centre);
    let long = if ext[0] >= ext[1] && ext[0] >= ext[2] {
        0
    } else if ext[1] >= ext[2] {
        1
    } else {
        2
    };
    let thin = (ext[(long + 1) % 3] * ext[(long + 2) % 3]).sqrt();
    if thin <= 1e-4 {
        return;
    }
    let n = ((ext[long] / thin).round() as usize).clamp(1, 4);
    if n == 1 {
        out.push((part, c, (ext[0] * ext[1] * ext[2]).cbrt()));
        return;
    }
    let dir = axes[long].normalize_or_zero();
    for i in 0..n {
        let f = (2 * i + 1) as f32 / n as f32 - 1.0;
        out.push((part, c + dir * ext[long] * f, thin));
    }
}

/// The most a vertex of the merged scenery is darkened by (surface.wgsl reads it from UV_0.y of an
/// `OBJECT_FRAME` mesh).
pub(super) const SHADE_MAX: f32 = 0.55;

/// How far above the grass of an island its darkening reaches (m).
const GROUND_REACH: f32 = 1.2;

/// How much a point (world `p`, normal `n`) of part `part` is hidden from the sky by the rest of its set piece:
/// by each ball of its other parts as a sphere hides the sky from a point (its solid angle, by how much the
/// point faces it), by the grass it stands close to, and a little more where it faces down. 0 to `SHADE_MAX`.
pub(super) fn occlusion(shade: &Shade, part: usize, p: Vec3, n: Vec3) -> f32 {
    let mut occ = 0.0;
    for &(i, c, r) in &shade.balls {
        if i == part {
            continue;
        }
        let d = c - p;
        let l2 = d.length_squared();
        // (Beyond 8 radii a ball hides under 1/64 of what it would close up.)
        if l2 < 1e-6 || l2 > r * r * 64.0 {
            continue;
        }
        let facing = n.dot(d) / l2.sqrt();
        if facing > 0.0 {
            occ += facing * (r * r / l2).min(1.0);
        }
    }
    if let Some(g) = shade.ground {
        let near = (1.0 - (p.y - g).max(0.0) / GROUND_REACH).clamp(0.0, 1.0);
        occ += 0.6 * near * near * (1.0 - 0.6 * n.y.max(0.0));
    }
    occ += 0.2 * (-n.y).max(0.0);
    (occ * 0.7).min(1.0) * SHADE_MAX
}

/// Writes the darkening of each vertex of a merged mesh into its UV_0.y (free in a mesh with frames: the
/// frame's position takes UV_0.x and UV_1). `spans`: each piece merged, in order, as the part's index, its set
/// piece and its vertex count.
fn shade_merged(mesh: &mut Mesh, origin: Vec3, spans: &[(usize, u32, usize)], shades: &HashMap<u32, Shade>) {
    let values: Vec<f32> = {
        let (Some(VertexAttributeValues::Float32x3(pos)), Some(VertexAttributeValues::Float32x3(nrm))) = (
            mesh.attribute(Mesh::ATTRIBUTE_POSITION),
            mesh.attribute(Mesh::ATTRIBUTE_NORMAL),
        ) else {
            return;
        };
        let mut out = Vec::with_capacity(pos.len());
        for &(part, piece, count) in spans {
            let shade = shades.get(&piece);
            let (start, end) = (out.len(), (out.len() + count).min(pos.len()));
            let (Some(ps), Some(ns)) = (pos.get(start..end), nrm.get(start..end)) else {
                break;
            };
            for (p, n) in ps.iter().zip(ns) {
                let (p, n) = (Vec3::from(*p) + origin, Vec3::from(*n));
                out.push(shade.map_or(0.0, |s| occlusion(s, part, p, n)));
            }
        }
        out
    };
    if let Some(VertexAttributeValues::Float32x2(uv)) = mesh.attribute_mut(Mesh::ATTRIBUTE_UV_0) {
        for (uv, s) in uv.iter_mut().zip(values) {
            uv[1] = s;
        }
    }
}
