//! The rooms of the server and who is where (port of `server/rooms/hub.ts` and `server/net/gateway.ts`).
//! Anyone may open one room of their own (they are its host whenever they are in it) and be in one room at
//! a time; a private room asks newcomers for its PIN. Rooms close when everybody has left. Practice rooms
//! are separate: one player, not listed.
use std::collections::BTreeMap;

use bevy::log::{error, info, warn};
use fb_maps::director;
use fb_proto::*;
use fb_shared::text::{sanitize_name, sanitize_title};
use fb_shared::*;

use super::room::{Practice, Room, RoomOptions, Who, make_pin};
use super::{ConnId, Inputs, Out, random_u32, ticks};
use crate::auth::{Auth, Limiter, same_key};

/// A connection that sent no hello within this long is closed (s).
const HELLO_TIMEOUT_S: f64 = 5.0;
/// Control messages per second from a connection at the room list (rooms have their own limit).
const LIST_RATE: u32 = 20;
/// Room codes avoid look-alike characters (no i, l, o, 0, 1).
const ID_CHARS: &[u8] = b"abcdefghjkmnpqrstuvwxyz23456789";
const ID_LENGTH: usize = 5;

/// A player behind a connection: at the room list (`room` None) or in a room.
#[derive(Clone, Debug)]
pub struct Member {
    /// Identity: the same person on every connection they make.
    pub uid: String,
    pub name: String,
    /// Suit colour asked for (a room gives it if it is free) and the outfit: they travel from room to room.
    pub color: Option<u8>,
    pub outfit: Outfit,
    /// Key of the room the player is in.
    pub room: Option<u32>,
    /// Player id in `room`.
    pub id: Pid,
    /// The connection no longer stands for the player: closed, or replaced by a newer one.
    pub gone: bool,
}

/// An open connection.
#[derive(Clone, Debug)]
pub struct Session {
    pub ip: String,
    opened: u64,
    pub member: Option<Member>,
    window: u64,
    count: u32,
    timed_out: bool,
}

pub struct Hub {
    auth: Auth,
    /// What every room is made with.
    base: RoomOptions,
    /// Every room being simulated, by key (listed ones and practice).
    pub rooms: BTreeMap<u32, Room>,
    next_key: u32,
    pub sessions: BTreeMap<ConnId, Session>,
    /// The live connection of each player (at the room list or in a room; practice is not counted).
    live: BTreeMap<String, ConnId>,
    /// Rooms nobody is in, and since when (they close after ROOM_EMPTY_S).
    empty_since: BTreeMap<u32, u64>,
    /// The room list as last sent, to skip changes that do not show in it.
    sent: String,
    guesses: Limiter,
    updating: bool,
    real: u64,
    pub out: Vec<Out>,
}

impl Hub {
    pub fn new(auth: Auth, base: RoomOptions, real: u64) -> Self {
        let mut hub = Self {
            auth,
            base,
            rooms: BTreeMap::new(),
            next_key: 1,
            sessions: BTreeMap::new(),
            live: BTreeMap::new(),
            empty_since: BTreeMap::new(),
            sent: String::new(),
            guesses: Limiter::default(),
            updating: false,
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
            self.open_room(id.into(), title.into(), None, None, true);
        }
    }

    pub fn updating(&self) -> bool {
        self.updating
    }

    /// Listed rooms (not practice).
    pub fn listed(&self) -> impl Iterator<Item = (&u32, &Room)> {
        self.rooms.iter().filter(|(_, r)| !r.practice())
    }

    pub fn member(&self, conn: ConnId) -> Option<&Member> {
        self.sessions.get(&conn)?.member.as_ref()
    }

