//! Built set pieces: towers and keeps, mills and gears, tents and wheels, pylons, lighthouses and pyramids.
use super::*;

pub(super) fn windmill(k: &mut Kit, g: Entity) {
    let white = k.col(Swatch::White);
    let red = k.col(Swatch::Red);
    let pink = k.col(Swatch::Pink);
    let yellow = k.col(Swatch::Yellow);
    let wall = k.plain(white, Kind::Wood);
    k.part(g, Shape::Taper, &wall, [0.0, 2.6, 0.0], [1.0, 5.2, 1.0], NO_ROT);
    let roof = k.plain(red, Kind::Wood);
    k.part(g, Shape::Cone, &roof, [0.0, 5.9, 0.0], [1.1, 1.6, 1.1], NO_ROT);
    let base = Transform::from_xyz(0.0, 4.6, 0.75);
    let hub = k.group(Some(g), base);
    let sail = k.stripes(white, pink, 2.2, [1.0, 0.0], Pattern::Stripes);
    for i in 0..4 {
        let arm = k.group(
            Some(hub),
            Transform::from_rotation(Quat::from_rotation_z(i as f32 * core::f32::consts::FRAC_PI_2)),
        );
        k.part(arm, Shape::Box, &sail, [0.0, 1.7, 0.0], [0.55, 3.0, 0.06], NO_ROT);
    }
    let y = k.plain(yellow, Kind::Plastic);
    k.part(hub, Shape::Sphere, &y, [0.0, 0.0, 0.05], [0.25, 0.25, 0.25], NO_ROT);
    let sp = 0.6 + k.rnd() * 0.6;
    k.tick(move |t, tx| tx.set(hub, base.with_rotation(Quat::from_rotation_z(t * sp))));
}

pub(super) fn tower(k: &mut Kit, g: Entity) {
    let stone = k.stripes(rgb(0xd8d2c6), rgb(0xbdb5a8), 1.6, [0.0, 1.0], Pattern::Checker);
    k.part(g, Shape::Cyl, &stone, [0.0, 3.5, 0.0], [1.4, 7.0, 1.4], NO_ROT);
    for i in 0..8 {
        let a = i as f32 / 8.0 * core::f32::consts::TAU;
        k.part(
            g,
            Shape::Box,
            &stone,
            [a.cos() * 1.35, 7.25, a.sin() * 1.35],
            [0.45, 0.5, 0.45],
            [0.0, -a, 0.0],
        );
    }
    let roof = k.pick(&[Swatch::Red, Swatch::Blue, Swatch::Purple]);
    let (c1, c2) = (k.col(roof), k.col2(roof));
    let rm = k.stripes(c1, c2, 2.0, [0.0, 1.0], Pattern::Stripes);
    k.part(g, Shape::Cone, &rm, [0.0, 8.8, 0.0], [1.65, 2.6, 1.65], NO_ROT);
    let door = k.plain(rgb(0x3a3048), Kind::Plastic);
    k.part(g, Shape::Box, &door, [0.0, 4.2, 1.38], [0.35, 0.7, 0.1], NO_ROT);
    let y = k.col(Swatch::Yellow);
    k.model(Model::Flag, g, [0.0, 9.9, 0.0], 0.45, Some(y));
}

pub(super) fn keep(k: &mut Kit, g: Entity) {
    let stone = k.stripes(rgb(0xd8d2c6), rgb(0xc4bcae), 1.4, [1.0, 1.0], Pattern::Checker);
    k.part(g, Shape::Box, &stone, [0.0, 2.4, 0.0], [4.0, 4.8, 4.0], NO_ROT);
    let blue = k.col(Swatch::Blue);
    let roof = k.plain(blue, Kind::Plastic);
    for sx in [-1.0, 1.0] {
        for sz in [-1.0, 1.0] {
            k.part(
                g,
                Shape::Cyl,
                &stone,
                [sx * 2.0, 3.0, sz * 2.0],
                [0.7, 6.0, 0.7],
                NO_ROT,
            );
            k.part(
                g,
                Shape::Cone,
                &roof,
                [sx * 2.0, 6.8, sz * 2.0],
                [0.85, 1.6, 0.85],
                NO_ROT,
            );
        }
    }
    let (r, y) = (k.col(Swatch::Red), k.col(Swatch::Yellow));
    let banner = k.stripes(r, y, 1.8, [1.0, 0.0], Pattern::Chevron);
    k.part(g, Shape::Box, &banner, [0.0, 3.2, 2.03], [1.3, 2.4, 0.05], NO_ROT);
    let door = k.plain(rgb(0x3a3048), Kind::Plastic);
    k.part(g, Shape::Box, &door, [0.0, 0.8, 2.02], [1.0, 1.6, 0.05], NO_ROT);
}

