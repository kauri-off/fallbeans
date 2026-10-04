//! The player's server list, what each says on `/fallbeans/health`, and the one being played on.
use core::time::Duration;
use std::collections::BTreeMap;
use std::net::IpAddr;
use std::sync::Mutex;
use std::sync::mpsc::{Receiver, TryRecvError, channel};

use bevy::prelude::*;
use bevy::settings::{ReflectSettingsGroup, SettingsGroup};
use fb_net::HTTP_PORT;

/// How often the list asks every server again while it is on screen.
const REFRESH_S: f32 = 10.0;

#[derive(Resource, SettingsGroup, Reflect, Clone, PartialEq, Default)]
#[reflect(Resource, SettingsGroup, Default)]
#[settings_group(group = "servers")]
pub struct Servers {
    /// As the player typed them.
    pub list: Vec<String>,
    /// The one played on last (first in the list).
    pub last: String,
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
        if self.last == addr {
            self.last.clear();
        }
    }

    /// The last one first, then as added.
    pub fn ordered(&self) -> Vec<String> {
        let mut v: Vec<String> = self.list.iter().filter(|s| **s == self.last).cloned().collect();
        v.extend(self.list.iter().filter(|s| **s != self.last).cloned());
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
        Some((h, p)) if !h.contains(':') || h.starts_with('[') => match p.parse::<u16>() {
            Ok(p) => (h, Some(p)),
            Err(_) => return vec![],
        },
        _ => (addr, None),
    };
    if host.is_empty() {
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
    pub updating: bool,
    pub rooms: u64,
    pub players: u64,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Status {
    Checking,
    Up(Info),
    Down,
}

fn agent() -> ureq::Agent {
    ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(3)))
        .build()
        .into()
}

/// Asks `addr`'s candidates in turn (blocking).
pub fn check(addr: &str) -> Status {
    let agent = agent();
    for base in candidates(addr) {
        let Ok(mut r) = agent.get(format!("{base}/health")).call() else {
            continue;
        };
        let Ok(v) = r.body_mut().read_json::<serde_json::Value>() else {
            continue;
        };
        if v.get("version").is_none() {
            continue;
        }
        let n = |k: &str| v[k].as_u64().unwrap_or(0);
        return Status::Up(Info {
            base,
            name: v["name"].as_str().filter(|s| !s.is_empty()).map(String::from),
            protocol: n("version") as u32,
            build: v["build"].as_str().unwrap_or_default().to_string(),
            updating: v["updating"].as_bool().unwrap_or(false),
            rooms: n("rooms"),
            players: n("players"),
        });
    }
    Status::Down
}

/// What every server in the list said last, and the checks under way.
#[derive(Resource, Default)]
pub struct States {
    pub of: BTreeMap<String, Status>,
    pending: Vec<(String, Mutex<Receiver<Status>>)>,
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

/// Checks the list's servers while the list is on screen.
pub fn poll(time: Res<Time>, servers: Res<Servers>, target: Res<Target>, mut states: ResMut<States>) {
    let states = &mut *states;
    let mut done = vec![];
    states.pending.retain(|(addr, rx)| {
        let got = rx.lock().unwrap_or_else(|e| e.into_inner()).try_recv();
        match got {
            Err(TryRecvError::Empty) => true,
            Ok(s) => {
                done.push((addr.clone(), s));
                false
            }
            Err(TryRecvError::Disconnected) => false,
        }
    });
    states.of.extend(done);
    states.of.retain(|a, _| servers.list.contains(a));
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
        let (tx, rx) = channel();
        let a = addr.clone();
        std::thread::spawn(move || {
            let _ = tx.send(check(&a));
        });
        states.pending.push((addr.clone(), Mutex::new(rx)));
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
        for bad in ["", "a b", "h:x", "ftp://h", "h/x", ":80"] {
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
        s.last = "b".into();
        assert_eq!(s.ordered(), ["b", "a"]);
        s.remove("b");
        assert_eq!((s.ordered(), s.last.as_str()), (vec!["a".to_string()], ""));
    }
}
