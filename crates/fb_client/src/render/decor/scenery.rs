//! Laying out the scenery around the course: islands, clouds, birds, balloons, set pieces and the land below.
use super::*;

pub(super) fn islands(k: &mut Kit, root: Entity, boxes: &[Aabb], all: Aabb) {
    let cx = (all.min.x + all.max.x) / 2.0;
    let cz = (all.min.z + all.max.z) / 2.0;
    let reach = (all.max.x - all.min.x).hypot(all.max.z - all.min.z) / 2.0;
    let count = 7;
    let flora = [Model::Tree, Model::Pine, Model::Mushroom, Model::Tree, Model::Pine];
    for n in 0..count {
        let scale = 0.8 + k.rnd() * 0.9;
        let mut pos = None;
        for _ in 0..24 {
            let a = n as f32 / count as f32 * core::f32::consts::TAU + k.rnd() * 0.8;
            let d = reach * 0.6 + 18.0 + k.rnd() * 40.0;
            let c = Vec3::new(cx + a.cos() * d, all.min.y - 10.0 - k.rnd() * 22.0, cz + a.sin() * d);
            if !blocked(boxes, c, 4.5 * scale + 2.0, 7.0 * scale, 8.0) {
                pos = Some(c);
                break;
            }
        }
        let Some(home) = pos else { continue };
        let yaw = k.rnd() * 6.3;
        let base = Transform::from_translation(home)
            .with_rotation(Quat::from_rotation_y(yaw))
            .with_scale(Vec3::splat(scale));
        let g = k.group(Some(root), base);
        k.island(g, Transform::default());
        let trees = if LEAFY.contains(&k.look.look.id) {
            1 + (k.rnd() * 3.0) as usize
        } else {
            0
        };
        for i in 0..trees {
            let name = flora[((k.rnd() * flora.len() as f32) as usize).min(flora.len() - 1)];
            let a = k.rnd() * 6.3;
            let r = if i == 0 && trees == 1 { 0.0 } else { 1.0 + k.rnd() * 1.6 };
            let (x, z) = (a.cos() * r, a.sin() * r);
            // (The foot a little into the grass, 0.42 up in the model: the roots flare into it.)
            let f = k.model(name, g, [x, 0.4, z], 1.0, None);
            let (fy, fs) = (k.rnd() * 6.3, 0.55 + k.rnd() * 0.35);
            k.set_tf(f, |t| {
                t.rotation = Quat::from_rotation_y(fy);
                t.scale = Vec3::splat(fs);
            });
            let spread = match name {
                Model::Tree => 1.6,
                Model::Pine => 1.5,
                _ => 1.1,
            };
            k.blob_shadow(g, [x, 0.47, z], spread * fs);
        }
        let ph = k.rnd() * 50.0;
        k.tick(move |t, tx| {
            let mut b = base;
            b.translation.y = home.y + (t * 0.25 + ph).sin() * 0.7;
            b.rotation = Quat::from_rotation_y(yaw + (t * 0.05 + ph).sin() * 0.15);
            tx.set(g, b);
        });
    }
}

pub(super) fn clouds(k: &mut Kit, root: Entity, boxes: &[Aabb], req: &SceneryRequest) {
    let cloud = k.look.look.sky.cloud;
    let paint = if cloud == Rgb::WHITE {
        Vec::new()
    } else {
        vec![("Cloud", cloud, 0.5)]
    };
    for _ in 0..req.clouds {
        for _ in 0..40 {
            let a = k.rnd() * core::f32::consts::TAU;
            let d = req.spread as f32 * (0.55 + k.rnd() * 0.9);
            let home = Vec3::new(
                req.cx as f32 + a.cos() * d,
                req.y_min as f32 + k.rnd() * (req.y_max - req.y_min) as f32,
                req.cz as f32 + a.sin() * d * 1.2,
            );
            let scale = 1.4 + k.rnd() * 2.8;
            if blocked(boxes, home, CLOUD_R * scale + DRIFT, CLOUD_H * scale, 10.0) {
                continue;
            }
            let (yaw, spin, ph, w) = (
                k.rnd() * 6.3,
                (k.rnd() - 0.5) * 0.04,
                k.rnd() * 100.0,
                0.05 + k.rnd() * 0.07,
            );
            let scene = k.assets.load(GltfAssetLabel::Scene(0).from_asset("models/cloud.glb"));
            let e = k
                .world
                .spawn((
                    Decor,
                    WorldAssetRoot(scene),
                    Prop::painted(Model::Cloud, paint.clone()),
                    Transform::from_translation(home).with_scale(Vec3::splat(scale)),
                    Visibility::default(),
                    NotShadowCaster,
                    ChildOf(root),
                ))
                .id();
            k.tick(move |t, tx| {
                let p = Vec3::new(
                    home.x + (t * w + ph).sin() * DRIFT,
                    home.y + (t * w * 2.3 + ph * 1.7).sin() * 0.6,
                    home.z + (t * w * 0.8 + ph).cos() * DRIFT,
                );
                let breathe = 1.0 + (t * 0.35 + ph).sin() * 0.035;
                tx.set(
                    e,
                    Transform {
                        translation: p,
                        rotation: Quat::from_rotation_y(yaw + t * spin),
                        scale: Vec3::new(scale * breathe, scale * (2.0 - breathe), scale * breathe),
                    },
                );
            });
            break;
        }
    }
}