pub(super) fn banners(k: &mut Kit, g: Entity) {
    let pole = k.plain_with(rgb(0xd8c090), Kind::Gold, |s| {
        s.metallic = Some(0.7);
        s.roughness = Some(0.3);
    });
    let ph = k.rnd() * 6.0;
    let white = k.col(Swatch::White);
    let mut cloths = Vec::new();
    for i in 0..3 {
        let x = (i as f32 - 1.0) * 1.6;
        k.part(g, Shape::Cyl, &pole, [x, 2.8, 0.0], [0.07, 5.6, 0.07], NO_ROT);
        k.part(g, Shape::Sphere, &pole, [x, 5.7, 0.0], [0.16, 0.16, 0.16], NO_ROT);
        let c = k.bright();
        let base = Transform::from_xyz(x, 5.3, 0.1);
        let cloth = k.group(Some(g), base);
        let m = k.stripes(c, white, 1.6, [0.0, 1.0], Pattern::Chevron);
        k.part(cloth, Shape::Box, &m, [0.0, -1.3, 0.0], [0.9, 2.6, 0.04], NO_ROT);
        cloths.push((cloth, base));
    }
    k.tick(move |t, tx| {
        for (i, (c, base)) in cloths.iter().enumerate() {
            let a = (t * 1.3 + ph + i as f32).sin() * 0.08;
            tx.set(*c, base.with_rotation(Quat::from_rotation_x(a)));
        }
    });
}

pub(super) fn gear(k: &mut Kit, g: Entity) {
    let base = Transform::from_xyz(0.0, 3.5, 0.0);
    let wheel = k.group(Some(g), base);
    let (o, y) = (k.col(Swatch::Orange), k.col(Swatch::Yellow));
    let c = k.pick(&[o, y, rgb(0x9aa3b0)]);
    let metal = k.plain_with(c, Kind::Metal, |s| {
        s.metallic = Some(0.6);
        s.roughness = Some(0.35);
    });
    let flat_x = [core::f32::consts::FRAC_PI_2, 0.0, 0.0];
    k.part(wheel, Shape::Cyl, &metal, [0.0; 3], [2.6, 0.6, 2.6], flat_x);
    for i in 0..12 {
        let a = i as f32 / 12.0 * core::f32::consts::TAU;
        k.part(
            wheel,
            Shape::Box,
            &metal,
            [a.cos() * 2.85, a.sin() * 2.85, 0.0],
            [0.7, 0.7, 0.6],
            [0.0, 0.0, a],
        );
    }
    let hub = k.plain(rgb(0x4a4f5a), Kind::Metal);
    k.part(wheel, Shape::Cyl, &hub, [0.0; 3], [0.6, 0.9, 0.6], flat_x);
    let bolt = k.plain(rgb(0x3a3f4a), Kind::Metal);
    for i in 0..4 {
        let a = i as f32 * 1.57;
        k.part(
            wheel,
            Shape::Cyl,
            &bolt,
            [a.cos() * 1.5, a.sin() * 1.5, 0.0],
            [0.35, 0.8, 0.35],
            flat_x,
        );
    }
    let yaw = k.rnd() * 6.3;
    k.set_tf(g, |t| t.rotation = Quat::from_rotation_y(yaw));
    let sp = (if k.rnd() < 0.5 { -1.0 } else { 1.0 }) * (0.3 + k.rnd() * 0.4);
    k.tick(move |t, tx| tx.set(wheel, base.with_rotation(Quat::from_rotation_z(t * sp))));
}

