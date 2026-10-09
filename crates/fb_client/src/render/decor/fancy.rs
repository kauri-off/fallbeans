//! Fanciful set pieces: crystals, planets and orbs, balloons, neon rings, shards and sweets.
use super::*;

fn crystals(k: &mut Kit, g: Entity, glowing: bool) {
    let base = Transform::from_xyz(0.0, 2.0, 0.0);
    let spin = k.group(Some(g), base);
    let n = 3 + (k.rnd() * 3.0) as usize;
    for i in 0..n {
        let mat = if glowing {
            let c = k.bright();
            k.glow(c, 0.9)
        } else {
            k.plain_with(rgb(0xcdeeff), Kind::Ice, |s| {
                s.roughness = Some(0.35);
                s.emissive = color(rgb(0x9fd8ff)).to_linear() * 0.25;
            })
        };
        let a = i as f32 / n as f32 * core::f32::consts::TAU;
        let r = if i > 0 { 0.8 } else { 0.0 };
        let s = if i > 0 { 0.35 + k.rnd() * 0.25 } else { 0.55 };
        let tilt = (k.rnd() - 0.5) * 0.5;
        k.part(
            spin,
            Shape::Octa,
            &mat,
            [a.cos() * r, 0.0, a.sin() * r],
            [s, s * 2.6, s],
            [0.0, a, tilt],
        );
    }
    let sp = 0.2 + k.rnd() * 0.3;
    let ph = k.rnd() * 6.0;
    k.tick(move |t, tx| {
        tx.set(
            spin,
            Transform::from_xyz(0.0, 2.0 + (t * 0.8 + ph).sin() * 0.4, 0.0)
                .with_rotation(Quat::from_rotation_y(t * sp)),
        );
    });
}

pub(super) fn crystals_cold(k: &mut Kit, g: Entity) {
    crystals(k, g, false);
}

pub(super) fn crystals_lit(k: &mut Kit, g: Entity) {
    crystals(k, g, true);
}

pub(super) fn planet(k: &mut Kit, g: Entity) {
    let body = k.group(Some(g), Transform::from_xyz(0.0, 3.0, 0.0));
    let c = k.bright();
    let white = k.col(Swatch::White);
    let m = k.stripes(c, white, 0.9, [0.0, 1.0], Pattern::Waves);
    k.part(body, Shape::Sphere, &m, [0.0; 3], [2.4; 3], NO_ROT);
    let tilt = k.group(
        Some(body),
        Transform::from_rotation(Quat::from_euler(EulerRot::XYZ, 0.5, 0.0, 0.3)),
    );
    let c1 = k.bright();
    let g1 = k.glow(c1, 0.85);
    let g2 = k.glow(white, 0.6);
    let flat_x = [core::f32::consts::FRAC_PI_2, 0.0, 0.0];
    k.part(tilt, Shape::Ring, &g1, [0.0; 3], [3.8, 3.8, 1.0], flat_x);
    k.part(tilt, Shape::Ring, &g2, [0.0; 3], [4.4, 4.4, 1.0], flat_x);
    let sp = 0.1 + k.rnd() * 0.15;
    k.tick(move |t, tx| {
        tx.set(
            body,
            Transform::from_xyz(0.0, 3.0, 0.0).with_rotation(Quat::from_rotation_y(t * sp)),
        )
    });
}

pub(super) fn orbs(k: &mut Kit, g: Entity) {
    let n = 3 + (k.rnd() * 3.0) as usize;
    let mut list = Vec::new();
    for _ in 0..n {
        let c = k.bright();
        let o = k.group(Some(g), Transform::default());
        let a = k.glow(c, 1.0);
        let b = k.glow(c, 0.25);
        k.part(o, Shape::Sphere, &a, [0.0; 3], [0.45; 3], NO_ROT);
        k.part(o, Shape::Sphere, &b, [0.0; 3], [0.8; 3], NO_ROT);
        let (x, z, y, ph) = (
            (k.rnd() - 0.5) * 4.0,
            (k.rnd() - 0.5) * 4.0,
            1.0 + k.rnd() * 3.0,
            k.rnd() * 6.0,
        );
        list.push((o, x, y, z, ph));
    }
    k.tick(move |t, tx| {
        for (o, x, y, z, ph) in &list {
            tx.set(
                *o,
                Transform::from_xyz(x + (t * 0.4 + ph).sin() * 0.4, y + (t * 0.9 + ph).sin() * 0.5, *z),
            );
        }
    });
}