/// A few small flocks circling well outside the course.
pub(super) fn birds(k: &mut Kit, root: Entity, all: Aabb) {
    let (flocks, per) = (2, 5);
    let mat = k.plain_with(rgb(0x4b3d7a), Kind::Fabric, |s| s.roughness = Some(0.7));
    let body = k
        .world
        .resource_mut::<Assets<Mesh>>()
        .add(Sphere::new(0.22).mesh().uv(10, 8).scaled_by(Vec3::new(0.8, 0.7, 1.6)));
    let wing = k
        .world
        .resource_mut::<Assets<Mesh>>()
        .add(Mesh::from(Cuboid::new(0.9, 0.04, 0.34)).translated_by(Vec3::new(0.45, 0.0, 0.0)));
    let Mat::S(m) = mat else { return };
    let cx = (all.min.x + all.max.x) / 2.0;
    let cz = (all.min.z + all.max.z) / 2.0;
    let reach = (all.max.x - all.min.x).hypot(all.max.z - all.min.z) / 2.0;
    let mut list = Vec::new();
    for f in 0..flocks {
        let r = reach + 25.0 + k.rnd() * 30.0;
        let y = all.max.y + 10.0 + k.rnd() * 14.0;
        let speed = (0.06 + k.rnd() * 0.05) * if f % 2 == 1 { -1.0 } else { 1.0 };
        let a0 = k.rnd() * 6.3;
        for i in 0..per {
            let back = i as f32 * 1.3;
            let side = (if i % 2 == 1 { 1.0 } else { -1.0 }) * (i as f32 / 2.0).ceil() * 1.1;
            let ph = k.rnd() * 6.0;
            let mut spawn = |mesh: &Handle<Mesh>| {
                k.world
                    .spawn((
                        Decor,
                        Mesh3d(mesh.clone()),
                        MeshMaterial3d(m.clone()),
                        Transform::default(),
                        Visibility::default(),
                        NotShadowCaster,
                        ChildOf(root),
                    ))
                    .id()
            };
            let parts = [spawn(&body), spawn(&wing), spawn(&wing)];
            list.push((parts, r, y, speed, a0, back, side, ph));
        }
    }
    k.tick(move |t, tx| {
        for (parts, r, y, speed, a0, back, side, ph) in &list {
            let a = a0 + t * speed;
            let dir = speed.signum();
            let aa = a - back / r * dir;
            let rr = r + side;
            let p = Vec3::new(
                cx + aa.cos() * rr,
                y + (t * 0.7 + ph).sin() * 0.8 + back * 0.15,
                cz + aa.sin() * rr,
            );
            // Heading along the circle.
            let yaw = (-aa.sin() * dir).atan2(aa.cos() * dir);
            let m = Transform::from_translation(p).with_rotation(Quat::from_euler(
                EulerRot::XYZ,
                0.0,
                yaw,
                (t * 0.5 + ph).sin() * 0.15 - dir * 0.25,
            ));
            tx.set(parts[0], m);
            let flap = (t * 9.0 + ph).sin() * 0.7 + 0.1;
            tx.set(
                parts[1],
                m.mul_transform(Transform::from_rotation(Quat::from_rotation_z(flap))),
            );
            let right =
                Transform::from_rotation(Quat::from_rotation_y(core::f32::consts::PI) * Quat::from_rotation_z(flap));
            tx.set(parts[2], m.mul_transform(right));
        }
    });
}

