//! The player's server list, what each says on `/fallbeans/health`, and the one being played on.
use core::time::Duration;
use std::collections::BTreeMap;
use std::net::IpAddr;

use bevy::ecs::system::SystemParam;
use bevy::prelude::*;
use bevy::settings::{ReflectSettingsGroup, SettingsGroup};
use fb_net::HTTP_PORT;
use fb_proto::Health;

use crate::job::Job;

/// How often the list asks every server again while it is on screen.
const REFRESH_S: f32 = 10.0;

#[derive(Resource, SettingsGroup, Reflect, Clone, PartialEq, Default)]
#[reflect(Resource, SettingsGroup, Default)]
#[settings_group(group = "servers")]
pub struct Servers {
    /// As the player typed them.
    pub list: Vec<String>,
    /// The one played on last (first in the list).
    pub last: Option<String>,
}

impl Servers {
    pub fn add(&mut self, addr: &str) -> bool {
        let addr = addr.trim().trim_end_matches('/');
        if addr.is_empty() || candidates(addr).is_empty() || self.list.iter().any(|s| s == addr) {
            return false;
        }
        self.list.push(addr.to_string());
        true
    }

    pub fn remove(&mut self, addr: &str) {
        self.list.retain(|s| s != addr);
        if self.last.as_deref() == Some(addr) {
            self.last = None;
        }
    }

    /// The last one first, then as added.
    pub fn ordered(&self) -> Vec<String> {
        let last = |s: &&String| Some(s.as_str()) == self.last.as_deref();
        let mut v: Vec<String> = self.list.iter().filter(last).cloned().collect();
        v.extend(self.list.iter().filter(|s| !last(s)).cloned());
        v
    }
}

/// Where the HTTP API of `addr` may be, in the order to try.
pub fn candidates(addr: &str) -> Vec<String> {
    let addr = addr.trim().trim_end_matches('/');
    if addr.is_empty() || addr.contains(char::is_whitespace) {
        return vec![];
    }
    if let Some((scheme, rest)) = addr.split_once("://") {
        if !matches!(scheme, "http" | "https") || rest.is_empty() {
            return vec![];
        }
        return vec![if rest.contains('/') {
            addr.to_string()
        } else {
            format!("{addr}/fallbeans")
        }];
    }
    if addr.contains('/') {
        return vec![];
    }
    if addr.parse::<IpAddr>().is_ok_and(|ip| ip.is_ipv6()) {
        return vec![format!("http://[{addr}]:{HTTP_PORT}/fallbeans")];
    }
    let (host, port) = match addr.rsplit_once(':') {
        // `[v6]` without a port.
        _ if addr.starts_with('[') && addr.ends_with(']') => (addr, None),
        Some((h, p)) if !h.contains(':') || h.starts_with('[') => match p.parse::<u16>() {
            Ok(p) => (h, Some(p)),
            Err(_) => return vec![],
        },
        _ => (addr, None),
    };
    if host.is_empty()
        || (host.starts_with('[')
            && !host
                .strip_prefix('[')
                .and_then(|h| h.strip_suffix(']'))
                .is_some_and(|h| h.parse::<core::net::Ipv6Addr>().is_ok()))
    {
        return vec![];
    }
    if let Some(p) = port {
        return vec![format!("http://{host}:{p}/fallbeans")];
    }
    if host.parse::<IpAddr>().is_ok() || host.starts_with('[') || !host.contains('.') {
        return vec![format!("http://{host}:{HTTP_PORT}/fallbeans")];
    }
    vec![
        format!("https://{host}/fallbeans"),
        format!("http://{host}:{HTTP_PORT}/fallbeans"),
    ]
}

/// What a server said about itself.
#[derive(Clone, Debug, PartialEq)]
pub struct Info {
    /// Its HTTP API (the candidate that answered).
    pub base: String,
    pub name: Option<String>,
    pub protocol: u32,
    pub build: String,
    pub rooms: u64,
    pub players: u64,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Status {
    Checking,
    Up(Info),
    Down,
}

/// A `/health` reply is a few dozen bytes; a server that sends more is not one of ours.
const HEALTH_MAX: u64 = 16 * 1024;
/// Characters of a server's build shown in the list.
const BUILD_MAX: usize = 48;

/// Asks `addr`'s candidates in turn (blocking).
pub fn check(addr: &str) -> Status {
    for base in candidates(addr) {
        let got = crate::net::agent()
            .get(format!("{base}/health"))
            .config()
            .timeout_global(Some(Duration::from_secs(3)))
            .build()
            .call();
        let Ok(mut r) = got else {
            continue;
        };
        let Ok(h) = r.body_mut().with_config().limit(HEALTH_MAX).read_json::<Health>() else {
            continue;
        };
        // (Whatever the server's owner wrote: one short line of visible characters, as a room title.)
        let name = h.name.as_deref().map(fb_shared::text::sanitize_title);
        let build = fb_shared::text::sanitize_chat(&h.build);
        return Status::Up(Info {
            base,
            name: name.filter(|s| !s.is_empty()),
            protocol: h.version,
            build: build.chars().take(BUILD_MAX).collect(),
            rooms: h.rooms,
            players: h.players,
        });
    }
    Status::Down
}

/// What every server in the list said last, and the checks under way.
#[derive(Resource, Default)]
pub struct States {
    pub of: BTreeMap<String, Status>,
    pending: Vec<(String, Job<Status>)>,
    next: Option<f32>,
}

impl States {
    pub fn get(&self, addr: &str) -> Status {
        self.of.get(addr).cloned().unwrap_or(Status::Checking)
    }

