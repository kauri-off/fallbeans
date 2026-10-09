//! Channels of the messages outside replication (the messages themselves are `fb_proto`'s).

/// Control messages both ways (room list, lobby, game flow, chat, dev): reliable, in order.
pub struct ControlChannel;

/// Map events with their tick (`fb_proto::MapEventMsg`): reliable, in order.
pub struct MapEventsChannel;

#[cfg(test)]
mod tests {
    use bevy_replicon::postcard;
    use fb_proto::{
        Cause, ClientMsg, DenyReason, DevCmd, Hello, MapEventKind, MapEventMsg, MapId, PlayerId, Playlist,
        RejectReason, ServerMsg,
    };
    use serde::Serialize;
    use serde::de::DeserializeOwned;

    /// Each message comes back the same, and no cut of it decodes to it (or panics).
    fn round_trip<T: Serialize + DeserializeOwned + PartialEq + core::fmt::Debug>(msg: &T) {
        let mut buf = [0u8; 4096];
        let bytes = postcard::to_slice(msg, &mut buf).unwrap();
        assert_eq!(&postcard::from_bytes::<T>(bytes).unwrap(), msg);
        for n in 0..bytes.len() {
            assert_ne!(
                postcard::from_bytes::<T>(&bytes[..n]).ok().as_ref(),
                Some(msg),
                "{msg:?} cut at {n}"
            );
        }
    }

    #[test]
    fn client_messages_round_trip() {
        let msgs = [
            ClientMsg::Hello(Hello {
                name: "Боб 🫘".into(),
                room: Some("k7qxm".into()),
                pin: Some("0042".into()),
                practice: Some(MapId::HexAGone),
                color: Some(3),
                outfit: Some(Default::default()),
            }),
            ClientMsg::Name("Имя".into()),
            ClientMsg::Create {
                title: "Комната".into(),
                private: true,
            },
            ClientMsg::Join {
                room: "k7qxm".into(),
                pin: None,
            },
            ClientMsg::Leave,
            ClientMsg::Color(12),
            ClientMsg::Playlist(Playlist {
                games: vec![MapId::DoorDash, MapId::StarFall],
                ..Default::default()
            }),
            ClientMsg::RemoveBot(PlayerId(u32::MAX)),
            ClientMsg::Access { private: false },
            ClientMsg::Fill(true),
            ClientMsg::Emote(5),
            ClientMsg::Chat("ы".repeat(320)),
            ClientMsg::Dev {
                q: Some(7),
                cmd: DevCmd::Teleport {
                    id: Some(PlayerId(2)),
                    p: [-1.5, 1e9, 0.0],
                    yaw: Some(-3.0),
                },
            },
        ];
        for m in &msgs {
            round_trip(m);
        }
    }

    #[test]
    fn server_messages_round_trip() {
        let msgs = [
            ServerMsg::Ready { dev: true },
            ServerMsg::Reject {
                reason: RejectReason::Moved,
                msg: "Игра открыта в другом окне".into(),
            },
            ServerMsg::Denied {
                room: Some("k7qxm".into()),
                reason: DenyReason::Pin,
                msg: None,
            },
            ServerMsg::Home {
                msg: Some("—".into())
            },
            ServerMsg::Welcome {
                id: PlayerId(9),
                room: "k7qxm".into(),
                solo: false,
                practice: true,
                resumed: true,
            },
            ServerMsg::Scores(vec![(PlayerId(1), -3), (PlayerId(2), i64::MAX)]),
            ServerMsg::Emote { id: PlayerId(1), e: 2 },
            ServerMsg::Chat {
                id: PlayerId(1),
                name: "Боб".into(),
                text: "привет".into(),
            },
            ServerMsg::Notice("PIN сменился".into()),
            ServerMsg::Left(PlayerId(4)),
            ServerMsg::DevAck {
                q: Some(1),
                result: Err("no round running".into()),
            },
            ServerMsg::Clock { rate: 0.25 },
        ];
        for m in &msgs {
            round_trip(m);
        }
        round_trip(&MapEventMsg {
            arena: 3,
            tick: 1200,
            ev: MapEventKind::Ko {
                id: PlayerId(2),
                out: true,
                by: Some(PlayerId(5)),
                cause: Cause::Tackle,
                shortcut: false,
            },
            history: true,
        });
    }

