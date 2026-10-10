//! The room's text chat, bottom left. Normally invisible; a new line shows it for a
//! few seconds; Enter opens the line to type in (Enter again sends, Esc closes).
use bevy::input_focus::InputFocus;
use bevy::prelude::*;
use bevy::text::EditableText;
use fb_proto::ClientMsg;
use fb_shared::CHAT_MAX;
use lightyear::prelude::client::Client;
use lightyear::prelude::*;

use super::*;
use crate::session::{ChatLine, ChatLog, Session};

/// How long the chat stays on screen after a new line.
const SHOW_S: f32 = 8.0;
/// Lines shown while the chat is not open.
const FRESH_LINES: usize = 6;

pub struct ChatPlugin;

impl Plugin for ChatPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, build_chat.after(super::setup));
        // (After the menu has seen the frame's Esc: the one that closes the chat must not open the menu too.)
        app.add_systems(
            Update,
            (open_close, lines.run_if(resource_changed::<ChatLog>), shown)
                .chain()
                .after(super::menu::menu_flow),
        );
    }
}

#[derive(Component)]
struct ChatBox;
#[derive(Component)]
struct LogBox;
/// A line of the chat: its number and time.
#[derive(Component)]
struct ChatRow(u32, f32);
#[derive(Component)]
struct ChatInput;

fn build_chat(mut commands: Commands, layers: Res<Layers>, f: Res<Fonts>) {
    let e = layers[Layer::Chat];
    let f = &*f;
    commands.entity(e).with_children(|l| {
        l.spawn((
            ChatBox,
            Node {
                position_type: PositionType::Absolute,
                left: px(24),
                bottom: px(24),
                width: px(480),
                max_width: percent(40),
                flex_direction: FlexDirection::Column,
                border_radius: BorderRadius::all(px(18)),
                ..default()
            },
            BackgroundColor(Color::NONE),
            Pickable::IGNORE,
        ))
        .with_children(|c| {
            c.spawn((
                LogBox,
                Node {
                    flex_direction: FlexDirection::Column,
                    align_items: AlignItems::FlexStart,
                    row_gap: px(4),
                    max_height: px(280),
                    overflow: Overflow::scroll_y(),
                    justify_content: JustifyContent::FlexEnd,
                    ..default()
                },
                Pickable::IGNORE,
            ));
            c.spawn((
                ChatInput,
                Node {
                    margin: UiRect::new(px(8), px(8), px(6), px(8)),
                    display: bevy::ui::Display::None,
                    ..default()
                },
            ))
            .with_children(|i| {
                field_with_hint(i, f, Field::Chat, "", text::CHAT_PLACEHOLDER, CHAT_MAX, None);
            });
        });
    });
}

/// Enter opens the line (in a room, not in practice, with nothing else taking the keyboard); Enter sends it,
/// Esc closes it, and so does a click elsewhere (the field loses focus).
fn open_close(
    keys: Res<ButtonInput<KeyCode>>,
    mut ui: ResMut<Ui>,
    session: Res<Session>,
    mut focus: ResMut<InputFocus>,
    mut fields: Query<(Entity, &Field, &mut EditableText)>,
    mut input: Query<&mut Node, With<ChatInput>>,
    mut senders: Query<&mut MessageSender<ClientMsg>, With<Client>>,
) {
    let Some((field, _, mut t)) = fields.iter_mut().find(|(_, f, _)| **f == Field::Chat) else {
        return;
    };
    let can = session.room.is_some() && !session.practice && session.arena.is_some();
    let enter = keys.any_just_pressed([KeyCode::Enter, KeyCode::NumpadEnter]);
    if ui.chat {
        let lost = focus.get() != Some(field);
        if !can || keys.just_pressed(KeyCode::Escape) || lost {
            ui.chat = false;
            super::set_field_text(&mut t, "");
        } else if enter && !t.is_composing() {
            let line = fb_shared::text::sanitize_chat(&t.value().to_string());
            if !line.is_empty() {
                crate::session::send(&mut senders, ClientMsg::Chat(line));
            }
            super::set_field_text(&mut t, "");
            ui.chat = false;
        }
        if !ui.chat && focus.get() == Some(field) {
            focus.clear();
        }
    } else if can && enter && !ui.menu && !focus.get().is_some_and(|e| fields.contains(e)) {
        ui.chat = true;
        focus.set(field, bevy::input_focus::FocusCause::Navigated);
    }
    if let Ok(mut n) = input.single_mut() {
        show(&mut n, ui.chat);
    }
}

