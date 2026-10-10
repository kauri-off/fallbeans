//! Key bindings: which physical keys move, jump, dive and grab (kept in the settings, changed in the
//! options). Keys are physical codes, so a Russian layout plays the same as a Latin one.
use bevy::prelude::*;
use bevy::reflect::enums::Enum;
use serde::de::Error as _;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Bind {
    Forward,
    Back,
    Left,
    Right,
    Jump,
    Dive,
    Grab,
}

pub const BINDS: [Bind; 7] = [
    Bind::Forward,
    Bind::Back,
    Bind::Left,
    Bind::Right,
    Bind::Jump,
    Bind::Dive,
    Bind::Grab,
];

impl Bind {
    pub fn defaults(self) -> &'static [KeyCode] {
        match self {
            Bind::Forward => &[KeyCode::KeyW, KeyCode::ArrowUp],
            Bind::Back => &[KeyCode::KeyS, KeyCode::ArrowDown],
            Bind::Left => &[KeyCode::KeyA, KeyCode::ArrowLeft],
            Bind::Right => &[KeyCode::KeyD, KeyCode::ArrowRight],
            Bind::Jump => &[KeyCode::Space],
            Bind::Dive => &[
                KeyCode::KeyE,
                KeyCode::ShiftLeft,
                KeyCode::ShiftRight,
                KeyCode::ControlLeft,
            ],
            Bind::Grab => &[KeyCode::KeyQ],
        }
    }
}

/// Keys the game itself answers to, never bound (a jump on F8 would write a report with every jump): the
/// F-keys (F3, F4, F8, F9 and the ones to come), the emotes 1–5 (`game::EMOTE_KEYS`), Esc, Enter (the chat).
pub const RESERVED: &[KeyCode] = &[
    KeyCode::F1,
    KeyCode::F2,
    KeyCode::F3,
    KeyCode::F4,
    KeyCode::F5,
    KeyCode::F6,
    KeyCode::F7,
    KeyCode::F8,
    KeyCode::F9,
    KeyCode::F10,
    KeyCode::F11,
    KeyCode::F12,
    KeyCode::Digit1,
    KeyCode::Digit2,
    KeyCode::Digit3,
    KeyCode::Digit4,
    KeyCode::Digit5,
    KeyCode::Escape,
    KeyCode::Enter,
    KeyCode::NumpadEnter,
];

/// Keys that may be bound and what the player reads. None of `RESERVED`: a file naming one gets the action's
/// defaults.
const TABLE: &[(KeyCode, &str)] = &[
    (KeyCode::KeyA, "A"),
    (KeyCode::KeyB, "B"),
    (KeyCode::KeyC, "C"),
    (KeyCode::KeyD, "D"),
    (KeyCode::KeyE, "E"),
    (KeyCode::KeyF, "F"),
    (KeyCode::KeyG, "G"),
    (KeyCode::KeyH, "H"),
    (KeyCode::KeyI, "I"),
    (KeyCode::KeyJ, "J"),
    (KeyCode::KeyK, "K"),
    (KeyCode::KeyL, "L"),
    (KeyCode::KeyM, "M"),
    (KeyCode::KeyN, "N"),
    (KeyCode::KeyO, "O"),
    (KeyCode::KeyP, "P"),
    (KeyCode::KeyQ, "Q"),
    (KeyCode::KeyR, "R"),
    (KeyCode::KeyS, "S"),
    (KeyCode::KeyT, "T"),
    (KeyCode::KeyU, "U"),
    (KeyCode::KeyV, "V"),
    (KeyCode::KeyW, "W"),
    (KeyCode::KeyX, "X"),
    (KeyCode::KeyY, "Y"),
    (KeyCode::KeyZ, "Z"),
    (KeyCode::Digit0, "0"),
    (KeyCode::Digit6, "6"),
    (KeyCode::Digit7, "7"),
    (KeyCode::Digit8, "8"),
    (KeyCode::Digit9, "9"),
    (KeyCode::Space, "Пробел"),
    (KeyCode::ShiftLeft, "Shift"),
    (KeyCode::ShiftRight, "Правый Shift"),
    (KeyCode::ControlLeft, "Ctrl"),
    (KeyCode::ControlRight, "Правый Ctrl"),
    (KeyCode::AltLeft, "Alt"),
    (KeyCode::AltRight, "Правый Alt"),
    (KeyCode::Tab, "Tab"),
    (KeyCode::CapsLock, "Caps Lock"),
    (KeyCode::Backquote, "`"),
    (KeyCode::Minus, "-"),
    (KeyCode::Equal, "="),
    (KeyCode::BracketLeft, "["),
    (KeyCode::BracketRight, "]"),
    (KeyCode::Backslash, "\\"),
    (KeyCode::Semicolon, ";"),
    (KeyCode::Quote, "'"),
    (KeyCode::Comma, ","),
    (KeyCode::Period, "."),
    (KeyCode::Slash, "/"),
    (KeyCode::ArrowUp, "Стрелка вверх"),
    (KeyCode::ArrowDown, "Стрелка вниз"),
    (KeyCode::ArrowLeft, "Стрелка влево"),
    (KeyCode::ArrowRight, "Стрелка вправо"),
    (KeyCode::Backspace, "Backspace"),
    (KeyCode::Insert, "Insert"),
    (KeyCode::Delete, "Delete"),
    (KeyCode::Home, "Home"),
    (KeyCode::End, "End"),
    (KeyCode::PageUp, "Page Up"),
    (KeyCode::PageDown, "Page Down"),
    (KeyCode::Numpad0, "Num 0"),
    (KeyCode::Numpad1, "Num 1"),
    (KeyCode::Numpad2, "Num 2"),
    (KeyCode::Numpad3, "Num 3"),
    (KeyCode::Numpad4, "Num 4"),
    (KeyCode::Numpad5, "Num 5"),
    (KeyCode::Numpad6, "Num 6"),
    (KeyCode::Numpad7, "Num 7"),
    (KeyCode::Numpad8, "Num 8"),
    (KeyCode::Numpad9, "Num 9"),
];

