//! The client's `--trace` kinds (feature `traces`, not in packages): `input`, `hits`, `clicks`.
use bevy::prelude::*;
use fb_net::trace::{TraceKind, TraceOpts};

use crate::opts::Opts;

mod clicks;
pub mod hits;
pub mod input;

pub struct TracePlugin {
    traces: TraceOpts,
    /// Names the default files: `client`, or `client-<profile>`.
    side: String,
    ui: bool,
}

impl TracePlugin {
    pub fn new(opts: &Opts) -> Self {
        Self {
            traces: opts.traces.clone(),
            side: opts
                .profile
                .as_ref()
                .map_or_else(|| "client".into(), |p| format!("client-{p}")),
            ui: !opts.headless,
        }
    }
}

impl Plugin for TracePlugin {
    fn build(&self, app: &mut App) {
        if let Some(out) = self.traces.open(TraceKind::Input, &self.side) {
            input::add(app, out);
        }
        if let Some(out) = self.traces.open(TraceKind::Hits, &self.side) {
            app.insert_resource(hits::Hits::new(out));
        }
        if self.ui
            && let Some(out) = self.traces.open(TraceKind::Clicks, &self.side)
        {
            clicks::add(app, out);
        }
    }
}