    fn member_mut(&mut self, conn: ConnId) -> Option<&mut Member> {
        self.sessions.get_mut(&conn)?.member.as_mut()
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
        error!(room = room.id, arena = room.arena.map.meta().id, "room crashed: closed");
        self.empty_since.remove(&key);
        let members: Vec<(ConnId, bool)> = self
            .sessions
            .iter()
            .filter(|(_, s)| s.member.as_ref().is_some_and(|m| m.room == Some(key)))
            .map(|(c, _)| (*c, room.practice()))
            .collect();
        for (c, practice) in members {
            if let Some(m) = self.member_mut(c) {
                m.room = None;
            }
            if practice {
                self.refuse(c, RejectReason::Busy, "Тренировка прервалась из-за ошибки сервера");
                continue;
            }
            self.send(
                c,
                ServerMsg::Home {
                    msg: "Комната закрылась из-за ошибки сервера".into(),
                },
            );
            self.send_directory(c, None);
        }
        self.changed();
    }

    // ------------------------------------------------------------------ connections

    pub fn open(&mut self, conn: ConnId, ip: String) {
        if self.updating {
            self.send(conn, ServerMsg::Updating);
            self.out.push(Out::Close(conn));
            return;
        }
        self.sessions.insert(
            conn,
            Session {
                ip,
                opened: self.real,
                member: None,
                window: 0,
                count: 0,
                timed_out: false,
            },
        );
    }

    /// The connection closed.
    pub fn close(&mut self, conn: ConnId) {
        if self.sessions.get(&conn).is_some_and(|s| s.member.is_some()) {
            self.drop_member(conn);
        }
        self.sessions.remove(&conn);
    }

    pub fn message(&mut self, conn: ConnId, m: ClientMsg) {
        let Some(s) = self.sessions.get(&conn) else { return };
        if let Err(what) = m.check() {
            warn!(ip = s.ip, id = s.member.as_ref().map(|m| m.id), what, "bad message");
            return;
        }
        let Some(member) = &s.member else {
            if let ClientMsg::Hello(h) = m {
                self.hello(conn, h);
            }
            return;
        };
        if member.gone {
            return;
        }
        let (room, id) = (member.room, member.id);
        let Some(member) = self.member_mut(conn) else { return };
        // (The name, colour and outfit travel with the player from room to room.)
        match &m {
            ClientMsg::Name(name) => {
                let name = sanitize_name(name);
                if !name.is_empty() {
                    member.name = name;
                }
            }
            ClientMsg::Color(c) => member.color = Some(*c),
            ClientMsg::Outfit(o) => member.outfit = *o,
            _ => {}
        }
        if let Some(key) = room {
            if m == ClientMsg::Leave {
                self.leave(conn);
            } else {
                self.with_room(key, |r| r.control(id, conn, &m));
            }
            return;
        }
        // At the room list.
        if !self.allow(conn) {
            return;
        }
        match m {
            ClientMsg::Create { title, private } => {
                self.create(conn, &title, private);
            }
            ClientMsg::Join { room, pin } => {
                self.join(conn, &room, pin.as_deref());
            }
            _ => {}
        }
    }

    fn allow(&mut self, conn: ConnId) -> bool {
        let real = self.real;
        let Some(s) = self.sessions.get_mut(&conn) else {
            return false;
        };
        if real.saturating_sub(s.window) > ticks(1.0) {
            s.window = real;
            s.count = 0;
        }
        s.count += 1;
        s.count <= LIST_RATE
    }

    /// An accepted hello: the connection learns its identity token and goes where the hello says.
    fn hello(&mut self, conn: ConnId, h: Hello) {
        // A player is whoever holds the identity token; a client without one gets a new identity.
        let known = h.token.as_deref().and_then(|t| self.auth.identity(t));
        let (uid, token) = match (known, &h.token) {
            (Some(uid), Some(t)) => (uid, t.clone()),
            _ => self.auth.issue_identity(),
        };
        let dev = self.base.dev;
        self.send(conn, ServerMsg::Ready { token, dev });
        self.enter(conn, uid, h);
    }

