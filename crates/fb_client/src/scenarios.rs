//! A player's paths through the client (`harness`): every crash found in play gets its path here first.
use fb_arena::ArenaKind;
use fb_net::ClientMsg;
use fb_proto::{DenyReason, DevCmd, Phase};

use bevy::input::keyboard::Key;
use bevy::prelude::{KeyCode, Vec2, World};

use crate::harness::Game;
use crate::keys::Bind;
use crate::session::Session;
use crate::ui::{Action, Field, HomeTab, Knob, MenuTab, Ui, UiAction};

/// In the dev room's lobby, where the menu opens by itself on entry.
fn in_lobby(g: &mut Game) {
    g.until(15.0, "in the dev room's lobby", |w| {
        let s = w.resource::<Session>();
        s.room.is_some() && s.arena.is_some() && w.resource::<Ui>().menu
    });
}

fn on_room_list(g: &mut Game) {
    g.until(10.0, "the room list", |w| {
        let s = w.resource::<Session>();
        s.rooms.is_some() && s.room.is_none()
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

/// The menu, opened with Esc if it is closed.
fn menu(g: &mut Game) {
    if !g.res::<Ui>().menu {
        g.escape();
    }
    assert!(g.res::<Ui>().menu, "Esc opens the menu");
}

/// A game of one round of `map` begun with the dev command, past its intro.
fn into_round(g: &mut Game, map: &str) {
    g.dev(DevCmd::Start {
        games: vec![map.into()],
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
    g.until(10.0, "the room list", |w| w.resource::<Session>().rooms.is_some());
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
    into_round(&mut g, "door-dash");
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
    into_round(&mut g, "hammer-swing");
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
    into_round(&mut g, "door-dash");

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
        let s = w.resource::<Session>();
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
        w.resource::<Session>()
            .denied
            .as_ref()
            .is_some_and(|d| !d.msg.is_empty())
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
    into_round(&mut g, "hammer-swing");

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
    into_round(&mut g, "door-dash");
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
    into_round(&mut g, "door-dash");
    menu(&mut g);
    g.press(2.0, "Настройки", |a| {
        matches!(a, Action::MenuTab(MenuTab::Settings))
    });
    g.press(2.0, "Клавиши", |a| matches!(a, Action::Fold("keys")));
    let jump = |g: &Game| g.res::<crate::settings::Bindings>().keys(Bind::Jump);
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
    assert!(g.res::<Ui>().server_bad, "a wrong address is told");
    g.erase(Field::Server);
    g.type_text("127.0.0.1:1");
    g.enter();
    assert!(!g.res::<Ui>().server_bad);
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
    let practice = |game: &'static str| move |a: &Action| matches!(a, Action::Practice(id) if *id == game);
    let in_practice = |w: &mut World| {
        let s = w.resource::<Session>();
        s.practice && s.arena.as_ref().is_some_and(|a| a.kind == ArenaKind::Round)
    };
    g.press(5.0, "Тренировка", |a| matches!(a, Action::Fold("practice")));
    g.press(2.0, "Тренировка: прыжки", practice("jump-club"));
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
    assert!(g.res::<Ui>().open.contains("practice"));
    g.press(2.0, "Тренировка: двери", practice("door-dash"));
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
    into_round(&mut g, "hammer-swing");
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
    g.press(2.0, "Клавиши", |a| matches!(a, Action::Fold("keys")));
    g.press(2.0, "Изменить: прыжок", |a| {
        matches!(a, Action::Rebind(Bind::Jump))
    });
    g.key(KeyCode::KeyK, Key::Character("k".into()), Some("k"));
    assert_eq!(g.res::<crate::settings::Bindings>().keys(Bind::Jump), [KeyCode::KeyK]);
    g.press(2.0, "Сбросить", |a| matches!(a, Action::ResetKeys));
    assert_ne!(g.res::<crate::settings::Bindings>().keys(Bind::Jump), [KeyCode::KeyK]);
    g.press(2.0, "Комнаты", |a| matches!(a, Action::HomeTab(HomeTab::Rooms)));
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
    into_round(&mut g, "portal-panic");
    menu(&mut g);
    g.press(2.0, "Настройки", |a| {
        matches!(a, Action::MenuTab(MenuTab::Settings))
    });
    g.press(2.0, "Графика", |a| matches!(a, Action::Fold("gfx")));
    let n = press_every(&mut g, |a| matches!(a, Action::Set(_) | Action::Gfx(_)));
    assert!(n >= 20, "only {n} options on screen");
    // Back through the presets, each with a few frames of the round behind the menu.
    for p in ["low", "medium", "high", "auto"] {
        g.press(
            2.0,
            p,
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
fn play_round(map: &str) {
    play_round_on(wgpu::DeviceType::DiscreteGpu, map);
}

fn play_round_on(gpu: wgpu::DeviceType, map: &str) {
    let mut g = Game::on(gpu, &["--room", "dev", "--autopilot"]);
    in_lobby(&mut g);
    g.dev(DevCmd::Start {
        games: vec![map.into()],
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

/// Tier T0 (OpenGL or a software device): the Low preset's own paths.
#[test]
fn round_on_the_lowest_tier() {
    play_round_on(wgpu::DeviceType::Cpu, "portal-panic");
}

macro_rules! rounds {
    ($($test:ident: $map:literal,)*) => {
        $(
            #[test]
            fn $test() {
                play_round($map);
            }
        )*

        #[test]
        fn every_map_has_a_round_test() {
            let tested = [$($map),*];
            for m in fb_maps::GAMES {
                assert!(tested.contains(&m.meta().id), "no round test for {}", m.meta().id);
            }
        }
    };
}

rounds! {
    round_door_dash: "door-dash",
    round_hammer_swing: "hammer-swing",
    round_ball_hill: "ball-hill",
    round_hidden_bridge: "hidden-bridge",
    round_drum_roll: "drum-roll",
    round_jump_club: "jump-club",
    round_roll_out: "roll-out",
    round_wall_rush: "wall-rush",
    round_tail_tag: "tail-tag",
    round_hex_a_gone: "hex-a-gone",
    round_crown_peak: "crown-peak",
    round_plate_drop: "plate-drop",
    round_portal_panic: "portal-panic",
    round_bounce_park: "bounce-park",
    round_cliff_climb: "cliff-climb",
    round_frost_sky: "frost-sky",
    round_star_fall: "star-fall",
}