/// Smoke: puffs from `y0` up by `rise` and along `drift` over their life, of size `size.0` growing by `size.1`,
/// `rate` lives a second.
pub(super) struct Smoke {
    pub y0: f32,
    pub rise: f32,
    pub drift: Vec3,
    pub size: (f32, f32),
    pub rate: f32,
}

/// Puffs of smoke rising from a spout (chimneys, volcanoes).
pub(super) fn puffs(k: &mut Kit, g: Entity, mat: &Mat, smoke: Smoke) {
    let Smoke {
        y0,
        rise,
        drift,
        size,
        rate,
    } = smoke;
    let list: Vec<Entity> = (0..4)
        .map(|_| k.part(g, Shape::Sphere, mat, [0.0, y0, 0.0], [size.0; 3], NO_ROT))
        .collect();
    let ph = k.rnd() * 4.0;
    k.tick(move |t, tx| {
        for (i, p) in list.iter().enumerate() {
            let f = (t * rate + ph + i as f32 / list.len() as f32).rem_euclid(1.0);
            tx.set(
                *p,
                Transform::from_translation(Vec3::new(drift.x * f, y0 + f * rise, drift.z * f))
                    .with_scale(Vec3::splat(size.0 + f * size.1)),
            );
            tx.show(*p, f < 0.95);
        }
    });
}

pub(super) fn chimney(k: &mut Kit, g: Entity) {
    let (r, w) = (k.col(Swatch::Red), k.col(Swatch::White));
    let m = k.stripes(r, w, 0.45, [0.0, 1.0], Pattern::Stripes);
    k.part(g, Shape::Taper, &m, [0.0, 5.0, 0.0], [0.9, 10.0, 0.9], NO_ROT);
    let top = k.plain(rgb(0x4a4f5a), Kind::Metal);
    k.part(g, Shape::Cyl, &top, [0.0, 10.1, 0.0], [0.62, 0.4, 0.62], NO_ROT);
    let smoke = k.plain_with(rgb(0xe8e4de), Kind::Cloud, |s| {
        s.color.alpha = 0.75;
        s.alpha = AlphaMode::Blend;
    });
    puffs(
        k,
        g,
        &smoke,
        Smoke {
            y0: 10.5,
            rise: 5.0,
            drift: Vec3::new(1.6, 0.0, 0.6),
            size: (0.5, 1.4),
            rate: 0.25,
        },
    );
}

pub(super) fn tank(k: &mut Kit, g: Entity) {
    let metal = k.plain_with(rgb(0xaeb6c2), Kind::Metal, |s| {
        s.metallic = Some(0.6);
        s.roughness = Some(0.3);
    });
    let y = k.col(Swatch::Yellow);
    let band = k.stripes(y, rgb(0x3a3f4a), 1.4, [1.0, 1.0], Pattern::Chevron);
    k.part(g, Shape::Cyl, &band, [0.0, 0.4, 0.0], [2.0, 0.8, 2.0], NO_ROT);
    k.part(g, Shape::Cyl, &metal, [0.0, 2.2, 0.0], [1.9, 2.8, 1.9], NO_ROT);
    k.part(g, Shape::Dome, &metal, [0.0, 3.6, 0.0], [1.9, 0.9, 1.9], NO_ROT);
    let teal = k.col(Swatch::Teal);
    let pipe = k.plain(teal, Kind::Metal);
    k.part(
        g,
        Shape::Cyl,
        &pipe,
        [2.2, 2.6, 0.0],
        [0.25, 2.2, 0.25],
        [0.0, 0.0, core::f32::consts::FRAC_PI_2],
    );
    k.part(g, Shape::Cyl, &pipe, [3.2, 1.6, 0.0], [0.25, 2.2, 0.25], NO_ROT);
}