    /// The fullest message each kind can be stays well under what a peer reassembles: past it the message is
    /// refused and the link dropped (`Transport::receive_failed`).
    #[test]
    #[expect(clippy::cast_possible_truncation, reason = "test data")]
    fn largest_messages_fit_a_fragmented_message() {
        use fb_proto::{
            ArenaInfo, Award, AwardKind, Lobby, LobbyPlayer, Mode, Phase, Playlist, RoomInfo, RoomRef, Standing,
        };
        use fb_shared::game::ArenaKind;
        use fb_shared::rules::{RoundNote, RoundRow};
        use fb_shared::{CHAT_MAX, MAX_PLAYERS, MAX_ROOMS, NAME_MAX, ROOM_TITLE_MAX};
        use lightyear::transport::channel::receive::MAX_FRAGMENTED_MESSAGE_BYTES;

        // Four-byte characters, as many as a sanitized field keeps.
        let text = |n: usize| "🫘".repeat(n);
        let pids = || {
            (1..=MAX_PLAYERS as u32)
                .map(|i| PlayerId(u32::MAX - i))
                .collect::<Vec<_>>()
        };
        let rooms = (0..MAX_ROOMS)
            .map(|_| RoomInfo {
                id: "k7qxmzzz".into(),
                title: text(ROOM_TITLE_MAX),
                private: true,
                host: Some(text(NAME_MAX)),
                players: u32::MAX,
                bots: u32::MAX,
                max: u32::MAX,
                phase: Phase::Podium,
            })
            .collect();
        let lobby = Lobby {
            room: RoomRef {
                id: Some("k7qxmzzz".into()),
                title: text(ROOM_TITLE_MAX),
                private: true,
            },
            phase: Phase::Podium,
            host: Some(PlayerId(u32::MAX)),
            min: u32::MAX,
            max: u32::MAX,
            players: pids()
                .into_iter()
                .map(|id| LobbyPlayer {
                    id,
                    name: text(NAME_MAX),
                    color: u8::MAX,
                    outfit: Default::default(),
                    score: i64::MIN,
                    crowns: u32::MAX,
                    spectator: true,
                    bot: true,
                    connected: true,
                    ping: u32::MAX,
                })
                .collect(),
            playlist: Playlist {
                mode: Mode::Custom,
                games: vec![MapId::Podium; 12],
                rounds: u32::MAX,
            },
            fill: true,
            pin: Some("0042".into()),
            next: Some(u32::MAX),
        };
        let row = |id| RoundRow {
            id,
            place: usize::MAX,
            points: i64::MIN,
            penalty: i64::MIN,
            delta: i64::MIN,
            total: i64::MIN,
            ok: true,
            note: RoundNote::Finish(Some(f64::MAX)),
            falls: u32::MAX,
        };
        let msgs = [
            ServerMsg::Rooms {
                rooms,
                mine: Some("k7qxmzzz".into()),
            },
            ServerMsg::Lobby(lobby),
            ServerMsg::Arena(ArenaInfo {
                id: u32::MAX,
                kind: ArenaKind::Podium,
                game: MapId::Podium,
                participants: pids(),
                index: u32::MAX,
                total: u32::MAX,
                practice: true,
                late: true,
                finished: pids(),
                out: pids(),
                scores: pids().into_iter().map(|id| (id, i64::MIN)).collect(),
            }),
            ServerMsg::RoundEnd {
                game: MapId::Podium,
                index: u32::MAX,
                total: u32::MAX,
                rows: pids().into_iter().map(row).collect(),
                practice: true,
            },
            ServerMsg::GameEnd {
                standings: pids()
                    .into_iter()
                    .map(|id| Standing {
                        id,
                        name: text(NAME_MAX),
                        color: u8::MAX,
                        place: u32::MAX,
                        total: i64::MIN,
                        wins: u32::MAX,
                        falls: u32::MAX,
                    })
                    .collect(),
                awards: vec![
                    Award {
                        kind: AwardKind::Sly,
                        id: PlayerId(u32::MAX),
                        value: u32::MAX,
                    };
                    6 * MAX_PLAYERS
                ],
            },
            ServerMsg::Chat {
                id: PlayerId(u32::MAX),
                name: text(NAME_MAX),
                text: text(CHAT_MAX),
            },
        ];
        let mut buf = vec![0u8; MAX_FRAGMENTED_MESSAGE_BYTES];
        for m in &msgs {
            let len = postcard::to_slice(m, &mut buf).map_or(usize::MAX, |b| b.len());
            assert!(len <= MAX_FRAGMENTED_MESSAGE_BYTES / 4, "{len} bytes: {m:?}");
        }
    }

    #[test]
    #[expect(clippy::cast_possible_truncation, reason = "random bytes: the low byte")]
    fn garbage_does_not_panic() {
        let mut x = 0x2545_f491_u32;
        for len in 0..64 {
            let bytes: Vec<u8> = (0..len)
                .map(|_| {
                    x ^= x << 13;
                    x ^= x >> 17;
                    x ^= x << 5;
                    x as u8
                })
                .collect();
            let _ = postcard::from_bytes::<ClientMsg>(&bytes);
            let _ = postcard::from_bytes::<ServerMsg>(&bytes);
        }
    }
}
