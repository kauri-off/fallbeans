//! Doors of every kind: rows where only some doors give way, doors sliding open and shut on their own
//! rhythms; in between, a few more challenges drawn from the seed; a bumper ramp to the line.
use fb_sim::builder::Builder;
use fb_sim::course::{
    CourseOpts, bumper_ramp, door_rows, moving_platforms, pick_sections, pistons, race_course, rotor_decks,
    timed_doors, with_rests,
};
use fb_sim::looks::LookId;
use fb_sim::map::{GameMeta, Genre, MapCtx, MapDef, MapId, MapSpec};

pub struct DoorDash;

static META: GameMeta = GameMeta::new(
    MapId::DoorDash,
    "Дверной переполох",
    Genre::Race,
    "Двери, которые ломаются (или нет), и двери по таймеру. Порядок испытаний каждый раз новый!",
    "Добегите до финиша",
    150.0,
);

impl MapDef for DoorDash {
    fn meta(&self) -> &'static GameMeta {
        &META
    }

    fn looks(&self) -> &'static [LookId] {
        &[LookId::Castle, LookId::Meadow, LookId::Royal]
    }

    fn build(&self, b: &mut Builder, ctx: &MapCtx) -> MapSpec {
        let pool = vec![
            timed_doors(3, 16.0),
            rotor_decks(2),
            moving_platforms(5),
            door_rows(3, 17.0),
            pistons(3, 14.0),
        ];
        let middle = pick_sections(&mut b.rng, pool, 4);
        let mut sections = vec![door_rows(2, 17.0)];
        sections.extend(middle);
        sections.push(bumper_ramp(4.0, 22.0));
        let opts = CourseOpts {
            sections: with_rests(sections, 7.0),
            ..Default::default()
        };
        race_course(b, ctx, opts)
    }
}
