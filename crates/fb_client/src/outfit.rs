//! Hats and glasses: trees of simple parts in the bean model's space
//! (head sphere r 0.5 around (0, 1.1, 0), face along +z, eyes at x ±0.115, y 1.23).
use core::f32::consts::{FRAC_PI_2, PI, TAU};
use std::collections::HashMap;

use bevy::ecs::system::SystemParam;
use bevy::light::NotShadowCaster;
use bevy::prelude::*;
use fb_shared::outfit::{Glasses, Hat};

use crate::render::quality::{Preset, Quality};
use crate::shapes;

/// Tipped back a little so that the front of a hat clears the visor.
const HAT_Y: f32 = 1.42;
const HAT_TILT: f32 = -0.1;
const EYE_X: f32 = 0.12;
const EYE_Y: f32 = 1.235;
/// Just in front of the eyes (their glints reach z ≈ 0.61).
const LENS_Z: f32 = 0.63;

const BLUE: Color = Color::srgb_u8(0x3f, 0xa9, 0xff);
const PINK: Color = Color::srgb_u8(0xff, 0x5f, 0xa2);
const EAR_PINK: Color = Color::srgb_u8(0xff, 0xb3, 0xcf);
const YELLOW: Color = Color::srgb_u8(0xff, 0xd2, 0x3f);
const CREAM: Color = Color::srgb_u8(0xff, 0xf4, 0xd6);
const RED: Color = Color::srgb_u8(0xff, 0x3b, 0x3b);
const GREEN: Color = Color::srgb_u8(0x4f, 0xdc, 0x6a);
const TEAL: Color = Color::srgb_u8(0x39, 0xe0, 0xd0);
const BROWN: Color = Color::srgb_u8(0x8b, 0x5a, 0x2b);
const GREY: Color = Color::srgb_u8(0x9e, 0xa3, 0xb0);
const DARK: Color = Color::srgb_u8(0x2b, 0x2b, 0x33);
const FRAME_BLACK: Color = Color::srgb_u8(0x15, 0x15, 0x1c);
const LENS: Color = Color::srgb_u8(0x1d, 0x22, 0x33);

/// A mesh by its recipe (the key of the mesh cache).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Shape {
    Dome(f32, f32),
    Ball(f32),
    Cyl(f32, f32, f32, u32),
    /// A part of a cylinder's side, from angle `start` round `length`; `open`: without its caps.
    CylPart {
        top: f32,
        bottom: f32,
        height: f32,
        segments: u32,
        open: bool,
        start: f32,
        length: f32,
    },
    Cone(f32, f32, u32),
    Torus(f32, f32, u32, u32),
    Box(f32, f32, f32),
    Circle(f32, u32),
    Heart(f32),
    CowboyBrim,
    CowboyCrown,
}

impl Shape {
    fn mesh(self) -> Mesh {
        match self {
            Shape::Dome(r, h) => shapes::dome(r, h),
            Shape::Ball(r) => shapes::ball(r),
            Shape::Cyl(rt, rb, h, n) => shapes::cylinder(rt, rb, h, n, false, (0.0, TAU)),
            Shape::CylPart {
                top,
                bottom,
                height,
                segments,
                open,
                start,
                length,
            } => shapes::cylinder(top, bottom, height, segments, open, (start, length)),
            Shape::Cone(r, h, n) => shapes::cone(r, h, n),
            Shape::Torus(r, t, a, b) => shapes::torus(r, t, a, b),
            Shape::Box(x, y, z) => Cuboid::new(x, y, z).into(),
            Shape::Circle(r, n) => shapes::circle(r, n),
            Shape::Heart(d) => shapes::heart(d),
            Shape::CowboyBrim => shapes::lathe(&[(0.28, 0.0), (0.48, 0.01), (0.58, 0.05), (0.63, 0.11)], 40),
            Shape::CowboyCrown => {
                shapes::lathe(&[(0.31, 0.0), (0.31, 0.2), (0.27, 0.3), (0.12, 0.33), (0.0, 0.29)], 32)
            }
        }
    }
}

/// A material by its options; `suit` takes the bean's own suit material.
#[derive(Clone, Debug, PartialEq)]
pub struct Mat {
    pub color: Color,
    pub rough: f32,
    pub metal: f32,
    pub glow: f32,
    pub both: bool,
    pub suit: bool,
}

