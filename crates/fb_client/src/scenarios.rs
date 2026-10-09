//! A player's paths through the client (`harness`): every crash found in play gets its path here first.
use bevy::input::keyboard::Key;
use bevy::prelude::{KeyCode, Vec2, World};
use fb_arena::ArenaKind;
use fb_net::ClientMsg;
use fb_proto::MapId;
use fb_proto::{DenyReason, DevCmd, Phase};

use crate::harness::Game;
use crate::keys::Bind;
use crate::render::quality::Preset;
use crate::session::{RoomList, Session};
use crate::ui::{Action, Field, Fold, Folds, Form, HomeTab, Knob, MenuTab, Ui, UiAction};

/// In the dev room's lobby, where the menu opens by itself on entry.
fn in_lobby(g: &mut Game) {
    g.until(15.0, "in the dev room's lobby", |w| {
        let s = w.resource::<Session>();
        s.room.is_some() && s.arena.is_some() && w.resource::<Ui>().menu
    });
}

fn on_room_list(g: &mut Game) {
    g.until(10.0, "the room list", |w| {
        w.resource::<RoomList>().rooms.is_some() && w.resource::<Session>().room.is_none()
    });
}

/// From the room list: names a room of one's own, opens it and waits in its lobby.
fn create_room(g: &mut Game, title: &str, private: bool) {
    on_room_list(g);
    if private {
        g.press(5.0, "Закрытая", |a| matches!(a, Action::CreatePrivate));
    }
    g.focus(Field::RoomTitle);
    g.type_text(title);
    g.blur();
    g.press(2.0, "Создать комнату", |a| {
        matches!(a, Action::CreateRoom)
    });
    in_lobby(g);
}

fn in_phase(g: &mut Game, secs: f32, phase: Phase) {
    g.until(secs, &format!("{phase:?}"), |w| {
        w.resource::<Session>().lobby.as_ref().is_some_and(|l| l.phase == phase)
    });
}

/// The menu, opened with Esc if it is closed. (Entering a room opens it by itself: an Esc in the same frame
/// closes it again, so it is checked over a few frames.)
fn menu(g: &mut Game) {
    for _ in 0..3 {
        if !g.res::<Ui>().menu {
            g.escape();
        }
        g.frames(3);
        if g.res::<Ui>().menu {
            return;
        }
    }
    panic!("Esc opens the menu");
}

/// A game of one round of `map` begun with the dev command, past its intro.
fn into_round(g: &mut Game, map: MapId) {
    g.dev(DevCmd::Start {
        games: vec![map],
        rounds: None,
        bots: Some(3),
    });
    g.until_arena(20.0, "the round", |a| a.kind == ArenaKind::Round && a.game == map);
    g.frames(60);
    g.dev(DevCmd::SkipIntro);
    g.frames(120);
}

#[test]
fn the_client_runs_without_a_gpu() {
    let mut g = Game::new(&[]);
    g.until(10.0, "the room list", |w| w.resource::<RoomList>().rooms.is_some());
}

/// The loading screen's warm-up (off on the bench but here): every map built and drawn as a round's, beans in
/// every hat, then its last map gone and the room list, whose connection waited for it.
#[test]
fn the_warm_up_builds_every_map() {
    use crate::render::warmup::Warmup;
    let mut g = Game::new(&["--warmup"]);
    g.until(300.0, "the warm-up's end", |w| !w.resource::<Warmup>().busy());
    assert!(
        !g.client().world().contains_resource::<crate::game::Map>(),
        "the warm-up's last map stays"
    );
    on_room_list(&mut g);
}

/// fb3af03: after a line was sent, the next one typed panicked (a cursor past the end of the cleared text).
/// Also: Enter opens the chat while the window holds the input focus (Bevy's default, and after a click on
/// nothing focusable).
#[test]
fn chat_lines_one_after_another() {
    let mut g = Game::new(&["--room", "dev"]);
    in_lobby(&mut g);
    g.escape();
    assert!(!g.res::<Ui>().menu, "Esc closes the menu");
    for line in ["привет 👋", "ещё", "ё"] {
        g.enter();
        assert!(g.res::<Ui>().chat, "Enter opens the chat");
        g.type_text(line);
        assert_eq!(g.text(Field::Chat), line);
        g.enter();
        assert!(!g.res::<Ui>().chat, "Enter sends the line and closes the chat");
        assert_eq!(g.text(Field::Chat), "");
    }
    g.enter();
    g.type_text("и снова");
    g.escape();
    assert!(!g.res::<Ui>().chat);
    assert_eq!(g.text(Field::Chat), "");
}

/// fb3af03: «Сохранить» a name the server tidies (spaces), then type in the field again.
#[test]
fn rename_in_the_lobby_menu() {
    let mut g = Game::new(&["--room", "dev", "--name", "Боб"]);
    in_lobby(&mut g);
    g.focus(Field::MenuName);
    g.type_text(" Ёжик  ");
    // (A click on the button takes the focus from the field.)
    g.blur();
    g.press(2.0, "Сохранить", |a| {
        matches!(a, Action::SaveName(Field::MenuName))
    });
    g.until(5.0, "the tidied name back in the field", |w| {
        w.resource::<crate::settings::Player>().name == "Боб Ёжик"
    });
    g.frames(30);
    assert_eq!(g.text(Field::MenuName), "Боб Ёжик");
    g.focus(Field::MenuName);
    g.type_text("ё");
    g.frames(5);
}

/// The keys belong to the game in play (`Gate`), and Esc opens the menu.
fn keys_in_play(g: &mut Game) {
    g.frames(10);
    assert!(!g.typing(), "a hidden field holds the keyboard");
    assert!(g.res::<crate::game::Gate>().play, "the game has no keys");
    menu(g);
}

