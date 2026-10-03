//! The rounds of one game (port of `server/rooms/director.ts`).
use fb_shared::game::{GameMeta, Genre};
use fb_shared::rng::{Rng, shuffle};

use crate::GAMES;

/// Game lengths the host can pick.
pub const ROUND_COUNTS: [u32; 3] = [3, 5, 7];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Mix,
    Races,
    Survival,
    Custom,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Playlist {
    pub mode: Mode,
    pub games: Vec<String>,
    pub rounds: u32,
}

impl Default for Playlist {
    fn default() -> Self {
        Self {
            mode: Mode::Mix,
            games: Vec::new(),
            rounds: 5,
        }
    }
}

pub fn game(id: &str) -> Option<&'static GameMeta> {
    GAMES.iter().map(|m| m.meta()).find(|g| g.id == id)
}

fn fits(g: &GameMeta, players: u32) -> bool {
    g.min_players.unwrap_or(1) <= players
}

fn pool_for(mode: Mode, players: u32) -> Vec<&'static GameMeta> {
    GAMES
        .iter()
        .map(|m| m.meta())
        .filter(|g| fits(g, players))
        .filter(|g| match mode {
            Mode::Races => g.genre == Genre::Race,
            Mode::Survival => g.genre != Genre::Race,
            Mode::Mix | Mode::Custom => true,
        })
        .collect()
}

/// The rounds of one game. Every player plays every round; genres alternate where possible and a big
/// "finale" map closes the game when the pool has one.
pub fn plan_game(players: u32, pl: &Playlist, rng: &mut Rng) -> Vec<&'static str> {
    let custom: Vec<&'static str> = pl
        .games
        .iter()
        .filter_map(|id| game(id))
        .filter(|g| fits(g, players))
        .map(|g| g.id)
        .collect();
    if pl.mode == Mode::Custom && !custom.is_empty() {
        return custom;
    }
    let n = pl.rounds.clamp(1, 12) as usize;
    let pool = pool_for(if pl.mode == Mode::Custom { Mode::Mix } else { pl.mode }, players);
    let mut finales: Vec<&GameMeta> = pool.iter().copied().filter(|g| g.finale).collect();
    let last = if n > 1 && !finales.is_empty() {
        shuffle(&mut finales, rng);
        Some(finales[0].id)
    } else {
        None
    };
    let mut bag: Vec<&'static str> = Vec::new();
    let mut rounds: Vec<&'static str> = Vec::new();
    let mut last_genre = None;
    let want = if last.is_some() { n - 1 } else { n };
    while rounds.len() < want {
        if bag.is_empty() {
            let mut ids: Vec<&'static str> = pool
                .iter()
                .map(|g| g.id)
                .filter(|&id| Some(id) != last || pool.len() == 1)
                .collect();
            shuffle(&mut ids, rng);
            bag.extend(ids);
        }
        let i = bag
            .iter()
            .position(|&id| game(id).map(|g| g.genre) != last_genre && !rounds.contains(&id))
            .unwrap_or(0);
        let id = bag.remove(i);
        rounds.push(id);
        last_genre = game(id).map(|g| g.genre);
    }
    if let Some(l) = last {
        rounds.push(l);
    }
    rounds
}

/// A playlist with unknown games dropped and the length one the host can pick.
pub fn valid_playlist(pl: &Playlist) -> Playlist {
    Playlist {
        mode: pl.mode,
        games: pl
            .games
            .iter()
            .filter(|id| game(id).is_some())
            .take(12)
            .cloned()
            .collect(),
        rounds: if ROUND_COUNTS.contains(&pl.rounds) {
            pl.rounds
        } else {
            5
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn with_rounds(rounds: u32) -> Playlist {
        Playlist {
            rounds,
            ..Default::default()
        }
    }

    #[test]
    fn plans_rounds_with_a_finale_map_last() {
        for rounds in ROUND_COUNTS {
            let mut rng = Rng::new(42);
            for n in 2..=8 {
                let plan = plan_game(n, &with_rounds(rounds), &mut rng);
                assert_eq!(plan.len(), rounds as usize);
                assert!(game(plan[plan.len() - 1]).unwrap().finale);
                let mut uniq = plan.clone();
                uniq.sort_unstable();
                uniq.dedup();
                assert_eq!(uniq.len(), plan.len());
            }
        }
    }

    #[test]
    fn alternates_genres_where_it_can() {
        let plan = plan_game(6, &with_rounds(5), &mut Rng::new(9));
        let genres: Vec<Genre> = plan[..plan.len() - 1]
            .iter()
            .map(|id| game(id).unwrap().genre)
            .collect();
        for w in genres.windows(2) {
            assert_ne!(w[0], w[1]);
        }
    }

    #[test]
    fn respects_minimum_player_counts_and_playlist_modes() {
        let mut rng = Rng::new(1);
        for _ in 0..50 {
            assert!(!plan_game(1, &Playlist::default(), &mut rng).contains(&"tail-tag"));
            let races = Playlist {
                mode: Mode::Races,
                ..Default::default()
            };
            for id in plan_game(5, &races, &mut rng) {
                assert_eq!(game(id).unwrap().genre, Genre::Race);
            }
        }
    }

    #[test]
    fn uses_custom_playlists_as_given() {
        let pl = Playlist {
            mode: Mode::Custom,
            games: vec!["jump-club".into(), "door-dash".into(), "crown-peak".into()],
            rounds: 5,
        };
        assert_eq!(
            plan_game(4, &pl, &mut Rng::new(3)),
            ["jump-club", "door-dash", "crown-peak"]
        );
    }

    #[test]
    fn sanitizes_playlists() {
        let pl = Playlist {
            mode: Mode::Custom,
            games: vec!["nope".into(), "hex-a-gone".into(), "wall-rush".into()],
            rounds: 4,
        };
        assert_eq!(
            valid_playlist(&pl),
            Playlist {
                mode: Mode::Custom,
                games: vec!["hex-a-gone".into(), "wall-rush".into()],
                rounds: 5,
            }
        );
    }
}
