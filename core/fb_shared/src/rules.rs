//! Scoring a round: placement points by rank, fines for falls and shortcuts.
use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use crate::PlayerId;
use crate::game::Genre;
use crate::rng::{Rng, shuffle};

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

/// The round as the rules see it. Beans that finish or drop out on the same tick come in the arena's pawn
/// order (the order they joined): such ties go to the one that joined first.
pub struct RoundView<'a> {
    pub genre: Genre,
    /// The beans the round started with (a later round of the same game may have more or fewer).
    pub participants: &'a [PlayerId],
    pub connected: &'a dyn Fn(PlayerId) -> bool,
    /// In finishing order.
    pub finished: &'a [PlayerId],
    /// In elimination order.
    pub out: &'a [PlayerId],
    /// When a bean in `out` dropped out: those of one moment keep their order between them.
    pub out_at: &'a dyn Fn(PlayerId) -> Option<f64>,
    pub scores: &'a BTreeMap<PlayerId, i64>,
    pub progress: &'a dyn Fn(PlayerId) -> f64,
    pub time_up: bool,
    /// Bots among the participants (a round with only bots left in it ends early).
    pub bots: Option<&'a BTreeSet<PlayerId>>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct RoundRow {
    pub id: PlayerId,
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
    pub note: RoundNote,
    pub falls: u32,
}

/// What a player's round came to, by its genre.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub enum RoundNote {
    /// The finish time (s), or None without one.
    Finish(Option<f64>),
    /// Seconds in play before falling out, or None: until the end.
    Survived(Option<f64>),
    Points(i64),
}

impl RoundView<'_> {
    fn in_round(&self) -> Vec<PlayerId> {
        self.participants
            .iter()
            .copied()
            .filter(|&id| (self.connected)(id))
            .collect()
    }

    fn remaining(&self) -> Vec<PlayerId> {
        self.in_round()
            .into_iter()
            .filter(|id| !self.finished.contains(id) && !self.out.contains(id))
            .collect()
    }

    fn score_of(&self, id: PlayerId) -> i64 {
        self.scores.get(&id).copied().unwrap_or(0)
    }

    /// Every human in the round has finished or is out, and only bots are still going.
    pub fn only_bots_left(&self) -> bool {
        let Some(bots) = self.bots else { return false };
        // (Points games have no finish nor eliminations: they run their time.)
        if self.genre == Genre::Points {
            return false;
        }
        let rem = self.remaining();
        !rem.is_empty() && rem.iter().all(|id| bots.contains(id)) && self.in_round().iter().any(|id| !bots.contains(id))
    }

    pub fn is_round_over(&self) -> bool {
        if self.time_up {
            return true;
        }
        let rem = self.remaining().len();
        if rem == 0 || self.only_bots_left() {
            return true;
        }
        // Survival: the last bean standing has nothing left to prove, once somebody has really dropped out
        // (not when the others only left, nor in a round it plays alone). Nobody drops out before the start.
        self.genre == Genre::Survival && rem <= 1 && !self.out.is_empty()
    }

    /// Ranked groups, best first; equal groups tie. Bots still racing are ranked by progress, and `rng` orders
    /// surviving bots.
    pub fn rank_groups(&self, rng: Option<&mut Rng>) -> Vec<Vec<PlayerId>> {
        let ids = self.in_round();
        let early = !self.time_up && self.only_bots_left();
        let bots: Vec<Vec<PlayerId>> = if !early {
            Vec::new()
        } else if self.genre == Genre::Race {
            group_by(&self.remaining(), |id| (self.progress)(id).round())
        } else {
            let mut rem = self.remaining();
            if let Some(rng) = rng {
                shuffle(&mut rem, rng);
            }
            rem.into_iter().map(|id| vec![id]).collect()
        };
        match self.genre {
            Genre::Race => {
                let fin: Vec<PlayerId> = self.finished.iter().copied().filter(|id| ids.contains(id)).collect();
                let rest: Vec<PlayerId> = ids
                    .iter()
                    .copied()
                    .filter(|id| !fin.contains(id) && !bots.iter().any(|b| b.contains(id)))
                    .collect();
                let mut out: Vec<Vec<PlayerId>> = fin.iter().map(|&id| vec![id]).collect();
                out.extend(bots);
                out.extend(group_by(&rest, |id| (self.progress)(id).round()));
                out
            }
            Genre::Survival => {
                let outs: Vec<PlayerId> = self.out.iter().copied().filter(|id| ids.contains(id)).collect();
                let stayed: Vec<PlayerId> = ids.iter().copied().filter(|id| !outs.contains(id)).collect();
                let mut gone = Vec::new();
                for same in outs.chunk_by(|&a, &b| (self.out_at)(a) == (self.out_at)(b)).rev() {
                    gone.extend(same.iter().map(|&id| vec![id]));
                }
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
            Genre::Points => group_by(&ids, |id| self.score_of(id) as f64),
        }
    }

    /// Did the player do what the round asked (finish, survive, score)?
    fn succeeded(&self, id: PlayerId) -> bool {
        match self.genre {
            Genre::Race => self.finished.contains(&id),
            Genre::Survival => !self.out.contains(&id),
            Genre::Points => self.score_of(id) > 0,
        }
    }

    fn note(&self, id: PlayerId, s: &RoundStats) -> RoundNote {
        match self.genre {
            Genre::Race => RoundNote::Finish(s.finish_at),
            Genre::Survival => RoundNote::Survived(s.out_at),
            Genre::Points => RoundNote::Points(self.score_of(id)),
        }
    }

    /// Scores a finished round; `totals` are the game totals before it, and a total never drops below 0.
    pub fn score_round(
        &self,
        stats: &BTreeMap<PlayerId, RoundStats>,
        totals: &BTreeMap<PlayerId, i64>,
        rng: Option<&mut Rng>,
    ) -> Vec<RoundRow> {
        let groups = self.rank_groups(rng);
        let placed = placement_points(&groups);
        let count: usize = groups.iter().map(Vec::len).sum();
        // Alone in a round there is nobody to rank against: points for doing the job.
        let solo_points = |id| {
            if self.succeeded(id) {
                TOP_POINTS
            } else {
                (TOP_POINTS + 1) / 2
            }
        };
        let mut rows = Vec::new();
        for g in &groups {
            for &id in g {
                let s = stats.get(&id).copied().unwrap_or_default();
                let (place, placed_points) = placed[&id];
                let points = if count == 1 { solo_points(id) } else { placed_points };
                // Falling out of a survival round already costs its place: only races and points games add a fine.
                let fall_fine = if self.genre == Genre::Survival {
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
                    ok: self.succeeded(id),
                    note: self.note(id, &s),
                    falls: s.falls,
                });
            }
        }
        rows
    }
}

