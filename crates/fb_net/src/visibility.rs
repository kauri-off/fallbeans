//! Visibility (Replicon filters, evaluated per client link on the server): a room's entities go to the
//! links in that room only; within it the full body goes to its owner only, the pose to everybody else.
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

/// On a link: the room (`fb_server` key) the player is in.
#[derive(Component, Clone, Copy, Debug, PartialEq, Eq)]
#[component(immutable)]
pub struct InRoom(pub u32);

/// On a room's entities (its `Round`, its beans): replicated only to the links `InRoom` the same room.
#[derive(Component, Clone, Copy, Debug, PartialEq, Eq)]
#[component(immutable)]
pub struct RoomTag(pub u32);

impl VisibilityFilter for RoomTag {
    type ClientComponent = InRoom;
    type Scope = Entity;

    fn is_visible(&self, _client: Entity, room: Option<&InRoom>) -> bool {
        room.is_some_and(|r| r.0 == self.0)
    }
}

/// Server only: registers the filters (needs Replicon's server plugins, so after `ServerPlugins`).
pub fn add_server_filters(app: &mut App) {
    app.add_visibility_filter::<OwnerOnly>();
    app.add_visibility_filter::<OthersOnly>();
    app.add_visibility_filter::<RoomTag>();
}
