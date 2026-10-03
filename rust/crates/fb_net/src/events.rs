//! Channels of the messages outside replication (the messages themselves are `fb_proto`'s).

/// Control messages both ways (room list, lobby, game flow, chat, dev): reliable, in order.
pub struct ControlChannel;

/// Map events with their tick (`fb_proto::MapEventMsg`): reliable, in order.
pub struct MapEventsChannel;