/// The round starts (the menu closes by itself) while the cursor is in the menu's name field: the keys went into
/// the hidden field, the game and Esc got none.
#[test]
fn a_round_starts_while_typing_a_name() {
    let mut g = Game::new(&["--room", "dev"]);
    in_lobby(&mut g);
    g.focus(Field::MenuName);
    g.type_text("ё");
    into_round(&mut g, MapId::DoorDash);
    assert!(!g.res::<Ui>().menu, "the round closes the menu");
    keys_in_play(&mut g);
}

/// Enter in the room's title creates it: the field left behind on the hidden room list kept the keyboard.
#[test]
fn a_room_created_with_enter() {
    let mut g = Game::new(&[]);
    on_room_list(&mut g);
    g.focus(Field::RoomTitle);
    g.type_text("Энтер");
    g.enter();
    in_lobby(&mut g);
    g.escape();
    assert!(!g.res::<Ui>().menu, "Esc closes the menu");
    keys_in_play(&mut g);
}

/// The host stops the game mid-round from the menu, then starts the next one with the lobby's button.
#[test]
fn abort_mid_round_and_start_again() {
    let mut g = Game::new(&["--room", "dev", "--autopilot"]);
    in_lobby(&mut g);
    into_round(&mut g, MapId::HammerSwing);
    menu(&mut g);
    g.press(2.0, "Прервать игру", |a| {
        matches!(a, Action::Send(ClientMsg::Abort))
    });
    in_phase(&mut g, 10.0, Phase::Lobby);
    g.until(10.0, "the lobby's arena", |w| {
        w.resource::<Session>()
            .arena
            .as_ref()
            .is_some_and(|a| a.kind == ArenaKind::Lobby)
    });
    g.frames(60);
    menu(&mut g);
    g.press(5.0, "Начать игру", |a| {
        matches!(a, Action::Send(ClientMsg::Start))
    });
    let rounds = |g: &Game| g.arenas().iter().filter(|a| a.kind == ArenaKind::Round).count();
    let end = std::time::Instant::now() + std::time::Duration::from_secs(20);
    while rounds(&g) < 2 {
        assert!(std::time::Instant::now() < end, "no second round");
        g.step();
    }
    g.frames(120);
}

/// Out of a room mid-round and back into the game; the owner away (a stand-in hosts) and back as host; out of
/// the podium. The room closes when its last person leaves (and the game ends with them).
#[test]
fn leave_the_room_and_come_back() {
    let mut g = Game::new(&["--autopilot"]);
    create_room(&mut g, "Комната 🫘 ё", false);
    let room = g.res::<Session>().room.clone().unwrap();
    let guest = g.add_peer(wgpu::DeviceType::DiscreteGpu, &["--autopilot"]);
    g.as_peer(guest);
    on_room_list(&mut g);
    g.press(5.0, "Войти", join(&room));
    in_lobby(&mut g);

    g.as_peer(0);
    g.until(5.0, "the guest in the lobby", |w| {
        w.resource::<Session>()
            .lobby
            .as_ref()
            .is_some_and(|l| l.players.len() == 2)
    });
    into_round(&mut g, MapId::DoorDash);

    g.as_peer(guest);
    g.until(10.0, "the guest's round", in_round);
    menu(&mut g);
    g.press(2.0, "Выйти из комнаты", |a| {
        matches!(a, Action::LeaveRoom)
    });
    on_room_list(&mut g);
    g.frames(30);
    g.press(5.0, "Войти", join(&room));
    g.until(10.0, "the guest back in the round", in_round);
    g.frames(60);

    g.as_peer(0);
    menu(&mut g);
    g.press(2.0, "Выйти из комнаты", |a| {
        matches!(a, Action::LeaveRoom)
    });
    on_room_list(&mut g);
    g.as_peer(guest);
    g.until(5.0, "the guest stands in as host", |w| w.resource::<Session>().host());
    g.as_peer(0);
    g.press(5.0, "Вернуться в свою комнату", join(&room));
    g.until(10.0, "the owner back in the round", in_round);
    g.until(5.0, "the owner hosts again", |w| w.resource::<Session>().host());
    g.frames(60);

    g.dev(DevCmd::EndRound);
    g.until_arena(30.0, "the podium", |a| a.kind == ArenaKind::Podium);
    g.frames(60);
    g.as_peer(guest);
    menu(&mut g);
    g.press(2.0, "Выйти из комнаты", |a| {
        matches!(a, Action::LeaveRoom)
    });
    on_room_list(&mut g);
    g.as_peer(0);
    menu(&mut g);
    g.press(2.0, "Выйти из комнаты", |a| {
        matches!(a, Action::LeaveRoom)
    });
    on_room_list(&mut g);
    g.until(5.0, "the room closed", |w| {
        let s = w.resource::<RoomList>();
        s.mine.is_none() && s.rooms.as_ref().is_some_and(|l| l.iter().all(|r| r.id != room))
    });
}

