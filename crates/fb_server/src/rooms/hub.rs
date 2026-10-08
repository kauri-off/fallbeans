//! The rooms of the server and who is where.
//! Anyone may open one room of their own (they are its host whenever they are in it) and be in one room at
//! a time; a private room asks newcomers for its PIN. Rooms close when everybody has left. Practice rooms
//! are separate: one player, not listed.
use std::collections::BTreeMap;
use std::net::IpAddr;

use bevy::log::{error, info, warn};
use fb_proto::*;
use fb_shared::text::{sanitize_person_name, sanitize_title};
use fb_shared::*;

use super::room::{Game, Practice, Room, RoomOptions, Who, make_pin};
use super::{Backoff, ConnId, Inputs, Out, RateWindow, Uid, secs, ticks};
use crate::auth::{AddrKey, GUESSES_PER_TARGET, Limiter, address_key, random_bytes, same_key};

/// A connection that sent no hello within this long is closed (s).
const HELLO_TIMEOUT_S: f64 = 5.0;
/// Control messages per second from a connection at the room list (rooms have their own limit).
const LIST_RATE: u32 = 20;
/// Room codes avoid look-alike characters (no i, l, o, 0, 1).
const ID_CHARS: &[u8] = b"abcdefghjkmnpqrstuvwxyz23456789";
const ID_LENGTH: usize = 5;
/// Per address (`auth::address_key`: IPv6 per /64, this machine exempt), so that one script cannot take
/// the whole server: open connections, rooms opened, practice rooms.
pub(super) const SESSIONS_PER_ADDRESS: usize = 32;
const ROOMS_PER_ADDRESS: usize = 3;
const PRACTICE_PER_ADDRESS: usize = 1;
/// Rooms open at once by default (`--max-rooms`). The target host is one core for 4 rooms of 8 players
/// (rounds with bots cost most); lobbies are cheap, so twice that, and never more than `MAX_ROOMS`.
pub const DEFAULT_MAX_ROOMS: usize = 8;
/// The room list goes out to the players at it at most this often (s): every join, leave and host change
/// in any room changes it.
const LIST_EVERY_S: f64 = 0.5;
/// A practice room nobody has pressed anything in for this long closes (s): there are only
/// MAX_PRACTICE_ROOMS on the server.
const PRACTICE_IDLE_S: f64 = 300.0;

/// A player behind a connection, as they travel from room to room.
#[derive(Clone, Debug)]
pub struct Member {
    /// Identity: the same person on every connection they make.
    pub uid: Uid,
    pub name: String,
    /// Suit colour asked for (a room gives it if it is free) and the outfit.
    pub color: Option<u8>,
    pub outfit: Outfit,
}

impl From<&Member> for Who {
    fn from(m: &Member) -> Self {
        Who {
            uid: m.uid.clone(),
            name: m.name.clone(),
            color: m.color,
            outfit: Some(m.outfit),
        }
    }
}

/// Where a connection is.
#[derive(Clone, Debug)]
pub enum Stand {
    /// No hello yet.
    Waiting,
    Listing(Member),
    /// In the room of key `room` as player `id`.
    InRoom {
        member: Member,
        room: u32,
        id: Pid,
    },
    /// Stands for nobody any more (no hello in time, or a newer connection of the player took over): what
    /// it sends is ignored until it closes.
    Gone,
}

impl Stand {
    pub fn member(&self) -> Option<&Member> {
        match self {
            Stand::Listing(m) | Stand::InRoom { member: m, .. } => Some(m),
            Stand::Waiting | Stand::Gone => None,
        }
    }

    /// The room's key and the player's id in it.
    pub fn seat(&self) -> Option<(u32, Pid)> {
        match self {
            Stand::InRoom { room, id, .. } => Some((*room, *id)),
            _ => None,
        }
    }
}

/// An open connection.
#[derive(Clone, Debug)]
pub struct Session {
    pub ip: Option<IpAddr>,
    /// `ip` as limits per address count it (`auth::address_key`).
    key: Option<AddrKey>,
    /// The player's id, from the connect token (`/api/session` checked their identity).
    pub uid: Uid,
    opened: u64,
    pub stand: Stand,
    /// Messages at the room list in the current second.
    list_rate: RateWindow,
    /// The "bad message" warning (a flood would fill the log).
    warned: Backoff,
}

