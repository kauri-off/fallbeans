//! What Bevy does with a failed system or command: panics in debug builds and tests (found before a player
//! sees it), logs `ECS ERROR` in release builds (a command to a despawned entity must not end a match).
use bevy::ecs::error::{BevyError, ErrorContext, FallbackErrorHandler, Severity, match_severity};
use bevy::prelude::*;

/// The words `cargo xtask stress` looks for in the logs.
pub const MARK: &str = "ECS ERROR";

fn log_it(err: BevyError, ctx: ErrorContext) {
    if matches!(err.severity(), Severity::Panic | Severity::Error) {
        error!("{MARK} in {} `{}`: {err}", ctx.kind(), ctx.name());
    } else {
        match_severity(err, ctx);
    }
}

pub struct ErrorPolicyPlugin;

impl Plugin for ErrorPolicyPlugin {
    fn build(&self, _: &mut App) {}

    fn finish(&self, app: &mut App) {
        if cfg!(debug_assertions) {
            return;
        }
        // (The main world and the others: the client's render world.)
        for sub in app.sub_apps_mut().iter_mut() {
            sub.insert_resource(FallbackErrorHandler(log_it));
        }
    }
}