/// A private room: a second player gets in with its PIN (after a wrong one), takes over as host, the owner
/// leaves mid-round, the new host stops the game.
#[test]
fn private_room_pin_and_a_second_player() {
    let mut g = Game::new(&["--name", "Хозяин"]);
    create_room(&mut g, "", true);
    g.until(5.0, "the PIN", |w| {
        w.resource::<Session>().lobby.as_ref().is_some_and(|l| l.pin.is_some())
    });
    let s = g.res::<Session>();
    let (room, pin) = (s.room.clone().unwrap(), s.lobby.as_ref().unwrap().pin.clone().unwrap());
    let wrong = if pin == "0000" { "1111" } else { "0000" };

    let guest = g.add_peer(wgpu::DeviceType::IntegratedGpu, &["--name", "Гость"]);
    g.as_peer(guest);
    on_room_list(&mut g);
    let r = room.clone();
    g.press(5.0, "Войти", move |a| matches!(a, Action::Join(id) if *id == r));
    g.until(5.0, "the PIN asked", |w| {
        w.resource::<Session>()
            .denied
            .as_ref()
            .is_some_and(|d| d.reason == DenyReason::Pin)
    });
    g.focus(Field::Pin);
    g.type_text("12a3456");
    assert_eq!(g.text(Field::Pin).chars().count(), 4, "four digits only");
    g.erase(Field::Pin);
    g.type_text(wrong);
    g.enter();
    g.until(5.0, "a wrong PIN told", |w| {
        w.resource::<Session>().denied.as_ref().is_some_and(|d| d.msg.is_some())
    });
    // (The box is redrawn with the message; the field keeps the keyboard.)
    g.erase(Field::Pin);
    g.type_text(&pin);
    g.enter();
    in_lobby(&mut g);
    let guest_id = g.res::<Session>().me.unwrap();

    g.as_peer(0);
    g.until(5.0, "the guest in the lobby", |w| {
        w.resource::<Session>()
            .lobby
            .as_ref()
            .is_some_and(|l| l.players.len() == 2)
    });
    g.press(
        5.0,
        "Сделать хостом",
        |a| matches!(a, Action::Send(ClientMsg::Host(id)) if *id == guest_id),
    );

    g.as_peer(guest);
    g.until(5.0, "the guest hosts", |w| w.resource::<Session>().host());
    g.press(5.0, "Начать игру", |a| {
        matches!(a, Action::Send(ClientMsg::Start))
    });
    g.until_arena(20.0, "the guest's round", |a| a.kind == ArenaKind::Round);
    g.as_peer(0);
    g.until_arena(20.0, "the owner's round", |a| a.kind == ArenaKind::Round);
    g.frames(60);
    menu(&mut g);
    assert!(
        !g.shows(|a| matches!(a, Action::Send(ClientMsg::Abort))),
        "only the host stops the game"
    );
    g.press(2.0, "Выйти из комнаты", |a| {
        matches!(a, Action::LeaveRoom)
    });
    on_room_list(&mut g);

    g.as_peer(guest);
    g.frames(60);
    menu(&mut g);
    g.press(2.0, "Прервать игру", |a| {
        matches!(a, Action::Send(ClientMsg::Abort))
    });
    in_phase(&mut g, 10.0, Phase::Lobby);
    g.frames(60);
}

fn join(room: &str) -> impl Fn(&Action) -> bool + use<> {
    let r = room.to_string();
    move |a: &Action| matches!(a, Action::Join(id) if *id == r)
}

fn in_round(w: &mut World) -> bool {
    w.resource::<Session>()
        .arena
        .as_ref()
        .is_some_and(|a| a.kind == ArenaKind::Round)
}

/// The owner leaves the lobby (no game on): the guest stands in as host and starts the game, the owner comes
/// back into the round and hosts again.
#[test]
fn the_owner_leaves_the_lobby() {
    let mut g = Game::new(&["--name", "Хозяин", "--autopilot"]);
    create_room(&mut g, "Ухожу", false);
    let room = g.res::<Session>().room.clone().unwrap();
    let guest = g.add_peer(wgpu::DeviceType::DiscreteGpu, &["--name", "Гость", "--autopilot"]);
    g.as_peer(guest);
    on_room_list(&mut g);
    g.press(5.0, "Войти", join(&room));
    in_lobby(&mut g);

    g.as_peer(0);
    g.until(5.0, "the guest in the lobby", |w| {
        w.resource::<Session>()
            .lobby
            .as_ref()
            .is_some_and(|l| l.players.len() == 2)
    });
    menu(&mut g);
    g.press(2.0, "Выйти из комнаты", |a| {
        matches!(a, Action::LeaveRoom)
    });
    on_room_list(&mut g);

    g.as_peer(guest);
    g.until(5.0, "the guest stands in as host", |w| w.resource::<Session>().host());
    menu(&mut g);
    g.press(5.0, "Начать игру", |a| {
        matches!(a, Action::Send(ClientMsg::Start))
    });
    g.until(20.0, "the guest's round", in_round);

    g.as_peer(0);
    g.press(5.0, "Вернуться в свою комнату", join(&room));
    g.until(10.0, "the owner in the round", in_round);
    g.until(5.0, "the owner hosts again", |w| w.resource::<Session>().host());
    g.frames(120);
}

/// Someone who was never in the room comes in mid-round: they watch it to the podium, then play the next game.
#[test]
fn a_newcomer_watches_the_round_in_play() {
    let mut g = Game::new(&["--autopilot"]);
    create_room(&mut g, "Идёт игра", false);
    let room = g.res::<Session>().room.clone().unwrap();
    into_round(&mut g, MapId::HammerSwing);

    let late = g.add_peer(wgpu::DeviceType::IntegratedGpu, &["--autopilot"]);
    g.as_peer(late);
    on_room_list(&mut g);
    g.press(5.0, "Войти", join(&room));
    g.until(10.0, "the newcomer sees the round", |w| {
        w.resource::<Session>()
            .arena
            .as_ref()
            .is_some_and(|a| a.kind == ArenaKind::Round && a.late)
    });
    let me = g.res::<Session>().me.unwrap();
    let first = g.res::<Session>().arena.clone().unwrap();
    assert!(!first.participants.contains(&me), "a newcomer only watches");
    g.frames(120);
    menu(&mut g);
    g.escape();
    g.frames(30);

    g.as_peer(0);
    g.dev(DevCmd::EndRound);
    g.as_peer(late);
    g.until_arena(30.0, "the podium", |a| a.kind == ArenaKind::Podium);
    g.frames(60);
    g.as_peer(0);
    into_round(&mut g, MapId::DoorDash);
    g.as_peer(late);
    g.until(20.0, "the next game's round, played", |w| {
        w.resource::<Session>()
            .arena
            .as_ref()
            .is_some_and(|a| a.kind == ArenaKind::Round && a.id != first.id && a.participants.contains(&me))
    });
    g.frames(120);
}