fn mat(color: Color) -> Mat {
    Mat {
        color,
        rough: 0.6,
        metal: 0.0,
        glow: 0.0,
        both: false,
        suit: false,
    }
}

impl Mat {
    fn rough(mut self, r: f32) -> Self {
        self.rough = r;
        self
    }
    fn metal(mut self, m: f32) -> Self {
        self.metal = m;
        self
    }
    fn glow(mut self, g: f32) -> Self {
        self.glow = g;
        self
    }
    fn both(mut self) -> Self {
        self.both = true;
        self
    }

    fn key(&self) -> MatKey {
        let c = self.color.to_srgba();
        MatKey {
            color: [c.red, c.green, c.blue, c.alpha].map(f32::to_bits),
            rough: self.rough.to_bits(),
            metal: self.metal.to_bits(),
            glow: self.glow.to_bits(),
            both: self.both,
            suit: self.suit,
        }
    }
}

/// A `Mat` as `Wardrobe`'s cache tells materials apart.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
struct MatKey {
    color: [u32; 4],
    rough: u32,
    metal: u32,
    glow: u32,
    both: bool,
    suit: bool,
}

/// Parts that move: driven every frame by the bean's time and speed.
#[derive(Component, Clone, Copy, Debug, PartialEq)]
pub enum Wiggle {
    Rotor,
    BunnyEar(usize),
    Halo,
    Flower,
    Antenna(usize),
}

/// One node of an accessory: a group, or a mesh.
#[derive(Clone, Debug)]
pub struct Part {
    pub tf: Transform,
    pub look: Option<(Shape, Mat)>,
    pub wiggle: Option<Wiggle>,
    pub kids: Vec<Part>,
}

fn group(pos: Vec3) -> Part {
    Part {
        tf: Transform::from_translation(pos),
        look: None,
        wiggle: None,
        kids: Vec::new(),
    }
}

fn part(shape: Shape, m: Mat, x: f32, y: f32, z: f32) -> Part {
    Part {
        look: Some((shape, m)),
        ..group(Vec3::new(x, y, z))
    }
}

impl Part {
    fn rot(mut self, x: f32, y: f32, z: f32) -> Self {
        self.tf.rotation = Quat::from_euler(EulerRot::XYZ, x, y, z);
        self
    }
    fn scale(mut self, x: f32, y: f32, z: f32) -> Self {
        self.tf.scale = Vec3::new(x, y, z);
        self
    }
    fn wiggle(mut self, w: Wiggle) -> Self {
        self.wiggle = Some(w);
        self
    }
    fn with(mut self, kids: impl IntoIterator<Item = Part>) -> Self {
        self.kids.extend(kids);
        self
    }
}

/// A second colour that stands out against `c` (bands, trims).
fn contrast(c: Color) -> Color {
    if Hsla::from(c).lightness < 0.35 { RED } else { DARK }
}

/// An accessory and how far above its usual place a winner's crown goes (on top of a hat).
pub struct Accessory {
    pub root: Part,
    pub crown_lift: f32,
}