/// Why a player cannot enter a room: they are told, and stay where they are.
#[derive(Clone, Debug, PartialEq)]
pub struct Denial {
    room: Option<String>,
    reason: DenyReason,
    msg: Option<&'static str>,
}

impl Denial {
    fn of(room: &str, reason: DenyReason, msg: &'static str) -> Self {
        Self {
            room: Some(room.into()),
            reason,
            msg: Some(msg),
        }
    }

    /// The room asks for its PIN.
    fn pin(room: &str) -> Self {
        Self {
            room: Some(room.into()),
            reason: DenyReason::Pin,
            msg: None,
        }
    }

    /// No room can be opened.
    fn limit(msg: &'static str) -> Self {
        Self {
            room: None,
            reason: DenyReason::Limit,
            msg: Some(msg),
        }
    }
}

impl From<Denial> for ServerMsg {
    fn from(d: Denial) -> Self {
        ServerMsg::Denied {
            room: d.room,
            reason: d.reason,
            msg: d.msg.map(Into::into),
        }
    }
}

pub struct Hub {
    /// What every room is made with.
    base: RoomOptions,
    /// Every room being simulated, by key (listed ones and practice).
    pub rooms: BTreeMap<u32, Room>,
    next_key: u32,
    pub sessions: BTreeMap<ConnId, Session>,
    /// The live connection of each player (at the room list or in a room; practice is not counted).
    live: BTreeMap<Uid, ConnId>,
    /// The room list as last sent (and the ids of every listed room), to skip changes that do not show in it.
    sent: Option<(Vec<RoomInfo>, Vec<String>)>,
    /// The room list may have changed since it last went out (at real tick `list_sent`).
    list_due: bool,
    list_sent: u64,
    /// Listed rooms open at once at most (`--max-rooms`).
    pub max_rooms: usize,
    /// PRACTICE_IDLE_S (tests wait less).
    pub(super) practice_idle_s: f64,
    guesses: Limiter,
    real: u64,
    pub out: Vec<Out>,
}

impl Hub {
    pub fn new(base: RoomOptions, real: u64) -> Self {
        let mut hub = Self {
            base,
            rooms: BTreeMap::new(),
            next_key: 1,
            sessions: BTreeMap::new(),
            live: BTreeMap::new(),
            sent: None,
            list_due: false,
            list_sent: 0,
            max_rooms: DEFAULT_MAX_ROOMS,
            practice_idle_s: PRACTICE_IDLE_S,
            guesses: Limiter::default(),
            real,
            out: Vec::new(),
        };
        // Dev server: a room that is always there, for the tools to meet in.
        if hub.base.dev {
            hub.open_permanent(DEV_ROOM_ID, "Dev");
        }
        hub
    }

    /// A room that stays open with nobody in it (the dev room; rooms stress runs meet in).
    pub fn open_permanent(&mut self, id: &str, title: &str) {
        if !self.listed().any(|(_, r)| r.id == id) {
            self.open_room(id.into(), title.into(), None, None, true, None);
        }
    }

    /// Server tick of the last update.
    pub fn real_tick(&self) -> u64 {
        self.real
    }

    /// Listed rooms (not practice).
    pub fn listed(&self) -> impl Iterator<Item = (&u32, &Room)> {
        self.rooms.iter().filter(|(_, r)| !r.practice())
    }

    pub fn member(&self, conn: ConnId) -> Option<&Member> {
        self.sessions.get(&conn)?.stand.member()
    }

    /// The room key and player id of a connection in a room.
    pub fn seat(&self, conn: ConnId) -> Option<(u32, Pid)> {
        self.sessions.get(&conn)?.stand.seat()
    }

    /// Every connection in a room, and the room's key.
    pub fn seated(&self) -> impl Iterator<Item = (ConnId, u32)> {
        self.sessions.iter().filter_map(|(c, s)| Some((*c, s.stand.seat()?.0)))
    }

    fn set_stand(&mut self, conn: ConnId, stand: Stand) {
        if let Some(s) = self.sessions.get_mut(&conn) {
            s.stand = stand;
        }
    }

    /// Out of its room: back at the room list (or `Gone`), and where it was.
    fn unseat(&mut self, conn: ConnId, gone: bool) -> Option<(u32, Pid)> {
        let s = self.sessions.get_mut(&conn)?;
        let seat = s.stand.seat();
        s.stand = match core::mem::replace(&mut s.stand, Stand::Gone) {
            _ if gone => Stand::Gone,
            Stand::InRoom { member, .. } => Stand::Listing(member),
            other => other,
        };
        seat
    }

