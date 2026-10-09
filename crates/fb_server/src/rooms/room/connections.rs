//! Connections: players joining, leaving and controlling the room; seats, hosts and bots.
use super::*;

impl Room {
    /// A player enters: someone new, or (same identity) the one who is already here, on a new connection.
    /// Returns the player id, or None when the room is full.
    pub fn join(&mut self, conn: ConnId, who: Who) -> Option<PlayerId> {
        if let Some(id) = self.player_of(&who.uid).map(|p| p.id) {
            let p = self.player_mut(id)?;
            let Kind::Human {
                conn: c,
                disconnected_at,
                ..
            } = &mut p.kind
            else {
                return None;
            };
            let old = c.replace(conn);
            *disconnected_at = None;
            if let Some(o) = who.outfit {
                p.outfit = o;
            }
            let name = p.name.clone();
            if let Some(old) = old.filter(|&c| c != conn) {
                self.out.push(Out::Close(old));
            }
            info!(room = %self.id, %id, name, "player resumed");
            self.seat_host(id);
            self.welcome(id, true);
            return Some(id);
        }
        if self.players.len() >= self.max {
            // A bot gives up its place to a person.
            let bot = self.spare_bot()?;
            self.drop_player(bot);
        }
        let id = self.next_id;
        self.next_id.0 += 1;
        let name = Some(sanitize_person_name(&who.name))
            .filter(|n| !n.is_empty())
            .unwrap_or_else(|| format!("Боб {id}"));
        let name = self.unique_name(id, name);
        let color = self.free_color(who.color);
        let outfit = who.outfit.unwrap_or_default();
        let mut p = Player::human(id, name, color, outfit, who.uid.clone(), conn);
        p.spectator = !matches!(self.stage, Stage::Lobby);
        self.active_at = self.clock.real();
        info!(room = %self.id, %id, name = p.name, practice = self.practice(), "player joined");
        self.players.push(p);
        // (Only behind a PIN: a public room would keep every identity that ever came.)
        if self.pin.is_some() {
            self.admitted.insert(who.uid);
        }
        self.seat_host(id);
        if self.arena.kind == ArenaKind::Lobby {
            self.arena.add_pawn(id, false);
        }
        self.sync_bots();
        self.welcome(id, false);
        if let Some(pr) = self.practice.clone()
            && matches!(self.stage, Stage::Lobby)
        {
            let min = pr.game.meta().min_players as usize;
            let need = pr.bots.max(min.saturating_sub(1));
            for _ in 0..need {
                if self.players.len() >= self.max {
                    break;
                }
                self.add_bot(false);
            }
            self.start_game();
        }
        self.check_round();
        Some(id)
    }

    /// The connection of player `id` is gone: they keep their place a while to come back to (LOBBY_GRACE_S in
    /// the lobby, RECONNECT_GRACE_S in a game; `update`).
    pub fn leave(&mut self, id: PlayerId, conn: ConnId) {
        let real = self.clock.real();
        let Some(p) = self.player_mut(id).filter(|p| p.conn() == Some(conn)) else {
            return;
        };
        if let Kind::Human {
            conn, disconnected_at, ..
        } = &mut p.kind
        {
            *conn = None;
            *disconnected_at = Some(real);
        }
        info!(room = %self.id, %id, "player disconnected");
        self.update_host();
        self.send_lobby();
    }

    /// Player `id` leaves for good (back to the room list, or into another room).
    pub fn quit(&mut self, id: PlayerId) {
        let Some(p) = self.player_mut(id) else { return };
        let Kind::Human { conn, .. } = &mut p.kind else {
            return;
        };
        *conn = None;
        info!(room = %self.id, %id, "player left");
        self.remove_player(id);
    }