/// A hat: `tint` its colour (None: its own default).
pub fn make_hat(kind: Hat, tint: Option<Color>) -> Option<Accessory> {
    let main = |def: Color| mat(tint.unwrap_or(def));
    let s = [-1.0f32, 1.0];
    let (kids, crown_lift, tilt_z): (Vec<Part>, f32, f32) = match kind {
        Hat::None => return None,
        Hat::Cap => {
            let m = main(BLUE);
            (
                vec![
                    part(Shape::Dome(0.41, 0.225), m.clone(), 0.0, 0.0, 0.0),
                    part(
                        Shape::CylPart {
                            top: 0.3,
                            bottom: 0.3,
                            height: 0.025,
                            segments: 28,
                            open: false,
                            start: -FRAC_PI_2,
                            length: PI,
                        },
                        m,
                        0.0,
                        0.015,
                        0.3,
                    )
                    .rot(-0.12, 0.0, 0.0)
                    .scale(1.0, 1.0, 1.1),
                    part(Shape::Ball(0.035), mat(contrast(tint.unwrap_or(BLUE))), 0.0, 0.22, 0.0),
                ],
                0.16,
                0.0,
            )
        }
        Hat::Beanie => {
            let m = main(PINK);
            (
                vec![
                    part(Shape::Dome(0.42, 0.3), m.clone(), 0.0, 0.0, 0.0),
                    part(Shape::Torus(0.41, 0.06, 10, 36), m, 0.0, 0.03, 0.0).rot(FRAC_PI_2, 0.0, 0.0),
                    part(Shape::Ball(0.1), mat(Color::WHITE).rough(0.9), 0.0, 0.36, 0.0),
                ],
                0.3,
                0.0,
            )
        }
        Hat::Party => {
            let c = tint.unwrap_or(YELLOW);
            (
                vec![
                    part(Shape::Cone(0.19, 0.5, 24), mat(c), 0.0, 0.33, 0.0),
                    part(Shape::Torus(0.15, 0.03, 8, 24), mat(contrast(c)), 0.0, 0.22, 0.0).rot(FRAC_PI_2, 0.0, 0.0),
                    part(Shape::Ball(0.06), mat(Color::WHITE).rough(0.9), 0.0, 0.6, 0.0),
                ],
                0.05,
                0.25,
            )
        }
        Hat::Tophat => {
            let c = tint.unwrap_or(DARK);
            let m = mat(c).rough(0.45);
            (
                vec![
                    part(Shape::Cyl(0.42, 0.42, 0.03, 36), m.clone(), 0.0, 0.03, 0.0),
                    part(Shape::Cyl(0.27, 0.29, 0.45, 32), m, 0.0, 0.255, 0.0),
                    part(Shape::Cyl(0.295, 0.295, 0.08, 32), mat(contrast(c)), 0.0, 0.1, 0.0),
                ],
                0.42,
                0.0,
            )
        }
        Hat::Cowboy => {
            let c = tint.unwrap_or(BROWN);
            let m = mat(c).rough(0.8).both();
            (
                vec![
                    part(Shape::CowboyBrim, m.clone(), 0.0, 0.02, 0.0).scale(1.0, 1.0, 0.85),
                    part(Shape::CowboyCrown, m, 0.0, 0.02, 0.0).scale(1.0, 1.0, 0.9),
                    part(
                        Shape::CylPart {
                            top: 0.315,
                            bottom: 0.315,
                            height: 0.06,
                            segments: 32,
                            open: true,
                            start: 0.0,
                            length: TAU,
                        },
                        mat(contrast(c)).both(),
                        0.0,
                        0.08,
                        0.0,
                    )
                    .scale(1.0, 1.0, 0.9),
                ],
                0.28,
                0.0,
            )
        }
        Hat::Viking => {
            let m = mat(tint.unwrap_or(GREY)).rough(0.35).metal(0.6);
            let horn = mat(CREAM).rough(0.5);
            let mut v = vec![
                part(Shape::Dome(0.43, 0.3), m, 0.0, 0.0, 0.0),
                part(
                    Shape::Torus(0.42, 0.04, 8, 36),
                    mat(YELLOW).rough(0.35).metal(0.7),
                    0.0,
                    0.01,
                    0.0,
                )
                .rot(FRAC_PI_2, 0.0, 0.0),
            ];
            v.extend(
                s.map(|s| part(Shape::Cone(0.08, 0.34, 16), horn.clone(), s * 0.44, 0.2, 0.0).rot(0.0, 0.0, -s * 0.8)),
            );
            (v, 0.26, 0.0)
        }
        Hat::Propeller => {
            let m = main(GREEN);
            let rotor = group(Vec3::Y * 0.34).wiggle(Wiggle::Rotor).with([
                part(Shape::Box(0.24, 0.012, 0.07), mat(RED), -0.13, 0.0, 0.0).rot(-0.25, 0.0, 0.0),
                part(Shape::Box(0.24, 0.012, 0.07), mat(BLUE), 0.13, 0.0, 0.0).rot(0.25, 0.0, 0.0),
                part(Shape::Ball(0.03), mat(YELLOW), 0.0, 0.0, 0.0),
            ]);
            (
                vec![
                    part(Shape::Dome(0.41, 0.24), m, 0.0, 0.0, 0.0),
                    part(
                        Shape::Cyl(0.015, 0.015, 0.12, 8),
                        mat(GREY).metal(0.6).rough(0.4),
                        0.0,
                        0.28,
                        0.0,
                    ),
                    rotor,
                ],
                0.36,
                0.0,
            )
        }
        Hat::Bunny => {
            let m = tint.map_or_else(|| mat(Color::WHITE).rough(0.85), mat);
            let inner = mat(EAR_PINK).rough(0.85);
            let ears = s.iter().enumerate().map(|(i, &s)| {
                group(Vec3::new(s * 0.13, 0.08, 0.0))
                    .rot(0.0, 0.0, -s * 0.18)
                    .wiggle(Wiggle::BunnyEar(i))
                    .with([
                        part(Shape::Ball(0.1), m.clone(), 0.0, 0.26, 0.0).scale(0.9, 3.0, 0.45),
                        part(Shape::Ball(0.1), inner.clone(), 0.0, 0.26, 0.03).scale(0.5, 2.3, 0.2),
                    ])
            });
            (ears.collect(), 0.0, 0.0)
        }
        Hat::Cat => {
            let m = tint.map_or_else(
                || Mat {
                    suit: true,
                    ..mat(Color::WHITE)
                },
                mat,
            );
            let inner = mat(EAR_PINK).rough(0.85);
            let ears = s.map(|s| {
                group(Vec3::new(s * 0.22, 0.08, 0.0)).rot(0.0, 0.0, -s * 0.45).with([
                    part(Shape::Cone(0.13, 0.24, 20), m.clone(), 0.0, 0.1, 0.0).scale(1.0, 1.0, 0.5),
                    part(Shape::Cone(0.08, 0.16, 16), inner.clone(), 0.0, 0.08, 0.035).scale(1.0, 1.0, 0.3),
                ])
            });
            (ears.into(), 0.0, 0.0)
        }
        Hat::Horns => {
            let m = mat(tint.unwrap_or(RED)).rough(0.4);
            let horns =
                s.map(|s| part(Shape::Cone(0.065, 0.2, 16), m.clone(), s * 0.19, 0.12, 0.06).rot(0.2, 0.0, -s * 0.4));
            (horns.into(), 0.0, 0.0)
        }
        Hat::Halo => (
            vec![
                part(
                    Shape::Torus(0.24, 0.028, 10, 40),
                    mat(tint.unwrap_or(YELLOW)).rough(0.3).glow(0.8),
                    0.0,
                    0.33,
                    0.0,
                )
                .rot(FRAC_PI_2, 0.0, 0.0)
                .wiggle(Wiggle::Halo),
            ],
            0.0,
            0.0,
        ),
        Hat::Flower => {
            let petal = mat(tint.unwrap_or(PINK));
            let mut head =
                group(Vec3::Y * 0.26)
                    .rot(0.35, 0.0, 0.0)
                    .with([part(Shape::Ball(0.05), mat(YELLOW), 0.0, 0.0, 0.02)]);
            for i in 0..6 {
                let a = i as f32 / 6.0 * TAU;
                head.kids.push(
                    part(Shape::Ball(0.055), petal.clone(), a.cos() * 0.08, a.sin() * 0.08, 0.0).scale(1.0, 1.0, 0.4),
                );
            }
            let flower = group(Vec3::new(0.14, 0.06, 0.06)).wiggle(Wiggle::Flower).with([
                part(Shape::Cyl(0.014, 0.014, 0.24, 8), mat(GREEN), 0.0, 0.12, 0.0),
                head,
            ]);
            (vec![flower], 0.0, 0.0)
        }
        Hat::Antenna => {
            let stalk = mat(DARK);
            let tip = mat(tint.unwrap_or(YELLOW)).rough(0.35).glow(0.35);
            let arms = s.iter().enumerate().map(|(i, &s)| {
                group(Vec3::new(s * 0.12, 0.1, 0.02))
                    .rot(0.0, 0.0, -s * 0.3)
                    .wiggle(Wiggle::Antenna(i))
                    .with([
                        part(Shape::Cyl(0.012, 0.012, 0.3, 6), stalk.clone(), 0.0, 0.15, 0.0),
                        part(Shape::Ball(0.055), tip.clone(), 0.0, 0.31, 0.0),
                    ])
            });
            (arms.collect(), 0.0, 0.0)
        }
    };
    Some(Accessory {
        root: group(Vec3::Y * HAT_Y).rot(HAT_TILT, 0.0, tilt_z).with(kids),
        crown_lift,
    })
}