pub(super) fn tent(k: &mut Kit, g: Entity) {
    let (a, b) = k.pick(&[
        (Swatch::Red, Swatch::White),
        (Swatch::Blue, Swatch::Yellow),
        (Swatch::Pink, Swatch::White),
        (Swatch::Purple, Swatch::Yellow),
    ]);
    let (ca, cb) = (k.col(a), k.col(b));
    let cloth = k.pattern(ca, cb, 2.4, [1.0, 0.0], Kind::Cloth, Pattern::Stripes);
    k.part(g, Shape::Cyl, &cloth, [0.0, 1.3, 0.0], [2.8, 2.6, 2.8], NO_ROT);
    k.part(g, Shape::Cone, &cloth, [0.0, 3.9, 0.0], [3.1, 2.6, 3.1], NO_ROT);
    let door = k.plain(rgb(0x3a2040), Kind::Plastic);
    k.part(g, Shape::Box, &door, [0.0, 0.9, 2.72], [1.2, 1.8, 0.2], NO_ROT);
    let y = k.col(Swatch::Yellow);
    let gold = k.plain(y, Kind::Gold);
    k.part(g, Shape::Cyl, &gold, [0.0, 5.4, 0.0], [0.06, 0.8, 0.06], NO_ROT);
    k.model(Model::Flag, g, [0.0, 5.2, 0.0], 0.35, Some(y));
    let (ma, mb) = (k.plain(ca, Kind::Plastic), k.plain(cb, Kind::Plastic));
    for i in 0..12 {
        let aa = i as f32 / 12.0 * core::f32::consts::TAU;
        let m = if i % 2 == 1 { &ma } else { &mb };
        k.part(
            g,
            Shape::Sphere,
            m,
            [aa.cos() * 2.95, 2.62, aa.sin() * 2.95],
            [0.22; 3],
            NO_ROT,
        );
    }
}

pub(super) fn ferris(k: &mut Kit, g: Entity) {
    let white = k.col(Swatch::White);
    let metal = k.plain_with(white, Kind::Metal, |s| s.metallic = Some(0.4));
    for sz in [-0.6, 0.6] {
        for sx in [-1.0f32, 1.0] {
            k.part(
                g,
                Shape::Box,
                &metal,
                [sx * 1.4, 2.6, sz],
                [0.25, 5.6, 0.25],
                [0.0, 0.0, sx * -0.25],
            );
        }
    }
    let base = Transform::from_xyz(0.0, 5.2, 0.0);
    let wheel = k.group(Some(g), base);
    let pink = k.col(Swatch::Pink);
    let rim = k.plain(pink, Kind::Metal);
    k.part(wheel, Shape::Ring, &rim, [0.0; 3], [3.8; 3], NO_ROT);
    k.part(wheel, Shape::Ring, &rim, [0.0; 3], [1.2; 3], NO_ROT);
    let mut cabins = Vec::new();
    for i in 0..8 {
        let a = i as f32 / 8.0 * core::f32::consts::TAU;
        k.part(
            wheel,
            Shape::Box,
            &metal,
            [a.cos() * 1.9, a.sin() * 1.9, 0.0],
            [3.8, 0.1, 0.1],
            [0.0, 0.0, a],
        );
        let at = Transform::from_xyz(a.cos() * 3.8, a.sin() * 3.8, 0.0);
        let cab = k.group(Some(wheel), at);
        let c = k.bright();
        let m = k.plain(c, Kind::Plastic);
        k.part(cab, Shape::Box, &m, [0.0, -0.45, 0.0], [0.7, 0.6, 0.7], NO_ROT);
        k.part(cab, Shape::Box, &metal, [0.0, -0.05, 0.0], [0.8, 0.08, 0.8], NO_ROT);
        cabins.push((cab, at));
    }
    let sp = 0.15 + k.rnd() * 0.1;
    k.tick(move |t, tx| {
        tx.set(wheel, base.with_rotation(Quat::from_rotation_z(t * sp)));
        for (c, at) in &cabins {
            tx.set(*c, at.with_rotation(Quat::from_rotation_z(-t * sp)));
        }
    });
}