    pub fn control(&mut self, id: PlayerId, conn: ConnId, m: &ClientMsg) {
        let real = self.clock.real();
        let Some(p) = self.player_mut(id).filter(|p| p.conn() == Some(conn)) else {
            return;
        };
        let count = p.msgs.hit(real);
        // (Once a second at most while it lasts, and less often the longer it goes on.)
        let hushed = (count == MSG_RATE + 1).then(|| p.rate_log.hit(secs(real))).flatten();
        if let Some(hushed) = hushed {
            warn!(room = %self.id, %id, hushed, "rate limited");
        }
        if count > MSG_RATE {
            return;
        }
        let host = self.host == Some(id);
        let lobby = matches!(self.stage, Stage::Lobby);
        match m {
            // Handled before a message reaches the room (see Hub).
            ClientMsg::Hello(_) | ClientMsg::Create { .. } | ClientMsg::Join { .. } | ClientMsg::Leave => {}
            // Name, colour and outfit change at once and go out together, at most every PROFILE_EVERY_S (`update`).
            ClientMsg::Name(name) => {
                let name = sanitize_person_name(name);
                if !name.is_empty() {
                    let name = self.unique_name(id, name);
                    if let Some(p) = self.player_mut(id).filter(|p| p.name != name) {
                        p.name = name;
                        self.lobby_due = true;
                    }
                }
            }
            ClientMsg::Color(c) => {
                if lobby
                    && !self.players.iter().any(|o| o.id != id && o.color == *c)
                    && let Some(p) = self.player_mut(id).filter(|p| p.color != *c)
                {
                    p.color = *c;
                    self.lobby_due = true;
                }
            }
            ClientMsg::Outfit(o) => {
                if let Some(p) = self.player_mut(id).filter(|p| p.outfit != *o) {
                    p.outfit = *o;
                    self.lobby_due = true;
                }
            }
            ClientMsg::Start => {
                let early = self
                    .started_at
                    .is_some_and(|at| real.saturating_sub(at) < ticks(START_GAP_S));
                if host && lobby && !early && self.roster().len() >= self.min_players {
                    self.started_at = Some(real);
                    self.start_game();
                }
            }
            ClientMsg::Abort => {
                if host && !lobby && !self.practice() {
                    self.back_to_lobby();
                }
            }
            ClientMsg::Playlist(pl) => {
                if host && lobby {
                    self.playlist = valid_playlist(pl);
                    self.send_lobby();
                }
            }
            ClientMsg::AddBot => {
                if host && lobby && self.players.len() < self.max {
                    self.add_bot(false);
                    self.send_lobby();
                }
            }
            ClientMsg::RemoveBot(b) => {
                if host && lobby && self.player(*b).is_some_and(Player::is_bot) {
                    self.remove_player(*b);
                }
            }
            ClientMsg::Fill(on) => {
                if host && lobby && !self.practice() {
                    self.fill = *on;
                    // Off: the bots that only filled places go; the ones the host added stay.
                    if !on {
                        let autos: Vec<PlayerId> = self.players.iter().filter(|b| b.auto()).map(|b| b.id).collect();
                        for b in autos {
                            self.drop_player(b);
                        }
                    }
                    self.sync_bots();
                    self.send_lobby();
                }
            }
            ClientMsg::Access { private } => {
                // Not in a room nobody owns (the dev room, `--open-rooms`): its stand-in host could lock it and
                // leave, and nobody would know the PIN until a restart.
                if host && !self.practice() && self.owner.is_some() && self.pin.is_some() != *private {
                    self.pin = private.then(make_pin);
                    // A new PIN: whoever is here stays welcome, those who left need it.
                    self.admitted = self.players.iter().filter_map(|h| h.uid().cloned()).collect();
                    info!(room = %self.id, by = %id, private, "room access changed");
                    self.send_lobby();
                }
            }
            ClientMsg::Host(to) => {
                // The host hands the role over to another connected player (any phase).
                if host && *to != id && self.player(*to).is_some_and(|t| t.conn().is_some()) {
                    // Given away by the owner, it stays given when they reconnect; given back, it is theirs again.
                    if self.is_owner(id) {
                        self.handed_over = true;
                    }
                    if self.is_owner(*to) {
                        self.handed_over = false;
                    }
                    self.host = Some(*to);
                    self.send_lobby();
                }
            }
            ClientMsg::Emote(e) => {
                if self.arena.pawn(id).is_some_and(|p| p.status == PawnStatus::Play) {
                    self.broadcast(ServerMsg::Emote { id, e: *e });
                }
            }
            ClientMsg::Chat(text) => {
                let text = sanitize_chat(text);
                let Some(p) = self.player_mut(id) else { return };
                if text.is_empty() || p.chat_at.is_some_and(|at| real.saturating_sub(at) < ticks(CHAT_GAP_S)) {
                    return;
                }
                p.chat_at = Some(real);
                let name = p.name.clone();
                self.broadcast(ServerMsg::Chat { id, name, text });
            }
            ClientMsg::Dev { q, cmd } => {
                let result = if !self.dev() {
                    Err(DevError::Off)
                } else if !host {
                    Err(DevError::NotHost)
                } else {
                    let result = self.dev_command(id, cmd);
                    let ok = result.is_ok();
                    let msg = result.as_ref().map_or_else(ToString::to_string, Clone::clone);
                    info!(room = %self.id, %id, cmd = cmd.name(), ok, msg, "dev");
                    self.arena.note(
                        "dev",
                        Some(id),
                        Some(json!({ "cmd": cmd.name(), "ok": ok, "msg": msg })),
                    );
                    result
                };
                let result = result.map_err(|e| e.to_string());
                self.send_to(id, DevReply { q: *q, result }.into());
            }
        }
    }