/// The arms of a pair of glasses, from the frames to the sides of the head.
fn temples(m: &Mat, r: f32) -> [Part; 2] {
    [-1.0f32, 1.0].map(|s| {
        let from = Vec3::new(s * (EYE_X + r), EYE_Y, LENS_Z - 0.01);
        let to = Vec3::new(s * 0.49, EYE_Y + 0.02, 0.02);
        let mid = from.lerp(to, 0.5);
        let mut p = part(
            Shape::Box(0.014, 0.018, from.distance(to)),
            m.clone(),
            mid.x,
            mid.y,
            mid.z,
        );
        // Its +z towards the target.
        p.tf.look_to(-(to - mid), Vec3::Y);
        p
    })
}

fn frames(m: &Mat, r: f32) -> Vec<Part> {
    let mut v: Vec<Part> = [-1.0f32, 1.0]
        .map(|s| part(Shape::Torus(r, 0.014, 8, 32), m.clone(), s * EYE_X, EYE_Y, LENS_Z))
        .into();
    v.push(
        part(
            Shape::Cyl(0.01, 0.01, 2.0 * (EYE_X - r) + 0.02, 6),
            m.clone(),
            0.0,
            EYE_Y + 0.02,
            LENS_Z,
        )
        .rot(0.0, 0.0, FRAC_PI_2),
    );
    v.extend(temples(m, r));
    v
}

