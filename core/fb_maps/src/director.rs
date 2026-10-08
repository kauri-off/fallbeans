//! The rounds of one game.
use fb_shared::game::{GameMeta, Genre};
use fb_shared::rng::{Rng, shuffle};
use serde::{Deserialize, Serialize};

use crate::GAMES;

/// Game lengths the host can pick.
pub const ROUND_COUNTS: [u32; 3] = [3, 5, 7];

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Mix,
    Races,
    Survival,
    Custom,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
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

/// The game can be played by this many. A plan is made for the players a game starts with: whoever runs
/// it checks again before each round (players leave), and skips a game that no longer fits.
pub fn fits(g: &GameMeta, players: u32) -> bool {
    g.min_players <= players
}

fn pool_for(mode: Mode, players: u32) -> Vec<&'static GameMeta> {
    GAMES
        .iter()
        .map(|m| m.meta())
        .filter(|g| fits(g, players))
        .filter(|g| match mode {
            Mode::Races => g.genre == Genre::Race,
            // As the name says: no points games (they are in the mix).
            Mode::Survival => g.genre == Genre::Survival,
            Mode::Mix | Mode::Custom => true,
        })
        .collect()
}

/// The rounds of one game. Every player plays every round; genres alternate where possible and a big
/// "finale" map closes the game when the pool has one.
pub fn plan_game(players: u32, pl: &Playlist, rng: &mut Rng) -> Vec<&'static str> {
    // (Nobody in the room yet: plan as for one, every game fits it.)
    let players = players.max(1);
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
    if pool.is_empty() {
        // (Every mode has games for one player; without any, the bag below would never fill.)
        return Vec::new();
    }
    let mut finales: Vec<&'static GameMeta> = pool.iter().copied().filter(|g| g.finale).collect();
    let last = if n > 1 && !finales.is_empty() {
        shuffle(&mut finales, rng);
        Some(finales[0])
    } else {
        None
    };
    let mut bag: Vec<&'static GameMeta> = Vec::new();
    let mut rounds: Vec<&'static GameMeta> = Vec::new();
    let mut last_genre = None;
    let want = if last.is_some() { n - 1 } else { n };
    while rounds.len() < want {
        if bag.is_empty() {
            let mut games: Vec<&'static GameMeta> = pool
                .iter()
                .copied()
                .filter(|g| last.is_none_or(|l| l.id != g.id) || pool.len() == 1)
                .collect();
            shuffle(&mut games, rng);
            bag.extend(games);
        }
        let i = bag
            .iter()
            .position(|g| Some(g.genre) != last_genre && !rounds.iter().any(|r| r.id == g.id))
            .unwrap_or(0);
        let g = bag.remove(i);
        rounds.push(g);
        last_genre = Some(g.genre);
    }
    rounds.extend(last);
    rounds.iter().map(|g| g.id).collect()
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
            let survival = Playlist {
                mode: Mode::Survival,
                ..Default::default()
            };
            for id in plan_game(5, &survival, &mut rng) {
                assert_eq!(game(id).unwrap().genre, Genre::Survival);
            }
        }
    }

    #[test]
    fn plans_a_game_for_an_empty_room() {
        for mode in [Mode::Mix, Mode::Races, Mode::Survival, Mode::Custom] {
            let pl = Playlist {
                mode,
                ..Default::default()
            };
            let plan = plan_game(0, &pl, &mut Rng::new(5));
            assert_eq!(plan.len(), 5, "{mode:?}");
            assert!(plan.iter().all(|id| fits(game(id).unwrap(), 1)), "{mode:?}");
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