pub(super) fn pylon(k: &mut Kit, g: Entity) {
    let dark = k.plain_with(rgb(0x1d1438), Kind::Metal, |s| {
        s.metallic = Some(0.5);
        s.roughness = Some(0.3);
    });
    k.part(g, Shape::Box, &dark, [0.0, 4.0, 0.0], [1.1, 8.0, 1.1], NO_ROT);
    let c = k.bright();
    let edge = k.glow(c, 1.0);
    for sx in [-1.0, 1.0] {
        for sz in [-1.0, 1.0] {
            k.part(
                g,
                Shape::Box,
                &edge,
                [sx * 0.57, 4.0, sz * 0.57],
                [0.08, 8.0, 0.08],
                NO_ROT,
            );
        }
    }
    let band = k.glow(c, 0.8);
    for i in 0..4 {
        k.part(
            g,
            Shape::Box,
            &band,
            [0.0, 1.0 + i as f32 * 2.0, 0.0],
            [1.16, 0.08, 1.16],
            NO_ROT,
        );
    }
    let c2 = k.bright();
    let tm = k.glow(c2, 1.0);
    let top = k.part(g, Shape::Octa, &tm, [0.0, 9.3, 0.0], [0.7, 0.9, 0.7], NO_ROT);
    let ph = k.rnd() * 6.0;
    k.tick(move |t, tx| {
        tx.set(
            top,
            tf(
                [0.0, 9.3 + (t * 1.5 + ph).sin() * 0.25, 0.0],
                [0.7, 0.9, 0.7],
                [0.0, t * 1.2, 0.0],
            ),
        )
    });
}

pub(super) fn lighthouse(k: &mut Kit, g: Entity) {
    let red = k.col(Swatch::Red);
    let m = k.stripes(red, rgb(0xffffff), 0.55, [0.0, 1.0], Pattern::Stripes);
    k.part(g, Shape::Taper, &m, [0.0, 3.5, 0.0], [1.2, 7.0, 1.2], NO_ROT);
    let metal = k.plain(rgb(0x3a3f4a), Kind::Metal);
    k.part(g, Shape::Cyl, &metal, [0.0, 7.1, 0.0], [1.05, 0.2, 1.05], NO_ROT);
    let glass = k.plain_with(rgb(0xffffff), Kind::Glass, |s| {
        s.color.alpha = 0.5;
        s.alpha = AlphaMode::Blend;
    });
    k.part(g, Shape::Cyl, &glass, [0.0, 7.7, 0.0], [0.6, 1.0, 0.6], NO_ROT);
    let lm = k.glow(rgb(0xfff2a0), 1.0);
    let lamp = k.part(g, Shape::Sphere, &lm, [0.0, 7.7, 0.0], [0.35; 3], NO_ROT);
    let roof = k.plain(red, Kind::Plastic);
    k.part(g, Shape::Cone, &roof, [0.0, 8.6, 0.0], [0.8, 0.9, 0.8], NO_ROT);
    let beam = k.group(Some(g), Transform::from_xyz(0.0, 7.7, 0.0));
    let bm = k.glow(rgb(0xfff6c0), 0.18);
    k.part(
        beam,
        Shape::Cone,
        &bm,
        [0.0, 0.0, 4.0],
        [0.9, 8.0, 0.9],
        [-core::f32::consts::FRAC_PI_2, 0.0, 0.0],
    );
    k.tick(move |t, tx| {
        tx.set(
            beam,
            Transform::from_xyz(0.0, 7.7, 0.0).with_rotation(Quat::from_rotation_y(t * 0.8)),
        );
        tx.set(
            lamp,
            Transform::from_xyz(0.0, 7.7, 0.0).with_scale(Vec3::splat(0.33 + (t * 4.0).sin() * 0.04)),
        );
    });
}

pub(super) fn pyramid(k: &mut Kit, g: Entity) {
    let sand = k.stripes(rgb(0xf0d090), rgb(0xe0bc78), 1.6, [0.0, 1.0], Pattern::Stripes);
    let q = core::f32::consts::FRAC_PI_4;
    k.part(g, Shape::Cone4, &sand, [0.0, 1.8, 0.0], [3.0, 3.6, 3.0], [0.0, q, 0.0]);
    let y = k.col(Swatch::Yellow);
    let gold = k.plain_with(y, Kind::Gold, |s| {
        s.metallic = Some(0.7);
        s.roughness = Some(0.3);
    });
    k.part(g, Shape::Cone4, &gold, [0.0, 3.35, 0.0], [0.5, 0.6, 0.5], [0.0, q, 0.0]);
}

