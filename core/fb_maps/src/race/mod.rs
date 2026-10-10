//! The race: one map whose course is composed from the seed out of a catalog of sections, as long as asked.
use fb_shared::game::{RACE_SECTIONS, RACE_SECTIONS_DEFAULT};
use fb_shared::rng::Rng;
use fb_sim::builder::Builder;
use fb_sim::course::{
    CourseOpts, FinishWith, Segment, bumper_ramp, conveyor, door_rows, glove_alley, hammer_bridges, moving_platforms,
    pistons, portal_fork, race_course, rotor_decks, seesaws, sliding_gates, timed_doors, tipping_bridge,
    trampoline_gap, with_rests,
};
use fb_sim::looks::LookId;
use fb_sim::m::MinMax;
use fb_sim::map::{GameMeta, Genre, MapCtx, MapDef, MapId, MapSpec};

mod balls;
mod bounce;
mod climb;
mod drums;
mod fork;
mod glass;
mod ice;
mod portals;
mod summit;

/// What a section asks of a bean: two in a row never ask the same.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Skill {
    Timing,
    Balance,
    Jump,
    Bounce,
    Climb,
    Push,
    Portal,
    Luck,
}

/// Where in a course a section may stand besides the middle.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Place {
    Middle,
    /// First too (an easy way in).
    Start,
    /// Last too (a climax before the finish).
    End,
}

struct SectionDef {
    name: &'static str,
    skill: Skill,
    /// 1 easy … 3 hard.
    tier: u8,
    place: Place,
    make: fn(&mut Rng) -> Segment,
}

const fn def(name: &'static str, skill: Skill, tier: u8, place: Place, make: fn(&mut Rng) -> Segment) -> SectionDef {
    SectionDef {
        name,
        skill,
        tier,
        place,
        make,
    }
}

use Place::{End, Middle, Start};
use Skill::{Balance, Bounce, Climb, Jump, Luck, Portal, Push, Timing};

static CATALOG: &[SectionDef] = &[
    def("door-rows", Push, 1, Start, |_| door_rows(2, 17.0)),
    def("hammer-fork", Timing, 1, Start, |_| fork::fork()),
    def("mushrooms", Bounce, 1, Start, |_| bounce::mushroom_forest(5, 16.0)),
    def("ledges", Climb, 1, Start, |_| climb::ledge_steps(4, 1.5)),
    def("glass-wide", Luck, 1, Start, |r| {
        glass::glass_bridge(8 + r.index(3), 5, &[])
    }),
    def("portal-islands", Portal, 1, Start, |_| portals::portal_islands(2)),
    def("ice-slope", Balance, 1, Start, |_| ice::ice_slope(5.0, 26.0)),
    def("climb-fork", Climb, 2, Start, |_| summit::climb_fork(8.0)),
    def("drum-stairs", Timing, 2, Start, |_| drums::drum_stairs()),
    def("carpet-slope", Balance, 2, Start, |_| balls::carpet_slope()),
    def("moving-platforms", Jump, 1, Middle, |_| moving_platforms(4)),
    def("seesaws", Balance, 1, Middle, |_| seesaws(3)),
    def("conveyor", Balance, 1, Middle, |_| conveyor(30.0)),
    def("trampoline-gap", Bounce, 1, Middle, |_| trampoline_gap()),
    def("portal-fork", Portal, 1, Middle, |_| portal_fork()),
    def("portal-hop", Portal, 1, Middle, |_| portals::last_hop()),
    def("timed-doors", Timing, 2, Middle, |_| timed_doors(3, 16.0)),
    def("rotor-decks", Jump, 2, Middle, |_| rotor_decks(2)),
    def("pistons", Timing, 2, Middle, |_| pistons(3, 14.0)),
    def("tipping-bridge", Balance, 2, Middle, |_| tipping_bridge(5)),
    def("glove-alley", Push, 2, Middle, |_| glove_alley(4)),
    def("hover-trampolines", Bounce, 2, Middle, |_| {
        bounce::hover_trampolines(3, 15.0)
    }),
    def("catapults", Bounce, 2, Middle, |_| bounce::pad_catapults(3)),
    def("ladder-wall", Climb, 2, Middle, |_| climb::ladder_wall(6.0)),
    def("glove-launch", Push, 2, Middle, |_| summit::glove_launch(5.0)),
    def("roller-bridge", Balance, 2, Middle, |_| drums::roller_bridge(10)),
    def("dive-gaps", Jump, 2, Middle, |_| ice::dive_gaps(3, 7.5)),
    def("dive-bars", Jump, 2, Middle, |_| ice::dive_bars(3)),
    def("blinking-portals", Portal, 2, Middle, |_| portals::blinking_portals()),
    def("portal-wall", Portal, 2, Middle, |_| portals::portal_wall()),
    def("glass", Luck, 2, Middle, |r| {
        glass::glass_bridge(6 + r.index(3), 4, &[2, 5])
    }),
    def("hammer-bridges", Timing, 3, Middle, |_| hammer_bridges(3)),
    def("ball-ramp", Timing, 3, Middle, |_| balls::ball_ramp(6.0)),
    def("zigzag-ledges", Climb, 3, Middle, |_| climb::zigzag_ledges(6, 1.6)),
    def("peg-drums", Timing, 3, Middle, |_| drums::peg_drums(3)),
    def("ice-rotors", Jump, 3, Middle, |_| ice::ice_rotors(2)),
    def("portal-cannons", Portal, 3, Middle, |_| portals::portal_cannons(2)),
    def("rotor-deck", Jump, 1, End, |_| rotor_decks(1)),
    def("bumper-ramp", Push, 2, End, |_| bumper_ramp(4.0, 22.0)),
    def("sliding-gates", Timing, 2, End, |_| sliding_gates(4, 5.0)),
    def("big-bounce", Bounce, 2, End, |_| bounce::big_bounce(6.0)),
    def("glass-narrow", Luck, 3, End, |r| {
        glass::glass_bridge(5 + r.index(3), 3, &[1, 3])
    }),
    def("log-run", Balance, 3, End, |_| drums::log_run(5)),
    def("ladder-tower", Climb, 3, End, |_| climb::ladder_tower(3, 4.0)),
    def("sky-shuttles", Jump, 3, End, |_| ice::sky_shuttles()),
];