/// A key rebound mid-round in the menu; Esc while waiting for a key keeps the old one and the menu open; the
/// round ending mid-wait (the podium closes the menu) lets it be.
#[test]
fn rebind_a_key_mid_round() {
    let mut g = Game::new(&["--room", "dev", "--autopilot"]);
    in_lobby(&mut g);
    into_round(&mut g, MapId::DoorDash);
    menu(&mut g);
    g.press(2.0, "Настройки", |a| {
        matches!(a, Action::MenuTab(MenuTab::Settings))
    });
    g.press(2.0, "Клавиши", |a| matches!(a, Action::Fold(Fold::Keys)));
    let jump = |g: &Game| g.res::<crate::settings::Bindings>().keys(Bind::Jump).to_vec();
    g.press(2.0, "Изменить: прыжок", |a| {
        matches!(a, Action::Rebind(Bind::Jump))
    });
    g.key(KeyCode::KeyK, Key::Character("k".into()), Some("k"));
    assert_eq!(jump(&g), [KeyCode::KeyK]);
    assert!(g.res::<Ui>().menu, "a key picked keeps the menu open");

    g.press(2.0, "Изменить: прыжок", |a| {
        matches!(a, Action::Rebind(Bind::Jump))
    });
    g.escape();
    assert_eq!(jump(&g), [KeyCode::KeyK], "Esc keeps the key");
    assert!(g.res::<Ui>().rebinding.is_none());
    assert!(g.res::<Ui>().menu, "Esc while waiting for a key keeps the menu open");

    g.press(2.0, "Изменить: прыжок", |a| {
        matches!(a, Action::Rebind(Bind::Jump))
    });
    let round = g.res::<Session>().arena.as_ref().unwrap().id;
    g.dev(DevCmd::EndRound);
    g.until(30.0, "the next arena", |w| {
        w.resource::<Session>().arena.as_ref().is_some_and(|a| a.id != round)
    });
    g.frames(10);
    assert!(g.res::<Ui>().rebinding.is_none(), "the wait ends with the menu");
    g.key(KeyCode::KeyL, Key::Character("l".into()), Some("l"));
    assert_eq!(jump(&g), [KeyCode::KeyK]);
    g.frames(60);
}

/// The server list: a wrong address, one added and removed, then this server added and entered.
#[test]
fn the_server_list() {
    let mut g = Game::new(&[]);
    on_room_list(&mut g);
    g.press(5.0, "К серверам", |a| matches!(a, Action::LeaveServer));
    g.until(5.0, "the server list", |w| {
        w.resource::<crate::servers::Target>().0.is_none()
    });
    g.focus(Field::Server);
    g.type_text("не адрес!");
    g.enter();
    assert!(g.res::<Form>().server_bad, "a wrong address is told");
    g.erase(Field::Server);
    g.type_text("127.0.0.1:1");
    g.enter();
    assert!(!g.res::<Form>().server_bad);
    assert!(
        g.res::<crate::servers::Servers>()
            .list
            .iter()
            .any(|s| s == "127.0.0.1:1")
    );
    g.frames(30);
    g.press(
        5.0,
        "× у сервера",
        |a| matches!(a, Action::RemoveServer(s) if s == "127.0.0.1:1"),
    );
    assert!(
        !g.res::<crate::servers::Servers>()
            .list
            .iter()
            .any(|s| s == "127.0.0.1:1")
    );
    let here = format!("127.0.0.1:{}", g.http_port());
    g.focus(Field::Server);
    g.erase(Field::Server);
    g.type_text(&here);
    g.blur();
    g.press(2.0, "Добавить", |a| matches!(a, Action::AddServer));
    let h = here.clone();
    g.press(
        10.0,
        "Играть на этом сервере",
        move |a| matches!(a, Action::Connect(s) if *s == h),
    );
    on_room_list(&mut g);
}

/// From the room list to the server list, `addr` added to it if it is not there, and «Играть» on it.
fn connect_through_the_list(g: &mut Game, addr: &str) {
    g.press(5.0, "К серверам", |a| matches!(a, Action::LeaveServer));
    g.until(5.0, "the server list", |w| {
        w.resource::<crate::servers::Target>().0.is_none()
    });
    if !g.res::<crate::servers::Servers>().list.iter().any(|s| s == addr) {
        g.focus(Field::Server);
        g.erase(Field::Server);
        g.type_text(addr);
        g.blur();
        g.press(2.0, "Добавить", |a| matches!(a, Action::AddServer));
    }
    let a = addr.to_string();
    g.press(
        10.0,
        "Играть на этом сервере",
        move |x| matches!(x, Action::Connect(s) if *s == a),
    );
}

