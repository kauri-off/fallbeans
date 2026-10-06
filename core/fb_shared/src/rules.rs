//! Scoring a round: placement points by rank, fines for falls and shortcuts.
use std::collections::{BTreeMap, BTreeSet};

use crate::game::Genre;
use crate::rng::{Rng, shuffle};
use serde::{Deserialize, Serialize};

/// Placement points for first place; last place gets 0, the rest are spread evenly between.
pub const TOP_POINTS: i64 = 10;
pub const FALL_PENALTY: i64 = 1;
pub const SHORTCUT_PENALTY: i64 = 2;
/// Penalties never take more than this from one round.
pub const MAX_PENALTY: i64 = 4;

/// Per-player numbers the server counts during a round.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct RoundStats {
    pub falls: u32,
    pub shortcuts: u32,
    /// Knockouts caused (others fell or were eliminated after our hit).
    pub kos: u32,
    pub grabs: u32,
    pub tackles: u32,
    /// Finish time in a race (s since the start).
    pub finish_at: Option<f64>,
    /// When the player was eliminated in a survival round (s since the start).
    pub out_at: Option<f64>,
}

pub struct RoundView<'a> {
    pub genre: Genre,
    pub participants: &'a [u32],
    pub connected: &'a dyn Fn(u32) -> bool,
    /// In finishing order.
    pub finished: &'a [u32],
    /// In elimination order.
    pub out: &'a [u32],
    pub scores: &'a BTreeMap<u32, f64>,
    pub progress: &'a dyn Fn(u32) -> f64,
    pub time_up: bool,
    pub solo: bool,
    /// Bots among the participants (a round with only bots left in it ends early).
    pub bots: Option<&'a BTreeSet<u32>>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct RoundRow {
    pub id: u32,
    pub place: usize,
    /// Placement points (0…10).
    pub points: i64,
    /// Points taken for falls and shortcuts.
    pub penalty: i64,
    /// Change of the game total (points − penalty, the total never going below 0).
    pub delta: i64,
    pub total: i64,
    /// Did the job: finished, survived, scored.
    pub ok: bool,
    pub note: String,
    pub falls: u32,
}

fn in_round(r: &RoundView) -> Vec<u32> {
    r.participants.iter().copied().filter(|&id| (r.connected)(id)).collect()
}

fn remaining(r: &RoundView) -> Vec<u32> {
    in_round(r)
        .into_iter()
        .filter(|id| !r.finished.contains(id) && !r.out.contains(id))
        .collect()
}

fn score_of(r: &RoundView, id: u32) -> f64 {
    r.scores.get(&id).copied().unwrap_or(0.0)
}

/// Every human in the round has finished or is out, and only bots are still going.
pub fn only_bots_left(r: &RoundView) -> bool {
    let Some(bots) = r.bots else { return false };
    // (Points games have no finish nor eliminations: they run their time.)
    if r.genre == Genre::Points {
        return false;
    }
    let rem = remaining(r);
    !rem.is_empty() && rem.iter().all(|id| bots.contains(id)) && in_round(r).iter().any(|id| !bots.contains(id))
}

pub fn is_round_over(r: &RoundView) -> bool {
    if r.time_up {
        return true;
    }
    let rem = remaining(r).len();
    if rem == 0 || only_bots_left(r) {
        return true;
    }
    // Survival: the last bean standing has nothing left to prove.
    r.genre == Genre::Survival && !r.solo && rem <= 1
}

