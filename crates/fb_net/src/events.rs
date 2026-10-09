//! Channels of the messages outside replication (the messages themselves are `fb_proto`'s).

/// Control messages both ways (room list, lobby, game flow, chat, dev): reliable, in order.
pub struct ControlChannel;

/// Map events with their tick (`fb_proto::MapEventMsg`): reliable, in order.
pub struct MapEventsChannel;

#[cfg(test)]
mod tests {
    use bevy_replicon::postcard;
    use fb_proto::{
        Cause, ClientMsg, DenyReason, DevCmd, Hello, MapEventKind, MapEventMsg, Playlist, RejectReason, ServerMsg,
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
                practice: Some("hex-a-gone".into()),
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
                games: vec!["door-dash".into(), "star-fall".into()],
                ..Default::default()
            }),
            ClientMsg::RemoveBot(u32::MAX),
            ClientMsg::Access { private: false },
            ClientMsg::Fill(true),
            ClientMsg::Emote(5),
            ClientMsg::Chat("ы".repeat(320)),
            ClientMsg::Dev {
                q: Some(7),
                cmd: DevCmd::Teleport {
                    id: Some(2),
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
                id: 9,
                room: "k7qxm".into(),
                solo: false,
                practice: true,
                resumed: true,
            },
            ServerMsg::Scores(vec![(1, -3), (2, i64::MAX)]),
            ServerMsg::Emote { id: 1, e: 2 },
            ServerMsg::Chat {
                id: 1,
                name: "Боб".into(),
                text: "привет".into(),
            },
            ServerMsg::Notice("PIN сменился".into()),
            ServerMsg::Left(4),
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
                id: 2,
                out: true,
                by: Some(5),
                cause: Cause::Tackle,
                shortcut: false,
            },
            history: true,
        });
    }

    #[test]
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