    /// The connection takes the place of any earlier connection of the same player, then goes where the
    /// hello says: a practice round, a room (asked for, or the one the player is still in), or the room list.
    fn enter(&mut self, conn: ConnId, uid: String, h: Hello) {
        let m = Member {
            uid: uid.clone(),
            name: sanitize_name(&h.name),
            color: h.color,
            outfit: h.outfit.unwrap_or_default(),
            room: None,
            id: 0,
            gone: false,
        };
        if let Some(game) = &h.practice {
            self.enter_practice(conn, m, game);
            return;
        }
        let old = self.live.get(&uid).copied().filter(|&c| c != conn);
        let old_room = old
            .and_then(|c| self.member(c))
            .and_then(|m| m.room)
            .and_then(|k| self.rooms.get(&k))
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
        if let Some(s) = self.sessions.get_mut(&conn) {
            s.member = Some(m);
        }
        if !target.is_some_and(|t| self.join(conn, &t, h.pin.as_deref())) {
            self.send_directory(conn, None);
        }
    }

    /// The connection is gone.
    fn drop_member(&mut self, conn: ConnId) {
        let Some(m) = self.member_mut(conn) else { return };
        if m.gone {
            return;
        }
        m.gone = true;
        let (uid, id) = (m.uid.clone(), m.id);
        let room = m.room.take();
        if self.live.get(&uid) == Some(&conn) {
            self.live.remove(&uid);
        }
        let Some(key) = room else { return };
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
    pub fn create(&mut self, conn: ConnId, title: &str, private: bool) -> bool {
        let Some(m) = self.member(conn).filter(|m| m.room.is_none() && !m.gone) else {
            return false;
        };
        let (uid, name) = (m.uid.clone(), m.name.clone());
        if let Some(mine) = self.owned(&uid) {
            let id = self.rooms[&mine].id.clone();
            return self.join(conn, &id, None);
        }
        if self.listed().count() >= MAX_ROOMS {
            return self.deny(
                conn,
                None,
                DenyReason::Limit,
                "Сейчас открыто слишком много комнат — зайдите в одну из них или попробуйте позже",
            );
        }
        let id = self.new_id();
        let title = Some(sanitize_title(title))
            .filter(|t| !t.is_empty())
            .unwrap_or_else(|| {
                if name.is_empty() {
                    format!("Комната {}", id.to_uppercase())
                } else {
                    format!("Комната {name}")
                }
            });
        self.open_room(id.clone(), title, Some(uid), private.then(make_pin), false);
        info!(room = id, private, rooms = self.listed().count(), "room opened");
        self.join(conn, &id, None)
    }

    /// Enters a room from the room list. On failure the player is told why and stays where they are.
    pub fn join(&mut self, conn: ConnId, id: &str, pin: Option<&str>) -> bool {
        let Some(m) = self.member(conn).filter(|m| m.room.is_none() && !m.gone).cloned() else {
            return false;
        };
        let Some(key) = self.listed().find(|(_, r)| r.id == id).map(|(k, _)| *k) else {
            return self.deny(conn, Some(id), DenyReason::Gone, "Этой комнаты больше нет");
        };
        let room = &self.rooms[&key];
        // The owner and those the room let in before come back without the PIN.
        let known =
            room.owner.as_ref() == Some(&m.uid) || room.admitted.contains(&m.uid) || room.player_of(&m.uid).is_some();
        if let Some(want) = room.pin.clone()
            && !known
        {
            let Some(pin) = pin else {
                return self.deny(conn, Some(id), DenyReason::Pin, "");
            };
            let ip = self.sessions[&conn].ip.clone();
            let now = self.real_ms();
            if !self.guesses.allow(&ip, now) {
                return self.deny(
                    conn,
                    Some(id),
                    DenyReason::Limit,
                    "Слишком много попыток — подождите минуту",
                );
            }
            if !same_key(pin, &want) {
                warn!(room = id, ip, "wrong room pin");
                return self.deny(conn, Some(id), DenyReason::Pin, "Неверный PIN-код");
            }
        }
        // (Set first: the room's own messages must not be followed by a room list for this player.)
        if let Some(mm) = self.member_mut(conn) {
            mm.room = Some(key);
        }
        let who = Who {
            uid: m.uid.clone(),
            name: m.name.clone(),
            color: m.color,
            outfit: Some(m.outfit),
        };
        let Some(pid) = self.with_room(key, |r| r.join(conn, who)).flatten() else {
            if let Some(mm) = self.member_mut(conn) {
                mm.room = None;
            }
            return self.deny(conn, Some(id), DenyReason::Full, "В комнате нет свободных мест");
        };
        if let Some(mm) = self.member_mut(conn) {
            mm.id = pid;
        }
        self.empty_since.remove(&key);
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
        true
    }

    /// Back to the room list.
    pub fn leave(&mut self, conn: ConnId) {
        let Some(m) = self.member(conn).filter(|m| !m.gone) else {
            return;
        };
        let (Some(key), id) = (m.room, m.id) else { return };
        if self.rooms.get(&key).is_none_or(Room::practice) {
            return;
        }
        self.with_room(key, |r| r.quit(id));
        if let Some(m) = self.member_mut(conn) {
            m.room = None;
        }
        self.send(conn, ServerMsg::Home { msg: String::new() });
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
                s.member.is_none() && !s.timed_out && real.saturating_sub(s.opened) > ticks(HELLO_TIMEOUT_S)
            })
            .map(|(c, s)| {
                s.timed_out = true;
                warn!(ip = s.ip, "no hello");
                *c
            })
            .collect();
        for c in late {
            self.out.push(Out::Close(c));
        }
        if real.is_multiple_of(ticks(1.0)) {
            self.sweep();
        }
        let keys: Vec<u32> = self.rooms.keys().copied().collect();
        for key in keys {
            self.with_room(key, |r| r.update(real, &mut *inputs));
        }
    }

    /// Closes rooms that have stood empty for a while.
    pub fn sweep(&mut self) {
        let real = self.real;
        let keys: Vec<(u32, bool)> = self
            .listed()
            .filter(|(_, r)| !r.permanent())
            .map(|(k, r)| (*k, r.empty()))
            .collect();
        for (key, empty) in keys {
            if !empty {
                self.empty_since.remove(&key);
                continue;
            }
            match self.empty_since.get(&key) {
                None => {
                    self.empty_since.insert(key, real);
                }
                Some(&since) if real.saturating_sub(since) >= ticks(ROOM_EMPTY_S) => self.close_room(key),
                _ => {}
            }
        }
    }

    /// Into or out of an update of the game: going in, every connection is told and closed.
    pub fn set_updating(&mut self, on: bool) {
        if on == self.updating {
            return;
        }
        self.updating = on;
        info!(
            connections = self.sessions.len(),
            "{}",
            if on {
                "updating: disconnecting everybody"
            } else {
                "update over: open again"
            }
        );
        if on {
            let conns: Vec<ConnId> = self.sessions.keys().copied().collect();
            for c in conns {
                self.send(c, ServerMsg::Updating);
                self.out.push(Out::Close(c));
            }
        }
    }

    pub fn take_out(&mut self) -> Vec<Out> {
        core::mem::take(&mut self.out)
    }

    // ------------------------------------------------------------------ internals

    fn open_room(
        &mut self,
        id: String,
        title: String,
        owner: Option<String>,
        pin: Option<String>,
        permanent: bool,
    ) -> u32 {
        let opts = RoomOptions {
            id,
            title,
            owner,
            pin,
            permanent,
            ..self.base.clone()
        };
        let key = self.next_key;
        self.next_key += 1;
        self.rooms.insert(key, Room::new(opts, self.real));
        key
    }

    fn close_room(&mut self, key: u32) {
        let Some(room) = self.rooms.remove(&key) else { return };
        self.empty_since.remove(&key);
        info!(room = room.id, rooms = self.listed().count(), "room closed");
        self.changed();
    }

    /// Someone left a room: if it is empty now it closes, at once or (a lost connection) after a while.
    fn vacated(&mut self, key: u32, now: bool) {
        let Some(room) = self.rooms.get(&key) else { return };
        if room.permanent() || !room.empty() {
            return;
        }
        if now {
            self.close_room(key);
        } else {
            self.empty_since.entry(key).or_insert(self.real);
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

    fn enter_practice(&mut self, conn: ConnId, mut m: Member, game: &str) {
        let Some(game) = director::game(game) else {
            return self.refuse(conn, RejectReason::Bad, "Нет такой карты");
        };
        if self.rooms.values().filter(|r| r.practice()).count() >= MAX_PRACTICE_ROOMS {
            return self.refuse(
                conn,
                RejectReason::Busy,
                "Сейчас слишком много тренировок — попробуйте позже",
            );
        }
        let opts = RoomOptions {
            min_players: 1,
            practice: Some(Practice {
                game: game.id.into(),
                bots: 3,
            }),
            permanent: false,
            ..self.base.clone()
        };
        let key = self.next_key;
        self.next_key += 1;
        self.rooms.insert(key, Room::new(opts, self.real));
        m.room = Some(key);
        let who = Who {
            uid: m.uid.clone(),
            name: m.name.clone(),
            color: m.color,
            outfit: Some(m.outfit),
        };
        if let Some(s) = self.sessions.get_mut(&conn) {
            s.member = Some(m);
        }
        let id = self.with_room(key, |r| r.join(conn, who)).flatten().unwrap_or(0);
        if let Some(m) = self.member_mut(conn) {
            m.id = id;
        }
    }

    /// A newer connection of the same player takes over: this one is told and closed.
    fn evict(&mut self, old: ConnId, stay_in: Option<&str>) {
        let Some(m) = self.member_mut(old) else { return };
        m.gone = true;
        let (room, id) = (m.room.take(), m.id);
        self.send(
            old,
            ServerMsg::Reject {
                reason: RejectReason::Moved,
                msg: "Игра открыта в другом окне".into(),
            },
        );
        // Going elsewhere: out of the room. Staying: Room::join moves the player to the new connection.
        if let Some(key) = room
            && self.rooms.get(&key).is_some_and(|r| Some(r.id.as_str()) != stay_in)
        {
            self.with_room(key, |r| r.quit(id));
            self.vacated(key, true);
        }
        self.out.push(Out::Close(old));
    }

    /// The room keeping a place for this player (they are in it, or dropped out of its game).
    fn room_of(&self, uid: &str) -> Option<u32> {
        self.listed().find(|(_, r)| r.player_of(uid).is_some()).map(|(k, _)| *k)
    }

    fn owned(&self, uid: &str) -> Option<u32> {
        self.listed()
            .find(|(_, r)| r.owner.as_deref() == Some(uid))
            .map(|(k, _)| *k)
    }

    fn new_id(&self) -> String {
        loop {
            let id: String = (0..ID_LENGTH)
                .map(|_| ID_CHARS[random_u32() as usize % ID_CHARS.len()] as char)
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

    /// Tells everyone at the room list when it changed.
    fn changed(&mut self) {
        let rooms = self.directory();
        let ids: Vec<&str> = self.listed().map(|(_, r)| r.id.as_str()).collect();
        let key = format!("{rooms:?}{ids:?}");
        if key == self.sent {
            return;
        }
        self.sent = key;
        let at_list: Vec<ConnId> = self
            .live
            .values()
            .copied()
            .filter(|c| self.member(*c).is_some_and(|m| m.room.is_none() && !m.gone))
            .collect();
        for c in at_list {
            self.send_directory(c, Some(rooms.clone()));
        }
    }

    fn deny(&mut self, conn: ConnId, room: Option<&str>, reason: DenyReason, msg: &str) -> bool {
        self.send(
            conn,
            ServerMsg::Denied {
                room: room.map(String::from),
                reason,
                msg: msg.into(),
            },
        );
        false
    }
}