/// Striped hot-air balloons far out, slowly rising, sinking and turning.
pub(super) fn balloons(k: &mut Kit, root: Entity, boxes: &[Aabb], all: Aabb) {
    let pals = [
        (rgb(0xff8cc8), rgb(0xffffff)),
        (rgb(0xffd84a), rgb(0xff9f4a)),
        (rgb(0x7ccfff), rgb(0xffffff)),
        (rgb(0xa98bff), rgb(0xffd84a)),
        (rgb(0x6fe08a), rgb(0xffffff)),
    ];
    let cx = (all.min.x + all.max.x) / 2.0;
    let cz = (all.min.z + all.max.z) / 2.0;
    let reach = (all.max.x - all.min.x).hypot(all.max.z - all.min.z) / 2.0;
    let basket = k.plain(rgb(0xb07a4a), Kind::Wood);
    let rope = k.plain(rgb(0x6b4f3a), Kind::Fabric);
    let count = 4;
    let envelope = k
        .world
        .resource_mut::<Assets<Mesh>>()
        .add(Sphere::new(3.2).mesh().uv(28, 18).scaled_by(Vec3::new(1.0, 1.15, 1.0)));
    let frustum = |top: f32, bottom: f32, h: f32, seg: u32| {
        ConicalFrustum {
            radius_top: top,
            radius_bottom: bottom,
            height: h,
        }
        .mesh()
        .resolution(seg)
        .build()
    };
    let sk = k.world.resource_mut::<Assets<Mesh>>().add(frustum(1.1, 0.7, 1.2, 18));
    let bk = k.world.resource_mut::<Assets<Mesh>>().add(frustum(0.75, 0.6, 0.8, 12));
    for n in 0..count {
        let mut pos = None;
        for _ in 0..20 {
            let a = n as f32 / count as f32 * core::f32::consts::TAU + k.rnd() * 1.2;
            let d = reach + 45.0 + k.rnd() * 50.0;
            let c = Vec3::new(cx + a.cos() * d, all.max.y + 4.0 + k.rnd() * 22.0, cz + a.sin() * d);
            if !blocked(boxes, c, 6.0, 6.0, 10.0) {
                pos = Some(c);
                break;
            }
        }
        let Some(home) = pos else { continue };
        let (c1, c2) = pals[((k.rnd() * pals.len() as f32) as usize).min(pals.len() - 1)];
        let g = k.group(Some(root), Transform::from_translation(home));
        let env = k.pattern(c1, c2, 0.9, [1.0, 0.0], Kind::Fabric, Pattern::Stripes);
        let skirt = k.plain(c2, Kind::Fabric);
        if let Mat::S(h) = &env {
            k.world.spawn((
                Decor,
                Mesh3d(envelope.clone()),
                MeshMaterial3d(h.clone()),
                Transform::from_xyz(0.0, 5.2, 0.0),
                Visibility::default(),
                NotShadowCaster,
                ChildOf(g),
            ));
        }
        for (mesh, mat, y) in [(sk.clone(), &skirt, 1.9), (bk.clone(), &basket, 0.0)] {
            if let Mat::S(h) = mat {
                k.world.spawn((
                    Decor,
                    Mesh3d(mesh),
                    MeshMaterial3d(h.clone()),
                    Transform::from_xyz(0.0, y, 0.0),
                    Visibility::default(),
                    NotShadowCaster,
                    ChildOf(g),
                ));
            }
        }
        for i in 0..4 {
            let a = i as f32 / 4.0 * core::f32::consts::TAU + core::f32::consts::FRAC_PI_4;
            k.part(
                g,
                Shape::Cyl,
                &rope,
                [a.cos() * 0.65, 0.95, a.sin() * 0.65],
                [0.03, 1.4, 0.03],
                NO_ROT,
            );
        }
        let ph = k.rnd() * 50.0;
        let spin = (k.rnd() - 0.5) * 0.08;
        k.tick(move |t, tx| {
            tx.set(
                g,
                Transform::from_xyz(
                    home.x + (t * 0.03 + ph).sin() * 4.0,
                    home.y + (t * 0.11 + ph).sin() * 2.5,
                    home.z + (t * 0.025 + ph).cos() * 4.0,
                )
                .with_rotation(Quat::from_euler(
                    EulerRot::XYZ,
                    (t * 0.4 + ph).sin() * 0.03,
                    t * spin,
                    (t * 0.33 + ph).cos() * 0.03,
                )),
            )
        });
    }
}

