//! Discord Rich Presence: where the player is, sent over Discord's local IPC from a thread of its own.
use core::time::Duration;
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender, channel};
use std::time::{SystemTime, UNIX_EPOCH};

use bevy::prelude::*;
use discord_rich_presence::activity::{Activity, Assets, Party, Timestamps};
use discord_rich_presence::{DiscordIpc, DiscordIpcClient};
use fb_proto::Phase;

use crate::session::Session;
use crate::settings::Display;

/// The Discord application whose name and art the status shows.
const APP_ID: &str = "1558807234840895610";
/// The art asset uploaded to that application.
const LOGO: &str = "logo";
/// How often to look for Discord again while it is not running.
const RETRY: Duration = Duration::from_secs(20);

pub struct DiscordPlugin;

impl Plugin for DiscordPlugin {
    fn build(&self, app: &mut App) {
        let (tx, rx) = channel();
        let spawned = std::thread::Builder::new()
            .name("discord".into())
            .spawn(move || run(rx));
        if let Err(e) = spawned {
            warn!("discord: no thread: {e}");
            return;
        }
        app.insert_resource(Link(tx));
        app.add_systems(Update, publish);
    }
}

#[derive(Resource)]
struct Link(Sender<Option<Status>>);

#[derive(Clone, PartialEq, Debug)]
struct Status {
    details: String,
    state: Option<String>,
    party: Option<[i32; 2]>,
    /// Unix milliseconds this scene began.
    since: i64,
}

/// What the scene is, without the time: a new one restarts the status's clock.
#[derive(Clone, PartialEq)]
struct Scene {
    details: String,
    state: Option<String>,
    party: Option<[i32; 2]>,
    arena: Option<u32>,
}

fn scene(session: &Session) -> Scene {
    let Some(lobby) = &session.lobby else {
        return Scene {
            details: "В меню".into(),
            state: None,
            party: None,
            arena: None,
        };
    };
    let players = lobby.players.iter().filter(|p| !p.spectator).count();
    let party = Some([players as i32, lobby.max as i32]);
    let arena = session.arena.as_ref();
    let round = |a: &fb_proto::ArenaInfo| {
        if a.practice || session.practice {
            "Тренировка".to_string()
        } else {
            format!("Раунд {} из {}", a.index, a.total)
        }
    };
    let (details, state, party) = match (lobby.phase, arena) {
        (Phase::Round, Some(a)) => (fb_maps::by_id(a.game).meta().title.to_string(), Some(round(a)), party),
        (Phase::Results, _) => ("Итоги раунда".into(), arena.map(round), party),
        (Phase::Podium, _) => ("Награждение".into(), None, party),
        _ => ("В лобби".into(), None, party),
    };
    Scene {
        details,
        state,
        party,
        arena: arena.map(|a| a.id).filter(|_| lobby.phase == Phase::Round),
    }
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as i64)
}

fn publish(
    session: Res<Session>,
    display: Res<Display>,
    link: Res<Link>,
    mut last: Local<Option<(Scene, i64)>>,
    mut sent: Local<Option<Option<Status>>>,
) {
    let status = display.discord.then(|| {
        let now = scene(&session);
        let since = match &*last {
            // (A count of players changing keeps the clock.)
            Some((s, t)) if s.details == now.details && s.arena == now.arena => *t,
            _ => now_ms(),
        };
        let status = Status {
            details: now.details.clone(),
            state: now.state.clone(),
            party: now.party,
            since,
        };
        *last = Some((now, since));
        status
    });
    if sent.as_ref() != Some(&status) {
        let _ = link.0.send(status.clone());
        *sent = Some(status);
    }
}

fn activity(s: &Status) -> Activity<'_> {
    let mut a = Activity::new()
        .details(s.details.as_str())
        .timestamps(Timestamps::new().start(s.since))
        .assets(Assets::new().large_image(LOGO).large_text("Fall Beans"));
    if let Some(state) = &s.state {
        a = a.state(state.as_str());
    }
    if let Some(size) = s.party {
        a = a.party(Party::new().size(size));
    }
    a
}

/// The IPC thread: the latest status wins, Discord is looked for again until it answers.
fn run(rx: Receiver<Option<Status>>) {
    let mut client = DiscordIpcClient::new(APP_ID);
    let mut connected = false;
    let mut want: Option<Status> = None;
    let mut dirty = false;
    loop {
        match rx.recv_timeout(RETRY) {
            Ok(s) => {
                want = s;
                dirty = true;
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => break,
        }
        while let Ok(s) = rx.try_recv() {
            want = s;
            dirty = true;
        }
        if !dirty {
            continue;
        }
        if !connected {
            connected = client.connect().is_ok();
            if !connected {
                continue;
            }
            info!("discord: connected");
        }
        let sent = match &want {
            Some(s) => client.set_activity(activity(s)),
            None => client.clear_activity(),
        };
        // (Discord answers every command: read it, or the answers pile up in the socket.)
        match sent.and_then(|()| client.recv()) {
            Ok(_) => dirty = false,
            Err(e) => {
                debug!("discord: {e}");
                let _ = client.close();
                connected = false;
            }
        }
    }
    if connected {
        let _ = client.close();
    }
}
