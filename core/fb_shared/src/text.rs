//! Player-typed text as the server passes it on.
use crate::{CHAT_MAX, NAME_MAX, ROOM_TITLE_MAX};

/// Unicode category C as far as it matters here: controls, format characters (zero-width, bidi
/// overrides, tags) and private use. Unassigned code points pass.
pub fn is_other(c: char) -> bool {
    let u = c as u32;
    c.is_control()
        || matches!(
            u,
            0xad | 0x600..=0x605
                | 0x61c
                | 0x6dd
                | 0x70f
                | 0x890..=0x891
                | 0x8e2
                | 0x180e
                | 0x200b..=0x200f
                | 0x202a..=0x202e
                | 0x2060..=0x2064
                | 0x2066..=0x206f
                | 0xfeff
                | 0xfff9..=0xfffb
                | 0x110bd
                | 0x110cd
                | 0x13430..=0x1343f
                | 0x1bca0..=0x1bca3
                | 0x1d173..=0x1d17a
                | 0xe0001
                | 0xe0020..=0xe007f
                | 0xe000..=0xf8ff
                | 0xf0000..=0x10ffff
        )
}

/// Drops `is_other` characters (control characters become `control`), but keeps a zero-width joiner
/// between two symbols: it is what holds emoji sequences like 👨‍👩‍👧 together.
fn visible(raw: &str, control: Option<char>) -> String {
    let symbol = |c: Option<&char>| c.is_some_and(|&c| !c.is_alphanumeric() && !c.is_whitespace() && !is_other(c));
    let mut out = String::with_capacity(raw.len());
    let mut chars = raw.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\u{200d}' && symbol(out.chars().next_back().as_ref()) && symbol(chars.peek()) {
            out.push(c);
        } else if c.is_control() {
            out.extend(control);
        } else if !is_other(c) {
            out.push(c);
        }
    }
    out
}

fn collapse_spaces(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

pub fn sanitize_name(raw: &str) -> String {
    let s: String = visible(raw, None).chars().filter(|&c| c != '<' && c != '>').collect();
    s.trim()
        .chars()
        .take(NAME_MAX)
        .collect::<String>()
        .trim_end()
        .to_string()
}

pub fn sanitize_title(raw: &str) -> String {
    let s: String = visible(raw, None).chars().filter(|&c| c != '<' && c != '>').collect();
    collapse_spaces(&s)
        .chars()
        .take(ROOM_TITLE_MAX)
        .collect::<String>()
        .trim_end()
        .to_string()
}

/// One line, no control or invisible characters (bidi overrides, zero-width), at most CHAT_MAX characters.
pub fn sanitize_chat(raw: &str) -> String {
    let s = visible(raw, Some(' '));
    collapse_spaces(&s).chars().take(CHAT_MAX).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitizes() {
        assert_eq!(sanitize_name("  <Аня>\u{200b}\u{7} "), "Аня");
        assert_eq!(sanitize_name("Очень длинное имя игрока"), "Очень длинное им");
        assert_eq!(sanitize_title(" Наша \n\t комната "), "Наша комната");
        assert_eq!(sanitize_chat("привет\n\nвсем  \u{1}!"), "привет всем !");
        assert_eq!(sanitize_chat(&"я".repeat(500)).chars().count(), CHAT_MAX);
        assert_eq!(sanitize_chat("  \n "), "");
        assert_eq!(sanitize_chat("a\u{202e}b\u{200b}c\u{2066}d"), "abcd");
        let family = "\u{1f468}\u{200d}\u{1f469}\u{200d}\u{1f467}";
        assert_eq!(sanitize_chat(family), family);
        assert_eq!(sanitize_name(&format!("Аня{family}")), format!("Аня{family}"));
        assert_eq!(sanitize_name("Bo\u{200d}b"), "Bob");
    }
}