/// The tier wanted at section `i` of `n`: easy first, hard at the end.
fn tier_at(i: usize, n: usize) -> u8 {
    if i == 0 {
        return 1;
    }
    1 + u8::try_from((3 * i / n).min(2)).unwrap_or(2)
}

/// The course's sections, drawn from the catalog: no section twice, no skill twice in a row, the first one
/// easy and fit to start, the last fit to end, the tiers rising in between.
fn compose(rng: &mut Rng, n: usize) -> Vec<&'static SectionDef> {
    let mut out: Vec<&'static SectionDef> = Vec::new();
    for i in 0..n {
        let want = tier_at(i, n);
        let place = match i {
            0 => Start,
            _ if i + 1 == n => End,
            _ => Middle,
        };
        let prev = out.last().map(|d| d.skill);
        let free = |d: &&SectionDef| !out.iter().any(|o| o.name == d.name);
        let weight = |d: &SectionDef| -> f64 {
            match d.tier.abs_diff(want) {
                0 => 4.0,
                1 => 1.0,
                _ => 0.0,
            }
        };
        let strict: Vec<&'static SectionDef> = CATALOG
            .iter()
            .filter(free)
            .filter(|d| Some(d.skill) != prev && (d.place == place || (place == Middle && d.place != Start)))
            .filter(|d| weight(d) > 0.0)
            .collect();
        let cands = if strict.is_empty() {
            CATALOG.iter().filter(free).collect()
        } else {
            strict
        };
        let total: f64 = cands.iter().map(|d| weight(d).at_least(0.1)).sum();
        let mut at = rng.unit() * total;
        let mut pick = cands[cands.len() - 1];
        for d in &cands {
            at -= weight(d).at_least(0.1);
            if at < 0.0 {
                pick = d;
                break;
            }
        }
        out.push(pick);
    }
    out
}

/// The names of the sections of a race of `n` sections on this seed, in order (and whether it ends at the
/// summit): what `Race::build` draws.
pub fn track(seed: u32, n: u8) -> (Vec<&'static str>, bool) {
    let mut rng = Rng::new(seed);
    let defs = compose(&mut rng, usize::from(n));
    for d in &defs {
        drop((d.make)(&mut rng));
    }
    (defs.iter().map(|d| d.name).collect(), rng.unit() < SUMMIT_SHARE)
}

/// Races that end at the summit with the crown instead of a finish line.
const SUMMIT_SHARE: f64 = 0.25;

pub struct Race {
    sections: u8,
    meta: GameMeta,
}

/// Round length (s) for so many sections.
const fn duration(sections: u8) -> f64 {
    let d = 30.0 + 20.0 * sections as f64;
    if d > 180.0 { 180.0 } else { d }
}

const fn race(sections: u8) -> Race {
    Race {
        sections,
        meta: GameMeta::new(
            MapId::Race,
            "Гонка",
            Genre::Race,
            "Трасса из случайных испытаний: двери, молоты, батуты, порталы, лёд, стеклянные мосты и многое другое. Каждый раз новая — или та же по коду трассы.",
            "Добегите до финиша",
            duration(sections),
        ),
    }
}