/// Audit #4: one identity was shared by every server. Server B was sent the token server A gave, and B's reply
/// replaced it, so back on A the player was somebody new (their room no longer theirs). Now each server gets
/// only its own.
#[test]
fn an_identity_for_each_server() {
    let mut g = Game::new(&[]);
    on_room_list(&mut g);
    let a_http = format!("http://127.0.0.1:{}/fallbeans", g.http_port());
    let a_addr = format!("127.0.0.1:{}", g.http_port());
    let id_of = |g: &Game, http: &str| g.res::<crate::settings::Identities>().get(http);
    let a = id_of(&g, &a_http).expect("server A's identity kept");
    let b_port = g.add_server();
    let b_http = format!("http://127.0.0.1:{b_port}/fallbeans");
    assert_eq!(id_of(&g, &b_http), None);

    connect_through_the_list(&mut g, &format!("127.0.0.1:{b_port}"));
    assert_ne!(
        g.res::<crate::net::Conn>().identity.as_deref(),
        Some(a.as_str()),
        "server B was sent A's identity"
    );
    on_room_list(&mut g);
    let b = id_of(&g, &b_http).expect("server B's identity kept");
    assert_ne!(a, b);
    assert_eq!(
        id_of(&g, &a_http).as_deref(),
        Some(a.as_str()),
        "B's reply took A's identity"
    );

    connect_through_the_list(&mut g, &a_addr);
    on_room_list(&mut g);
    assert_eq!(
        g.res::<crate::net::Conn>().identity.as_deref(),
        Some(a.as_str()),
        "back on A, the same player"
    );
    assert_eq!(id_of(&g, &b_http).as_deref(), Some(b.as_str()));
}

/// Leaving a room takes its map along: every room's first lobby is arena 1 with seed 1, and the next room
/// used to keep the last one's map (its players, bonuses and decorations).
#[test]
fn the_map_goes_with_the_room() {
    use crate::game::Map;
    let mut g = Game::new(&[]);
    create_room(&mut g, "Первая", false);
    g.until(10.0, "the lobby's map", |w| w.contains_resource::<Map>());
    let first = g.res::<Map>().generation;
    menu(&mut g);
    g.press(2.0, "Выйти из комнаты", |a| {
        matches!(a, Action::LeaveRoom)
    });
    on_room_list(&mut g);
    g.until(5.0, "the map gone and the room closed", |w| {
        !w.contains_resource::<Map>() && w.resource::<RoomList>().mine.is_none()
    });
    create_room(&mut g, "Вторая", false);
    g.until(10.0, "the new lobby's own map", |w| {
        w.get_resource::<Map>().is_some_and(|m| m.generation > first)
    });
    g.frames(30);
}

/// The Esc that closes the chat line does not open the menu as well.
#[test]
fn esc_closes_the_chat_not_the_menu() {
    let mut g = Game::new(&["--room", "dev"]);
    in_lobby(&mut g);
    g.escape();
    assert!(!g.res::<Ui>().menu, "Esc closes the menu");
    g.enter();
    assert!(g.res::<Ui>().chat, "Enter opens the chat");
    g.type_text("ой");
    g.escape();
    assert!(!g.res::<Ui>().chat, "Esc closes the chat");
    assert!(!g.res::<Ui>().menu, "the Esc that closed the chat opened the menu");
    g.escape();
    assert!(g.res::<Ui>().menu, "the next Esc opens it");
}

/// A key taken from an action that had only it: the two swap (it used to get its defaults back, the taken key
/// among them, so Q both jumped and grabbed). The game's own keys (F8, the emotes) are not taken.
#[test]
fn rebind_takes_a_key_from_another_action() {
    let mut g = Game::new(&[]);
    on_room_list(&mut g);
    g.press(5.0, "Настройки", |a| {
        matches!(a, Action::HomeTab(HomeTab::Settings))
    });
    g.press(2.0, "Клавиши", |a| matches!(a, Action::Fold(Fold::Keys)));
    let keys = |g: &Game, b: Bind| g.res::<crate::settings::Bindings>().keys(b).to_vec();
    g.press(2.0, "Изменить: прыжок", |a| {
        matches!(a, Action::Rebind(Bind::Jump))
    });
    g.key(KeyCode::F8, Key::F8, None);
    g.key(KeyCode::Digit1, Key::Character("1".into()), Some("1"));
    assert_eq!(g.res::<Ui>().rebinding, Some(Bind::Jump), "F8 and 1 are the game's");
    g.key(KeyCode::KeyQ, Key::Character("q".into()), Some("q"));
    assert_eq!(keys(&g, Bind::Jump), [KeyCode::KeyQ]);
    assert_eq!(keys(&g, Bind::Grab), [KeyCode::Space], "grab takes jump's key");
    g.press(2.0, "Изменить: нырок", |a| {
        matches!(a, Action::Rebind(Bind::Dive))
    });
    g.key(KeyCode::KeyW, Key::Character("w".into()), Some("w"));
    assert_eq!(keys(&g, Bind::Dive), [KeyCode::KeyW]);
    assert_eq!(
        keys(&g, Bind::Forward),
        [KeyCode::ArrowUp],
        "forward keeps its other key"
    );
    g.frames(10);
}

/// fuzz-ui seed 22: a second click on «Играть» in the frame the button went, after the first press had its link:
/// two links connected and replicon panicked.
#[test]
fn connect_pressed_twice() {
    let mut g = Game::new(&[]);
    on_room_list(&mut g);
    g.press(5.0, "К серверам", |a| matches!(a, Action::LeaveServer));
    let here = format!("127.0.0.1:{}", g.http_port());
    g.focus(Field::Server);
    g.type_text(&here);
    g.blur();
    g.press(2.0, "Добавить", |a| matches!(a, Action::AddServer));
    let h = here.clone();
    g.press(
        10.0,
        "Играть на этом сервере",
        move |a| matches!(a, Action::Connect(s) if *s == h),
    );
    g.until(5.0, "the link", |w| {
        w.get_resource::<crate::net::Conn>().is_some_and(|c| c.entity.is_some())
    });
    g.client().world_mut().write_message(UiAction(Action::Connect(here)));
    on_room_list(&mut g);
    g.frames(120);
}