pub(super) fn balloon_bunch(k: &mut Kit, g: Entity) {
    let sway = k.group(Some(g), Transform::default());
    let string = k.plain(rgb(0xffffff), Kind::Fabric);
    for i in 0..7 {
        let a = i as f32 / 7.0 * core::f32::consts::TAU;
        let r = 0.6 + k.rnd() * 0.6;
        let (x, z) = (a.cos() * r, a.sin() * r);
        let y = 3.6 + k.rnd() * 1.4;
        let c = k.bright();
        let m = k.plain_with(c, Kind::Rubber, |s| s.roughness = Some(0.25));
        k.part(sway, Shape::Sphere, &m, [x, y, z], [0.55, 0.68, 0.55], NO_ROT);
        let v = Vec3::new(x, y, z);
        let s = k.part(
            sway,
            Shape::Cyl,
            &string,
            (v / 2.0).to_array(),
            [0.015, v.length(), 0.015],
            NO_ROT,
        );
        k.set_tf(s, |t| t.rotation = Quat::from_rotation_arc(Vec3::Y, v.normalize()));
    }
    let ph = k.rnd() * 6.0;
    k.tick(move |t, tx| {
        tx.set(
            sway,
            Transform::from_rotation(Quat::from_euler(
                EulerRot::XYZ,
                (t * 0.7 + ph).sin() * 0.08,
                t * 0.1,
                (t * 0.6 + ph).cos() * 0.08,
            )),
        )
    });
}

pub(super) fn neon_rings(k: &mut Kit, g: Entity) {
    let mut rings = Vec::new();
    for i in 0..3 {
        let c = k.bright();
        let m = k.glow(c, 1.0);
        let s = 2.8 - i as f32 * 0.6;
        rings.push((k.part(g, Shape::Ring, &m, [0.0, 3.5, 0.0], [s; 3], NO_ROT), s));
    }
    let white = k.col(Swatch::White);
    let w = k.glow(white, 1.0);
    k.part(g, Shape::Sphere, &w, [0.0, 3.5, 0.0], [0.4; 3], NO_ROT);
    let sp = 0.4 + k.rnd() * 0.5;
    k.tick(move |t, tx| {
        for (i, (r, s)) in rings.iter().enumerate() {
            let i = i as f32;
            tx.set(
                *r,
                tf(
                    [0.0, 3.5, 0.0],
                    [*s; 3],
                    [t * sp * (i + 1.0) * 0.6, t * sp * (1.4 - i * 0.3), i],
                ),
            );
        }
    });
}

pub(super) fn shards(k: &mut Kit, g: Entity) {
    let obsidian = k.plain_with(rgb(0x241a2a), Kind::Glass, |s| {
        s.roughness = Some(0.15);
        s.metallic = Some(0.3);
    });
    for i in 0..5 {
        let a = k.rnd() * 6.3;
        let r = if i > 0 { 0.6 + k.rnd() * 1.2 } else { 0.0 };
        let h = 1.5 + k.rnd() * 3.0;
        let rx = (k.rnd() - 0.5) * 0.3;
        let rz = (k.rnd() - 0.5) * 0.3;
        k.part(
            g,
            Shape::Cone4,
            &obsidian,
            [a.cos() * r, h / 2.0, a.sin() * r],
            [0.4, h, 0.4],
            [rx, a, rz],
        );
    }
    let glow = k.glow(rgb(0xff7a2a), 0.8);
    k.part(g, Shape::Sphere, &glow, [0.0, 0.1, 0.0], [1.4, 0.1, 1.4], NO_ROT);
}

