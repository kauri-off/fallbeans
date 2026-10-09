//! Set pieces that grow or lie about: trees, flowers, snow, palms, cacti, mesas, volcanoes and rocks.
use super::*;

fn flora(k: &mut Kit, g: Entity) {
    let n = 2 + (k.rnd() * 2.0) as usize;
    for i in 0..n {
        let a = k.rnd() * 6.3;
        let r = if i > 0 { 1.2 + k.rnd() * 1.2 } else { 0.0 };
        let name = k.pick(&[Model::Tree, Model::Pine, Model::Tree, Model::Mushroom]);
        let s = 0.55 + k.rnd() * 0.3;
        k.model(name, g, [a.cos() * r, 0.0, a.sin() * r], s, None);
    }
}

pub(super) fn grove(k: &mut Kit, g: Entity) {
    flora(k, g);
}

pub(super) fn flowers(k: &mut Kit, g: Entity) {
    let stem = k.plain(rgb(0x4fae4a), Kind::Leaf);
    let yellow = k.col(Swatch::Yellow);
    let heart = k.plain(yellow, Kind::Plastic);
    for _ in 0..9 {
        let a = k.rnd() * 6.3;
        let r = 0.4 + k.rnd() * 2.2;
        let h = 0.6 + k.rnd() * 0.9;
        let (x, z) = (a.cos() * r, a.sin() * r);
        k.part(g, Shape::Cyl, &stem, [x, h / 2.0, z], [0.05, h, 0.05], NO_ROT);
        let c = k.bright();
        let head = k.plain(c, Kind::Fabric);
        for p in 0..5 {
            let pa = p as f32 / 5.0 * core::f32::consts::TAU;
            k.part(
                g,
                Shape::Sphere,
                &head,
                [x + pa.cos() * 0.2, h, z + pa.sin() * 0.2],
                [0.18, 0.07, 0.18],
                NO_ROT,
            );
        }
        k.part(g, Shape::Sphere, &heart, [x, h + 0.03, z], [0.11, 0.08, 0.11], NO_ROT);
    }
}

pub(super) fn snow_pine(k: &mut Kit, g: Entity) {
    // Pines under snow: the needles frosted pale (the tiers' drooping tips read as snow-laden).
    for i in 0..2 {
        let a = k.rnd() * 6.3;
        let r = if i > 0 { 1.4 } else { 0.0 };
        let s = 0.7 + k.rnd() * 0.4;
        let frost = if i > 0 { rgb(0xd4ece6) } else { rgb(0xe6f5f2) };
        k.model_painted(
            Model::Pine,
            g,
            [a.cos() * r, 0.0, a.sin() * r],
            s,
            vec![("Pine", frost, 0.0)],
        );
    }
}

pub(super) fn snowman(k: &mut Kit, g: Entity) {
    let snow = k.plain(rgb(0xffffff), Kind::Cloth);
    k.part(g, Shape::Sphere, &snow, [0.0, 0.9, 0.0], [1.0, 0.95, 1.0], NO_ROT);
    k.part(g, Shape::Sphere, &snow, [0.0, 2.2, 0.0], [0.72, 0.7, 0.72], NO_ROT);
    k.part(g, Shape::Sphere, &snow, [0.0, 3.15, 0.0], [0.52, 0.5, 0.52], NO_ROT);
    let coal = k.plain(rgb(0x2a2a33), Kind::Plastic);
    for sx in [-1.0, 1.0] {
        k.part(g, Shape::Sphere, &coal, [sx * 0.18, 3.28, 0.44], [0.06; 3], NO_ROT);
    }
    for i in 0..3 {
        k.part(
            g,
            Shape::Sphere,
            &coal,
            [0.0, 2.5 - i as f32 * 0.3, 0.68],
            [0.07; 3],
            NO_ROT,
        );
    }
    let nose = k.plain(rgb(0xff8a3d), Kind::Plastic);
    k.part(
        g,
        Shape::Cone,
        &nose,
        [0.0, 3.15, 0.62],
        [0.08, 0.45, 0.08],
        [core::f32::consts::FRAC_PI_2, 0.0, 0.0],
    );
    k.part(g, Shape::Cyl, &coal, [0.0, 3.62, 0.0], [0.55, 0.06, 0.55], NO_ROT);
    k.part(g, Shape::Cyl, &coal, [0.0, 3.9, 0.0], [0.36, 0.55, 0.36], NO_ROT);
    let red = k.col(Swatch::Red);
    let scarf = k.plain(red, Kind::Fabric);
    k.part(
        g,
        Shape::Torus,
        &scarf,
        [0.0, 2.72, 0.0],
        [0.5, 0.5, 0.35],
        [core::f32::consts::FRAC_PI_2, 0.0, 0.0],
    );
    let yaw = k.rnd() * 6.3;
    k.set_tf(g, |t| t.rotation = Quat::from_rotation_y(yaw));
}

