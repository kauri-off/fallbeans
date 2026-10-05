//! Someone in a room (port of `server/rooms/players.ts`).
use fb_proto::{Outfit, Pid};

use super::ConnId;
use super::awards::GameStats;

/// A person (connected, or within the reconnect grace) or a bot.
#[derive(Clone, Debug)]
pub struct Player {
    pub id: Pid,
    pub name: String,
    /// Index into `fb_shared::COLORS`.
    pub color: u8,
    pub outfit: Outfit,
    /// Points in the current game.
    pub score: i64,
    pub crowns: u32,
    pub stats: GameStats,
    /// The person's identity; empty for bots.
    pub uid: String,
    pub bot: bool,
    /// A bot added to fill the empty places (it goes when the host turns that off).
    pub auto: bool,
    pub conn: Option<ConnId>,
    /// Server tick the connection was lost at.
    pub disconnected_at: u64,
    pub spectator: bool,
    pub msg_window: u64,
    pub msg_count: u32,
    pub chat_at: Option<u64>,
    /// Round trip (ms), as the network layer measures it.
    pub rtt: u32,
}

impl Player {
    pub fn new(id: Pid, name: String, color: u8, outfit: Outfit) -> Self {
        Self {
            id,
            name,
            color,
            outfit,
            score: 0,
            crowns: 0,
            stats: GameStats::default(),
            uid: String::new(),
            bot: true,
            auto: false,
            conn: None,
            disconnected_at: 0,
            spectator: false,
            msg_window: 0,
            msg_count: 0,
            chat_at: None,
            rtt: 0,
        }
    }
}

const BOT_NAMES: [&str; 10] = [
    "Кекс",
    "Пончик",
    "Жужа",
    "Бублик",
    "Мармелад",
    "Хрустик",
    "Пельмень",
    "Зефир",
    "Кнопка",
    "Шмель",
];

/// A bot name nobody in the room has yet.
pub fn bot_name<'a>(taken: impl IntoIterator<Item = &'a str> + Clone, id: Pid) -> String {
    BOT_NAMES
        .iter()
        .map(|n| format!("Бот {n}"))
        .find(|n| !taken.clone().into_iter().any(|t| t == n))
        .unwrap_or_else(|| format!("Бот {id}"))
}