    fn welcome(&mut self, id: PlayerId, resumed: bool) {
        self.send_to(
            id,
            ServerMsg::Welcome {
                id,
                room: self.id.clone(),
                solo: self.min_players <= 1,
                practice: self.practice(),
                resumed,
            },
        );
        if self.clock.shifted() {
            self.send_to(id, ServerMsg::Clock { rate: self.clock.rate });
        }
        self.send_lobby();
        self.send_to(id, ServerMsg::Arena(self.arena_info(true)));
        if let Some(conn) = self.player(id).and_then(Player::conn) {
            for ev in &self.history {
                let ev = MapEventMsg {
                    history: true,
                    ..ev.clone()
                };
                self.out.push(Out::Event(conn, ev));
            }
        }
    }

    // ------------------------------------------------------------------ players, host, bots

    fn free_color(&self, wish: Option<u8>) -> u8 {
        let used = |c: u8| self.players.iter().any(|p| p.color == c);
        if let Some(w) = wish.filter(|&w| (w as usize) < COLORS.len() && !used(w)) {
            return w;
        }
        (0u8..).take(COLORS.len()).find(|&c| !used(c)).unwrap_or(0)
    }

    pub(super) fn add_bot(&mut self, auto: bool) -> PlayerId {
        let id = self.next_id;
        self.next_id.0 += 1;
        let name = bot_name(self.players.iter().map(|p| p.name.as_str()), id);
        let p = Player::bot(id, name, self.free_color(None), bot_outfit(id), auto);
        self.players.push(p);
        if self.arena.kind == ArenaKind::Lobby {
            self.arena.add_pawn(id, true);
        }
        id
    }

    /// With "fill with bots" on, the lobby is kept full while someone is there to play with them.
    pub(super) fn sync_bots(&mut self) {
        if !self.fill || !matches!(self.stage, Stage::Lobby) || self.practice() {
            return;
        }
        if !self.players.iter().any(|p| p.conn().is_some()) {
            return;
        }
        while self.players.len() < self.max {
            self.add_bot(true);
        }
    }

