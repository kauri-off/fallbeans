//! A fall as the client predicts it (`fb_client::game::predict_respawn`) ends where the arena puts the bean.
use fb_arena::{Arena, ArenaKind, FallBehaviour, PawnStatus, fell, respawn, respawn_point};
use fb_shared::input::InputFrame;
use fb_sim::math::V3;
use fb_sim::physics::Body;

#[test]
fn predicted_respawn_matches_the_arena() {
    let mut checked = 0;
    for map in fb_maps::GAMES {
        let id = map.meta().id;
        let (mut arena, _) = Arena::new(*map, ArenaKind::Round, 7, 0, &[3, 8], false);
        if arena.fall == FallBehaviour::Out {
            arena.fall = FallBehaviour::Spawn;
        }
        arena.add_pawn_at(3, false, Some(0));
        arena.add_pawn_at(8, false, Some(1));
        for k in 0..30 {
            arena.step(k, |_| InputFrame::IDLE);
        }
        for (i, pid) in [3u32, 8].into_iter().enumerate() {
            let Some(p) = arena.pawn(pid).filter(|p| p.status == PawnStatus::Play) else {
                continue;
            };
            let at = p.body.pos;
            // The last checkpoint, if the map has any, else none (the spawn).
            let checkpoint = arena.spec.checkpoints.len().checked_sub(1).filter(|_| i == 0);
            let idx = arena.pawns.iter().position(|q| q.id == pid).unwrap();
            arena.pawns[idx].checkpoint = checkpoint;
            let low = V3::new(at.x, arena.spec.kill_y - 20.0, at.z);
            assert!(arena.dev_teleport(pid, low, None), "{id}");
            let teleports = arena.pawn(pid).unwrap().teleports;
            let spawn_i = arena.pawn(pid).unwrap().spawn_i;
            arena.step(30, |_| InputFrame::IDLE);
            let p = arena.pawn(pid).unwrap();
            let to = respawn_point(&arena.spec, ArenaKind::Round, arena.fall, checkpoint, spawn_i).unwrap();
            let mut predicted = Body::new(pid as i32);
            assert!(fell(&arena.spec, low), "{id}");
            respawn(&arena.spec, pid, &mut predicted, to);
            assert_eq!(p.teleports, teleports + 1, "{id}: respawned");
            assert_eq!(
                (p.body.pos, p.body.yaw),
                (predicted.pos, predicted.yaw),
                "{id} pawn {pid}"
            );
            checked += 1;
        }
    }
    assert_eq!(checked, 2 * fb_maps::GAMES.len());
}

#[test]
fn the_lobby_respawns_where_nobody_stands() {
    let lobby = fb_maps::by_id("lobby").unwrap();
    let (arena, _) = Arena::new(lobby, ArenaKind::Lobby, 1, -1, &[1], false);
    assert_eq!(respawn_point(&arena.spec, ArenaKind::Lobby, arena.fall, None, 0), None);
}