    /// Asks every server again at the next frame.
    pub fn refresh(&mut self) {
        self.next = Some(0.0);
    }
}

/// The server being played on: its HTTP API (None at the server list).
#[derive(Resource, Default, Clone, Debug)]
pub struct Target(pub Option<String>);

/// The servers added, what they answered, the one being played on, and the identities they gave.
#[derive(SystemParam)]
pub struct ServerBook<'w> {
    pub servers: ResMut<'w, Servers>,
    pub states: ResMut<'w, States>,
    pub target: ResMut<'w, Target>,
    pub ids: Res<'w, crate::settings::Identities>,
}

/// Checks the list's servers while the list is on screen.
pub fn poll(time: Res<Time<Real>>, servers: Res<Servers>, target: Res<Target>, mut res: ResMut<States>) {
    // (Marked changed only when what a server said did: the list on screen is redrawn then.)
    let states = res.bypass_change_detection();
    let (done, pending) = std::mem::take(&mut states.pending)
        .into_iter()
        .partition::<Vec<_>, _>(|(_, job)| job.ready());
    states.pending = pending;
    let was = states.of.clone();
    states
        .of
        .extend(done.into_iter().filter_map(|(addr, job)| Some((addr, job.join()?))));
    states.of.retain(|a, _| servers.list.contains(a));
    if states.of != was {
        res.set_changed();
    }
    let states = res.bypass_change_detection();
    let now = time.elapsed_secs();
    if target.0.is_some() {
        states.next = Some(0.0);
        return;
    }
    if states.next.is_some_and(|t| now < t) {
        return;
    }
    states.next = Some(now + REFRESH_S);
    for addr in &servers.list {
        if states.pending.iter().any(|(a, _)| a == addr) {
            continue;
        }
        let a = addr.clone();
        if let Ok(job) = Job::spawn("server-check", move || check(&a)) {
            states.pending.push((addr.clone(), job));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn where_the_api_is() {
        let c = |a: &str| candidates(a);
        assert_eq!(c("192.168.1.10"), ["http://192.168.1.10:5887/fallbeans"]);
        assert_eq!(c("192.168.1.10:7000"), ["http://192.168.1.10:7000/fallbeans"]);
        assert_eq!(c("mypc"), ["http://mypc:5887/fallbeans"]);
        assert_eq!(c("mypc:6000"), ["http://mypc:6000/fallbeans"]);
        assert_eq!(
            c("game.example.com"),
            [
                "https://game.example.com/fallbeans",
                "http://game.example.com:5887/fallbeans"
            ]
        );
        assert_eq!(c("https://game.example.com/"), ["https://game.example.com/fallbeans"]);
        assert_eq!(c("http://h:1/x/fallbeans"), ["http://h:1/x/fallbeans"]);
        assert_eq!(c("::1"), ["http://[::1]:5887/fallbeans"]);
        assert_eq!(c("[::1]:7000"), ["http://[::1]:7000/fallbeans"]);
        assert_eq!(c("[::1]"), ["http://[::1]:5887/fallbeans"]);
        for bad in ["", "a b", "h:x", "ftp://h", "h/x", ":80", "[x]", "[::1]:x", "[mypc]:80"] {
            assert!(c(bad).is_empty(), "{bad}");
        }
    }

    #[test]
    fn last_first() {
        let mut s = Servers::default();
        assert!(s.add("a "));
        assert!(s.add("b"));
        assert!(!s.add("a"));
        assert!(!s.add("a b"));
        s.last = Some("b".into());
        assert_eq!(s.ordered(), ["b", "a"]);
        s.remove("b");
        assert_eq!((s.ordered(), s.last), (vec!["a".to_string()], None));
    }
}