/// Ranked groups, best first; players in one group tie. `rng` places the bots left when a round
/// ends early with only bots in it.
pub fn rank_groups(r: &RoundView, rng: Option<&mut Rng>) -> Vec<Vec<u32>> {
    let ids = in_round(r);
    let early = !r.time_up && only_bots_left(r);
    let bots: Vec<Vec<u32>> = if early {
        let mut rem = remaining(r);
        if let Some(rng) = rng {
            shuffle(&mut rem, rng);
        }
        rem.into_iter().map(|id| vec![id]).collect()
    } else {
        Vec::new()
    };
    match r.genre {
        Genre::Race => {
            let fin: Vec<u32> = r.finished.iter().copied().filter(|id| ids.contains(id)).collect();
            let rest: Vec<u32> = ids
                .iter()
                .copied()
                .filter(|id| !fin.contains(id) && !bots.iter().any(|b| b[0] == *id))
                .collect();
            let mut out: Vec<Vec<u32>> = fin.iter().map(|&id| vec![id]).collect();
            out.extend(bots);
            out.extend(group_by(&rest, |id| (r.progress)(id).round()));
            out
        }
        Genre::Survival => {
            let outs: Vec<u32> = r.out.iter().copied().filter(|id| ids.contains(id)).collect();
            let stayed: Vec<u32> = ids.iter().copied().filter(|id| !outs.contains(id)).collect();
            let gone = outs.iter().rev().map(|&id| vec![id]);
            if early {
                return bots.into_iter().chain(gone).collect();
            }
            let mut out = Vec::new();
            if !stayed.is_empty() {
                out.push(stayed);
            }
            out.extend(gone);
            out
        }
        Genre::Points => group_by(&ids, |id| score_of(r, id)),
    }
}

/// Sorted by key, best first (a stable sort), equal keys grouped. A NaN key ranks last (as −∞):
/// the comparison stays a total order and those players share one group.
fn group_by(ids: &[u32], key: impl Fn(u32) -> f64) -> Vec<Vec<u32>> {
    let key = |id| {
        let k = key(id);
        if k.is_nan() { f64::NEG_INFINITY } else { k }
    };
    let mut sorted = ids.to_vec();
    sorted.sort_by(|&a, &b| key(b).partial_cmp(&key(a)).unwrap_or(core::cmp::Ordering::Equal));
    let mut groups: Vec<Vec<u32>> = Vec::new();
    for id in sorted {
        match groups.last_mut() {
            Some(last) if key(last[0]) == key(id) => last.push(id),
            _ => groups.push(vec![id]),
        }
    }
    groups
}

/// Placement points: ties share the average of the places they occupy. Returns (id, place, points).
pub fn placement_points(groups: &[Vec<u32>]) -> BTreeMap<u32, (usize, i64)> {
    let n: usize = groups.iter().map(Vec::len).sum();
    let mut out = BTreeMap::new();
    let mut pos = 0usize;
    for g in groups {
        let avg = pos as f64 + (g.len() as f64 - 1.0) / 2.0;
        let points = if n <= 1 {
            TOP_POINTS
        } else {
            ((TOP_POINTS as f64 * (n as f64 - 1.0 - avg)) / (n as f64 - 1.0)).round() as i64
        };
        for &id in g {
            out.insert(id, (pos + 1, points));
        }
        pos += g.len();
    }
    out
}

/// Did the player do what the round asked (finish, survive, score)?
fn succeeded(r: &RoundView, id: u32) -> bool {
    match r.genre {
        Genre::Race => r.finished.contains(&id),
        Genre::Survival => !r.out.contains(&id),
        Genre::Points => score_of(r, id) > 0.0,
    }
}

fn note(r: &RoundView, id: u32, s: &RoundStats) -> String {
    match r.genre {
        Genre::Race => s.finish_at.map_or_else(
            || "без финиша".to_string(),
            |t| format!("финиш за {:.1} с", t).replace('.', ","),
        ),
        Genre::Survival => s
            .out_at
            .map_or_else(|| "до конца раунда".to_string(), |t| format!("в игре {} с", t.floor())),
        Genre::Points => format!("очки: {}", score_of(r, id)),
    }
}