    /// The bot that gives up its place to a person: in a round, one that no longer plays if there is one.
    fn spare_bot(&self) -> Option<PlayerId> {
        let bots: Vec<PlayerId> = self.players.iter().rev().filter(|p| p.is_bot()).map(|p| p.id).collect();
        bots.iter()
            .copied()
            .find(|&b| self.arena.pawn(b).is_none_or(|p| p.status != PawnStatus::Play))
            .or(bots.first().copied())
    }

    /// `id` just connected: the room's owner takes the host role back (unless they gave it away), anyone else
    /// may fill a vacancy.
    fn seat_host(&mut self, id: PlayerId) {
        if self.is_owner(id) && !self.handed_over {
            self.host = Some(id);
        } else {
            self.update_host();
        }
    }

    /// Player `id` is the person who owns the room.
    fn is_owner(&self, id: PlayerId) -> bool {
        self.owner.is_some() && self.player(id).and_then(Player::uid) == self.owner.as_ref()
    }

    /// The host must be connected: the owner if they are here, else whoever has been here longest.
    pub(super) fn update_host(&mut self) {
        let cur = self.host.and_then(|h| self.player(h));
        if cur.is_some_and(|c| c.conn().is_some()) {
            return;
        }
        let cur = cur.map(|c| c.id);
        let humans: Vec<&Player> = self.players.iter().filter(|p| !p.is_bot()).collect();
        let next = humans
            .iter()
            .find(|p| p.conn().is_some() && p.uid() == self.owner.as_ref())
            .or_else(|| humans.iter().find(|p| p.conn().is_some()));
        self.host = next.map(|p| p.id).or(cur).or(humans.first().map(|p| p.id));
        // The role is back with the owner: a handover before this one no longer counts.
        if self.host.is_some_and(|h| self.is_owner(h)) {
            self.handed_over = false;
        }
    }

    /// `name`, or with a number after it when someone else in the room is already called so.
    fn unique_name(&self, id: PlayerId, name: String) -> String {
        let taken = |n: &str| {
            let n = n.to_lowercase();
            self.players.iter().any(|p| p.id != id && p.name.to_lowercase() == n)
        };
        if !taken(&name) {
            return name;
        }
        for k in 2..=MAX_PLAYERS + 1 {
            let suffix = format!(" {k}");
            let keep = NAME_MAX.saturating_sub(suffix.chars().count());
            let base: String = name.chars().take(keep).collect();
            let candidate = format!("{}{suffix}", base.trim_end());
            if !taken(&candidate) {
                return candidate;
            }
        }
        name
    }

    /// Someone kept guessing the PIN (`Hub::join`): a new one, which the host sees in the lobby. Those the room let
    /// in before still come back without it.
    pub fn new_pin(&mut self) {
        if self.pin.is_none() {
            return;
        }
        self.pin = Some(make_pin());
        warn!(room = %self.id, "too many wrong PINs: the room has a new one");
        if let Some(h) = self.host {
            let text = "Кто-то подбирал PIN-код комнаты — он сменился, новый видно в лобби".into();
            self.send_to(h, ServerMsg::Notice(text));
        }
        self.send_lobby();
    }

    /// Takes a player out of the room and its arena (what that means for the room: `remove_player`).
    fn drop_player(&mut self, id: PlayerId) -> bool {
        let Some(i) = self.players.iter().position(|p| p.id == id) else {
            return false;
        };
        self.players.remove(i);
        self.arena.remove_pawn(id);
        self.broadcast(ServerMsg::Left(id));
        true
    }

    pub(super) fn remove_player(&mut self, id: PlayerId) {
        let bot = self.player(id).is_some_and(Player::is_bot);
        if !self.drop_player(id) {
            return;
        }
        if !bot && self.empty() {
            // The last person left: the bots go too, and the room waits in its lobby.
            self.players.clear();
            self.back_to_lobby();
            return;
        }
        self.update_host();
        self.sync_bots();
        self.send_lobby();
        self.check_round();
    }

    /// Game tick `s` seconds from now.
    pub(super) fn after(&self, s: f64) -> f64 {
        self.now() + ticks(s) as f64
    }
}
