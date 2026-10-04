//! Doors of every kind: rows where only some doors give way, a heavy gate that opens by itself only now
//! and then (or while someone holds a button for the others), doors sliding open and shut on their own
//! rhythms; in between, a few more challenges drawn from the seed; a bumper ramp to the line.
use fb_sim::builder::Builder;
use fb_sim::course::{
    CourseOpts, bumper_ramp, coop_gate, door_rows, moving_platforms, pick_sections, pistons, race_course, rotor_decks,
    timed_doors, with_rests,
};
use fb_sim::map::{GameMeta, Genre, MapCtx, MapDef, MapSpec};

pub struct DoorDash;

static META: GameMeta = GameMeta::new(
    "door-dash",
    "Дверной переполох",
    Genre::Race,
    "Двери, которые ломаются (или нет), двери по таймеру и тяжёлые ворота: кто-то должен встать на кнопку и подержать их для остальных. Порядок испытаний каждый раз новый!",
    "Добегите до финиша",
    150.0,
);

impl MapDef for DoorDash {
    fn meta(&self) -> &'static GameMeta {
        &META
    }

    fn looks(&self) -> &'static [&'static str] {
        &["castle", "meadow", "royal"]
    }

    fn build(&self, b: &mut Builder, ctx: &MapCtx) -> MapSpec {
        let pool = vec![
            timed_doors(3, 16.0),
            rotor_decks(2),
            moving_platforms(5),
            door_rows(3, 17.0),
            pistons(3, 14.0),
        ];
        let middle = pick_sections(&mut b.rng, pool, 3);
        let mut sections = vec![door_rows(2, 17.0), coop_gate(16.0)];
        sections.extend(middle);
        sections.push(bumper_ramp(4.0, 22.0));
        let opts = CourseOpts {
            sections: with_rests(sections, 7.0),
            ..Default::default()
        };
        race_course(b, ctx, opts)
    }
}