/// Scores a finished round: placement points by rank (scaled to the number of players), minus
/// penalties for falls and shortcuts, capped.
/// `totals` are the game totals before this round; a total never drops below 0.
pub fn score_round(
    r: &RoundView,
    stats: &BTreeMap<u32, RoundStats>,
    totals: &BTreeMap<u32, i64>,
    rng: Option<&mut Rng>,
) -> Vec<RoundRow> {
    let groups = rank_groups(r, rng);
    let placed = placement_points(&groups);
    let count: usize = groups.iter().map(Vec::len).sum();
    // Alone in a round there is nobody to rank against: points for doing the job.
    let solo_points = |id| {
        if succeeded(r, id) {
            TOP_POINTS
        } else {
            (TOP_POINTS as f64 / 2.0).round() as i64
        }
    };
    let mut rows = Vec::new();
    for g in &groups {
        for &id in g {
            let s = stats.get(&id).copied().unwrap_or_default();
            let (place, placed_points) = placed[&id];
            let points = if r.solo || count == 1 {
                solo_points(id)
            } else {
                placed_points
            };
            // Falling out of a survival round already costs its place: only races and points games add a fine.
            let fall_fine = if r.genre == Genre::Survival {
                0
            } else {
                i64::from(s.falls) * FALL_PENALTY
            };
            let penalty = MAX_PENALTY.min(fall_fine + i64::from(s.shortcuts) * SHORTCUT_PENALTY);
            let before = totals.get(&id).copied().unwrap_or(0);
            let total = (before + points - penalty).max(0);
            rows.push(RoundRow {
                id,
                place,
                points,
                penalty,
                delta: total - before,
                total,
                ok: succeeded(r, id),
                note: note(r, id, &s),
                falls: s.falls,
            });
        }
    }
    rows
}

#[cfg(test)]
mod tests {
    use super::*;

    struct V {
        genre: Genre,
        finished: Vec<u32>,
        out: Vec<u32>,
        scores: BTreeMap<u32, f64>,
        progress: BTreeMap<u32, f64>,
        time_up: bool,
    }

    fn v(genre: Genre) -> V {
        V {
            genre,
            finished: vec![],
            out: vec![],
            scores: BTreeMap::new(),
            progress: BTreeMap::new(),
            time_up: false,
        }
    }

    const IDS: [u32; 4] = [1, 2, 3, 4];

    fn with<R>(x: &V, f: impl FnOnce(&RoundView) -> R) -> R {
        let connected = |_| true;
        let progress = |id| x.progress.get(&id).copied().unwrap_or(0.0);
        let view = RoundView {
            genre: x.genre,
            participants: &IDS,
            connected: &connected,
            finished: &x.finished,
            out: &x.out,
            scores: &x.scores,
            progress: &progress,
            time_up: x.time_up,
            solo: false,
            bots: None,
        };
        f(&view)
    }

    fn stats(list: &[(u32, RoundStats)]) -> BTreeMap<u32, RoundStats> {
        list.iter().copied().collect()
    }

    fn score(x: &V, s: &BTreeMap<u32, RoundStats>, totals: &BTreeMap<u32, i64>) -> Vec<RoundRow> {
        with(x, |r| score_round(r, s, totals, None))
    }

    #[test]
    fn spreads_placement_points_ties_sharing() {
        let p = placement_points(&[vec![1], vec![2, 3], vec![4]]);
        assert_eq!(p[&1], (1, 10));
        assert_eq!(p[&2], (2, 5));
        assert_eq!(p[&3], (2, 5));
        assert_eq!(p[&4], (4, 0));
    }

    #[test]
    fn ranks_races_by_finish_order_then_progress() {
        let mut x = v(Genre::Race);
        x.finished = vec![2, 1];
        x.progress = [(3, 50.0), (4, 80.0)].into();
        let s = stats(&[
            (
                2,
                RoundStats {
                    finish_at: Some(40.0),
                    ..Default::default()
                },
            ),
            (
                1,
                RoundStats {
                    finish_at: Some(45.0),
                    ..Default::default()
                },
            ),
        ]);
        let rows = score(&x, &s, &BTreeMap::new());
        assert_eq!(rows.iter().map(|r| r.id).collect::<Vec<_>>(), [2, 1, 4, 3]);
        assert_eq!(rows.iter().map(|r| r.points).collect::<Vec<_>>(), [10, 7, 3, 0]);
        assert_eq!(rows[0].note, "финиш за 40,0 с");
    }