pub fn make_glasses(kind: Glasses) -> Option<Accessory> {
    let kids: Vec<Part> = match kind {
        Glasses::None => return None,
        Glasses::Round => frames(&mat(DARK).rough(0.3).metal(0.4), 0.1),
        Glasses::Shades => {
            let mut v = frames(&mat(FRAME_BLACK).rough(0.3), 0.105);
            let lens = mat(LENS).rough(0.08).metal(0.5);
            v.extend(
                [-1.0f32, 1.0].map(|s| {
                    part(Shape::Circle(0.105, 32), lens.clone(), s * EYE_X, EYE_Y, LENS_Z).scale(1.0, 0.85, 1.0)
                }),
            );
            v
        }
        Glasses::Hearts => {
            let m = mat(PINK).rough(0.2).glow(0.2);
            let mut v: Vec<Part> = [-1.0f32, 1.0]
                .map(|s| part(Shape::Heart(0.02), m.clone(), s * EYE_X, EYE_Y, LENS_Z - 0.015).scale(1.05, 1.05, 1.05))
                .into();
            v.push(
                part(Shape::Cyl(0.01, 0.01, 0.06, 6), m.clone(), 0.0, EYE_Y + 0.04, LENS_Z).rot(0.0, 0.0, FRAC_PI_2),
            );
            v.extend(temples(&m, 0.11));
            v
        }
        Glasses::Monocle => {
            let gold = mat(YELLOW).rough(0.25).metal(0.8);
            vec![
                part(Shape::Torus(0.1, 0.016, 8, 32), gold.clone(), -EYE_X, EYE_Y, LENS_Z),
                part(
                    Shape::Cyl(0.006, 0.006, 0.34, 6),
                    gold,
                    -EYE_X - 0.1,
                    EYE_Y - 0.25,
                    LENS_Z - 0.06,
                )
                .rot(-0.25, 0.0, -0.35),
            ]
        }
        Glasses::Visor => {
            let (r, arc) = (0.55, 0.62);
            let rim = mat(DARK).rough(0.4);
            let mut v = vec![part(
                Shape::CylPart {
                    top: r,
                    bottom: r,
                    height: 0.15,
                    segments: 32,
                    open: true,
                    start: -arc,
                    length: arc * 2.0,
                },
                mat(TEAL).rough(0.1).metal(0.3).glow(0.35).both(),
                0.0,
                EYE_Y,
                LENS_Z - r,
            )];
            for dy in [-1.0, 1.0] {
                v.push(part(
                    Shape::CylPart {
                        top: r + 0.005,
                        bottom: r + 0.005,
                        height: 0.02,
                        segments: 32,
                        open: true,
                        start: -arc,
                        length: arc * 2.0,
                    },
                    rim.clone().both(),
                    0.0,
                    EYE_Y + dy * 0.08,
                    LENS_Z - r,
                ));
            }
            v.extend(temples(&rim, 0.2));
            v
        }
    };
    Some(Accessory {
        root: group(Vec3::ZERO).with(kids),
        crown_lift: 0.0,
    })
}

/// Shared meshes and materials of accessories.
#[derive(Resource, Default)]
pub struct Wardrobe {
    meshes: Vec<(Shape, Handle<Mesh>)>,
    mats: HashMap<MatKey, Handle<StandardMaterial>>,
}