pub(super) fn pillar(k: &mut Kit, g: Entity) {
    let gold = k.plain_with(rgb(0xf2c14e), Kind::Gold, |s| {
        s.metallic = Some(0.8);
        s.roughness = Some(0.25);
    });
    let marble = k.stripes(rgb(0xfff8ee), rgb(0xefe6f6), 2.0, [1.0, 1.0], Pattern::Waves);
    k.part(g, Shape::Box, &marble, [0.0, 0.3, 0.0], [1.8, 0.6, 1.8], NO_ROT);
    k.part(g, Shape::Cyl, &marble, [0.0, 3.4, 0.0], [0.55, 5.6, 0.55], NO_ROT);
    for i in 0..3 {
        k.part(
            g,
            Shape::Torus,
            &gold,
            [0.0, 1.0 + i as f32 * 2.2, 0.0],
            [0.58; 3],
            [core::f32::consts::FRAC_PI_2, 0.0, 0.0],
        );
    }
    k.part(g, Shape::Box, &marble, [0.0, 6.35, 0.0], [1.4, 0.3, 1.4], NO_ROT);
    let orb = k.part(g, Shape::Sphere, &gold, [0.0, 7.0, 0.0], [0.45; 3], NO_ROT);
    let ph = k.rnd() * 6.0;
    k.tick(move |t, tx| {
        tx.set(
            orb,
            Transform::from_xyz(0.0, 7.1 + (t * 1.2 + ph).sin() * 0.15, 0.0).with_scale(Vec3::splat(0.45)),
        )
    });
}

pub(super) fn crown(k: &mut Kit, g: Entity) {
    let spin = k.group(
        Some(g),
        Transform::from_xyz(0.0, 2.0, 0.0).with_rotation(Quat::from_rotation_x(0.15)),
    );
    let gold = k.plain_with(rgb(0xffcf3f), Kind::Gold, |s| {
        s.metallic = Some(0.85);
        s.roughness = Some(0.2);
    });
    k.part(spin, Shape::Cyl, &gold, [0.0; 3], [2.0, 0.7, 2.0], NO_ROT);
    let red = k.col(Swatch::Red);
    let velvet = k.plain(red, Kind::Fabric);
    k.part(spin, Shape::Cyl, &velvet, [0.0, 0.1, 0.0], [1.9, 0.72, 1.9], NO_ROT);
    let (blue, pink, teal) = (k.col(Swatch::Blue), k.col(Swatch::Pink), k.col(Swatch::Teal));
    let (gb, gp, gt) = (k.glow(blue, 1.0), k.glow(pink, 1.0), k.glow(teal, 1.0));
    for i in 0..7 {
        let a = i as f32 / 7.0 * core::f32::consts::TAU;
        let (c, s) = (a.cos(), a.sin());
        k.part(
            spin,
            Shape::Cone,
            &gold,
            [c * 1.85, 0.95, s * 1.85],
            [0.35, 1.2, 0.35],
            NO_ROT,
        );
        let gem = if i % 2 == 1 { &gb } else { &gp };
        k.part(spin, Shape::Sphere, gem, [c * 1.85, 1.6, s * 1.85], [0.16; 3], NO_ROT);
        k.part(
            spin,
            Shape::Octa,
            &gt,
            [c * 2.02, 0.0, s * 2.02],
            [0.14, 0.2, 0.14],
            NO_ROT,
        );
    }
    let ph = k.rnd() * 6.0;
    k.tick(move |t, tx| {
        tx.set(
            spin,
            tf(
                [0.0, 2.0 + (t * 0.6 + ph).sin() * 0.4, 0.0],
                [1.0; 3],
                [0.15, t * 0.25, 0.0],
            ),
        )
    });
}