/// Sorted by key, best first (a stable sort), equal keys grouped. A NaN key ranks last (as −∞):
/// the comparison stays a total order and those players share one group.
fn group_by(ids: &[PlayerId], key: impl Fn(PlayerId) -> f64) -> Vec<Vec<PlayerId>> {
    let key = |id| {
        let k = key(id);
        if k.is_nan() { f64::NEG_INFINITY } else { k }
    };
    let mut sorted = ids.to_vec();
    sorted.sort_by(|&a, &b| key(b).total_cmp(&key(a)));
    let mut groups: Vec<Vec<PlayerId>> = Vec::new();
    for id in sorted {
        match groups.last_mut() {
            Some(last) if key(last[0]) == key(id) => last.push(id),
            _ => groups.push(vec![id]),
        }
    }
    groups
}

/// Placement points: ties share the average of the places they occupy. Returns (id, place, points).
#[expect(clippy::cast_possible_truncation, reason = "rounded points within 0…TOP_POINTS")]
pub fn placement_points(groups: &[Vec<PlayerId>]) -> BTreeMap<PlayerId, (usize, i64)> {
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

#[cfg(test)]
mod tests {
    use super::*;

    struct V {
        genre: Genre,
        ids: Vec<u32>,
        /// Those of `ids` who left.
        gone: Vec<u32>,
        finished: Vec<u32>,
        out: Vec<u32>,
        /// Unset: each bean in `out` on a tick of its own.
        out_at: BTreeMap<u32, f64>,
        scores: BTreeMap<u32, i64>,
        progress: BTreeMap<u32, f64>,
        time_up: bool,
        bots: Option<BTreeSet<u32>>,
    }

    fn v(genre: Genre) -> V {
        V {
            genre,
            ids: IDS.to_vec(),
            gone: vec![],
            finished: vec![],
            out: vec![],
            out_at: BTreeMap::new(),
            scores: BTreeMap::new(),
            progress: BTreeMap::new(),
            time_up: false,
            bots: None,
        }
    }

    const IDS: [u32; 4] = [1, 2, 3, 4];

    fn pids(ids: &[u32]) -> Vec<PlayerId> {
        ids.iter().copied().map(PlayerId).collect()
    }

    fn plain(groups: Vec<Vec<PlayerId>>) -> Vec<Vec<u32>> {
        groups
            .into_iter()
            .map(|g| g.into_iter().map(|id| id.0).collect())
            .collect()
    }

    fn with<R>(x: &V, f: impl FnOnce(&RoundView) -> R) -> R {
        let connected = |id: PlayerId| !x.gone.contains(&id.0);
        let progress = |id: PlayerId| x.progress.get(&id.0).copied().unwrap_or(0.0);
        let out_at = |id: PlayerId| {
            let at = x.out.iter().position(|&o| o == id.0).map(|i| i as f64);
            x.out_at.get(&id.0).copied().or(at)
        };
        let (ids, finished, out) = (pids(&x.ids), pids(&x.finished), pids(&x.out));
        let scores = x.scores.iter().map(|(&id, &v)| (PlayerId(id), v)).collect();
        let bots = x
            .bots
            .as_ref()
            .map(|b| b.iter().copied().map(PlayerId).collect::<BTreeSet<_>>());
        let view = RoundView {
            genre: x.genre,
            participants: &ids,
            connected: &connected,
            finished: &finished,
            out: &out,
            out_at: &out_at,
            scores: &scores,
            progress: &progress,
            time_up: x.time_up,
            bots: bots.as_ref(),
        };
        f(&view)
    }

    fn stats(list: &[(u32, RoundStats)]) -> BTreeMap<PlayerId, RoundStats> {
        list.iter().map(|&(id, s)| (PlayerId(id), s)).collect()
    }

    fn score(x: &V, s: &BTreeMap<PlayerId, RoundStats>, totals: &BTreeMap<u32, i64>) -> Vec<RoundRow> {
        let totals = totals.iter().map(|(&id, &v)| (PlayerId(id), v)).collect();
        with(x, |r| r.score_round(s, &totals, None))
    }

    #[test]
    fn spreads_placement_points_ties_sharing() {
        let p = placement_points(&[pids(&[1]), pids(&[2, 3]), pids(&[4])]);
        assert_eq!(p[&PlayerId(1)], (1, 10));
        assert_eq!(p[&PlayerId(2)], (2, 5));
        assert_eq!(p[&PlayerId(3)], (2, 5));
        assert_eq!(p[&PlayerId(4)], (4, 0));
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
        assert_eq!(rows.iter().map(|r| r.id.0).collect::<Vec<_>>(), [2, 1, 4, 3]);
        assert_eq!(rows.iter().map(|r| r.points).collect::<Vec<_>>(), [10, 7, 3, 0]);
        assert_eq!(rows[0].note, RoundNote::Finish(Some(40.0)));
    }

    #[test]
    fn ranks_survival_by_elimination_survivors_share_the_top() {
        let mut x = v(Genre::Survival);
        x.out = vec![4, 3];
        x.time_up = true;
        let rows = score(&x, &BTreeMap::new(), &BTreeMap::new());
        let pts = |id| rows.iter().find(|r| r.id.0 == id).unwrap().points;
        assert_eq!([pts(1), pts(2), pts(3), pts(4)], [8, 8, 3, 0]);
    }

    #[test]
    fn beans_out_on_one_tick_rank_in_join_order() {
        let mut x = v(Genre::Survival);
        x.out = vec![4, 2, 3];
        x.out_at = [(4, 5.0), (2, 9.0), (3, 9.0)].into();
        x.time_up = true;
        assert_eq!(
            plain(with(&x, |r| r.rank_groups(None))),
            [vec![1], vec![2], vec![3], vec![4]]
        );
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
        let by = |id| rows.iter().find(|r| r.id.0 == id).unwrap().clone();
        assert_eq!((by(1).points, by(1).penalty, by(1).delta), (10, 2, 8));
        assert_eq!((by(2).points, by(2).penalty, by(2).delta), (7, 4, 3));
        assert_eq!((by(3).points, by(3).penalty, by(3).delta), (3, 2, 1));
        assert_eq!((by(4).points, by(4).total), (0, 3));
        let mut y = v(Genre::Race);
        y.finished = vec![2, 1];
        let broke = score(&y, &stats(&[(1, st(3, 0))]), &[(1, 1)].into());
        assert_eq!(broke.iter().find(|r| r.id.0 == 1).unwrap().total, 1 + 7 - 3);
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
        assert_eq!(rows.iter().find(|r| r.id.0 == 1).unwrap().penalty, 0);
    }

    #[test]
    fn ranks_points_games_by_score() {
        let mut x = v(Genre::Points);
        x.scores = [(1, 5), (2, 20), (3, 5), (4, 0)].into();
        let rows = score(&x, &BTreeMap::new(), &BTreeMap::new());
        let got: Vec<(u32, i64)> = rows.iter().map(|r| (r.id.0, r.points)).collect();
        assert_eq!(got, [(2, 10), (1, 5), (3, 5), (4, 0)]);
    }

    #[test]
    fn nan_keys_rank_last_in_one_group() {
        let key = |id: PlayerId| match id.0 {
            1 | 3 => f64::NAN,
            2 => 3.0,
            _ => 1.0,
        };
        assert_eq!(plain(group_by(&pids(&IDS), key)), [vec![2], vec![4], vec![1, 3]]);
    }

    #[test]
    fn round_over_rules() {
        let over = |genre, finished: &[u32], out: &[u32], time_up| {
            let mut x = v(genre);
            x.finished = finished.to_vec();
            x.out = out.to_vec();
            x.time_up = time_up;
            with(&x, |r| r.is_round_over())
        };
        assert!(over(Genre::Survival, &[], &[1, 2, 3], false));
        assert!(!over(Genre::Survival, &[], &[1, 2], false));
        assert!(!over(Genre::Race, &[1, 2], &[], false));
        assert!(over(Genre::Race, &[1, 2, 3, 4], &[], false));
        assert!(over(Genre::Race, &[], &[], true));
    }

    #[test]
    fn survival_with_one_bean_in_it_is_played_out() {
        // The game started with others, but this round has one bean in it.
        let mut x = v(Genre::Survival);
        x.ids = vec![1];
        assert!(!with(&x, |r| r.is_round_over()));
        x.time_up = true;
        let rows = score(&x, &BTreeMap::new(), &BTreeMap::new());
        assert_eq!((rows[0].id.0, rows[0].points, rows[0].ok), (1, TOP_POINTS, true));
        x.time_up = false;
        x.out = vec![1];
        assert!(with(&x, |r| r.is_round_over()));
    }

    #[test]
    fn survival_ends_early_only_once_somebody_dropped_out() {
        let mut x = v(Genre::Survival);
        x.ids = vec![1, 2];
        // The other one left (during the intro, say): nobody has been beaten.
        x.gone = vec![2];
        assert!(!with(&x, |r| r.is_round_over()));
        x.gone = vec![];
        x.out = vec![2];
        assert!(with(&x, |r| r.is_round_over()));
    }

    #[test]
    fn a_game_started_alone_still_ends_and_scores_a_round_of_three() {
        // The game started alone, then two more came.
        let mut x = v(Genre::Survival);
        x.ids = vec![1, 2, 3];
        x.out = vec![2, 3];
        assert!(with(&x, |r| r.is_round_over()));
        let rows = score(&x, &BTreeMap::new(), &BTreeMap::new());
        let pts = |id| rows.iter().find(|r| r.id.0 == id).unwrap().points;
        assert_eq!([pts(1), pts(3), pts(2)], [10, 5, 0]);
    }

    #[test]
    fn bots_left_racing_rank_by_progress() {
        let mut x = v(Genre::Race);
        x.bots = Some([2, 3, 4].into());
        x.finished = vec![1];
        x.progress = [(2, 10.0), (3, 50.2), (4, 30.0)].into();
        assert!(with(&x, |r| r.is_round_over()));
        let groups = with(&x, |r| r.rank_groups(Some(&mut Rng::new(3))));
        assert_eq!(plain(groups), [vec![1], vec![3], vec![4], vec![2]]);
    }
}
