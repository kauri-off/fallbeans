//! Beans against each other after every body has stepped: capsule contacts, bumps, and tackles.
use std::collections::BTreeMap;

use fb_shared::PlayerId;

use crate::m::{self, MinMax};
use crate::math::V3;
use crate::physics::{BEAN_GAP, Body, BodyState, OtherBody, R, SUBSTEP_REACH, StepEvents, sphere_at};

const BUMP_E: f64 = 0.45;
/// A contact normal steeper than this: the upper bean stands on the other.
const ON_TOP: f64 = 0.55;
/// A tackle: the least speed of the tackler at the other, and of the two closing on each other (m/s).
const TACKLE_SPEED: f64 = 4.0;
const TACKLE_CLOSING: f64 = 2.0;
/// Only what is ahead of the tackler (cosine).
const TACKLE_AHEAD: f64 = 0.25;
/// Knock-over speed of a tackle: at least, per m/s of closing speed, at most (m/s).
const TACKLE_BASE: f64 = 5.0;
const TACKLE_K: f64 = 0.4;
const TACKLE_MAX: f64 = 10.0;
/// The share of its speed a tackler keeps (against a heavier bean, less).
const TACKLE_KEEP: f64 = 0.45;
const TACKLE_KEEP_HEAVY: f64 = 0.2;

/// A bean as a capsule between its two spheres, and how it moves.
#[derive(Clone, Copy, Debug)]
pub struct Capsule {
    pub a: V3,
    pub b: V3,
    pub r: f64,
    pub vel: V3,
    pub mass: f64,
}

impl Capsule {
    pub fn of_body(b: &Body) -> Self {
        Self::at(b.pos, b.tilt, b.tilt_dir, b.size, b.vel, b.mass())
    }

    pub fn of_other(o: &OtherBody) -> Self {
        Self::at(
            V3::new(o.x, o.y, o.z),
            o.tilt,
            o.tilt_dir,
            o.size,
            V3::new(o.vx, o.vy, o.vz),
            o.mass(),
        )
    }

    fn at(pos: V3, tilt: f64, tilt_dir: f64, size: f64, vel: V3, mass: f64) -> Self {
        Self {
            a: sphere_at(pos, tilt, tilt_dir, size, 0),
            b: sphere_at(pos, tilt, tilt_dir, size, 1),
            r: BEAN_GAP / 2.0 * size,
            vel,
            mass,
        }
    }
}

/// Closest points of segments p1–q1 and p2–q2.
fn closest(p1: V3, q1: V3, p2: V3, q2: V3) -> (V3, V3) {
    let d1 = q1 - p1;
    let d2 = q2 - p2;
    let r = p1 - p2;
    let a = d1.dot(d1);
    let e = d2.dot(d2);
    let f = d2.dot(r);
    let eps = 1e-12;
    if a <= eps && e <= eps {
        return (p1, p2);
    }
    let (s, t);
    if a <= eps {
        s = 0.0;
        t = m::clamp(f / e, 0.0, 1.0);
    } else {
        let c = d1.dot(r);
        if e <= eps {
            t = 0.0;
            s = m::clamp(-c / a, 0.0, 1.0);
        } else {
            let b = d1.dot(d2);
            let denom = a * e - b * b;
            let s0 = if denom > eps {
                m::clamp((b * f - c * e) / denom, 0.0, 1.0)
            } else {
                0.0
            };
            let t0 = (b * s0 + f) / e;
            if t0 < 0.0 {
                t = 0.0;
                s = m::clamp(-c / a, 0.0, 1.0);
            } else if t0 > 1.0 {
                t = 1.0;
                s = m::clamp((b - c) / a, 0.0, 1.0);
            } else {
                t = t0;
                s = s0;
            }
        }
    }
    (p1 + d1 * s, p2 + d2 * t)
}

/// Normal from `x` to `y` and depth where they overlap (`apart`: the normal of two that coincide).
pub fn contact(x: &Capsule, y: &Capsule, apart: V3) -> Option<(V3, f64)> {
    let (cx, cy) = closest(x.a, x.b, y.a, y.b);
    let d = cy - cx;
    let dist = d.length();
    let sum = x.r + y.r;
    if dist >= sum {
        return None;
    }
    let n = if dist > 1e-6 { d / dist } else { apart };
    Some((n, sum - dist))
}