/// The land far below the course, in the look's colours.
pub(super) fn ground(k: &mut Kit, root: Entity, all: Aabb) {
    let Some(gr) = k.look.look.ground else { return };
    // (Not padded: its metre-wide cushions read as a fine grid on land seen from 70 m up.)
    let kind = if gr.glow { Kind::Glossy } else { Kind::Plastic };
    let mut spec = Spec {
        paint: Some(Paint {
            c1: color(gr.c1).to_linear(),
            c2: color(gr.c2).to_linear(),
            freq: gr.freq as f32,
            dir: Vec2::new(1.0, 0.6),
            speed: gr.speed as f32,
            kind: gr.kind,
        }),
        ..Spec::plain(LinearRgba::WHITE, Some(kind))
    };
    if gr.glow {
        spec.emissive = color(gr.c1).to_linear() * 0.8;
    }
    let Mat::S(m) = k.surface(spec) else { return };
    let disc = k.world.resource_mut::<Assets<Mesh>>().add(
        Circle::new(900.0)
            .mesh()
            .resolution(72)
            .build()
            .rotated_by(Quat::from_rotation_x(-core::f32::consts::FRAC_PI_2)),
    );
    k.world.spawn((
        Decor,
        Mesh3d(disc),
        MeshMaterial3d(m),
        Transform::from_xyz(
            (all.min.x + all.max.x) / 2.0,
            all.min.y - 70.0,
            (all.min.z + all.max.z) / 2.0,
        ),
        Visibility::default(),
        NotShadowCaster,
        NotShadowReceiver,
        ChildOf(root),
    ));
}

/// Places the look's pieces round the course.
pub(super) fn decorate(k: &mut Kit, root: Entity, boxes: &[Aabb], all: Aabb) {
    let set = set_of(k.look.look.id);
    if set.is_empty() {
        return;
    }
    let total: f32 = set.iter().map(|p| p.weight).sum();
    let mut placed: Vec<(Vec3, f32)> = Vec::new();
    let w = all.max.x - all.min.x;
    let d = all.max.z - all.min.z;
    // More for bigger maps (long races), within a budget.
    let count = (12.0 + (w + d) / 14.0).min(26.0).round() as usize;
    for _ in 0..count {
        let mut x = k.rnd() * total;
        let piece = set
            .iter()
            .find(|p| {
                x -= p.weight;
                x <= 0.0
            })
            .unwrap_or(&set[0]);
        let s = 0.8 + k.rnd() * 0.6;
        let r = piece.r * s + if piece.island { 1.0 } else { 0.0 };
        for _ in 0..30 {
            let p = Vec3::new(
                all.min.x - 40.0 + k.rnd() * (w + 80.0),
                if piece.island {
                    all.min.y - 24.0 + k.rnd() * (all.max.y - all.min.y + 20.0)
                } else {
                    all.min.y - 16.0 + k.rnd() * (all.max.y - all.min.y + 26.0)
                },
                all.min.z - 40.0 + k.rnd() * (d + 80.0),
            );
            // (An island's rock reaches 7 m below it.)
            let (mid, half) = if piece.island {
                (p.y + (piece.h * s - 7.0) / 2.0, (piece.h * s + 7.0) / 2.0)
            } else {
                (p.y + piece.h * s / 2.0, piece.h * s / 2.0)
            };
            if blocked(boxes, Vec3::new(p.x, mid, p.z), r + 2.0, half, 8.0) {
                continue;
            }
            if placed
                .iter()
                .any(|(q, qr)| (q.x - p.x).hypot(q.z - p.z) < qr + r + 2.0 && (q.y - p.y).abs() < 12.0)
            {
                continue;
            }
            placed.push((p, r));
            let at = k.group(Some(root), Transform::from_translation(p));
            let obj = k.group(Some(at), Transform::default());
            k.piece += 1;
            (piece.make)(k, obj);
            if piece.island {
                k.island(at, Transform::from_scale(Vec3::splat(s * 0.85)));
                k.grounds.push((k.piece, obj));
                // (In the piece's own units: `obj` is scaled by `s` below.)
                k.blob_shadow(obj, [0.0, 0.07, 0.0], piece.r * 0.9);
            }
            let spin = if piece.island { 0.0 } else { k.rnd() * 6.3 };
            // (On the island's grass, 0.42 up in the model.)
            let lift = if piece.island { 0.42 * s * 0.85 } else { 0.0 };
            k.set_tf(obj, |t| {
                t.translation.y += lift;
                t.scale *= s;
                t.rotation = Quat::from_rotation_y(spin) * t.rotation;
            });
            break;
        }
    }
}