pub(super) fn palm(k: &mut Kit, g: Entity) {
    let bark = k.stripes(rgb(0xb88a5a), rgb(0x9a6f45), 3.0, [0.0, 1.0], Pattern::Stripes);
    let lean = (k.rnd() - 0.5) * 0.5;
    let (mut x, mut y) = (0.0f32, 0.0f32);
    for i in 0..7 {
        let f = i as f32;
        x += lean * 0.3 * (f / 3.0);
        k.part(
            g,
            Shape::Cyl,
            &bark,
            [x, y + 0.4, 0.0],
            [0.26 - f * 0.015, 0.82, 0.26 - f * 0.015],
            [0.0, 0.0, -lean * 0.3 * (f / 3.0)],
        );
        y += 0.78;
    }
    let lc = if k.look.look.id == LookId::Jungle {
        rgb(0x2fae4a)
    } else {
        rgb(0x4fcf5a)
    };
    let leaf = k.plain(lc, Kind::Leaf);
    for i in 0..7 {
        let a = i as f32 / 7.0 * core::f32::consts::TAU;
        let l = k.group(
            Some(g),
            Transform::from_xyz(x, y, 0.0).with_rotation(Quat::from_rotation_y(a)),
        );
        k.part(
            l,
            Shape::Sphere,
            &leaf,
            [1.3, -0.35, 0.0],
            [1.4, 0.08, 0.35],
            [0.0, 0.0, -0.45],
        );
    }
    let nut = k.plain(rgb(0x7a5030), Kind::Plastic);
    for i in 0..3 {
        let a = i as f32 * 2.1;
        k.part(
            g,
            Shape::Sphere,
            &nut,
            [x + a.cos() * 0.3, y - 0.3, a.sin() * 0.3],
            [0.2; 3],
            NO_ROT,
        );
    }
}

pub(super) fn beach(k: &mut Kit, g: Entity) {
    let c = k.bright();
    let pole = k.plain(rgb(0xffffff), Kind::Plastic);
    k.part(g, Shape::Cyl, &pole, [0.0, 1.3, 0.0], [0.05, 2.6, 0.05], NO_ROT);
    let shade = k.pattern(c, rgb(0xffffff), 3.0, [1.0, 0.0], Kind::Cloth, Pattern::Stripes);
    k.part(g, Shape::Cone, &shade, [0.0, 2.75, 0.0], [1.6, 0.6, 1.6], NO_ROT);
    let c2 = k.bright();
    let towel = k.stripes(c2, rgb(0xffffff), 2.0, [1.0, 0.0], Pattern::Stripes);
    k.part(g, Shape::Box, &towel, [0.6, 0.03, 1.2], [1.0, 0.04, 1.9], NO_ROT);
    let c3 = k.bright();
    let ball = k.pattern(c3, rgb(0xffffff), 2.2, [1.0, 0.0], Kind::Rubber, Pattern::Stripes);
    k.part(g, Shape::Sphere, &ball, [-1.3, 0.35, 0.8], [0.35; 3], NO_ROT);
    let yaw = k.rnd() * 6.3;
    k.set_tf(g, |t| t.rotation = Quat::from_rotation_y(yaw));
}

pub(super) fn cactus(k: &mut Kit, g: Entity) {
    let green = k.stripes(rgb(0x5a9a4a), rgb(0x6aae56), 5.0, [1.0, 0.0], Pattern::Stripes);
    let h = 3.2 + k.rnd() * 1.5;
    k.part(g, Shape::Cyl, &green, [0.0, h / 2.0, 0.0], [0.45, h, 0.45], NO_ROT);
    k.part(g, Shape::Sphere, &green, [0.0, h, 0.0], [0.45; 3], NO_ROT);
    for side in [-1.0, 1.0] {
        let ay = 1.2 + k.rnd() * 1.2;
        let up = 0.8 + k.rnd() * 0.8;
        k.part(
            g,
            Shape::Cyl,
            &green,
            [side * 0.6, ay, 0.0],
            [0.26, 0.7, 0.26],
            [0.0, 0.0, core::f32::consts::FRAC_PI_2],
        );
        k.part(
            g,
            Shape::Cyl,
            &green,
            [side * 0.9, ay + up / 2.0, 0.0],
            [0.26, up, 0.26],
            NO_ROT,
        );
        k.part(g, Shape::Sphere, &green, [side * 0.9, ay + up, 0.0], [0.26; 3], NO_ROT);
    }
    let pink = k.col(Swatch::Pink);
    let flower = k.plain(pink, Kind::Fabric);
    k.part(g, Shape::Sphere, &flower, [0.0, h + 0.4, 0.0], [0.2, 0.15, 0.2], NO_ROT);
    let yaw = k.rnd() * 6.3;
    k.set_tf(g, |t| t.rotation = Quat::from_rotation_y(yaw));
}

