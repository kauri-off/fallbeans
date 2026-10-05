//! Tail tag: a tail never leaves the room with its holder.
use fb_arena::{Arena, ArenaKind};
use fb_shared::TICK_RATE;
use fb_shared::input::InputFrame;

#[test]
fn a_leaving_holder_hands_the_tail_on() {
    let map = fb_maps::by_id("tail-tag").unwrap();
    for seed in 1..=8 {
        let (mut arena, _) = Arena::new(map, ArenaKind::Round, seed, 0, &[1, 2, 3, 4], false);
        for id in 1..=4 {
            arena.add_pawn_at(id, false, Some(id as usize - 1));
        }
        arena.step(0, |_| InputFrame::IDLE);
        // Everybody but bean 4 leaves: whatever tails they held end up with it, and it scores.
        for id in 1..=3 {
            arena.remove_pawn(id);
        }
        let end = 3 * i64::from(TICK_RATE);
        for k in 1..=end {
            arena.step(k, |_| InputFrame::IDLE);
        }
        let score = arena.scores.get(&4).copied().unwrap_or(0.0);
        assert!(score >= 2.0, "seed {seed}: the last bean scored {score}");
    }
}