static RACES: [Race; 6] = [race(3), race(4), race(5), race(6), race(7), race(8)];

/// The race of this many sections (clamped to `RACE_SECTIONS`).
pub const fn of(sections: u8) -> &'static Race {
    let (lo, hi) = (*RACE_SECTIONS.start(), *RACE_SECTIONS.end());
    let n = if sections < lo {
        lo
    } else if sections > hi {
        hi
    } else {
        sections
    };
    &RACES[(n - lo) as usize]
}

impl MapDef for Race {
    fn meta(&self) -> &'static GameMeta {
        &of(self.sections).meta
    }

    fn looks(&self) -> &'static [LookId] {
        &[
            LookId::Meadow,
            LookId::Castle,
            LookId::Factory,
            LookId::Snow,
            LookId::Starlight,
            LookId::Circus,
            LookId::Neon,
            LookId::Ocean,
            LookId::Desert,
            LookId::Jungle,
            LookId::Lava,
            LookId::Royal,
            LookId::Candy,
        ]
    }

    fn sections(&self) -> u8 {
        self.sections
    }

    fn build(&self, b: &mut Builder, ctx: &MapCtx) -> MapSpec {
        let defs = compose(&mut b.rng, usize::from(self.sections));
        let sections: Vec<Segment> = defs.iter().map(|d| (d.make)(&mut b.rng)).collect();
        let finish_with: Option<FinishWith> = (b.rng.unit() < SUMMIT_SHARE).then(|| Box::new(summit::summit) as _);
        let opts = CourseOpts {
            sections: with_rests(sections, 7.0),
            finish_with,
        };
        race_course(b, ctx, opts)
    }
}

/// Every section of the catalog in one course, ending at the summit: the warm-up draws it so that no section
/// compiles anything in play.
pub struct Showcase;

impl MapDef for Showcase {
    fn meta(&self) -> &'static GameMeta {
        &of(*RACE_SECTIONS.end()).meta
    }

    fn looks(&self) -> &'static [LookId] {
        of(RACE_SECTIONS_DEFAULT).looks()
    }

    fn build(&self, b: &mut Builder, ctx: &MapCtx) -> MapSpec {
        let sections: Vec<Segment> = CATALOG.iter().map(|d| (d.make)(&mut b.rng)).collect();
        let opts = CourseOpts {
            sections: with_rests(sections, 7.0),
            finish_with: Some(Box::new(summit::summit)),
        };
        race_course(b, ctx, opts)
    }
}

#[cfg(test)]
mod tests {
    use fb_sim::map::spec_problems;

    use super::*;

    #[test]
    fn courses_rise_in_tiers_without_repeats() {
        for n in RACE_SECTIONS {
            for seed in 0..200 {
                let defs = compose(&mut Rng::new(seed), usize::from(n));
                assert_eq!(defs.len(), usize::from(n));
                assert_eq!(defs[0].place, Start, "seed {seed}");
                assert_eq!(defs[defs.len() - 1].place, End, "seed {seed}");
                for w in defs.windows(2) {
                    assert_ne!(w[0].skill, w[1].skill, "seed {seed}: {} then {}", w[0].name, w[1].name);
                }
                let mut names: Vec<_> = defs.iter().map(|d| d.name).collect();
                names.sort_unstable();
                names.dedup();
                assert_eq!(names.len(), defs.len());
            }
        }
    }

    #[test]
    fn every_section_shows_up() {
        let mut seen: Vec<&str> = Vec::new();
        for seed in 0..2000 {
            seen.extend(compose(&mut Rng::new(seed), 8).iter().map(|d| d.name));
        }
        for d in CATALOG {
            assert!(seen.contains(&d.name), "{} never drawn", d.name);
        }
    }

    #[test]
    fn lengths_have_their_own_round_times() {
        for n in RACE_SECTIONS {
            let r = of(n);
            assert_eq!(r.sections(), n);
            assert!(r.meta().problems().is_empty(), "{:?}", r.meta().problems());
        }
        assert!(of(3).meta().duration < of(8).meta().duration);
    }

    fn build(def: &dyn MapDef, seed: u32) -> MapSpec {
        let ctx = MapCtx {
            server: true,
            seed,
            participants: &[],
        };
        let spec = def.build(&mut Builder::new(seed, false), &ctx);
        assert!(
            spec_problems(&spec).is_empty(),
            "seed {seed}: {:?}",
            spec_problems(&spec)
        );
        spec
    }

    #[test]
    fn every_length_builds_and_the_showcase_has_every_section() {
        for n in RACE_SECTIONS {
            for seed in 0..20 {
                assert!(build(of(n), seed).finish.is_some());
            }
        }
        let all = build(&Showcase, 1);
        assert!(all.checkpoints.len() > CATALOG.len());
    }
}
