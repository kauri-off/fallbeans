use bevy::prelude::*;
use clap::Parser;
use fb_server::Opts;

fn main() -> AppExit {
    let opts = Opts::parse();
    if let Some(bad) = opts.open_rooms.iter().find(|r| !fb_proto::valid_room_id(r)) {
        eprintln!("--open-rooms: {bad:?} is not a room id (2–8 of a-z, 0-9)");
        return AppExit::from_code(2);
    }
    std::panic::set_hook(Box::new(panic));
    fb_server::app(opts, true).run()
}

/// A panic in the log (and `/api/debug/logs`) and its backtrace on stderr, without `RUST_BACKTRACE`: a room's
/// panic only closes the room, and journald keeps both.
fn panic(info: &std::panic::PanicHookInfo) {
    let msg = info
        .payload()
        .downcast_ref::<&str>()
        .map(|s| s.to_string())
        .or_else(|| info.payload().downcast_ref::<String>().cloned())
        .unwrap_or_else(|| "(not a string)".into());
    let place = info.location().map_or("?".into(), |l| l.to_string());
    let thread = std::thread::current().name().unwrap_or("?").to_string();
    error!(thread, at = place, "panic: {msg}");
    eprintln!("backtrace:\n{}", std::backtrace::Backtrace::force_capture());
}
