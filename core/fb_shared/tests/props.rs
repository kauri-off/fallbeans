//! Properties of the shared maths, input, randomness, colours and player text, on generated inputs.
use fb_shared::input::{InputFrame, quantize_axis};
use fb_shared::m::{self, MinMax};
use fb_shared::rng::{Rng, shuffle};
use fb_shared::text::{is_other, sanitize_chat, sanitize_name, sanitize_person_name, sanitize_title};
use fb_shared::{CHAT_MAX, NAME_MAX, ROOM_TITLE_MAX, Rgb, rgb};
use proptest::prelude::*;

fn any_f64() -> impl Strategy<Value = f64> {
    prop_oneof![
        prop::num::f64::ANY,
        Just(0.0),
        Just(-0.0),
        Just(f64::INFINITY),
        Just(f64::NEG_INFINITY),
        -2.0..2.0f64,
    ]
}

/// Text that reaches the sanitizers' corner cases: marks, joiners, fillers, controls, bidi, `<>`.
fn tricky_text() -> impl Strategy<Value = String> {
    let piece = prop_oneof![
        any::<char>().prop_map(String::from),
        "[a-zA-Zа-яА-Я0-9 ]{1,4}",
        prop::sample::select(vec![
            "\u{301}",
            "\u{489}",
            "\u{200d}",
            "\u{200b}",
            "\u{202e}",
            "\u{2066}",
            "\u{3164}",
            "\u{2800}",
            "\u{fe0f}",
            "\u{1f468}",
            "\u{2764}",
            "<",
            ">",
            "\n",
            "\t",
            "\u{7}",
            "  ",
            "Бот",
            "бот ",
        ])
        .prop_map(String::from),
    ];
    prop::collection::vec(piece, 0..40).prop_map(|v| v.concat())
}

/// The same bits, or both NaN (which NaN payload comes out is not the game's concern).
fn same(x: f64, y: f64) -> bool {
    x.to_bits() == y.to_bits() || x.is_nan() && y.is_nan()
}

fn mark_run(s: &str) -> usize {
    let mut run = 0;
    let mut most = 0;
    for c in s.chars() {
        if matches!(c as u32, 0x0300..=0x036f | 0x0483..=0x0489) {
            run += 1;
            most = most.max(run);
        } else {
            run = 0;
        }
    }
    most
}