    #[test]
    fn ranks_survival_by_elimination_survivors_share_the_top() {
        let mut x = v(Genre::Survival);
        x.out = vec![4, 3];
        x.time_up = true;
        let rows = score(&x, &BTreeMap::new(), &BTreeMap::new());
        let pts = |id| rows.iter().find(|r| r.id == id).unwrap().points;
        assert_eq!([pts(1), pts(2), pts(3), pts(4)], [8, 8, 3, 0]);
    }

    #[test]
    fn fines_falls_and_shortcuts_capped() {
        let mut x = v(Genre::Race);
        x.finished = vec![1, 2, 3, 4];
        let st = |f: u32, sc: u32| RoundStats {
            falls: f,
            shortcuts: sc,
            ..Default::default()
        };
        let s = stats(&[(1, st(2, 0)), (2, st(9, 0)), (3, st(0, 1))]);
        let rows = score(&x, &s, &[(4, 3)].into());
        let by = |id| rows.iter().find(|r| r.id == id).unwrap().clone();
        assert_eq!((by(1).points, by(1).penalty, by(1).delta), (10, 2, 8));
        assert_eq!((by(2).points, by(2).penalty, by(2).delta), (7, 4, 3));
        assert_eq!((by(3).points, by(3).penalty, by(3).delta), (3, 2, 1));
        assert_eq!((by(4).points, by(4).total), (0, 3));
        let mut y = v(Genre::Race);
        y.finished = vec![2, 1];
        let broke = score(&y, &stats(&[(1, st(3, 0))]), &[(1, 1)].into());
        assert_eq!(broke.iter().find(|r| r.id == 1).unwrap().total, 1 + 7 - 3);
    }

    #[test]
    fn no_fall_fines_in_survival() {
        let mut x = v(Genre::Survival);
        x.out = vec![1];
        let s = stats(&[(
            1,
            RoundStats {
                falls: 1,
                ..Default::default()
            },
        )]);
        let rows = score(&x, &s, &BTreeMap::new());
        assert_eq!(rows.iter().find(|r| r.id == 1).unwrap().penalty, 0);
    }

    #[test]
    fn ranks_points_games_by_score() {
        let mut x = v(Genre::Points);
        x.scores = [(1, 5.0), (2, 20.0), (3, 5.0), (4, 0.0)].into();
        let rows = score(&x, &BTreeMap::new(), &BTreeMap::new());
        let got: Vec<(u32, i64)> = rows.iter().map(|r| (r.id, r.points)).collect();
        assert_eq!(got, [(2, 10), (1, 5), (3, 5), (4, 0)]);
    }

    #[test]
    fn nan_keys_rank_last_in_one_group() {
        let key = |id| match id {
            1 | 3 => f64::NAN,
            2 => 3.0,
            _ => 1.0,
        };
        assert_eq!(group_by(&IDS, key), [vec![2], vec![4], vec![1, 3]]);
    }

    #[test]
    fn round_over_rules() {
        let over = |genre, finished: &[u32], out: &[u32], time_up| {
            let mut x = v(genre);
            x.finished = finished.to_vec();
            x.out = out.to_vec();
            x.time_up = time_up;
            with(&x, is_round_over)
        };
        assert!(over(Genre::Survival, &[], &[1, 2, 3], false));
        assert!(!over(Genre::Survival, &[], &[1, 2], false));
        assert!(!over(Genre::Race, &[1, 2], &[], false));
        assert!(over(Genre::Race, &[1, 2, 3, 4], &[], false));
        assert!(over(Genre::Race, &[], &[], true));
    }
}
