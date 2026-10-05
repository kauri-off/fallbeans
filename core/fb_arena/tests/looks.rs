//! The specials of every map as a client builds them: each look runs through the round without
//! panicking, places only parts its special has, and every kind turns up.
use std::collections::BTreeSet;

use fb_arena::build_map;
use fb_shared::m::MinMax;
use fb_sim::scene::{LookOut, SceneItem};

#[test]
fn looks_run_through_rounds() {
    let mut kinds = BTreeSet::new();
    for &map in fb_maps::MAPS {
        let end = map.meta().duration.at_most(150.0);
        for seed in 1..=4 {
            let (mut b, _) = build_map(map, seed, true, &[1, 2]);
            b.world.finalize(0.0);
            let scene = b.scene.take().expect("a client build has a scene");
            let mut out = LookOut::default();
            for item in &scene.items {
                let SceneItem::Special {
                    kind,
                    parts,
                    pieces,
                    look,
                    ..
                } = item
                else {
                    continue;
                };
                kinds.insert(*kind);
                let id = map.meta().id;
                assert!(pieces.iter().all(|p| (p.part as usize) < parts.len()), "{id} {kind}");
                let Some(look) = look else { continue };
                let mut t = -3.0;
                while t < end {
                    out.pieces.clear();
                    out.tints.clear();
                    (look.0)(&b.world, t, &mut out);
                    for p in &out.pieces {
                        assert!((p.part as usize) < parts.len(), "{id} {kind}: part {}", p.part);
                        assert!(
                            p.pos.is_finite() && p.rot.is_finite() && p.scale.is_finite(),
                            "{id} {kind} at {t}"
                        );
                    }
                    assert!(out.tints.iter().all(|x| x.k.is_finite()), "{id} {kind} at {t}");
                    t += 0.37;
                }
            }
        }
    }
    for kind in [
        "hex-tiles",
        "glass-bridge",
        "portal",
        "portal-exit",
        "portal-lamp",
        "gate-lamp",
        "drum-rims",
        "plate-tint",
        "bell",
        "stars",
        "sign",
        "medal",
        "confetti",
    ] {
        assert!(kinds.contains(kind), "no map drew {kind}");
    }
}
