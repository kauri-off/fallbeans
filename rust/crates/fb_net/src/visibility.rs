//! Component-level visibility (Replicon filters, evaluated per client link on the server): the full
//! body goes to its owner only, the pose to everybody else. Without them every client would receive
//! every bean's full state.
use bevy::prelude::*;
use bevy_replicon::prelude::{AppVisibilityExt, ScopeLifetime, SingleComponent, VisibilityFilter};
use lightyear::prelude::RemoteId;

use crate::{BodyFull, RemotePose};

/// On a pawn: `BodyFull` is replicated only to this client link (the owner).
#[derive(Component, Clone, Copy, Debug)]
#[component(immutable)]
pub struct OwnerOnly(pub Entity);

impl VisibilityFilter for OwnerOnly {
    type ClientComponent = RemoteId;
    type Scope = SingleComponent<BodyFull>;
    const LIFETIME: ScopeLifetime = ScopeLifetime::WhileVisible;

    fn is_visible(&self, client: Entity, remote: Option<&RemoteId>) -> bool {
        remote.is_some() && client == self.0
    }
}

/// On a pawn: `RemotePose` is replicated to every client link except this one (the owner).
#[derive(Component, Clone, Copy, Debug)]
#[component(immutable)]
pub struct OthersOnly(pub Entity);

impl VisibilityFilter for OthersOnly {
    type ClientComponent = RemoteId;
    type Scope = SingleComponent<RemotePose>;
    const LIFETIME: ScopeLifetime = ScopeLifetime::WhileVisible;

    fn is_visible(&self, client: Entity, remote: Option<&RemoteId>) -> bool {
        remote.is_some() && client != self.0
    }
}

/// Server only: registers the filters (needs Replicon's server plugins, so after `ServerPlugins`).
pub fn add_server_filters(app: &mut App) {
    app.add_visibility_filter::<OwnerOnly>();
    app.add_visibility_filter::<OthersOnly>();
}
