//! Checks `protocol.txt`, the wire schema traced from the types; `FB_BLESS=1 cargo test -p fb_net schema` rewrites it.
use std::collections::BTreeSet;

use bevy::prelude::*;
use bevy::state::app::StatesPlugin;
use fb_proto::{AwardKind, Cause, DenyReason, DevCmd, Goto, Hazard, MapEvent, MapEventKind, Mode, Phase, RejectReason};
use fb_shared::TICK_RATE;
use fb_shared::game::{ArenaKind, FallBehaviour};
use fb_shared::outfit::{Glasses, Hat, Tint};
use fb_shared::rules::RoundNote;
use fb_sim::map::SegEvent;
use fb_sim::physics::{BodyState, Power};
use lightyear::prelude::server::ServerPlugins;
use lightyear::prelude::{ChannelRegistry, ComponentRegistry, MessageRegistry};
use serde::de::DeserializeOwned;
use serde_reflection::{ContainerFormat, Format, Named, Tracer, TracerConfig, VariantFormat};

use crate::wire::{POS_STEPS, SIZE_STEPS, TILT_DIR_STEPS, TILT_STEPS, VEL_STEPS, YAW_STEPS};
use crate::{
    Anim, BeanColor, BeanId, BodyFull, ClientMsg, FbInput, Hold, MapEventMsg, ProtocolPlugin, RemotePose, Round,
    ServerMsg, TICK,
};

/// Crates whose own framing goes over the wire (from `Cargo.lock`).
fn on_the_wire(name: &str) -> bool {
    name.starts_with("lightyear") || name.starts_with("aeronet") || name == "bevy_replicon" || name == "postcard"
}

fn crates() -> Vec<String> {
    let lock = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../../Cargo.lock")).unwrap();
    let mut out = BTreeSet::new();
    let mut name = "";
    for line in lock.lines() {
        if let Some(n) = line.strip_prefix("name = ") {
            name = n.trim_matches('"');
        } else if let Some(v) = line.strip_prefix("version = ")
            && on_the_wire(name)
        {
            out.insert(format!("{name} {}", v.trim_matches('"')));
        }
    }
    out.into_iter().collect()
}

/// A type name without its module paths (`a::B<c::D>` → `B<D>`): moving a type does not change the protocol.
fn short(name: &str) -> String {
    let (mut out, mut word) = (String::new(), String::new());
    let mut chars = name.chars().peekable();
    while let Some(c) = chars.next() {
        if c.is_alphanumeric() || c == '_' {
            word.push(c);
        } else if c == ':' && chars.peek() == Some(&':') {
            chars.next();
            word.clear();
        } else {
            out += &word;
            word.clear();
            out.push(c);
        }
    }
    out + &word
}

/// Lightyear numbers channels, messages and components in the order they are registered.
fn registrations() -> Vec<String> {
    let mut app = App::new();
    app.add_plugins((
        MinimalPlugins,
        StatesPlugin,
        ServerPlugins { tick_duration: TICK },
        ProtocolPlugin,
    ));
    app.finish();
    let world = app.world();
    let mut out = vec!["channels".to_string()];
    let channels = world.resource::<ChannelRegistry>();
    let map = channels.kind_map();
    for id in 0.. {
        let Some(kind) = map.kind(id) else { break };
        let mode = format!("{:?}", channels.settings(*kind).unwrap().mode);
        let mode = mode.split(['(', ' ']).next().unwrap();
        out.push(format!("    {id} {} {mode}", short(map.name(kind).unwrap())));
    }
    out.push("messages".into());
    let map = &world.resource::<MessageRegistry>().kind_map;
    for id in 0.. {
        let Some(kind) = map.kind(id) else { break };
        out.push(format!("    {id} {}", short(map.name(kind).unwrap())));
    }
    out.push("components".into());
    let map = &world.resource::<ComponentRegistry>().kind_map;
    for id in 0.. {
        let Some(kind) = map.kind(id) else { break };
        out.push(format!("    {id} {}", short(map.name(kind).unwrap())));
    }
    out
}

fn trace<T: DeserializeOwned>(tracer: &mut Tracer) {
    if let Err(e) = tracer.trace_simple_type::<T>() {
        panic!("{}: {e}", std::any::type_name::<T>());
    }
}

fn ty(f: &Format) -> String {
    match f {
        Format::TypeName(n) => n.clone(),
        Format::Unit => "()".into(),
        Format::Option(f) => format!("Option<{}>", ty(f)),
        Format::Seq(f) => format!("Vec<{}>", ty(f)),
        Format::Map { key, value } => format!("Map<{}, {}>", ty(key), ty(value)),
        Format::Tuple(fs) => format!("({})", list(fs)),
        Format::TupleArray { content, size } => format!("[{}; {size}]", ty(content)),
        primitive => format!("{primitive:?}").to_lowercase(),
    }
}

fn list(fs: &[Format]) -> String {
    fs.iter().map(ty).collect::<Vec<_>>().join(", ")
}

