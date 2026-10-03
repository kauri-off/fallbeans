//! Fun titles at the end of a game (port of `server/rooms/awards.ts`).
use fb_proto::{Award, Pid};

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

fn plural(n: i64, one: &str, few: &str, many: &str) -> String {
    let (m10, m100) = (n % 10, n % 100);
    let word = if m10 == 1 && m100 != 11 {
        one
    } else if (2..=4).contains(&m10) && !(12..=14).contains(&m100) {
        few
    } else {
        many
    };
    format!("{n} {word}")
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
    let mut awards = Vec::new();
    let mut add = |key: &str, icon: &str, title: &str, r: Option<(Pid, f64)>, text: &dyn Fn(i64) -> String| {
        if let Some((id, v)) = r {
            awards.push(Award {
                key: key.into(),
                title: title.into(),
                icon: icon.into(),
                id,
                text: text(v as i64),
            });
        }
    };
    let rank = |s: &GameStats| {
        (!s.race_ranks.is_empty()).then(|| 1.0 - s.race_ranks.iter().sum::<f64>() / s.race_ranks.len() as f64)
    };
    add("fastest", "⚡", "Молния", best(players, rank, 0.5), &|_| {
        "лучшие места в гонках".into()
    });
    add(
        "survivor",
        "🛡️",
        "Несокрушимость",
        best(players, |s| Some(fb_shared::m::round_js(s.survived)), 1.0),
        &|v| format!("{v} с в игре"),
    );
    add(
        "bully",
        "💥",
        "Задира",
        best(players, |s| Some(s.kos.into()), 1.0),
        &|v| plural(v, "сбитый соперник", "сбитых соперника", "сбитых соперников"),
    );
    add(
        "grabber",
        "🤲",
        "Цепкие руки",
        best(players, |s| Some(s.grabs.into()), 3.0),
        &|v| plural(v, "захват", "захвата", "захватов"),
    );
    add(
        "clumsy",
        "🍌",
        "Неваляшка",
        best(players, |s| Some(s.falls.into()), 2.0),
        &|v| plural(v, "падение", "падения", "падений"),
    );
    add(
        "sly",
        "🦊",
        "Хитрая лиса",
        best(players, |s| Some(s.shortcuts.into()), 1.0),
        &|v| format!("{} пути (и штрафы за них)", plural(v, "срезка", "срезки", "срезок")),
    );
    awards
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
        let keys: Vec<(&str, Pid, &str)> = awards.iter().map(|w| (&*w.key, w.id, &*w.text)).collect();
        assert_eq!(
            keys,
            [
                ("fastest", 1, "лучшие места в гонках"),
                ("survivor", 1, "42 с в игре"),
                ("bully", 1, "2 сбитых соперника"),
            ]
        );
        assert_eq!(plural(21, "a", "b", "c"), "21 a");
        assert_eq!(plural(12, "a", "b", "c"), "12 c");
    }
}