pub(super) fn mesa(k: &mut Kit, g: Entity) {
    let layers = [
        rgb(0xc8764a),
        rgb(0xe0a070),
        rgb(0xb8603a),
        rgb(0xe8b888),
        rgb(0xa8503a),
    ];
    let mut y = 0.0;
    for (i, c) in layers.into_iter().enumerate() {
        let r = 4.0 - i as f32 * 0.35 - k.rnd() * 0.2;
        let h = 0.8 + k.rnd() * 0.6;
        let m = k.plain(c, Kind::Rock);
        k.part(g, Shape::Cyl, &m, [0.0, y - h / 2.0, 0.0], [r, h, r], NO_ROT);
        y -= h;
    }
    let rock = k.plain(rgb(0xa8503a), Kind::Rock);
    k.part(
        g,
        Shape::Cone,
        &rock,
        [0.0, y - 1.2, 0.0],
        [3.0, 2.4, 3.0],
        [core::f32::consts::PI, 0.0, 0.0],
    );
    let red = k.col(Swatch::Red);
    k.model(Model::Flag, g, [0.0; 3], 0.5, Some(red));
}

pub(super) fn big_plant(k: &mut Kit, g: Entity) {
    let leaf = k.stripes(rgb(0x2f9f3f), rgb(0x48b858), 2.0, [1.0, 0.0], Pattern::Stripes);
    for i in 0..7 {
        let yaw = i as f32 / 7.0 * core::f32::consts::TAU + k.rnd() * 0.3;
        let l = k.group(Some(g), Transform::from_rotation(Quat::from_rotation_y(yaw)));
        let tilt = 0.5 + k.rnd() * 0.3;
        k.part(
            l,
            Shape::Sphere,
            &leaf,
            [1.1, 0.9, 0.0],
            [1.3, 0.1, 0.45],
            [0.0, 0.0, tilt],
        );
    }
    let c = k.bright();
    let petal = k.plain(c, Kind::Fabric);
    for p in 0..6 {
        let a = p as f32 / 6.0 * core::f32::consts::TAU;
        k.part(
            g,
            Shape::Sphere,
            &petal,
            [a.cos() * 0.45, 2.2, a.sin() * 0.45],
            [0.42, 0.1, 0.25],
            [0.0, -a, 0.25],
        );
    }
    let y = k.col(Swatch::Yellow);
    let heart = k.plain(y, Kind::Plastic);
    k.part(g, Shape::Sphere, &heart, [0.0, 2.25, 0.0], [0.22, 0.18, 0.22], NO_ROT);
    let stem = k.plain(rgb(0x3f8f3a), Kind::Leaf);
    k.part(g, Shape::Cyl, &stem, [0.0, 1.1, 0.0], [0.08, 2.2, 0.08], NO_ROT);
}

pub(super) fn volcano(k: &mut Kit, g: Entity) {
    let rock = k.plain(rgb(0x3a2a2a), Kind::Rock);
    k.part(g, Shape::Taper, &rock, [0.0, 1.6, 0.0], [4.0, 3.2, 4.0], NO_ROT);
    k.part(
        g,
        Shape::Cone,
        &rock,
        [0.0, -1.5, 0.0],
        [4.0, 3.0, 4.0],
        [core::f32::consts::PI, 0.0, 0.0],
    );
    let crater = k.glow(rgb(0xff7a2a), 1.0);
    k.part(g, Shape::Cyl, &crater, [0.0, 3.22, 0.0], [2.3, 0.05, 2.3], NO_ROT);
    let lava = k.glow(rgb(0xffb030), 0.9);
    for _ in 0..3 {
        let a = k.rnd() * 6.3;
        k.part(
            g,
            Shape::Box,
            &lava,
            [a.cos() * 2.95, 1.6, a.sin() * 2.95],
            [0.25, 3.0, 0.05],
            [0.3, -a + core::f32::consts::FRAC_PI_2, 0.0],
        );
    }
    let smoke = k.plain_with(rgb(0x5a4a4a), Kind::Cloud, |s| {
        s.color.alpha = 0.6;
        s.alpha = AlphaMode::Blend;
    });
    puffs(k, g, &smoke, 3.4, 7.0, Vec3::new(2.0, 0.0, -1.0), (0.8, 2.2), 0.18);
}

pub(super) fn rocks(k: &mut Kit, g: Entity) {
    let dark = k.plain(rgb(0x2e2426), Kind::Rock);
    let first = k.plain(rgb(0x4a2e2a), Kind::Rock);
    let ember = k.glow(rgb(0xff8a3a), 1.0);
    let mut list = Vec::new();
    for i in 0..4 {
        let at = [(k.rnd() - 0.5) * 4.0, 1.0 + k.rnd() * 3.0, (k.rnd() - 0.5) * 4.0];
        let size = [0.6 + k.rnd() * 0.9, 0.6 + k.rnd() * 0.9, 0.6 + k.rnd() * 0.9];
        let m = if i > 0 { &dark } else { &first };
        let o = k.part(g, Shape::Rock, m, at, size, NO_ROT);
        k.part(o, Shape::Octa, &ember, [0.0; 3], [0.3; 3], NO_ROT);
        let ph = k.rnd() * 6.0;
        list.push((o, at, size, ph));
    }
    k.tick(move |t, tx| {
        for (o, at, size, ph) in &list {
            tx.set(
                *o,
                tf(
                    [at[0], at[1] + (t * 0.5 + ph).sin() * 0.3, at[2]],
                    *size,
                    [0.0, t * 0.1 + ph, 0.0],
                ),
            );
        }
    });
}