fn fields(fs: &[Named<Format>]) -> String {
    let fs: Vec<_> = fs.iter().map(|f| format!("{}: {}", f.name, ty(&f.value))).collect();
    format!("{{ {} }}", fs.join(", "))
}

fn types() -> Vec<String> {
    let mut tracer = Tracer::new(TracerConfig::default());
    trace::<ClientMsg>(&mut tracer);
    trace::<ServerMsg>(&mut tracer);
    trace::<MapEventMsg>(&mut tracer);
    trace::<FbInput>(&mut tracer);
    trace::<BeanId>(&mut tracer);
    trace::<BeanColor>(&mut tracer);
    trace::<Round>(&mut tracer);
    trace::<BodyFull>(&mut tracer);
    trace::<RemotePose>(&mut tracer);
    trace::<Hold>(&mut tracer);
    // Enums inside them: `registry()` names the ones missing here.
    trace::<Anim>(&mut tracer);
    trace::<ArenaKind>(&mut tracer);
    trace::<AwardKind>(&mut tracer);
    trace::<BodyState>(&mut tracer);
    trace::<Cause>(&mut tracer);
    trace::<Hazard>(&mut tracer);
    trace::<DenyReason>(&mut tracer);
    trace::<DevCmd>(&mut tracer);
    trace::<FallBehaviour>(&mut tracer);
    trace::<Glasses>(&mut tracer);
    trace::<Goto>(&mut tracer);
    trace::<Hat>(&mut tracer);
    trace::<MapEvent>(&mut tracer);
    trace::<MapEventKind>(&mut tracer);
    trace::<Mode>(&mut tracer);
    trace::<Phase>(&mut tracer);
    trace::<Power>(&mut tracer);
    trace::<RejectReason>(&mut tracer);
    trace::<Result<String, String>>(&mut tracer);
    trace::<RoundNote>(&mut tracer);
    trace::<SegEvent>(&mut tracer);
    trace::<Tint>(&mut tracer);
    let registry = tracer
        .registry()
        .expect("every enum traced: add the missing ones above");
    let mut out = vec![];
    for (name, c) in &registry {
        match c {
            ContainerFormat::UnitStruct => out.push(format!("struct {name}")),
            ContainerFormat::NewTypeStruct(f) => out.push(format!("struct {name}({})", ty(f))),
            ContainerFormat::TupleStruct(fs) => out.push(format!("struct {name}({})", list(fs))),
            ContainerFormat::Struct(fs) => {
                out.push(format!("struct {name} {{"));
                out.extend(fs.iter().map(|f| format!("    {}: {}", f.name, ty(&f.value))));
                out.push("}".into());
            }
            ContainerFormat::Enum(vs) => {
                out.push(format!("enum {name} {{"));
                for (i, v) in vs {
                    let body = match &v.value {
                        VariantFormat::Unit => String::new(),
                        VariantFormat::NewType(f) => format!("({})", ty(f)),
                        VariantFormat::Tuple(fs) => format!("({})", list(fs)),
                        VariantFormat::Struct(fs) => format!(" {}", fields(fs)),
                        VariantFormat::Variable(_) => panic!("{name}::{} untraced", v.name),
                    };
                    out.push(format!("    {i} {}{body}", v.name));
                }
                out.push("}".into());
            }
        }
    }
    out
}

fn schema() -> String {
    let mut out = vec![
        "# The wire protocol, generated from the types by fb_net's `schema` test: do not edit.".to_string(),
        "# PROTOCOL_VERSION hashes it with the simulation's fingerprint.".into(),
        String::new(),
        "crates".into(),
    ];
    out.extend(crates().into_iter().map(|c| format!("    {c}")));
    out.push(format!("tick rate {TICK_RATE}"));
    out.push(format!(
        "pose steps: pos {POS_STEPS}/m, vel {VEL_STEPS}/(m/s), yaw {YAW_STEPS}/turn, tilt {TILT_STEPS}/π, \
         tilt_dir {TILT_DIR_STEPS}/turn, size {SIZE_STEPS}/1"
    ));
    out.push(String::new());
    out.extend(registrations());
    out.push(String::new());
    out.extend(types());
    out.join("\n") + "\n"
}

#[test]
fn schema_is_as_recorded() {
    let got = schema();
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/protocol.txt");
    let want = std::fs::read_to_string(path).unwrap_or_default().replace("\r\n", "\n");
    if std::env::var("FB_BLESS").is_ok() {
        std::fs::write(path, &got).unwrap();
        println!("wrote {path}: commit it");
        return;
    }
    for (n, (got, want)) in got.lines().zip(want.lines()).enumerate() {
        assert_eq!(
            got,
            want,
            "protocol.txt:{}: the wire protocol changed; bless (FB_BLESS=1 cargo test -p fb_net schema) and commit",
            n + 1
        );
    }
    assert_eq!(
        got, want,
        "lines added or removed: bless (FB_BLESS=1 cargo test -p fb_net schema)"
    );
}
