//! Someone in a room.
use fb_proto::{Outfit, PlayerId};

use super::awards::GameStats;
use super::{Backoff, ConnId, RateWindow, Uid};

/// A person (connected, or within the reconnect grace) or a bot.
#[derive(Clone, Debug)]
pub struct Player {
    pub id: PlayerId,
    pub name: String,
    /// Index into `fb_shared::COLORS`.
    pub color: u8,
    pub outfit: Outfit,
    /// Points in the current game.
    pub score: i64,
    pub crowns: u32,
    pub stats: GameStats,
    pub kind: Kind,
    pub spectator: bool,
    /// Control messages in the current second.
    pub msgs: RateWindow,
    pub chat_at: Option<u64>,
    pub emote_at: Option<u64>,
    /// The "rate limited" warning about this player.
    pub rate_log: Backoff,
    /// Round trip (ms), as the network layer measures it.
    pub rtt: u32,
}

#[derive(Clone, Debug)]
pub enum Kind {
    Human {
        uid: Uid,
        conn: Option<ConnId>,
        /// Server tick the connection was lost at (None while connected).
        disconnected_at: Option<u64>,
    },
    /// `auto`: added to fill the empty places (it goes when the host turns that off).
    Bot { auto: bool },
}

impl Player {
    fn new(id: PlayerId, name: String, color: u8, outfit: Outfit, kind: Kind) -> Self {
        Self {
            id,
            name,
            color,
            outfit,
            score: 0,
            crowns: 0,
            stats: GameStats::default(),
            kind,
            spectator: false,
            msgs: RateWindow::default(),
            chat_at: None,
            emote_at: None,
            rate_log: Backoff::default(),
            rtt: 0,
        }
    }

    pub fn human(id: PlayerId, name: String, color: u8, outfit: Outfit, uid: Uid, conn: ConnId) -> Self {
        let kind = Kind::Human {
            uid,
            conn: Some(conn),
            disconnected_at: None,
        };
        Self::new(id, name, color, outfit, kind)
    }

    pub fn bot(id: PlayerId, name: String, color: u8, outfit: Outfit, auto: bool) -> Self {
        Self::new(id, name, color, outfit, Kind::Bot { auto })
    }

    pub fn is_bot(&self) -> bool {
        matches!(self.kind, Kind::Bot { .. })
    }

    /// A bot that only fills an empty place.
    pub fn auto(&self) -> bool {
        matches!(self.kind, Kind::Bot { auto: true })
    }

    pub fn uid(&self) -> Option<&Uid> {
        match &self.kind {
            Kind::Human { uid, .. } => Some(uid),
            Kind::Bot { .. } => None,
        }
    }

    /// The person's connection (None for bots and the disconnected).
    pub fn conn(&self) -> Option<ConnId> {
        match self.kind {
            Kind::Human { conn, .. } => conn,
            Kind::Bot { .. } => None,
        }
    }

    /// Takes part: a bot, or a connected person.
    pub fn present(&self) -> bool {
        match self.kind {
            Kind::Human { conn, .. } => conn.is_some(),
            Kind::Bot { .. } => true,
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
pub fn bot_name<'a>(taken: impl IntoIterator<Item = &'a str> + Clone, id: PlayerId) -> String {
    BOT_NAMES
        .iter()
        .map(|n| format!("Бот {n}"))
        .find(|n| !taken.clone().into_iter().any(|t| t == n))
        .unwrap_or_else(|| format!("Бот {id}"))
}
