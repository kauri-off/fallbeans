//! Fun titles at the end of a game.
use fb_proto::{Award, AwardKind, Pid};

/// A player's numbers over a whole game.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct GameStats {
    pub falls: u32,
    pub kos: u32,
    pub grabs: u32,
    pub tackles: u32,
    pub shortcuts: u32,
    /// Rounds won (placed first).
    pub wins: u32,
    /// Places in race rounds, normalized to 0 (first) … 1 (last).
    pub race_ranks: Vec<f64>,
    /// Seconds survived in survival rounds (the full round when not eliminated).
    pub survived: f64,
}

/// The single best player for a score at or above `min` (ties: nobody).
fn best(players: &[(Pid, &GameStats)], score: impl Fn(&GameStats) -> Option<f64>, min: f64) -> Option<(Pid, f64)> {
    let mut top: Option<(Pid, f64)> = None;
    let mut tie = false;
    for (id, s) in players {
        let Some(v) = score(s) else { continue };
        if v < min {
            continue;
        }
        match top {
            Some((_, t)) if v == t => tie = true,
            Some((_, t)) if v < t => {}
            _ => {
                top = Some((*id, v));
                tie = false;
            }
        }
    }
    top.filter(|_| !tie)
}

/// Each award goes to the single best player for it.
pub fn compute_awards(players: &[(Pid, &GameStats)]) -> Vec<Award> {
    let rank = |s: &GameStats| {
        (!s.race_ranks.is_empty()).then(|| 1.0 - s.race_ranks.iter().sum::<f64>() / s.race_ranks.len() as f64)
    };
    [
        (AwardKind::Fastest, best(players, rank, 0.5)),
        (AwardKind::Survivor, best(players, |s| Some(s.survived.round()), 1.0)),
        (AwardKind::Bully, best(players, |s| Some(s.kos.into()), 1.0)),
        (AwardKind::Grabber, best(players, |s| Some(s.grabs.into()), 3.0)),
        (AwardKind::Clumsy, best(players, |s| Some(s.falls.into()), 2.0)),
        (AwardKind::Sly, best(players, |s| Some(s.shortcuts.into()), 1.0)),
    ]
    .into_iter()
    .filter_map(|(kind, r)| {
        r.map(|(id, v)| Award {
            kind,
            id,
            value: v as u32,
        })
    })
    .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn awards_the_single_best() {
        let a = GameStats {
            falls: 3,
            kos: 2,
            race_ranks: vec![0.0, 0.5],
            survived: 41.6,
            ..Default::default()
        };
        let b = GameStats {
            falls: 3,
            grabs: 2,
            race_ranks: vec![1.0],
            ..Default::default()
        };
        let awards = compute_awards(&[(1, &a), (2, &b)]);
        let got: Vec<(AwardKind, Pid, u32)> = awards.iter().map(|w| (w.kind, w.id, w.value)).collect();
        assert_eq!(
            got,
            [
                (AwardKind::Fastest, 1, 0),
                (AwardKind::Survivor, 1, 42),
                (AwardKind::Bully, 1, 2),
            ]
        );
    }
}