/// New lines are added, lines gone from the log taken away.
fn lines(
    q: Single<Entity, With<LogBox>>,
    log: Res<ChatLog>,
    rows: Query<(Entity, &ChatRow)>,
    session: Res<Session>,
    f: Res<Fonts>,
    mut commands: Commands,
) {
    let mut last = None;
    for (e, r) in &rows {
        if log.0.iter().any(|l| l.n == r.0 && l.at == r.1) {
            last = last.max(Some(r.0));
        } else {
            commands.entity(e).despawn();
        }
    }
    let f = &*f;
    commands.entity(*q).with_children(|p| {
        for l in log.0.iter().filter(|l| last.is_none_or(|n| l.n > n)) {
            line(p, f, &session, l);
        }
    });
}

fn line(p: &mut ChildSpawnerCommands, f: &Fonts, session: &Session, l: &ChatLine) {
    let color = l.id.and_then(|id| session.player(id)).map_or(MUTED, |p| suit(p.color));
    // (A suit too light to read on the light panel goes darker.)
    let name_ink = color.mix(&INK, 0.45);
    p.spawn((
        ChatRow(l.n, l.at),
        Node {
            padding: UiRect::axes(px(12), px(6)),
            border_radius: BorderRadius::all(px(12)),
            ..default()
        },
        BackgroundColor(Color::NONE),
        Text::default(),
        TextLayout::default(),
        Pickable::IGNORE,
    ))
    .with_children(|t| {
        // (A name may hold emoji too: the Black font has none.)
        for (run, emoji) in runs(&format!("{}: ", l.name)) {
            t.spawn((
                TextSpan::new(run),
                TextFont {
                    font: if emoji { f.emoji.clone() } else { f.strong.clone() }.into(),
                    font_size: FontSize::Px(15.0),
                    ..default()
                },
                TextColor(name_ink),
            ));
        }
        for (run, emoji) in runs(&l.text) {
            t.spawn((
                TextSpan::new(run),
                TextFont {
                    font: if emoji { f.emoji.clone() } else { f.body.clone() }.into(),
                    font_size: FontSize::Px(15.0),
                    ..default()
                },
                TextColor(if l.id.is_some() { INK } else { FAINT }),
            ));
        }
    });
}

type BoxLook = (Entity, &'static mut BackgroundColor, &'static mut Node);

/// Open, every line on a card with the field; for a while after a new one, the last few, each on its own; otherwise
/// none.
fn shown(
    ui: Res<Ui>,
    log: Res<ChatLog>,
    time: Res<Time<Real>>,
    mut q: Single<BoxLook, (With<ChatBox>, Without<ChatRow>)>,
    mut rows: Query<(&ChatRow, &mut Node, &mut BackgroundColor)>,
    mut commands: Commands,
) {
    let now = time.elapsed_secs();
    let fresh = log.0.back().is_some_and(|l| now - l.at < SHOW_S);
    let from = if ui.chat {
        Some(0)
    } else if fresh {
        log.0.get(log.0.len().saturating_sub(FRESH_LINES)).map(|l| l.n)
    } else {
        None
    };
    let (e, ref mut bg, ref mut node) = *q;
    bg.set_if_neq(BackgroundColor(if ui.chat { PANEL } else { Color::NONE }));
    let pad = if ui.chat {
        UiRect::new(px(2), px(2), px(6), px(0))
    } else {
        UiRect::ZERO
    };
    if node.padding != pad {
        node.padding = pad;
        if ui.chat {
            commands
                .entity(e)
                .insert(BoxShadow::new(SHADOW.with_alpha(0.1), px(0), px(2), px(0), px(10)));
        } else {
            commands.entity(e).remove::<BoxShadow>();
        }
    }
    let line_bg = if ui.chat {
        Color::NONE
    } else {
        Color::srgba(0.984, 0.973, 0.953, 0.95)
    };
    for (r, mut node, mut bg) in &mut rows {
        show(&mut node, from.is_some_and(|n| r.0 >= n));
        bg.set_if_neq(BackgroundColor(line_bg));
        let pad = if ui.chat {
            UiRect::axes(px(12), px(1))
        } else {
            UiRect::axes(px(12), px(6))
        };
        if node.padding != pad {
            node.padding = pad;
        }
    }
}