proptest! {
    #[test]
    fn max_min_agree_in_both_orders(a in any_f64(), b in any_f64()) {
        prop_assert!(same(m::max(a, b), m::max(b, a)));
        prop_assert!(same(m::min(a, b), m::min(b, a)));
        prop_assert!(same(a.at_least(b), m::max(a, b)));
        prop_assert!(same(a.at_most(b), m::min(a, b)));
        if !a.is_nan() && !b.is_nan() {
            let (hi, lo) = (m::max(a, b), m::min(a, b));
            prop_assert!(hi >= a && hi >= b && lo <= a && lo <= b);
            prop_assert!(hi.to_bits() == a.to_bits() || hi.to_bits() == b.to_bits());
            prop_assert!(lo.to_bits() == a.to_bits() || lo.to_bits() == b.to_bits());
        }
    }

    #[test]
    fn clamp_stays_within_bounds(x in any_f64(), lo in -1e6..1e6f64, span in 0.0..1e6f64) {
        let hi = lo + span;
        let c = m::clamp(x, lo, hi);
        prop_assert!((lo..=hi).contains(&c), "{x} in [{lo}, {hi}] -> {c}");
        if (lo..=hi).contains(&x) && x != 0.0 {
            prop_assert_eq!(c.to_bits(), x.to_bits());
        }
    }

    #[test]
    fn sign_is_minus_one_zero_or_one(x in any_f64()) {
        let s = m::sign(x);
        if x.is_nan() {
            prop_assert!(s.is_nan());
        } else if x == 0.0 {
            prop_assert_eq!(s.to_bits(), x.to_bits());
        } else {
            prop_assert_eq!(s, if x > 0.0 { 1.0 } else { -1.0 });
        }
    }

    #[test]
    fn hypot_is_a_length(a in -1e6..1e6f64, b in -1e6..1e6f64, c in -1e6..1e6f64) {
        let h = m::hypot(a, b);
        prop_assert!(h >= a.abs() && h >= b.abs());
        prop_assert_eq!(h.to_bits(), m::hypot(b, a).to_bits());
        prop_assert_eq!(h.to_bits(), m::hypot(-a, -b).to_bits());
        prop_assert!(m::hypot3(a, b, c) >= h);
    }

    #[test]
    fn sticks_arrive_within_reach(x in any_f64(), z in any_f64(), buttons: u8) {
        let f = InputFrame::from_stick(x, z, buttons);
        prop_assert!(i32::from(f.mx).pow(2) + i32::from(f.mz).pow(2) <= 127 * 127, "{f:?}");
        prop_assert_eq!(f.clamped(), f);
        prop_assert_eq!(f.buttons, buttons & 7);
    }

    #[test]
    fn quantizing_keeps_order(a in any_f64(), b in any_f64()) {
        let (qa, qb) = (quantize_axis(a), quantize_axis(b));
        prop_assert!((-127..=127).contains(&qa));
        if !a.is_nan() && !b.is_nan() && a <= b {
            prop_assert!(qa <= qb, "{a} -> {qa}, {b} -> {qb}");
        }
    }

    #[test]
    fn rng_draws_stay_in_range(seed: u32, len in 1usize..1000) {
        let mut r = Rng::new(seed);
        for _ in 0..32 {
            let u = r.unit();
            prop_assert!((0.0..1.0).contains(&u));
            prop_assert!(r.index(len) < len);
        }
    }

    #[test]
    fn shuffling_permutes(seed: u32, mut v in prop::collection::vec(any::<u16>(), 0..64)) {
        let mut sorted = v.clone();
        sorted.sort_unstable();
        let mut again = v.clone();
        shuffle(&mut v, &mut Rng::new(seed));
        shuffle(&mut again, &mut Rng::new(seed));
        prop_assert_eq!(&v, &again);
        v.sort_unstable();
        prop_assert_eq!(v, sorted);
    }

    #[test]
    fn colours_survive_text(v in 0u32..=0xff_ffff, alpha: u8) {
        let c = Rgb { rgb: v, alpha };
        prop_assert_eq!(Rgb::parse(&c.to_string()), Some(c));
        prop_assert_eq!(Rgb::from_bytes(rgb(v).bytes()), rgb(v));
    }

    #[test]
    fn colour_parsing_never_panics(s in "#?[0-9a-fA-F+-]{0,10}") {
        if let Some(c) = Rgb::parse(&s) {
            prop_assert_eq!(Rgb::parse(&c.to_string()), Some(c));
        }
    }

    #[test]
    fn names_are_short_visible_and_settled(raw in tricky_text()) {
        let s = sanitize_name(&raw);
        prop_assert!(s.chars().count() <= NAME_MAX);
        prop_assert!(!s.chars().any(|c| is_other(c) && c != '\u{200d}' || c == '<' || c == '>'), "{s:?}");
        prop_assert_eq!(s.trim(), &s);
        prop_assert!(mark_run(&s) <= 2, "{s:?}");
        prop_assert_eq!(&sanitize_name(&s), &s);
        let p = sanitize_person_name(&raw);
        prop_assert!(p.is_empty() || p == s);
        prop_assert_eq!(sanitize_person_name(&p), p);
    }

    #[test]
    fn titles_are_short_visible_and_settled(raw in tricky_text()) {
        let s = sanitize_title(&raw);
        prop_assert!(s.chars().count() <= ROOM_TITLE_MAX);
        prop_assert!(!s.chars().any(|c| is_other(c) && c != '\u{200d}' || c == '<' || c == '>'), "{s:?}");
        prop_assert!(!s.contains("  ") && s.trim() == s, "{s:?}");
        prop_assert!(mark_run(&s) <= 2, "{s:?}");
        prop_assert_eq!(sanitize_title(&s), s);
    }

    #[test]
    fn chat_is_one_short_line_and_settled(raw in tricky_text()) {
        let s = sanitize_chat(&raw);
        prop_assert!(s.chars().count() <= CHAT_MAX);
        prop_assert!(!s.chars().any(|c| is_other(c) && c != '\u{200d}'), "{s:?}");
        prop_assert!(!s.contains("  ") && !s.starts_with(' '), "{s:?}");
        prop_assert!(mark_run(&s) <= 2, "{s:?}");
        prop_assert_eq!(sanitize_chat(&s), s);
    }
}