/// The horizontal part of `n`, of unit length (`or` when there is none).
fn flat(n: V3, or: V3) -> V3 {
    let h = m::hypot(n.x, n.z);
    if h > 1e-6 { V3::new(n.x / h, 0.0, n.z / h) } else { or }
}

fn forward(b: &Body) -> V3 {
    V3::new(m::sin(b.yaw), 0.0, m::cos(b.yaw))
}

/// Which way two beans in the same spot part (by the first one's actor, as both sides agree on).
fn apart(b: &Body) -> V3 {
    V3::new(m::sin(b.actor as f64 * 2.4), 0.0, m::cos(b.actor as f64 * 2.4))
}

/// A dive, or a slide still fast enough to knock over.
pub fn tackling(b: &Body) -> bool {
    b.state == BodyState::Dive || (b.state == BodyState::Slide && m::hypot(b.vel.x, b.vel.z) > 6.0)
}

/// A tackle that connects: the horizontal normal from the tackler to the other, their closing speed, and
/// whether the other was over or under the tackler.
#[derive(Clone, Copy, Debug)]
struct Tackle {
    n: V3,
    closing: f64,
    sweep: bool,
    /// Judged by where the tackler's player saw the other (not where it is).
    seen: bool,
}

/// Why a tackling bean touching another did not knock it over.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Miss {
    /// Not ahead of the tackler (its side or back).
    Behind,
    /// The tackler is too slow at it.
    Slow,
    /// Not closing in fast enough.
    Apart,
}

/// What happened between beans (and a bean's dives and bonks) in a step, for `--trace-hits`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Note {
    Tackle {
        on: PlayerId,
        closing: f64,
        knock: f64,
        sweep: bool,
        seen: bool,
    },
    Miss {
        on: PlayerId,
        why: Miss,
        at: f64,
        closing: f64,
        seen: bool,
    },
    Bump {
        with: PlayerId,
        closing: f64,
    },
    Stand {
        on: PlayerId,
    },
    Bonk {
        speed: f64,
    },
    Dive {
        air: bool,
        speed: f64,
        vy: f64,
    },
}

/// Ticks within which a repeat of the same steady note (`Note::repeats`) is not logged again.
pub const NOTE_QUIET: i64 = 12;

impl Note {
    /// What a steady contact repeats every tick (standing, pushing, a tackler touching and missing), else None.
    pub fn repeats(&self) -> Option<(u8, PlayerId, u8)> {
        match *self {
            Note::Stand { on } => Some((0, on, 0)),
            Note::Bump { with, .. } => Some((1, with, 0)),
            Note::Miss { on, why, .. } => Some((2, on, why as u8)),
            _ => None,
        }
    }
}

/// Which notes of a bean to log: a repeat of a steady one within NOTE_QUIET ticks of the last is not.
#[derive(Default)]
pub struct NoteQuiet(BTreeMap<(PlayerId, (u8, PlayerId, u8)), i64>);

impl NoteQuiet {
    pub fn fresh(&mut self, k: i64, id: PlayerId, n: &Note) -> bool {
        let Some(key) = n.repeats() else { return true };
        let last = self.0.insert((id, key), k);
        last.is_none_or(|l| k - l > NOTE_QUIET || k < l)
    }
}

impl core::fmt::Display for Note {
    fn fmt(&self, f: &mut core::fmt::Formatter) -> core::fmt::Result {
        let flag = |on: bool, s: &'static str| if on { s } else { "" };
        match *self {
            Note::Tackle {
                on,
                closing,
                knock,
                sweep,
                seen,
            } => write!(
                f,
                "tackle on={on} closing={closing:.2} knock={knock:.2}{}{}",
                flag(sweep, " sweep"),
                flag(seen, " seen")
            ),
            Note::Miss {
                on,
                why,
                at,
                closing,
                seen,
            } => write!(
                f,
                "miss on={on} why={why:?} at={at:.2} closing={closing:.2}{}",
                flag(seen, " seen")
            ),
            Note::Bump { with, closing } => write!(f, "bump with={with} closing={closing:.2}"),
            Note::Stand { on } => write!(f, "stand on={on}"),
            Note::Bonk { speed } => write!(f, "bonk speed={speed:.2}"),
            Note::Dive { air, speed, vy } => write!(f, "dive speed={speed:.2} vy={vy:.2}{}", flag(air, " air")),
        }
    }
}