/// Practice from the room list and its end; practice from a room's menu and back into that room.
#[test]
fn practice_and_back() {
    let mut g = Game::new(&["--autopilot"]);
    on_room_list(&mut g);
    let practice = |game: MapId| move |a: &Action| matches!(a, Action::Practice(id) if *id == game);
    let in_practice = |w: &mut World| {
        let s = w.resource::<Session>();
        s.practice && s.arena.as_ref().is_some_and(|a| a.kind == ArenaKind::Round)
    };
    g.press(5.0, "Тренировка", |a| {
        matches!(a, Action::Fold(Fold::Practice))
    });
    g.press(2.0, "Тренировка: прыжки", practice(MapId::JumpClub));
    g.until(15.0, "the practice round", in_practice);
    g.frames(120);
    g.dev(DevCmd::EndRound);
    g.frames(120);
    menu(&mut g);
    g.press(5.0, "К списку комнат", |a| {
        matches!(a, Action::EndPractice)
    });
    on_room_list(&mut g);

    create_room(&mut g, "Тренируюсь", false);
    let room = g.res::<Session>().room.clone();
    // (The fold stays open as it was left at the room list.)
    assert!(g.res::<Folds>().open(Fold::Practice));
    g.press(2.0, "Тренировка: двери", practice(MapId::DoorDash));
    g.until(15.0, "the practice round", in_practice);
    g.frames(120);
    menu(&mut g);
    g.press(5.0, "Вернуться в комнату", |a| {
        matches!(a, Action::EndPractice)
    });
    in_lobby(&mut g);
    assert_eq!(g.res::<Session>().room, room);
    assert!(!g.res::<Session>().practice);
}

/// Into practice and back reconnects (`net::restart`), and what the server sent goes with the old link. Found by
/// `practice_and_back` under load: a bean's face was attached in the frame of the reconnect, and its `insert`
/// hit the despawned bean. Here a system reconnects and the next one, with no sync point between them, gives
/// every bean a command; then the same with leaving the server (`net::close`).
#[test]
fn reconnecting_spares_commands_queued_on_beans() {
    use bevy::prelude::*;
    #[derive(Component)]
    struct Touched;
    #[derive(Resource, Default)]
    struct Now(Option<bool>);
    let hang_up = |mut now: ResMut<Now>, conn: Option<ResMut<crate::net::Conn>>, mut commands: Commands| {
        let (Some(close), Some(mut conn)) = (now.0.take(), conn) else {
            return;
        };
        if close {
            crate::net::close(&mut commands, &conn);
        } else {
            crate::net::restart(&mut commands, &mut conn);
        }
    };
    let touch = |mut commands: Commands, beans: Query<Entity, With<crate::beans::Rig>>| {
        for e in &beans {
            commands.entity(e).insert(Touched);
        }
    };
    let mut g = Game::new(&["--room", "dev"]);
    g.client().init_resource::<Now>();
    g.client().add_systems(Update, (hang_up, touch).chain_ignore_deferred());
    let beans = |w: &mut World| w.query_filtered::<(), With<crate::beans::Rig>>().iter(w).count() > 0;
    for close in [false, true] {
        in_lobby(&mut g);
        g.until(10.0, "a bean", beans);
        g.client().world_mut().resource_mut::<Now>().0 = Some(close);
        g.frames(30);
        if close {
            assert!(!beans(g.client().world_mut()), "the server's beans gone with it");
        } else {
            in_lobby(&mut g);
        }
    }
}

/// The server stops answering mid-round: the client reconnects by itself, back into the round it was in.
#[test]
fn reconnect_after_the_server_was_away() {
    let mut g = Game::new(&["--room", "dev", "--autopilot"]);
    in_lobby(&mut g);
    into_round(&mut g, MapId::HammerSwing);
    let room = g.res::<Session>().room.clone();
    g.server_away(crate::harness::LINK_TIMEOUT_S as f32 + 4.0);
    assert!(!g.res::<crate::net::Conn>().connected, "the link is down");
    g.until(30.0, "connected again", |w| w.resource::<crate::net::Conn>().connected);
    g.until(10.0, "back in the round", |w| {
        w.resource::<Session>()
            .arena
            .as_ref()
            .is_some_and(|a| a.kind == ArenaKind::Round)
    });
    assert_eq!(g.res::<Session>().room, room);
    g.frames(120);
    g.dev(DevCmd::EndRound);
    g.until_arena(30.0, "the podium", |a| a.kind == ArenaKind::Podium);
}

/// Presses every button on screen whose action passes `which`, one by one; how many there were.
fn press_every(g: &mut Game, which: impl Fn(&Action) -> bool) -> usize {
    let all = g.actions(which);
    for a in &all {
        let want = format!("{a:?}");
        g.press(2.0, &want, |b| format!("{b:?}") == want);
        g.frames(10);
    }
    all.len()
}

fn sliders(g: &mut Game) {
    for (knob, v) in [
        (Knob::MouseSens, 2.0),
        (Knob::StickSens, 0.5),
        (Knob::Fov, 90.0),
        (Knob::Volume, 0.0),
        (Knob::UiScale, 1.25),
    ] {
        g.slide(knob, v);
        g.frames(5);
    }
    assert_eq!(g.res::<crate::settings::Display>().ui_scale, 1.25);
}