pub(super) fn lollipop(k: &mut Kit, g: Entity) {
    let h = 3.5 + k.rnd() * 1.5;
    let stick = k.plain(rgb(0xffffff), Kind::Plastic);
    k.part(g, Shape::Cyl, &stick, [0.0, h / 2.0, 0.0], [0.09, h, 0.09], NO_ROT);
    let c = k.bright();
    let p = k.pick(&[Pattern::Waves, Pattern::Stripes, Pattern::Chevron]);
    let candy = k.pattern(c, rgb(0xffffff), 2.6, [1.0, 1.0], Kind::Glossy, p);
    let at = [0.0, h + 1.2, 0.0];
    let disc = k.part(
        g,
        Shape::Cyl,
        &candy,
        at,
        [1.3, 0.35, 1.3],
        [core::f32::consts::FRAC_PI_2, 0.0, 0.0],
    );
    let ph = k.rnd() * 6.0;
    k.tick(move |t, tx| {
        tx.set(
            disc,
            tf(
                at,
                [1.3, 0.35, 1.3],
                [core::f32::consts::FRAC_PI_2, (t * 0.5 + ph).sin() * 0.4, 0.0],
            ),
        )
    });
}

pub(super) fn cane(k: &mut Kit, g: Entity) {
    let red = k.col(Swatch::Red);
    let stripes = k.pattern(red, rgb(0xffffff), 2.2, [1.0, 1.6], Kind::Glossy, Pattern::Stripes);
    k.part(g, Shape::Cyl, &stripes, [0.0, 2.2, 0.0], [0.28, 4.4, 0.28], NO_ROT);
    k.part(g, Shape::HalfTorus, &stripes, [0.7, 4.4, 0.0], [1.0; 3], NO_ROT);
    let yaw = k.rnd() * 6.3;
    k.set_tf(g, |t| t.rotation = Quat::from_rotation_y(yaw));
}

pub(super) fn donut(k: &mut Kit, g: Entity) {
    let tilt = 0.4 + k.rnd() * 0.5;
    let spin = k.group(
        Some(g),
        Transform::from_xyz(0.0, 1.5, 0.0).with_rotation(Quat::from_rotation_x(tilt)),
    );
    let dough = k.plain(rgb(0xe8b878), Kind::Plastic);
    let flat_x = [core::f32::consts::FRAC_PI_2, 0.0, 0.0];
    k.part(spin, Shape::Torus, &dough, [0.0; 3], [1.4; 3], flat_x);
    let c = k.bright();
    let icing = k.plain_with(c, Kind::Glossy, |s| s.roughness = Some(0.3));
    k.part(spin, Shape::Torus, &icing, [0.0, 0.14, 0.0], [1.42, 1.42, 1.2], flat_x);
    for _ in 0..9 {
        let a = k.rnd() * 6.3;
        let r = 1.1 + k.rnd() * 0.6;
        let c = k.bright();
        let m = k.plain(c, Kind::Plastic);
        let yaw = k.rnd() * 3.0;
        k.part(
            spin,
            Shape::Box,
            &m,
            [a.cos() * r, 0.55, a.sin() * r],
            [0.06, 0.06, 0.2],
            [0.0, yaw, 0.0],
        );
    }
    let ph = k.rnd() * 6.0;
    k.tick(move |t, tx| {
        tx.set(
            spin,
            tf(
                [0.0, 1.5 + (t * 0.7 + ph).sin() * 0.3, 0.0],
                [1.0; 3],
                [tilt, 0.0, t * 0.3],
            ),
        )
    });
}

pub(super) fn gumdrops(k: &mut Kit, g: Entity) {
    for i in 0..6 {
        let a = k.rnd() * 6.3;
        let r = if i > 0 { 0.8 + k.rnd() * 1.6 } else { 0.0 };
        let s = 0.45 + k.rnd() * 0.4;
        let c = k.bright();
        let m = k.plain_with(c, Kind::Glossy, |sp| sp.roughness = Some(0.2));
        k.part(
            g,
            Shape::Dome,
            &m,
            [a.cos() * r, 0.0, a.sin() * r],
            [s, s * 1.5, s],
            NO_ROT,
        );
    }
}