    /// Connections in the room of key `key`.
    fn seated_in(&self, key: u32) -> Vec<ConnId> {
        self.seated().filter(|&(_, k)| k == key).map(|(c, _)| c).collect()
    }

    fn real_ms(&self) -> u64 {
        self.real * 1000 / TICK_RATE as u64
    }

    fn send(&mut self, conn: ConnId, msg: ServerMsg) {
        self.out.push(Out::Msg(conn, msg));
    }

    /// Runs `f` on a room and passes on what it says. A panic closes the room, not the server.
    pub(super) fn with_room<R>(&mut self, key: u32, f: impl FnOnce(&mut Room) -> R) -> Option<R> {
        let room = self.rooms.get_mut(&key)?;
        let Ok(r) = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| f(room))) else {
            self.crashed(key);
            return None;
        };
        self.out.append(&mut room.out);
        if core::mem::take(&mut room.changed) && !room.practice() {
            self.changed();
        }
        Some(r)
    }

    /// A room panicked: it closes, and whoever was in it is back at the room list.
    fn crashed(&mut self, key: u32) {
        let Some(room) = self.rooms.remove(&key) else { return };
        // (The panic, its place and backtrace are in the log just above: `panic` in `main.rs`.)
        error!(
            room = room.id,
            arena = room.arena.map.meta().id,
            phase = ?room.phase(),
            tick = room.arena.tick,
            players = ?room.players.iter().map(|p| p.id).collect::<Vec<_>>(),
            "room crashed: closed"
        );
        let practice = room.practice();
        for c in self.seated_in(key) {
            self.unseat(c, practice);
            if practice {
                self.refuse(c, RejectReason::Busy, "Тренировка прервалась из-за ошибки сервера");
                continue;
            }
            self.send(
                c,
                ServerMsg::Home {
                    msg: Some("Комната закрылась из-за ошибки сервера".into()),
                },
            );
            self.send_directory(c, None);
        }
        self.changed();
    }

    // ------------------------------------------------------------------ connections

    /// A connection of player `uid` (None: the token had no identity, the connection is refused) from `ip`.
    pub fn open(&mut self, conn: ConnId, ip: Option<IpAddr>, uid: Option<Uid>) {
        let Some(uid) = uid else {
            warn!(ip = ip.map(display), "connection without an identity");
            return self.refuse(conn, RejectReason::Auth, "Нет входа: перезапустите игру");
        };
        let key = ip.and_then(address_key);
        if let Some(key) = key
            && self.connections_from(key) >= SESSIONS_PER_ADDRESS
        {
            warn!(ip = ip.map(display), "too many connections from one address");
            return self.refuse(conn, RejectReason::Busy, "С этого адреса уже слишком много подключений");
        }
        self.sessions.insert(
            conn,
            Session {
                ip,
                key,
                uid,
                opened: self.real,
                stand: Stand::Waiting,
                list_rate: RateWindow::default(),
                warned: Backoff::default(),
            },
        );
    }

    /// The connection closed.
    pub fn close(&mut self, conn: ConnId) {
        self.drop_member(conn);
        self.sessions.remove(&conn);
    }

    pub fn message(&mut self, conn: ConnId, m: ClientMsg) {
        let now = secs(self.real);
        let Some(s) = self.sessions.get_mut(&conn) else { return };
        if let Err(what) = m.check() {
            if let Some(hushed) = s.warned.hit(now) {
                let id = s.stand.seat().map(|(_, id)| id);
                warn!(ip = s.ip.map(display), id, what, hushed, "bad message");
            }
            return;
        }
        match &mut s.stand {
            Stand::Gone => return,
            Stand::Waiting => {
                if let ClientMsg::Hello(h) = m {
                    self.hello(conn, h);
                }
                return;
            }
            // (The name, colour and outfit travel with the player from room to room.)
            Stand::Listing(member) | Stand::InRoom { member, .. } => match &m {
                ClientMsg::Name(name) => {
                    let name = sanitize_person_name(name);
                    if !name.is_empty() {
                        member.name = name;
                    }
                }
                ClientMsg::Color(c) => member.color = Some(*c),
                ClientMsg::Outfit(o) => member.outfit = *o,
                _ => {}
            },
        }
        if let Some((key, id)) = s.stand.seat() {
            if m == ClientMsg::Leave {
                // Counted with the room list's messages: leaving and joining again over and over is a lobby
                // and a welcome to everybody each time.
                if self.allow(conn) {
                    self.leave(conn);
                }
            } else {
                self.with_room(key, |r| r.control(id, conn, &m));
            }
            return;
        }
        // At the room list.
        if !self.allow(conn) {
            return;
        }
        let Some(member) = self.member(conn).cloned() else {
            return;
        };
        let entered = match m {
            ClientMsg::Create { title, private } => self.create(conn, &member, &title, private),
            ClientMsg::Join { room, pin } => self.join(conn, &member, &room, pin.as_deref()),
            _ => Ok(()),
        };
        if let Err(d) = entered {
            self.send(conn, d.into());
        }
    }

    fn allow(&mut self, conn: ConnId) -> bool {
        let real = self.real;
        self.sessions
            .get_mut(&conn)
            .is_some_and(|s| s.list_rate.hit(real) <= LIST_RATE)
    }

    /// An accepted hello: the connection goes where the hello says, as the player of its token.
    fn hello(&mut self, conn: ConnId, h: Hello) {
        let Some(uid) = self.sessions.get(&conn).map(|s| s.uid.clone()) else {
            return;
        };
        let dev = self.base.dev;
        self.send(conn, ServerMsg::Ready { dev });
        self.enter(conn, uid, h);
    }

    /// The connection takes the place of any earlier connection of the same player, then goes where the
    /// hello says: a practice round, a room (asked for, or the one the player is still in), or the room list.
    fn enter(&mut self, conn: ConnId, uid: Uid, h: Hello) {
        let m = Member {
            uid: uid.clone(),
            name: sanitize_person_name(&h.name),
            color: h.color,
            outfit: h.outfit.unwrap_or_default(),
        };
        if let Some(game) = &h.practice {
            return self.enter_practice(conn, m, game);
        }
        let old = self.live.get(&uid).copied().filter(|&c| c != conn);
        let old_room = old
            .and_then(|c| self.seat(c))
            .and_then(|(k, _)| self.rooms.get(&k))
            .map(|r| r.id.clone());
        let target = h
            .room
            .clone()
            .or(old_room)
            .or_else(|| self.room_of(&uid).map(|k| self.rooms[&k].id.clone()));
        if let Some(old) = old {
            self.evict(old, target.as_deref());
        }
        self.live.insert(uid, conn);
        self.set_stand(conn, Stand::Listing(m.clone()));
        let entered = match target {
            Some(t) => self.join(conn, &m, &t, h.pin.as_deref()),
            None => Err(Denial::of("", DenyReason::Gone, "")),
        };
        match entered {
            Ok(()) => {}
            Err(d) if d.room.as_deref() == Some("") => self.send_directory(conn, None),
            Err(d) => {
                self.send(conn, d.into());
                self.send_directory(conn, None);
            }
        }
    }

    /// The connection is gone.
    fn drop_member(&mut self, conn: ConnId) {
        let Some(uid) = self.member(conn).map(|m| m.uid.clone()) else {
            return;
        };
        let seat = self.unseat(conn, true);
        if self.live.get(&uid) == Some(&conn) {
            self.live.remove(&uid);
        }
        let Some((key, id)) = seat else { return };
        self.with_room(key, |r| r.leave(id, conn));
        // A practice room lives only while its player is connected.
        if self.rooms.get(&key).is_some_and(Room::practice) {
            self.rooms.remove(&key);
        } else {
            self.vacated(key, false);
        }
    }

    // ------------------------------------------------------------------ rooms

    /// Opens the player's own room and enters it; if they already have one, they return to it.
    fn create(&mut self, conn: ConnId, m: &Member, title: &str, private: bool) -> Result<(), Denial> {
        if let Some(mine) = self.owned(&m.uid) {
            let id = self.rooms[&mine].id.clone();
            return self.join(conn, m, &id, None);
        }
        let creator = self.sessions.get(&conn).and_then(|s| s.key);
        if let Some(c) = creator
            && self.listed().filter(|(_, r)| r.creator() == Some(c)).count() >= ROOMS_PER_ADDRESS
        {
            return Err(Denial::limit(
                "С этого адреса открыто слишком много комнат — зайдите в одну из них",
            ));
        }
        if self.listed().count() >= self.max_rooms.min(MAX_ROOMS) {
            return Err(Denial::limit(
                "Сейчас открыто слишком много комнат — зайдите в одну из них или попробуйте позже",
            ));
        }
        let id = self.new_id();
        let title = Some(sanitize_title(title))
            .filter(|t| !t.is_empty())
            .unwrap_or_else(|| {
                if m.name.is_empty() {
                    format!("Комната {}", id.to_uppercase())
                } else {
                    format!("Комната {}", m.name)
                }
            });
        let pin = private.then(make_pin);
        self.open_room(id.clone(), title, Some(m.uid.clone()), pin, false, creator);
        info!(room = id, private, rooms = self.listed().count(), "room opened");
        self.join(conn, m, &id, None)
    }

    /// Enters a room from the room list.
    fn join(&mut self, conn: ConnId, m: &Member, id: &str, pin: Option<&str>) -> Result<(), Denial> {
        let Some(key) = self.listed().find(|(_, r)| r.id == id).map(|(k, _)| *k) else {
            return Err(Denial::of(id, DenyReason::Gone, "Этой комнаты больше нет"));
        };
        let room = &self.rooms[&key];
        // The owner and those the room let in before come back without the PIN.
        let known =
            room.owner.as_ref() == Some(&m.uid) || room.admitted.contains(&m.uid) || room.player_of(&m.uid).is_some();
        if let Some(want) = room.pin.clone()
            && !known
        {
            let Some(pin) = pin else {
                return Err(Denial::pin(id));
            };
            let ip = self.sessions.get(&conn).and_then(|s| s.ip);
            let now = self.real_ms();
            if !self.guesses.allow(ip, id, now) {
                return Err(Denial::of(
                    id,
                    DenyReason::Limit,
                    "Слишком много попыток — подождите минуту",
                ));
            }
            if !same_key(pin, &want) {
                self.guesses.failed(ip, id, now);
                warn!(room = id, ip = ip.map(display), "wrong room pin");
                // Somebody is going through the PINs (from many addresses: each one is limited): the room
                // takes a new one, and what they ruled out no longer helps.
                if self.guesses.misses_at(id, now) >= GUESSES_PER_TARGET {
                    self.guesses.forget_target(id);
                    self.with_room(key, Room::new_pin);
                }
                return Err(Denial::of(id, DenyReason::Pin, "Неверный PIN-код"));
            }
        }
        let pid = match self.with_room(key, |r| r.join(conn, m.into())) {
            Some(Some(pid)) => pid,
            Some(None) => return Err(Denial::of(id, DenyReason::Full, "В комнате нет свободных мест")),
            None => return Err(Denial::of(id, DenyReason::Gone, "Этой комнаты больше нет")),
        };
        self.set_stand(
            conn,
            Stand::InRoom {
                member: m.clone(),
                room: key,
                id: pid,
            },
        );
        if let Some(r) = self.rooms.get_mut(&key) {
            r.empty_since = None;
        }
        // One room at a time: a room still keeping this player's place (a game they dropped out of) lets go.
        let others: Vec<(u32, Pid)> = self
            .listed()
            .filter(|(k, _)| **k != key)
            .filter_map(|(k, r)| r.player_of(&m.uid).map(|p| (*k, p.id)))
            .collect();
        for (k, p) in others {
            self.with_room(k, |r| r.quit(p));
            self.vacated(k, true);
        }
        self.changed();
        Ok(())
    }

    /// Back to the room list.
    pub fn leave(&mut self, conn: ConnId) {
        let Some((key, id)) = self.seat(conn) else { return };
        if self.rooms.get(&key).is_none_or(Room::practice) {
            return;
        }
        self.with_room(key, |r| r.quit(id));
        self.unseat(conn, false);
        self.send(conn, ServerMsg::Home { msg: None });
        self.vacated(key, true);
        self.send_directory(conn, None);
    }

    /// Server tick: hello timeouts, empty rooms, and every room's simulation.
    pub fn update(&mut self, real: u64, inputs: &mut dyn Inputs) {
        self.real = real;
        let late: Vec<ConnId> = self
            .sessions
            .iter_mut()
            .filter(|(_, s)| {
                matches!(s.stand, Stand::Waiting) && real.saturating_sub(s.opened) > ticks(HELLO_TIMEOUT_S)
            })
            .map(|(c, s)| {
                s.stand = Stand::Gone;
                warn!(ip = s.ip.map(display), "no hello");
                *c
            })
            .collect();
        for c in late {
            self.out.push(Out::Close(c));
        }
        if real.is_multiple_of(ticks(1.0)) {
            self.sweep();
            self.close_idle_practice();
        }
        let keys: Vec<u32> = self.rooms.keys().copied().collect();
        for key in keys {
            self.with_room(key, |r| r.update(real, &mut *inputs));
        }
        if self.list_due && real.saturating_sub(self.list_sent) >= ticks(LIST_EVERY_S) {
            self.list_due = false;
            self.list_sent = real;
            self.send_list();
        }
    }

    /// Practice rooms nobody has pressed anything in for PRACTICE_IDLE_S: their player is let go.
    fn close_idle_practice(&mut self) {
        let real = self.real;
        let idle: Vec<u32> = self
            .rooms
            .iter()
            .filter(|(_, r)| r.practice() && real.saturating_sub(r.active_at()) > ticks(self.practice_idle_s))
            .map(|(k, _)| *k)
            .collect();
        for key in idle {
            self.rooms.remove(&key);
            info!("practice closed: nobody played in it");
            for c in self.seated_in(key) {
                self.unseat(c, true);
                self.refuse(
                    c,
                    RejectReason::Busy,
                    "Тренировка закрылась: в ней давно ничего не нажимали",
                );
            }
        }
    }

    /// The server is going down: everyone is told and goes back to the room list (their games reconnect to
    /// it once it is up again).
    pub fn shutdown(&mut self, msg: &str) {
        let conns: Vec<ConnId> = self
            .sessions
            .iter()
            .filter(|(_, s)| s.stand.member().is_some())
            .map(|(c, _)| *c)
            .collect();
        for c in conns {
            self.send(c, ServerMsg::Home { msg: Some(msg.into()) });
        }
    }

    /// Closes rooms that have stood empty for a while.
    pub fn sweep(&mut self) {
        let real = self.real;
        let mut expired = Vec::new();
        for (key, room) in self.rooms.iter_mut().filter(|(_, r)| !r.practice() && !r.permanent()) {
            if !room.empty() {
                room.empty_since = None;
                continue;
            }
            match room.empty_since {
                None => room.empty_since = Some(real),
                Some(since) if real.saturating_sub(since) >= ticks(ROOM_EMPTY_S) => expired.push(*key),
                Some(_) => {}
            }
        }
        for key in expired {
            self.close_room(key);
        }
    }

    pub fn take_out(&mut self) -> Vec<Out> {
        core::mem::take(&mut self.out)
    }

    // ------------------------------------------------------------------ internals

    fn insert_room(&mut self, opts: RoomOptions) -> u32 {
        let key = self.next_key;
        self.next_key += 1;
        self.rooms.insert(key, Room::new(opts, self.real));
        key
    }

    fn open_room(
        &mut self,
        id: String,
        title: String,
        owner: Option<Uid>,
        pin: Option<String>,
        permanent: bool,
        creator: Option<AddrKey>,
    ) -> u32 {
        self.insert_room(RoomOptions {
            id,
            title,
            owner,
            pin,
            permanent,
            creator,
            ..self.base.clone()
        })
    }

    fn close_room(&mut self, key: u32) {
        let Some(room) = self.rooms.remove(&key) else { return };
        info!(room = room.id, rooms = self.listed().count(), "room closed");
        self.changed();
    }

    /// Someone left a room: if it is empty now it closes, at once or (a lost connection) after a while.
    fn vacated(&mut self, key: u32, now: bool) {
        let real = self.real;
        let Some(room) = self.rooms.get_mut(&key) else { return };
        if room.permanent() || !room.empty() {
            return;
        }
        if now {
            self.close_room(key);
        } else {
            room.empty_since.get_or_insert(real);
        }
    }

    fn refuse(&mut self, conn: ConnId, reason: RejectReason, msg: &str) {
        self.send(
            conn,
            ServerMsg::Reject {
                reason,
                msg: msg.into(),
            },
        );
        self.out.push(Out::Close(conn));
    }

    fn enter_practice(&mut self, conn: ConnId, m: Member, game: &str) {
        let Some(game) = Game::by_id(game) else {
            return self.refuse(conn, RejectReason::Bad, "Нет такой карты");
        };
        let creator = self.sessions.get(&conn).and_then(|s| s.key);
        if let Some(c) = creator
            && self
                .rooms
                .values()
                .filter(|r| r.practice() && r.creator() == Some(c))
                .count()
                >= PRACTICE_PER_ADDRESS
        {
            return self.refuse(conn, RejectReason::Busy, "С этого адреса уже идёт тренировка");
        }
        if self.rooms.values().filter(|r| r.practice()).count() >= MAX_PRACTICE_ROOMS {
            return self.refuse(
                conn,
                RejectReason::Busy,
                "Сейчас слишком много тренировок — попробуйте позже",
            );
        }
        let key = self.insert_room(RoomOptions {
            min_players: 1,
            practice: Some(Practice { game, bots: 3 }),
            permanent: false,
            creator,
            ..self.base.clone()
        });
        match self.with_room(key, |r| r.join(conn, (&m).into())).flatten() {
            Some(id) => self.set_stand(
                conn,
                Stand::InRoom {
                    member: m,
                    room: key,
                    id,
                },
            ),
            None => {
                self.rooms.remove(&key);
                self.refuse(conn, RejectReason::Busy, "Тренировка прервалась из-за ошибки сервера");
            }
        }
    }

    /// A newer connection of the same player takes over: this one is told and closed.
    fn evict(&mut self, old: ConnId, stay_in: Option<&str>) {
        if self.member(old).is_none() {
            return;
        }
        let seat = self.unseat(old, true);
        self.send(
            old,
            ServerMsg::Reject {
                reason: RejectReason::Moved,
                msg: "Игра открыта в другом окне".into(),
            },
        );
        // Going elsewhere: out of the room. Staying: Room::join moves the player to the new connection.
        if let Some((key, id)) = seat
            && self.rooms.get(&key).is_some_and(|r| Some(r.id.as_str()) != stay_in)
        {
            self.with_room(key, |r| r.quit(id));
            self.vacated(key, true);
        }
        self.out.push(Out::Close(old));
    }

    /// The room keeping a place for this player (they are in it, or dropped out of its game).
    fn room_of(&self, uid: &Uid) -> Option<u32> {
        self.listed().find(|(_, r)| r.player_of(uid).is_some()).map(|(k, _)| *k)
    }

    /// Open connections from the address `key` (`auth::address_key`).
    fn connections_from(&self, key: AddrKey) -> usize {
        self.sessions.values().filter(|s| s.key == Some(key)).count()
    }

    fn owned(&self, uid: &Uid) -> Option<u32> {
        self.listed()
            .find(|(_, r)| r.owner.as_ref() == Some(uid))
            .map(|(k, _)| *k)
    }

    fn new_id(&self) -> String {
        loop {
            let id: String = random_bytes::<ID_LENGTH>()
                .iter()
                .map(|&b| ID_CHARS[b as usize % ID_CHARS.len()] as char)
                .collect();
            if id != DEV_ROOM_ID && !self.listed().any(|(_, r)| r.id == id) {
                return id;
            }
        }
    }

    /// The room list: rooms someone is in (an empty one is about to close).
    fn directory(&self) -> Vec<RoomInfo> {
        self.listed()
            .filter(|(_, r)| r.permanent() || !r.empty())
            .map(|(_, r)| r.info())
            .collect()
    }

    fn send_directory(&mut self, conn: ConnId, rooms: Option<Vec<RoomInfo>>) {
        let rooms = rooms.unwrap_or_else(|| self.directory());
        let mine = self
            .member(conn)
            .and_then(|m| self.owned(&m.uid))
            .map(|k| self.rooms[&k].id.clone());
        self.send(conn, ServerMsg::Rooms { rooms, mine });
    }

    /// The room list may have changed: it goes out with the next update, at most every LIST_EVERY_S.
    fn changed(&mut self) {
        self.list_due = true;
    }

    /// Tells everyone at the room list when it changed.
    fn send_list(&mut self) {
        let rooms = self.directory();
        let ids: Vec<String> = self.listed().map(|(_, r)| r.id.clone()).collect();
        if self.sent.as_ref().is_some_and(|(r, i)| *r == rooms && *i == ids) {
            return;
        }
        self.sent = Some((rooms.clone(), ids));
        let at_list: Vec<ConnId> = self
            .live
            .values()
            .copied()
            .filter(|c| {
                self.sessions
                    .get(c)
                    .is_some_and(|s| matches!(s.stand, Stand::Listing(_)))
            })
            .collect();
        for c in at_list {
            self.send_directory(c, Some(rooms.clone()));
        }
    }
}