/// The options at the room list: sliders, a key rebound and the keys reset.
#[test]
fn settings_at_the_room_list() {
    let mut g = Game::new(&[]);
    on_room_list(&mut g);
    g.press(5.0, "Настройки", |a| {
        matches!(a, Action::HomeTab(HomeTab::Settings))
    });
    sliders(&mut g);
    g.press(2.0, "Клавиши", |a| matches!(a, Action::Fold(Fold::Keys)));
    g.press(2.0, "Изменить: прыжок", |a| {
        matches!(a, Action::Rebind(Bind::Jump))
    });
    g.key(KeyCode::KeyK, Key::Character("k".into()), Some("k"));
    assert_eq!(g.res::<crate::settings::Bindings>().keys(Bind::Jump), [KeyCode::KeyK]);
    g.press(2.0, "Сбросить", |a| matches!(a, Action::ResetKeys));
    assert_ne!(g.res::<crate::settings::Bindings>().keys(Bind::Jump), [KeyCode::KeyK]);
    g.press(2.0, "Комнаты", |a| matches!(a, Action::HomeTab(HomeTab::Main)));
    on_room_list(&mut g);
}

/// A press on the menu's panel between its buttons is not one on the field: the menu stays open. One beside
/// the panel closes it.
#[test]
fn a_click_on_the_menu_panel_keeps_it_open() {
    let mut g = Game::new(&["--room", "dev"]);
    in_lobby(&mut g);
    g.frames(10);
    let swatches = g.rects(|a| matches!(a, Action::Color(_)));
    let last = swatches
        .iter()
        .max_by(|a, b| a.max.x.total_cmp(&b.max.x))
        .expect("colour swatches");
    let panel = g.rects(|a| matches!(a, Action::Resume))[0];
    let gap = Vec2::new((last.max.x + panel.max.x) / 2.0, last.center().y);
    assert!(
        gap.x - last.max.x > 4.0,
        "no room right of the swatches: {last:?} in {panel:?}"
    );
    g.click_at(gap);
    assert!(g.res::<Ui>().menu, "a click on the panel closed the menu");
    g.click_at(Vec2::new(panel.max.x + 200.0, panel.center().y));
    assert!(!g.res::<Ui>().menu, "a click beside the panel left the menu open");
}

/// A button pressed at its very top edge, the mouse still: pressing moves it down, and it still takes the click.
#[test]
fn a_button_takes_a_click_at_its_edge() {
    let mut g = Game::new(&["--room", "dev"]);
    in_lobby(&mut g);
    g.frames(10);
    let r = g.rects(|a| matches!(a, Action::Resume))[0];
    g.click_at(Vec2::new(r.center().x, r.min.y + 0.5));
    assert!(!g.res::<Ui>().menu, "«Продолжить» pressed at its top edge did nothing");
}

/// Pressed, a button's face moves down and its text with it: a click at any height on it still counts, the
/// text sliding under the pointer or not.
#[test]
fn a_button_takes_a_click_anywhere_on_it() {
    let mut g = Game::new(&["--room", "dev"]);
    in_lobby(&mut g);
    g.frames(10);
    let r = g.rects(|a| matches!(a, Action::Resume))[0];
    let mut y = r.min.y + 0.5;
    while y < r.max.y {
        g.click_at(Vec2::new(r.center().x, y));
        assert!(
            !g.res::<Ui>().menu,
            "«Продолжить» pressed at y {y} of {r:?} did nothing"
        );
        g.escape();
        g.frames(3);
        assert!(g.res::<Ui>().menu, "Esc did not open the menu again");
        y += 1.0;
    }
}

/// Every option of the menu mid-round on T2, each graphics preset, upscaler and toggle applied on the fly.
#[test]
fn every_setting_mid_round() {
    let mut g = Game::new(&["--room", "dev", "--autopilot"]);
    in_lobby(&mut g);
    into_round(&mut g, MapId::PortalPanic);
    menu(&mut g);
    g.press(2.0, "Настройки", |a| {
        matches!(a, Action::MenuTab(MenuTab::Settings))
    });
    g.press(2.0, "Графика", |a| matches!(a, Action::Fold(Fold::Gfx)));
    let n = press_every(&mut g, |a| matches!(a, Action::Set(_) | Action::Gfx(_)));
    assert!(n >= 14, "only {n} options on screen");
    // Back through the presets, each with a few frames of the round behind the menu.
    for p in [Preset::Low, Preset::High] {
        g.press(
            2.0,
            crate::ui::text::preset(p),
            |a| matches!(a, Action::Gfx(crate::ui::GfxPick::Preset(x)) if *x == p),
        );
        g.frames(30);
    }
    sliders(&mut g);
    g.escape();
    g.frames(120);
}

/// One round of `map` with bots: the intro, a few seconds of play by the autopilot, time warped ahead (the
/// map's moving parts, falls, events), the end of the round and the podium.
fn play_round(map: MapId) {
    play_round_on(wgpu::DeviceType::DiscreteGpu, map);
}

fn play_round_on(gpu: wgpu::DeviceType, map: MapId) {
    let mut g = Game::on(gpu, &["--room", "dev", "--autopilot"]);
    in_lobby(&mut g);
    g.dev(DevCmd::Start {
        games: vec![map],
        rounds: Some(1),
        bots: Some(3),
    });
    g.until_arena(20.0, "the round", |a| a.kind == ArenaKind::Round && a.game == map);
    g.frames(60);
    g.dev(DevCmd::SkipIntro);
    g.frames(240);
    // (A survival round may end by itself in the warp: then `EndRound` finds none, as the server says.)
    for _ in 0..3 {
        g.dev(DevCmd::Warp { s: 10.0 });
        g.frames(60);
    }
    g.dev(DevCmd::EndRound);
    g.until_arena(30.0, "the podium", |a| a.kind == ArenaKind::Podium);
    g.frames(120);
}

