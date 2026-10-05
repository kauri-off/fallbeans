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
use crate::session::Session;

/// How long the chat stays on screen after a new line.
const SHOW_S: f32 = 8.0;
/// Lines shown while the chat is not open.
const FRESH_LINES: usize = 6;

pub struct ChatPlugin;

impl Plugin for ChatPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, build_chat.after(super::setup));
        app.add_systems(Update, (open_close, log).chain());
    }
}

#[derive(Component)]
struct ChatLog;
#[derive(Component)]
struct ChatInput;

fn build_chat(mut commands: Commands, layers: Query<(Entity, &Layer)>, f: Res<Fonts>) {
    let Some((e, _)) = layers.iter().find(|(_, l)| **l == Layer::Chat) else {
        return;
    };
    let f = &*f;
    commands.entity(e).with_children(|l| {
        l.spawn((
            Node {
                position_type: PositionType::Absolute,
                left: rem(0.75),
                bottom: rem(3.2),
                width: rem(24.0),
                max_width: percent(40),
                flex_direction: FlexDirection::Column,
                row_gap: rem(0.375),
                ..default()
            },
            Pickable::IGNORE,
        ))
        .with_children(|c| {
            c.spawn((
                ChatLog,
                Section::default(),
                Node {
                    flex_direction: FlexDirection::Column,
                    row_gap: px(2),
                    padding: UiRect::axes(rem(0.625), rem(0.4)),
                    border_radius: BorderRadius::all(rem(0.75)),
                    max_height: rem(14.0),
                    overflow: Overflow::scroll_y(),
                    justify_content: JustifyContent::FlexEnd,
                    ..default()
                },
                BackgroundColor(Color::NONE),
                Pickable::IGNORE,
            ));
            c.spawn((
                ChatInput,
                Node {
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

fn log(
    mut q: Query<(Entity, &mut Section, &mut BackgroundColor), With<ChatLog>>,
    ui: Res<Ui>,
    session: Res<Session>,
    time: Res<Time<Real>>,
    f: Res<Fonts>,
    mut commands: Commands,
) {
    let Ok((e, mut sec, mut bg)) = q.single_mut() else {
        return;
    };
    let now = time.elapsed_secs();
    let fresh = session.chat.last().is_some_and(|l| now - l.at < SHOW_S);
    let shown: Vec<_> = if ui.chat {
        session.chat.iter().collect()
    } else if fresh {
        let n = session.chat.len().saturating_sub(FRESH_LINES);
        session.chat[n..].iter().collect()
    } else {
        Vec::new()
    };
    let alpha = if ui.chat { 0.8 } else { 0.45 };
    bg.set_if_neq(BackgroundColor(if shown.is_empty() {
        Color::NONE
    } else {
        INK.with_alpha(alpha * 0.8)
    }));
    let ns: Vec<u32> = shown.iter().map(|l| l.n).collect();
    if !sec.stale(key_of(&(ns, ui.chat))) {
        return;
    }
    let f = &*f;
    rebuild(&mut commands, e, |p| {
        for l in shown {
            let color = session.player(l.id).map_or(Color::WHITE, |p| super::suit(p.color));
            let name_ink = color.mix(&Color::WHITE, 0.5);
            let mut line = p.spawn((Text::default(), TextLayout::default(), Pickable::IGNORE));
            line.with_children(|t| {
                t.spawn((
                    TextSpan::new(format!("{}: ", l.name)),
                    TextFont {
                        font: f.black.clone().into(),
                        font_size: FontSize::Px(14.0),
                        ..default()
                    },
                    TextColor(name_ink),
                ));
                for (run, emoji) in runs(&l.text) {
                    t.spawn((
                        TextSpan::new(run),
                        TextFont {
                            font: if emoji { f.emoji.clone() } else { f.bold.clone() }.into(),
                            font_size: FontSize::Px(14.0),
                            ..default()
                        },
                        TextColor(Color::WHITE),
                    ));
                }
            });
        }
    });
}
