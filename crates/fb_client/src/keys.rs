//! Key bindings: which physical keys move, jump, dive and grab (kept in the settings, changed in the
//! options). Keys are physical codes, so a Russian layout plays the same as a Latin one.
use bevy::prelude::*;

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
    pub fn defaults(self) -> &'static str {
        match self {
            Bind::Forward => "KeyW ArrowUp",
            Bind::Back => "KeyS ArrowDown",
            Bind::Left => "KeyA ArrowLeft",
            Bind::Right => "KeyD ArrowRight",
            Bind::Jump => "Space",
            Bind::Dive => "KeyE ShiftLeft ShiftRight ControlLeft",
            Bind::Grab => "KeyQ",
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

/// Keys that may be bound: the code, its name in the settings file, and what the player reads. None of
/// `RESERVED`: a file naming one gets the action's defaults.
const TABLE: &[(KeyCode, &str, &str)] = &[
    (KeyCode::KeyA, "KeyA", "A"),
    (KeyCode::KeyB, "KeyB", "B"),
    (KeyCode::KeyC, "KeyC", "C"),
    (KeyCode::KeyD, "KeyD", "D"),
    (KeyCode::KeyE, "KeyE", "E"),
    (KeyCode::KeyF, "KeyF", "F"),
    (KeyCode::KeyG, "KeyG", "G"),
    (KeyCode::KeyH, "KeyH", "H"),
    (KeyCode::KeyI, "KeyI", "I"),
    (KeyCode::KeyJ, "KeyJ", "J"),
    (KeyCode::KeyK, "KeyK", "K"),
    (KeyCode::KeyL, "KeyL", "L"),
    (KeyCode::KeyM, "KeyM", "M"),
    (KeyCode::KeyN, "KeyN", "N"),
    (KeyCode::KeyO, "KeyO", "O"),
    (KeyCode::KeyP, "KeyP", "P"),
    (KeyCode::KeyQ, "KeyQ", "Q"),
    (KeyCode::KeyR, "KeyR", "R"),
    (KeyCode::KeyS, "KeyS", "S"),
    (KeyCode::KeyT, "KeyT", "T"),
    (KeyCode::KeyU, "KeyU", "U"),
    (KeyCode::KeyV, "KeyV", "V"),
    (KeyCode::KeyW, "KeyW", "W"),
    (KeyCode::KeyX, "KeyX", "X"),
    (KeyCode::KeyY, "KeyY", "Y"),
    (KeyCode::KeyZ, "KeyZ", "Z"),
    (KeyCode::Digit0, "Digit0", "0"),
    (KeyCode::Digit6, "Digit6", "6"),
    (KeyCode::Digit7, "Digit7", "7"),
    (KeyCode::Digit8, "Digit8", "8"),
    (KeyCode::Digit9, "Digit9", "9"),
    (KeyCode::Space, "Space", "Пробел"),
    (KeyCode::ShiftLeft, "ShiftLeft", "Shift"),
    (KeyCode::ShiftRight, "ShiftRight", "Правый Shift"),
    (KeyCode::ControlLeft, "ControlLeft", "Ctrl"),
    (KeyCode::ControlRight, "ControlRight", "Правый Ctrl"),
    (KeyCode::AltLeft, "AltLeft", "Alt"),
    (KeyCode::AltRight, "AltRight", "Правый Alt"),
    (KeyCode::Tab, "Tab", "Tab"),
    (KeyCode::CapsLock, "CapsLock", "Caps Lock"),
    (KeyCode::Backquote, "Backquote", "`"),
    (KeyCode::Minus, "Minus", "-"),
    (KeyCode::Equal, "Equal", "="),
    (KeyCode::BracketLeft, "BracketLeft", "["),
    (KeyCode::BracketRight, "BracketRight", "]"),
    (KeyCode::Backslash, "Backslash", "\\"),
    (KeyCode::Semicolon, "Semicolon", ";"),
    (KeyCode::Quote, "Quote", "'"),
    (KeyCode::Comma, "Comma", ","),
    (KeyCode::Period, "Period", "."),
    (KeyCode::Slash, "Slash", "/"),
    (KeyCode::ArrowUp, "ArrowUp", "Стрелка вверх"),
    (KeyCode::ArrowDown, "ArrowDown", "Стрелка вниз"),
    (KeyCode::ArrowLeft, "ArrowLeft", "Стрелка влево"),
    (KeyCode::ArrowRight, "ArrowRight", "Стрелка вправо"),
    (KeyCode::Backspace, "Backspace", "Backspace"),
    (KeyCode::Insert, "Insert", "Insert"),
    (KeyCode::Delete, "Delete", "Delete"),
    (KeyCode::Home, "Home", "Home"),
    (KeyCode::End, "End", "End"),
    (KeyCode::PageUp, "PageUp", "Page Up"),
    (KeyCode::PageDown, "PageDown", "Page Down"),
    (KeyCode::Numpad0, "Numpad0", "Num 0"),
    (KeyCode::Numpad1, "Numpad1", "Num 1"),
    (KeyCode::Numpad2, "Numpad2", "Num 2"),
    (KeyCode::Numpad3, "Numpad3", "Num 3"),
    (KeyCode::Numpad4, "Numpad4", "Num 4"),
    (KeyCode::Numpad5, "Numpad5", "Num 5"),
    (KeyCode::Numpad6, "Numpad6", "Num 6"),
    (KeyCode::Numpad7, "Numpad7", "Num 7"),
    (KeyCode::Numpad8, "Numpad8", "Num 8"),
    (KeyCode::Numpad9, "Numpad9", "Num 9"),
];

pub fn parse(s: &str) -> Vec<KeyCode> {
    s.split_whitespace()
        .filter_map(|n| TABLE.iter().find(|t| t.1 == n).map(|t| t.0))
        .collect()
}

pub fn names(keys: &[KeyCode]) -> String {
    keys.iter()
        .filter_map(|k| TABLE.iter().find(|t| t.0 == *k).map(|t| t.1))
        .collect::<Vec<_>>()
        .join(" ")
}

/// A key as the player reads it (None: it cannot be bound).
pub fn label(k: KeyCode) -> Option<&'static str> {
    TABLE.iter().find(|t| t.0 == k).map(|t| t.2)
}

/// A key an action may take (the next one pressed while rebinding: others are let pass).
pub fn bindable(k: KeyCode) -> bool {
    !RESERVED.contains(&k) && label(k).is_some()
}

/// An action's keys as the player reads them («—»: none).
pub fn labels(keys: &[KeyCode]) -> String {
    let s = keys.iter().filter_map(|k| label(*k)).collect::<Vec<_>>().join(" / ");
    if s.is_empty() { "—".into() } else { s }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_parse_back() {
        for b in BINDS {
            let keys = parse(b.defaults());
            assert!(!keys.is_empty());
            assert_eq!(names(&keys), b.defaults());
        }
    }

    #[test]
    fn reserved_keys_cannot_be_bound() {
        for k in RESERVED {
            assert!(label(*k).is_none(), "{k:?}");
        }
        assert!(parse("F8 Digit1").is_empty());
        for b in BINDS {
            assert!(parse(b.defaults()).iter().all(|k| !RESERVED.contains(k)), "{b:?}");
        }
    }
}