/// Tier T0 (a software device): the Low preset's own paths.
#[test]
fn round_on_the_lowest_tier() {
    play_round_on(wgpu::DeviceType::Cpu, MapId::PortalPanic);
}

macro_rules! rounds {
    ($($test:ident: $map:ident,)*) => {
        $(
            #[test]
            fn $test() {
                play_round(MapId::$map);
            }
        )*

        #[test]
        fn every_map_has_a_round_test() {
            let tested = [$(MapId::$map),*];
            for m in fb_maps::GAMES {
                assert!(tested.contains(&m.meta().id), "no round test for {}", m.meta().id);
            }
        }
    };
}

rounds! {
    round_door_dash: DoorDash,
    round_hammer_swing: HammerSwing,
    round_ball_hill: BallHill,
    round_hidden_bridge: HiddenBridge,
    round_drum_roll: DrumRoll,
    round_jump_club: JumpClub,
    round_roll_out: RollOut,
    round_wall_rush: WallRush,
    round_tail_tag: TailTag,
    round_hex_a_gone: HexAGone,
    round_crown_peak: CrownPeak,
    round_plate_drop: PlateDrop,
    round_portal_panic: PortalPanic,
    round_bounce_park: BouncePark,
    round_cliff_climb: CliffClimb,
    round_frost_sky: FrostSky,
    round_star_fall: StarFall,
}

/// A tester's run on a real server: through a round's intro the own bean stood where it was in the lobby, and
/// the server's spawn point came only as a respawn of 4–20 m some ticks after the start. Here, in the intro,
/// the predicted bean must already be where the server has it.
#[test]
fn the_own_bean_is_at_its_spawn_in_the_intro() {
    use bevy::prelude::*;
    use fb_net::{BeanId, BodyFull};
    use lightyear::prelude::Predicted;

    // (A real network's delay: on the bench's loopback the server's spawn came back before anything moved.)
    let mut g = Game::new(&["--room", "dev", "--lag", "40", "--jitter", "5"]);
    in_lobby(&mut g);
    // Somewhere in the lobby that is no round's spawn.
    g.frames(30);
    g.dev(DevCmd::Start {
        games: vec![MapId::DoorDash],
        rounds: None,
        bots: Some(3),
    });
    g.until_arena(20.0, "the round", |a| {
        a.kind == ArenaKind::Round && a.game == MapId::DoorDash
    });
    g.frames(90);
    let me = g.res::<Session>().me.expect("in a room");
    let mine = |w: &mut World| {
        let mut q = w.query_filtered::<(&BeanId, &BodyFull), With<Predicted>>();
        q.iter(w).find(|(p, _)| p.0 == me).map(|(_, f)| f.body.pos)
    };
    let client = mine(g.client().world_mut()).expect("the own bean is predicted");
    let mut q = g.server.world_mut().query::<(&BeanId, &BodyFull)>();
    let server: Vec<_> = q
        .iter(g.server.world())
        .filter(|(p, _)| p.0 == me)
        .map(|(_, f)| f.body.pos)
        .collect();
    assert!(
        server.iter().any(|s| (*s - client).length() < 0.5),
        "the client has its bean at {client:?}, the server at {server:?}"
    );
}

/// Every stress client's one unexplained divergence: a client that joins as a round starts has its clock set in
/// the intro, and Lightyear checks no server tick before the first one it predicted. The own bean stood where it
/// was in the lobby until the server's tick came that far (in an intro where nothing moves, until the start). It
/// must be where the server has it from the first ticks after the clock is set.
#[test]
fn the_own_bean_is_at_its_spawn_once_the_clock_is_set() {
    use bevy::prelude::*;
    use fb_net::{BeanId, BodyFull};
    use lightyear::prelude::{Predicted, PredictionHistory};

    #[rustfmt::skip]
    let mut g = Game::new(&[
        "--room", "dev", "--lag", "40", "--jitter", "5", "--start", "door-dash", "--start-players", "1",
    ]);
    let set = |w: &mut World| {
        let mut q = w.query_filtered::<&PredictionHistory<BodyFull>, With<Predicted>>();
        q.iter(w).any(|h| !h.is_empty())
    };
    g.until(20.0, "the clock is set", set);
    let in_round = g
        .client()
        .world()
        .get_resource::<crate::game::Map>()
        .is_some_and(|m| m.round.kind == ArenaKind::Round);
    assert!(
        in_round,
        "the round began only after the clock was set: nothing to check"
    );
    let me = g.res::<Session>().me.expect("in a room");
    let mut off = Vec::new();
    for _ in 0..60 {
        g.step();
        let w = g.client().world_mut();
        let mut q = w.query_filtered::<(&BeanId, &BodyFull), With<Predicted>>();
        let client = q.iter(w).find(|(p, _)| p.0 == me).map(|(_, f)| f.body.pos);
        let mut q = g.server.world_mut().query::<(&BeanId, &BodyFull)>();
        let server = q
            .iter(g.server.world())
            .find(|(p, _)| p.0 == me)
            .map(|(_, f)| f.body.pos);
        if let (Some(c), Some(s)) = (client, server)
            && (c - s).length() > 0.5
        {
            off.push((c, s));
        }
    }
    assert!(
        off.len() <= 3,
        "{} of 60 frames the client had its bean elsewhere (client, server): {:?}",
        off.len(),
        off.first()
    );
}
