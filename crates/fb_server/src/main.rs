use bevy::prelude::*;
use clap::Parser;
use fb_server::Opts;

fn main() -> AppExit {
    let opts = Opts::parse();
    if let Some(bad) = opts.open_rooms.iter().find(|r| !fb_proto::valid_room_id(r)) {
        eprintln!("--open-rooms: {bad:?} is not a room id (2–8 of a-z, 0-9)");
        return AppExit::from_code(2);
    }
    fb_server::app(opts, true).run()
}