impl Wardrobe {
    fn mesh(&mut self, shape: Shape, meshes: &mut Assets<Mesh>) -> Handle<Mesh> {
        if let Some((_, h)) = self.meshes.iter().find(|(s, _)| *s == shape) {
            return h.clone();
        }
        let h = meshes.add(shape.mesh());
        self.meshes.push((shape, h.clone()));
        h
    }

    pub fn material(&mut self, m: &Mat, materials: &mut Assets<StandardMaterial>) -> Handle<StandardMaterial> {
        self.mats
            .entry(m.key())
            .or_insert_with(|| {
                let c = m.color;
                materials.add(StandardMaterial {
                    base_color: c,
                    perceptual_roughness: m.rough,
                    metallic: m.metal,
                    emissive: c.to_linear() * m.glow,
                    double_sided: m.both,
                    cull_mode: if m.both {
                        None
                    } else {
                        Some(bevy::render::render_resource::Face::Back)
                    },
                    ..default()
                })
            })
            .clone()
    }
}

/// What beans are dressed with: the accessories' meshes and materials, the assets they are made in, and the
/// quality (no clearcoat on Low).
#[derive(SystemParam)]
pub struct Tailor<'w> {
    pub assets: Res<'w, AssetServer>,
    pub wardrobe: ResMut<'w, Wardrobe>,
    pub meshes: ResMut<'w, Assets<Mesh>>,
    pub materials: ResMut<'w, Assets<StandardMaterial>>,
    quality: Option<Res<'w, Quality>>,
}

impl Tailor<'_> {
    /// Suits get a faint clearcoat (a second specular layer in every bean pixel), except on Low.
    pub fn coat(&self) -> bool {
        self.quality.as_ref().is_none_or(|q| q.preset != Preset::Low)
    }

    /// Spawns `part` under `parent`; `suit` stands in for materials that take the bean's suit.
    pub fn spawn(
        &mut self,
        commands: &mut Commands,
        parent: Entity,
        part: &Part,
        suit: &Handle<StandardMaterial>,
        shadows: bool,
        wiggles: &mut Vec<Entity>,
    ) -> Entity {
        let mut e = commands.spawn((part.tf, Visibility::default(), ChildOf(parent)));
        if let Some((shape, m)) = &part.look {
            let mat = if m.suit {
                suit.clone()
            } else {
                self.wardrobe.material(m, &mut self.materials)
            };
            e.insert((
                Mesh3d(self.wardrobe.mesh(*shape, &mut self.meshes)),
                MeshMaterial3d(mat),
            ));
            if !shadows {
                e.insert(NotShadowCaster);
            }
        }
        if let Some(w) = part.wiggle {
            e.insert((w, Base(part.tf)));
            wiggles.push(e.id());
        }
        let id = e.id();
        for k in &part.kids {
            self.spawn(commands, id, k, suit, shadows, wiggles);
        }
        id
    }
}

/// Where a wiggling part rests.
#[derive(Component)]
pub struct Base(pub Transform);

/// The moving parts of hats, by the bean's time and ground speed.
pub fn wiggle(w: Wiggle, base: &Transform, tf: &mut Transform, t: f32, speed: f32, rotor: &mut f32, dt: f32) {
    let euler = |x: f32, y: f32, z: f32| Quat::from_euler(EulerRot::XYZ, x, y, z);
    let (bx, by, bz) = base.rotation.to_euler(EulerRot::XYZ);
    match w {
        Wiggle::Rotor => {
            *rotor = (*rotor + dt.clamp(0.0, 0.1) * (5.0 + speed * 3.0)).rem_euclid(TAU);
            tf.rotation = Quat::from_rotation_y(*rotor);
        }
        Wiggle::BunnyEar(i) => {
            let flop = (speed * 0.04).min(0.35);
            tf.rotation = euler(-flop + (t * 3.0 + i as f32).sin() * 0.05, by, bz);
        }
        Wiggle::Halo => tf.translation.y = 0.45 + (t * 2.2).sin() * 0.025,
        Wiggle::Flower => tf.rotation = euler(bx, by, (t * 2.4).sin() * 0.12 - (speed * 0.03).min(0.3)),
        Wiggle::Antenna(i) => {
            let s = if i == 1 { 1.0 } else { -1.0 };
            tf.rotation = euler(
                -(speed * 0.05).min(0.4),
                by,
                -s * 0.3 + (t * 7.0 + i as f32 * 1.7).sin() * (0.06 + (speed * 0.025).min(0.2)),
            );
        }
    }
}