/// A bindable key by its name in the settings file (`KeyW`, the variant's).
fn from_name(n: &str) -> Option<KeyCode> {
    TABLE.iter().find(|t| t.0.variant_name() == n).map(|t| t.0)
}

/// A key as the player reads it (None: it cannot be bound).
pub fn label(k: KeyCode) -> Option<&'static str> {
    TABLE.iter().find(|t| t.0 == k).map(|t| t.1)
}

/// A key an action may take (the next one pressed while rebinding: others are let pass).
pub fn bindable(k: KeyCode) -> bool {
    !RESERVED.contains(&k) && label(k).is_some()
}

/// An action's keys as the player reads them, one keycap each («—»: none).
pub fn each_label(keys: &[KeyCode]) -> Vec<&'static str> {
    let v: Vec<_> = keys.iter().filter_map(|k| label(*k)).collect();
    if v.is_empty() { vec!["—"] } else { v }
}

/// An action's keys, saved as a list of names; an older file's "KeyW ArrowUp" (or "none") loads too. A file
/// that names none that can be bound does not load: bevy-settings keeps the action's defaults.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash, Reflect)]
#[reflect(opaque)]
#[reflect(Serialize, Deserialize, Clone, PartialEq, Debug, Default)]
pub struct Keys(pub Vec<KeyCode>);

impl Serialize for Keys {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.collect_seq(self.0.iter().map(|k| k.variant_name()))
    }
}

impl<'de> Deserialize<'de> for Keys {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Saved {
            List(Vec<String>),
            Old(String),
        }
        let names = match Saved::deserialize(d)? {
            Saved::Old(s) if s.trim() == "none" => return Ok(Keys::default()),
            Saved::Old(s) => s.split_whitespace().map(str::to_string).collect(),
            Saved::List(names) if names.is_empty() => return Ok(Keys::default()),
            Saved::List(names) => names,
        };
        let keys: Vec<KeyCode> = names.iter().filter_map(|n| from_name(n)).collect();
        if keys.is_empty() {
            return Err(D::Error::custom("no key that can be bound"));
        }
        Ok(Keys(keys))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn load(v: toml::Value) -> Option<Vec<KeyCode>> {
        Keys::deserialize(v).ok().map(|k| k.0)
    }

    #[test]
    fn keys_load_back() {
        for b in BINDS {
            let saved = toml::Value::try_from(Keys(b.defaults().to_vec())).unwrap();
            assert_eq!(load(saved).as_deref(), Some(b.defaults()), "{b:?}");
        }
        let old = |s: &str| load(toml::Value::String(s.into()));
        assert_eq!(old("KeyW ArrowUp").as_deref(), Some(Bind::Forward.defaults()));
        assert_eq!(old("none"), Some(Vec::new()));
        assert_eq!(old(""), None);
        assert_eq!(load(toml::Value::Array(Vec::new())), Some(Vec::new()));
    }

    #[test]
    fn reserved_keys_cannot_be_bound() {
        for k in RESERVED {
            assert!(label(*k).is_none(), "{k:?}");
        }
        assert_eq!(load(toml::Value::String("F8 Digit1".into())), None);
        for b in BINDS {
            assert!(b.defaults().iter().all(|k| !RESERVED.contains(k)), "{b:?}");
        }
    }
}
