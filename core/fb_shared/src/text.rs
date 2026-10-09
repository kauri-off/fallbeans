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

/// Invisible characters that would make a name look empty.
fn is_filler(c: char) -> bool {
    matches!(
        c as u32,
        0x115f | 0x1160 | 0x3164 | 0xffa0 | 0x2800 | 0x034f | 0x17b4 | 0x17b5 | 0x180b..=0x180d | 0x180f
    )
}

/// Combining marks; variation selectors are excluded, since emoji need them.
fn is_mark(c: char) -> bool {
    matches!(
        c as u32,
        0x0300..=0x036f
            | 0x0483..=0x0489
            | 0x0591..=0x05bd
            | 0x05bf
            | 0x05c1..=0x05c2
            | 0x05c4..=0x05c5
            | 0x05c7
            | 0x0610..=0x061a
            | 0x064b..=0x065f
            | 0x0670
            | 0x06d6..=0x06dc
            | 0x06df..=0x06e4
            | 0x06e7..=0x06e8
            | 0x06ea..=0x06ed
            | 0x0e31
            | 0x0e34..=0x0e3a
            | 0x0e47..=0x0e4e
            | 0x0eb1
            | 0x0eb4..=0x0ebc
            | 0x0ec8..=0x0ece
            | 0x1ab0..=0x1aff
            | 0x1dc0..=0x1dff
            | 0x20d0..=0x20ff
            | 0xfe20..=0xfe2f
    )
}

/// Combining marks kept on one character (more stack into text taller than the line).
const MARKS_MAX: usize = 2;

/// Drops invisible characters and extra marks, but keeps the ZWJ that joins emoji sequences.
fn visible(raw: &str, control: Option<char>, drop: impl Fn(char) -> bool) -> String {
    let kept: String = raw
        .chars()
        .filter_map(|c| {
            if c == '\u{200d}' {
                Some(c)
            } else if c.is_control() {
                control
            } else if is_other(c) || is_filler(c) || drop(c) {
                None
            } else {
                Some(c)
            }
        })
        .collect();
    let mut out = String::with_capacity(kept.len());
    // Marks on the character last kept.
    let mut marks = 0;
    for c in joined(&kept).chars() {
        if is_mark(c) {
            if marks < MARKS_MAX {
                out.push(c);
                marks += 1;
            }
        } else {
            out.push(c);
            marks = 0;
        }
    }
    out
}

/// Drops zero-width joiners that do not stand between two symbols (not marks: a joiner must not stack them).
fn joined(s: &str) -> String {
    let symbol =
        |c: Option<char>| c.is_some_and(|c| !c.is_alphanumeric() && !c.is_whitespace() && !is_other(c) && !is_mark(c));
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '\u{200d}' || symbol(out.chars().next_back()) && symbol(chars.peek().copied()) {
            out.push(c);
        }
    }
    out
}

/// The first `max` characters, without a joiner left hanging at the cut or spaces after it.
fn cut(s: &str, max: usize) -> String {
    joined(&s.chars().take(max).collect::<String>()).trim_end().to_string()
}

fn collapse_spaces(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn angle(c: char) -> bool {
    c == '<' || c == '>'
}

pub fn sanitize_name(raw: &str) -> String {
    cut(visible(raw, None, angle).trim(), NAME_MAX)
}

/// A person's name (`sanitize_name`): empty when it would pass for a bot's (bots are «Бот …»).
pub fn sanitize_person_name(raw: &str) -> String {
    let name = sanitize_name(raw);
    let lower = name.to_lowercase();
    let botlike = lower
        .strip_prefix("бот")
        .is_some_and(|rest| rest.chars().next().is_none_or(|c| !c.is_alphanumeric()));
    if botlike { String::new() } else { name }
}

pub fn sanitize_title(raw: &str) -> String {
    cut(&collapse_spaces(&visible(raw, None, angle)), ROOM_TITLE_MAX)
}

/// One line, no control or invisible characters (bidi overrides, zero-width), at most CHAT_MAX characters.
pub fn sanitize_chat(raw: &str) -> String {
    cut(&collapse_spaces(&visible(raw, Some(' '), |_| false)), CHAT_MAX)
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
        // A joiner whose neighbour goes (a filler, `<`, the cut at the length limit) goes too.
        assert_eq!(sanitize_chat("\u{fe0f}\u{200d}\u{2800}\u{20000}"), "\u{fe0f}\u{20000}");
        assert_eq!(sanitize_title("<\u{200d}\u{3347a}"), "\u{3347a}");
        assert_eq!(
            sanitize_name("абвгдежзиклмно\u{1f468}\u{200d}\u{1f469}"),
            "абвгдежзиклмно\u{1f468}"
        );
    }

    #[test]
    fn drops_fillers_and_stacked_marks() {
        // Fillers that count as letters: a name of them would look empty.
        assert_eq!(sanitize_name("\u{3164}\u{115f}\u{1160}\u{ffa0}\u{2800}"), "");
        assert_eq!(sanitize_name("А\u{3164}н\u{2800}я"), "Аня");
        // At most two combining marks on one character, wherever the rest of them come.
        let zalgo = format!("a{}", "\u{301}".repeat(15));
        assert_eq!(sanitize_name(&zalgo), "a\u{301}\u{301}");
        assert_eq!(sanitize_chat(&format!("{zalgo}b\u{300}")), "a\u{301}\u{301}b\u{300}");
        assert_eq!(sanitize_name("a\u{301}\u{301}\u{7}\u{301}"), "a\u{301}\u{301}");
        assert_eq!(sanitize_chat(&"\u{489}".repeat(159)), "\u{489}\u{489}");
        // What real text and emoji need stays.
        assert_eq!(sanitize_name("Йо\u{308}жик"), "Йо\u{308}жик");
        assert_eq!(sanitize_chat("1\u{fe0f}\u{20e3}"), "1\u{fe0f}\u{20e3}");
        assert_eq!(
            sanitize_chat("\u{2764}\u{fe0f}\u{200d}\u{1f525}"),
            "\u{2764}\u{fe0f}\u{200d}\u{1f525}"
        );
    }

    #[test]
    fn people_are_not_called_like_bots() {
        assert_eq!(sanitize_person_name("Бот Кекс"), "");
        assert_eq!(sanitize_person_name("  бот "), "");
        assert_eq!(sanitize_person_name("БОТ_1"), "");
        assert_eq!(sanitize_person_name("Ботаник"), "Ботаник");
        assert_eq!(sanitize_person_name("Робот"), "Робот");
    }
}