/// Whether `att` (its capsule `ca`) tackles a bean it sees as `victim`; Err(None): not tackling, or not touching.
fn tackle_of(att: &Body, ca: &Capsule, victim: &Capsule) -> Result<Tackle, Option<(Miss, f64, f64)>> {
    if !tackling(att) {
        return Err(None);
    }
    let f = forward(att);
    let (n, _) = contact(ca, victim, f).ok_or(None)?;
    let sweep = n.y.abs() > ON_TOP;
    // Over or under the tackler (a hop, a landing on it): swept off its feet along the dive.
    let n = if sweep { flat(att.vel, f) } else { flat(n, f) };
    let at = att.vel.x * n.x + att.vel.z * n.z;
    let closing = at - (victim.vel.x * n.x + victim.vel.z * n.z);
    let why = if !sweep && n.dot(f) < TACKLE_AHEAD {
        Miss::Behind
    } else if at < TACKLE_SPEED {
        Miss::Slow
    } else if closing < TACKLE_CLOSING {
        Miss::Apart
    } else {
        return Ok(Tackle {
            n,
            closing,
            sweep,
            seen: false,
        });
    };
    Err(Some((why, at, closing)))
}

/// `tackle_of` against where the tackler's player saw the other (`saw`) and where it is now: either connects (what
/// the player saw it hit, and what it really ran into).
fn judge(
    att: &Body,
    ca: &Capsule,
    now: &Capsule,
    saw: Option<&OtherBody>,
    ev: &mut StepEvents,
    on: PlayerId,
) -> Option<Tackle> {
    if att.tackled == Some(on) {
        return None;
    }
    let seen = saw.map(|s| tackle_of(att, ca, &Capsule::of_other(s)));
    if let Some(Ok(t)) = seen {
        return Some(Tackle { seen: true, ..t });
    }
    let real = tackle_of(att, ca, now);
    let miss = match (seen, real) {
        (_, Ok(t)) => return Some(t),
        (Some(Err(Some(m))), _) => Some((m, true)),
        (_, Err(Some(m))) => Some((m, false)),
        _ => None,
    };
    if let Some(((why, at, closing), seen)) = miss {
        ev.notes.push(Note::Miss {
            on,
            why,
            at,
            closing,
            seen,
        });
    }
    None
}

/// Knocked over by a tackle; its speed at the tackler is stopped first, so a head-on hit is as hard as any.
/// Returns the knock's speed.
fn take_tackle(v: &mut Body, ev: &mut StepEvents, by: PlayerId, by_mass: f64, by_fwd: V3, hit: Tackle) -> f64 {
    let n = hit.n;
    let toward = v.vel.x * n.x + v.vel.z * n.z;
    if toward < 0.0 {
        v.vel.x -= n.x * toward;
        v.vel.z -= n.z * toward;
    }
    let heavy = m::clamp(2.0 * by_mass / (by_mass + v.mass()), 0.6, 1.4);
    let k = (TACKLE_BASE + TACKLE_K * hit.closing).at_most(TACKLE_MAX) * heavy;
    v.knock(ev, n.x * k + by_fwd.x * 2.0, n.z * k + by_fwd.z * 2.0, 4.5, 0.9, false);
    ev.tackled_by = Some(by);
    k
}

/// The tackler spends its momentum on the hit.
fn spend_tackle(att: &mut Body, ev: &mut StepEvents, on: PlayerId, victim_mass: f64) {
    att.tackled = Some(on);
    let keep = if victim_mass > att.mass() {
        TACKLE_KEEP_HEAVY
    } else {
        TACKLE_KEEP
    };
    att.vel.x *= keep;
    att.vel.z *= keep;
    ev.tackles += 1;
}

fn note_tackle(ev: &mut StepEvents, on: PlayerId, hit: Tackle, knock: f64) {
    ev.notes.push(Note::Tackle {
        on,
        closing: hit.closing,
        knock,
        sweep: hit.sweep,
        seen: hit.seen,
    });
}

/// Climbing a ledge or a ladder: nothing pushes it about.
fn fixed(b: &Body) -> bool {
    matches!(b.state, BodyState::Climb | BodyState::Ladder)
}

/// The upper bean of a contact stands on the other: lifted out of it, its fall stopped.
fn stand(b: &mut Body, ev: &mut StepEvents, on: PlayerId, depth: f64) {
    if b.vel.y > 0.0 || fixed(b) {
        return;
    }
    b.pos.y += depth.at_most(R * b.size * SUBSTEP_REACH);
    b.vel.y = 0.0;
    b.grounded = true;
    ev.notes.push(Note::Stand { on });
}

/// One bean of a pair, stepped this tick.
pub struct Side<'a> {
    pub id: PlayerId,
    pub body: &'a mut Body,
    pub ev: &'a mut StepEvents,
}

/// The other bean of a pair: stepped too, or only seen (a client's view: nothing moves it).
pub enum Other<'a> {
    Stepped(Side<'a>),
    Seen(&'a OtherBody),
}

/// `x` against another bean; `x_saw`/`y_saw`: where each one's player saw the other (lag compensation).
pub fn resolve(x: Side, y: Other, x_saw: Option<&OtherBody>, y_saw: Option<&OtherBody>) {
    let cx = Capsule::of_body(x.body);
    let (cy, y_id) = match &y {
        Other::Stepped(s) => (Capsule::of_body(s.body), s.id),
        Other::Seen(o) => (Capsule::of_other(o), o.id),
    };
    let mut y = y;
    // Both tackles are decided before either is applied: in a head-on pair of dives both go over.
    let tx = judge(x.body, &cx, &cy, x_saw, x.ev, y_id);
    let ty = match &mut y {
        Other::Stepped(s) => judge(s.body, &cy, &cx, y_saw, s.ev, x.id),
        Other::Seen(_) => None,
    };
    let (x_mass, x_fwd) = (cx.mass, forward(x.body));
    if let Some(hit) = tx {
        let knock = match &mut y {
            Other::Stepped(s) => take_tackle(s.body, s.ev, x.id, x_mass, x_fwd, hit),
            Other::Seen(_) => 0.0,
        };
        note_tackle(x.ev, y_id, hit, knock);
        if ty.is_none() {
            spend_tackle(x.body, x.ev, y_id, cy.mass);
        } else {
            x.body.tackled = Some(y_id);
        }
    }
    if let Some(hit) = ty
        && let Other::Stepped(s) = &mut y
    {
        let fwd = forward(s.body);
        let knock = take_tackle(x.body, x.ev, y_id, cy.mass, fwd, hit);
        note_tackle(s.ev, x.id, hit, knock);
        if tx.is_none() {
            spend_tackle(s.body, s.ev, x.id, x_mass);
        } else {
            s.body.tackled = Some(x.id);
        }
    }
    let tackled = tx.is_some() || ty.is_some();

    let Some((n, depth)) = contact(&cx, &cy, apart(x.body)) else {
        return;
    };
    if n.y > ON_TOP {
        if let Other::Stepped(s) = &mut y {
            stand(s.body, s.ev, x.id, depth);
        }
        return;
    }
    if n.y < -ON_TOP {
        stand(x.body, x.ev, y_id, depth);
        return;
    }
    let n = flat(n, apart(x.body));
    let total = cx.mass + cy.mass;
    let (x_share, y_share) = (cy.mass / total, cx.mass / total);
    // (At most a substep's reach a tick: coincident beans must not be thrown through a wall.)
    if !fixed(x.body) {
        x.body.pos -= n * (depth * x_share).at_most(R * x.body.size * SUBSTEP_REACH);
    }
    let closing = (cx.vel.x - cy.vel.x) * n.x + (cx.vel.z - cy.vel.z) * n.z;
    let bump = !tackled && closing > 0.0;
    if bump {
        bounce(x.body, x.ev, y_id, n * -1.0, closing, x_share);
    }
    if let Other::Stepped(s) = &mut y {
        if !fixed(s.body) {
            s.body.pos += n * (depth * y_share).at_most(R * s.body.size * SUBSTEP_REACH);
        }
        if bump {
            bounce(s.body, s.ev, x.id, n, closing, y_share);
        }
    }
}

/// A bump: pushed `away` by its share of the closing speed; a hard one lifts it off its feet a little.
fn bounce(b: &mut Body, ev: &mut StepEvents, with: PlayerId, away: V3, closing: f64, share: f64) {
    if fixed(b) {
        return;
    }
    let j = (1.0 + BUMP_E) * closing * share;
    b.vel.x += away.x * j;
    b.vel.z += away.z * j;
    if closing > 6.0 && b.grounded {
        b.vel.y = b.vel.y.at_least((closing * 0.3).at_most(4.0));
    }
    ev.bumped = ev.bumped.at_least(closing);
    // (A steady push is a bump every tick: only those worth a sound.)
    if closing > 1.0 {
        ev.notes.push(Note::Bump { with, closing });
    }
}

/// How far apart two capsules are (negative: overlapping).
pub fn gap(x: &Capsule, y: &Capsule) -> f64 {
    let (cx, cy) = closest(x.a, x.b, y.a, y.b);
    (cy - cx).length() - x.r - y.r
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::physics::DIVE_SPEED;

    fn bean(actor: i32, x: f64, z: f64) -> Body {
        let mut b = Body::new(actor);
        b.reset(V3::new(x, 0.0, z), 0.0);
        b.grounded = true;
        b
    }

    fn diving(actor: i32, x: f64, z: f64, yaw: f64) -> Body {
        let mut b = bean(actor, x, z);
        b.state = BodyState::Dive;
        b.yaw = yaw;
        b.tilt = 1.3;
        b.tilt_dir = yaw;
        b.vel = V3::new(m::sin(yaw) * DIVE_SPEED, 0.0, m::cos(yaw) * DIVE_SPEED);
        b.grounded = false;
        b
    }

    fn pair(x: &mut Body, y: &mut Body) -> (StepEvents, StepEvents) {
        let (mut ex, mut ey) = (StepEvents::default(), StepEvents::default());
        resolve(
            Side {
                id: PlayerId(1),
                body: x,
                ev: &mut ex,
            },
            Other::Stepped(Side {
                id: PlayerId(2),
                body: y,
                ev: &mut ey,
            }),
            None,
            None,
        );
        (ex, ey)
    }

    #[test]
    fn overlapping_beans_part_and_trade_momentum() {
        let mut x = bean(1, 0.0, 0.0);
        let mut y = bean(2, 0.0, 0.8);
        x.vel.z = 4.0;
        let (ex, ey) = pair(&mut x, &mut y);
        assert!(x.pos.z < 0.0 && y.pos.z > 0.8);
        assert!(x.vel.z < 4.0 && y.vel.z > 0.0, "{} {}", x.vel.z, y.vel.z);
        assert!(ex.bumped > 0.0 && ey.bumped > 0.0);
        assert_eq!(x.state, BodyState::Normal);
        assert_eq!(y.state, BodyState::Normal);
    }

    #[test]
    fn a_dive_into_someone_ahead_knocks_them_over() {
        let mut x = diving(1, 0.0, 0.0, 0.0);
        let mut y = bean(2, 0.0, 1.4);
        let (ex, ey) = pair(&mut x, &mut y);
        assert_eq!(y.state, BodyState::Tumble);
        assert_eq!(ey.tackled_by, Some(PlayerId(1)));
        assert_eq!(ex.tackles, 1);
        assert!(x.vel.z < DIVE_SPEED * 0.5, "{}", x.vel.z);
        assert!(y.vel.z > 5.0, "{}", y.vel.z);
    }

    #[test]
    fn a_dive_does_not_knock_over_what_is_behind_or_too_far() {
        let mut x = diving(1, 0.0, 0.0, 0.0);
        let mut y = bean(2, 0.0, -0.9);
        pair(&mut x, &mut y);
        assert_eq!(y.state, BodyState::Normal);
        let mut x = diving(1, 0.0, 0.0, 0.0);
        let mut y = bean(2, 0.0, 2.0);
        pair(&mut x, &mut y);
        assert_eq!(y.state, BodyState::Normal);
    }

    #[test]
    fn a_dive_under_someone_hopping_sweeps_them_off_their_feet() {
        let mut x = diving(1, 0.0, 0.0, 0.0);
        let mut y = bean(2, 0.0, 0.2);
        y.pos.y = 0.9;
        y.vel.y = -3.0;
        y.grounded = false;
        let (ex, ey) = pair(&mut x, &mut y);
        assert_eq!(y.state, BodyState::Tumble);
        assert_eq!((ex.tackles, ey.tackled_by), (1, Some(PlayerId(1))));
        assert!(y.vel.z > 5.0 && !y.grounded, "{:?}", y.vel);
    }

    #[test]
    fn head_on_dives_knock_both_over_whoever_comes_first() {
        for flip in [false, true] {
            let mut x = diving(1, 0.0, 0.0, 0.0);
            let mut y = diving(2, 0.0, 1.5, m::PI);
            let (ex, ey) = if flip {
                pair(&mut y, &mut x)
            } else {
                pair(&mut x, &mut y)
            };
            assert_eq!((x.state, y.state), (BodyState::Tumble, BodyState::Tumble));
            assert!(ex.tackled_by.is_some() && ey.tackled_by.is_some());
            assert!(x.vel.z < 0.0 && y.vel.z > 0.0, "{} {}", x.vel.z, y.vel.z);
        }
    }

    #[test]
    fn a_tackle_from_behind_is_softer_than_one_head_on() {
        let hit = |vz: f64| {
            let mut x = diving(1, 0.0, 0.0, 0.0);
            let mut y = bean(2, 0.0, 1.4);
            y.vel.z = vz;
            pair(&mut x, &mut y);
            assert_eq!(y.state, BodyState::Tumble);
            y.vel.z
        };
        let fleeing = hit(8.0);
        let standing = hit(0.0);
        assert!(fleeing - 8.0 < standing, "{fleeing} {standing}");
    }

    #[test]
    fn a_bean_lands_on_another_ones_head() {
        let mut x = bean(1, 0.0, 0.0);
        let mut y = bean(2, 0.1, 0.0);
        y.pos.y = 1.5;
        y.vel.y = -3.0;
        y.grounded = false;
        pair(&mut x, &mut y);
        assert!(y.grounded && y.vel.y == 0.0 && y.pos.y > 1.5);
        assert!(x.pos == V3::new(0.0, 0.0, 0.0));
    }

    #[test]
    fn a_tackle_connects_with_where_its_player_saw_the_other() {
        let saw = OtherBody {
            id: PlayerId(2),
            z: 1.4,
            size: 1.0,
            ..Default::default()
        };
        for (lagged, hit) in [(false, false), (true, true)] {
            let mut x = diving(1, 0.0, 0.0, 0.0);
            let mut y = bean(2, 0.0, 3.0);
            let (mut ex, mut ey) = (StepEvents::default(), StepEvents::default());
            resolve(
                Side {
                    id: PlayerId(1),
                    body: &mut x,
                    ev: &mut ex,
                },
                Other::Stepped(Side {
                    id: PlayerId(2),
                    body: &mut y,
                    ev: &mut ey,
                }),
                lagged.then_some(&saw),
                None,
            );
            assert_eq!(y.state == BodyState::Tumble, hit);
            assert_eq!(ey.tackled_by.is_some(), hit);
        }
    }

    #[test]
    fn a_dive_that_really_runs_into_someone_knocks_them_over_though_its_player_saw_it_miss() {
        let saw = OtherBody {
            id: PlayerId(2),
            z: 3.0,
            size: 1.0,
            ..Default::default()
        };
        let mut x = diving(1, 0.0, 0.0, 0.0);
        let mut y = bean(2, 0.0, 1.4);
        let (mut ex, mut ey) = (StepEvents::default(), StepEvents::default());
        resolve(
            Side {
                id: PlayerId(1),
                body: &mut x,
                ev: &mut ex,
            },
            Other::Stepped(Side {
                id: PlayerId(2),
                body: &mut y,
                ev: &mut ey,
            }),
            Some(&saw),
            None,
        );
        assert_eq!(y.state, BodyState::Tumble);
        assert_eq!(ex.bumped, 0.0);
    }

    #[test]
    fn a_seen_bean_moves_only_the_one_stepped() {
        let mut x = diving(1, 0.0, 0.0, 0.0);
        let o = OtherBody {
            id: PlayerId(2),
            z: 1.4,
            size: 1.0,
            ..Default::default()
        };
        let mut ex = StepEvents::default();
        resolve(
            Side {
                id: PlayerId(1),
                body: &mut x,
                ev: &mut ex,
            },
            Other::Seen(&o),
            None,
            None,
        );
        assert_eq!(ex.tackles, 1);
        assert!(x.vel.z < DIVE_SPEED * 0.5);
    }
}
