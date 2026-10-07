//! The whole client in `cargo test`: wgpu's noop device instead of a GPU, a window nothing opens, a real server
//! (`fb_server`) in the same process. A test steps both apps and acts as a player would: buttons, keys, text.
use std::net::{TcpListener, UdpSocket};
use std::sync::Arc;
use std::sync::atomic::{AtomicU16, Ordering};
use std::time::{Duration, Instant};

use bevy::app::PluginsState;
use bevy::ecs::system::RunSystemOnce;
use bevy::input::ButtonState;
use bevy::input::keyboard::{Key, KeyboardInput};
use bevy::input::mouse::{MouseButtonInput, MouseMotion};
use bevy::input_focus::{FocusCause, InputFocus};
use bevy::prelude::*;
use bevy::render::RenderApp;
use bevy::render::render_resource::{CachedPipelineState, PipelineCache, PipelineDescriptor};
use bevy::render::renderer::{
    RenderAdapter, RenderAdapterInfo, RenderDevice, RenderInstance, RenderQueue, WgpuWrapper,
};
use bevy::render::settings::RenderCreation;
use bevy::shader::ShaderCacheError;
use bevy::tasks::{block_on, tick_global_task_pools_on_main_thread};
use bevy::text::EditableText;
use bevy::ui::InteractionDisabled;
use bevy::ui_widgets::{Activate, SliderRange, ValueChange};
use bevy::window::{CursorMoved, PrimaryWindow, WindowEvent};
use clap::Parser;
use fb_net::ClientMsg;
use fb_proto::{ArenaInfo, DevCmd};
use lightyear::prelude::{Client, MessageSender};

use crate::opts::Opts;
use crate::session::Session;
use crate::ui::{Act, Action, Field, Knob};

/// Silence before a link is given up: a busy machine stalls the test's server and clients together for seconds
/// (3 s, the game's own, cut links under parallel tests). `server_away` lasts longer to cut one.
pub const LINK_TIMEOUT_S: i32 = 8;

/// A frame of both apps, about the rate a player's client runs at.
const FRAME: Duration = Duration::from_millis(8);

/// wgpu's noop device, named as a GPU of `kind`: the tier, and with it the effects, follow it (`quality.rs`).
fn noop_gpu(kind: wgpu::DeviceType) -> RenderCreation {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
        backends: wgpu::Backends::NOOP,
        flags: wgpu::InstanceFlags::default(),
        memory_budget_thresholds: Default::default(),
        display: None,
        backend_options: wgpu::BackendOptions {
            noop: wgpu::NoopBackendOptions { enable: true },
            ..Default::default()
        },
    });
    let adapter = block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default())).expect("noop adapter");
    let (device, queue) = block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        // What Bevy asks a desktop GPU for, without wgpu's experimental features (they need `unsafe`) and
        // without adapter-specific texture formats: the noop adapter names none, WebGPU's own then apply.
        required_features: adapter.features()
            - wgpu::Features::all_experimental_mask()
            - wgpu::Features::MAPPABLE_PRIMARY_BUFFERS
            - wgpu::Features::TEXTURE_ADAPTER_SPECIFIC_FORMAT_FEATURES,
        required_limits: adapter.limits(),
        ..Default::default()
    }))
    .expect("noop device");
    RenderCreation::manual(
        RenderDevice::from(device),
        RenderQueue(Arc::new(WgpuWrapper::new(queue))),
        RenderAdapterInfo(WgpuWrapper::new(wgpu::AdapterInfo {
            device_type: kind,
            ..adapter.get_info()
        })),
        RenderAdapter(Arc::new(WgpuWrapper::new(adapter))),
        RenderInstance(Arc::new(WgpuWrapper::new(instance))),
        #[cfg(feature = "dlss")]
        bevy::render::renderer::raw_vulkan_init::AdditionalVulkanFeatures::default(),
    )
}

/// A port no other test of this process got, free when asked (tests run in parallel).
fn free_port(udp: bool) -> String {
    static NEXT: AtomicU16 = AtomicU16::new(0);
    let _ = NEXT.compare_exchange(
        0,
        20_000 + (std::process::id() % 20_000) as u16,
        Ordering::Relaxed,
        Ordering::Relaxed,
    );
    loop {
        let p = NEXT.fetch_add(1, Ordering::Relaxed);
        let free = if udp {
            UdpSocket::bind(("127.0.0.1", p)).is_ok()
        } else {
            TcpListener::bind(("127.0.0.1", p)).is_ok()
        };
        if free {
            return p.to_string();
        }
    }
}

/// A server with `--dev --solo` on free ports: the app, its WebSocket and HTTP ports.
fn new_server() -> (App, String, String) {
    let (udp, ws, http) = (free_port(true), free_port(false), free_port(false));
    #[rustfmt::skip]
    let opts = fb_server::Opts::parse_from([
        "fb_server", "--dev", "--solo", "--udp-port", &udp, "--ws-port", &ws, "--http-port", &http,
        "--ws-addr", "127.0.0.1", "--http-addr", "127.0.0.1", "--public-host", "127.0.0.1",
        "--link-timeout", &LINK_TIMEOUT_S.to_string(),
    ]);
    let mut server = fb_server::app(opts, false);
    ready(&mut server);
    (server, ws, http)
}

/// What `App::run` does before the first frame.
fn ready(app: &mut App) {
    while app.plugins_state() == PluginsState::Adding {
        tick_global_task_pools_on_main_thread();
    }
    app.finish();
    app.cleanup();
}

/// One client of the game and the arenas it has been in, in order.
pub struct Peer {
    pub app: App,
    pub arenas: Vec<ArenaInfo>,
}

pub struct Game {
    pub server: App,
    /// More servers (`add_server`), stepped with the first.
    pub more: Vec<App>,
    pub peers: Vec<Peer>,
    /// The client the player's actions go to (`as_peer`).
    pub at: usize,
    http: String,
    ws: String,
}

impl Game {
    /// A server with `--dev --solo` and a client on a discrete GPU (tier T2, every effect) sent to it (`args`:
    /// more of the client's flags).
    pub fn new(args: &[&str]) -> Self {
        Self::on(wgpu::DeviceType::DiscreteGpu, args)
    }

    pub fn on(gpu: wgpu::DeviceType, args: &[&str]) -> Self {
        let (server, ws, http) = new_server();
        let mut g = Self {
            server,
            more: Vec::new(),
            peers: Vec::new(),
            at: 0,
            http,
            ws,
        };
        g.add_peer(gpu, args);
        g
    }

    /// Another player's client on the same server (it does not become the acting one).
    pub fn add_peer(&mut self, gpu: wgpu::DeviceType, args: &[&str]) -> usize {
        #[rustfmt::skip]
        let base = [
            "fb_client", "--server", "127.0.0.1", "--http-port", &self.http, "--ws-port", &self.ws,
            "--transport", "udp", "--no-update",
        ];
        let opts = Opts::parse_from(base.iter().chain(args));
        let mut app = App::new();
        crate::build(&mut app, opts, Some(noop_gpu(gpu)));
        // (The preset of the kind of GPU tested: every machine starts on High, a player may pick Low.)
        app.world_mut().resource_mut::<crate::settings::Graphics>().preset = match gpu {
            wgpu::DeviceType::DiscreteGpu | wgpu::DeviceType::IntegratedGpu => "high",
            _ => "low",
        }
        .into();
        ready(&mut app);
        self.peers.push(Peer {
            app,
            arenas: Vec::new(),
        });
        self.peers.len() - 1
    }

    pub fn http_port(&self) -> &str {
        &self.http
    }

    /// Another server (its own secret: its identities are not the first one's); its HTTP port.
    pub fn add_server(&mut self) -> String {
        let (server, _, http) = new_server();
        self.more.push(server);
        http
    }

    /// The player's actions go to client `i` from now on.
    pub fn as_peer(&mut self, i: usize) {
        self.at = i;
    }

    pub fn client(&mut self) -> &mut App {
        &mut self.peers[self.at].app
    }

    /// The arenas the acting client has been in.
    pub fn arenas(&self) -> &[ArenaInfo] {
        &self.peers[self.at].arenas
    }

    pub fn step(&mut self) {
        self.server.update();
        for s in &mut self.more {
            s.update();
        }
        self.step_clients();
    }

    /// The server stops answering for `secs` (a host's network down, a frozen process); the clients go on.
    pub fn server_away(&mut self, secs: f32) {
        let end = Instant::now() + Duration::from_secs_f32(secs);
        while Instant::now() < end {
            self.step_clients();
        }
    }

    fn step_clients(&mut self) {
        for (i, p) in self.peers.iter_mut().enumerate() {
            p.app.update();
            if let Some(exit) = p.app.should_exit() {
                panic!("client {i} quit: {exit:?}");
            }
            if let Some(a) = &p.app.world().resource::<Session>().arena
                && p.arenas.last().is_none_or(|l| l.id != a.id)
            {
                p.arenas.push(a.clone());
            }
        }
        std::thread::sleep(FRAME);
    }

    pub fn frames(&mut self, n: usize) {
        for _ in 0..n {
            self.step();
        }
    }

    /// Steps until `done` holds for the acting client; fails the test after `secs`.
    pub fn until(&mut self, secs: f32, what: &str, mut done: impl FnMut(&mut World) -> bool) {
        let end = Instant::now() + Duration::from_secs_f32(secs);
        while !done(self.client().world_mut()) {
            assert!(Instant::now() < end, "not within {secs} s: {what}");
            self.step();
        }
    }

    /// Steps until the acting client has been in an arena that passes `which` (now or since the start).
    pub fn until_arena(&mut self, secs: f32, what: &str, which: impl Fn(&ArenaInfo) -> bool) {
        let end = Instant::now() + Duration::from_secs_f32(secs);
        while !self.arenas().iter().any(&which) {
            assert!(
                Instant::now() < end,
                "not within {secs} s: {what} (arenas: {:?})",
                self.arenas().iter().map(|a| (a.kind, &a.game)).collect::<Vec<_>>()
            );
            self.step();
        }
    }

    /// A button whose action passes `which` is on screen and enabled.
    pub fn shows(&mut self, which: impl Fn(&Action) -> bool) -> bool {
        !self.buttons(&which).is_empty()
    }

    /// The actions of the enabled buttons on screen that pass `which`, each once, in screen order.
    pub fn actions(&mut self, which: impl Fn(&Action) -> bool) -> Vec<Action> {
        let mut out: Vec<Action> = Vec::new();
        for e in self.buttons(&which) {
            let a = self.client().world().get::<Act>(e).unwrap().0.clone();
            if !out.iter().any(|o| format!("{o:?}") == format!("{a:?}")) {
                out.push(a);
            }
        }
        out
    }

    /// Lets go of the slider of `knob` at `v`, as a drag ends.
    pub fn slide(&mut self, knob: Knob, v: f32) {
        let w = self.client().world_mut();
        let e = w
            .query::<(Entity, &Knob, &InheritedVisibility)>()
            .iter(w)
            .find(|(_, k, vis)| **k == knob && vis.get())
            .map(|(e, ..)| e)
            .unwrap_or_else(|| panic!("no slider {knob:?} on screen"));
        w.trigger(ValueChange {
            source: e,
            value: v,
            is_final: true,
        });
        self.step();
    }

    /// The enabled buttons on screen whose action passes `which`.
    fn buttons(&mut self, which: &impl Fn(&Action) -> bool) -> Vec<Entity> {
        let w = self.client().world_mut();
        w.query_filtered::<(Entity, &Act, &ComputedNode, &InheritedVisibility), Without<InteractionDisabled>>()
            .iter(w)
            .filter(|(_, a, n, v)| which(&a.0) && v.get() && n.size().x > 0.0)
            .map(|(e, ..)| e)
            .collect()
    }

    /// Waits up to `secs` for the button and presses it, as a click would.
    pub fn press(&mut self, secs: f32, what: &str, which: impl Fn(&Action) -> bool) {
        let end = Instant::now() + Duration::from_secs_f32(secs);
        let e = loop {
            if let Some(&e) = self.buttons(&which).first() {
                break e;
            }
            assert!(Instant::now() < end, "no button on screen within {secs} s: {what}");
            self.step();
        };
        self.client().world_mut().trigger(Activate { entity: e });
        self.step();
    }

    /// The text field on screen that holds `which` (waits up to 5 s: the UI draws a frame after the state it shows).
    pub fn field(&mut self, which: Field) -> Entity {
        let end = Instant::now() + Duration::from_secs(5);
        loop {
            let w = self.client().world_mut();
            let found = w
                .query::<(Entity, &Field, &InheritedVisibility)>()
                .iter(w)
                .find(|(_, f, v)| **f == which && v.get())
                .map(|(e, ..)| e);
            if let Some(e) = found {
                return e;
            }
            assert!(Instant::now() < end, "no field {which:?} on screen within 5 s");
            self.step();
        }
    }

    pub fn text(&mut self, which: Field) -> String {
        let e = self.field(which);
        self.client()
            .world()
            .get::<EditableText>(e)
            .unwrap()
            .value()
            .to_string()
    }

    /// Focuses the field (as a click into it does).
    pub fn focus(&mut self, which: Field) {
        let e = self.field(which);
        self.client()
            .world_mut()
            .resource_mut::<InputFocus>()
            .set(e, FocusCause::Pressed);
        self.step();
    }

    /// Nothing has the keyboard (a click outside every field).
    pub fn blur(&mut self) {
        self.client().world_mut().resource_mut::<InputFocus>().clear();
        self.step();
    }

    fn window(&mut self) -> Entity {
        let w = self.client().world_mut();
        w.query_filtered::<Entity, With<PrimaryWindow>>().single(w).unwrap()
    }

    fn key_event(&mut self, code: KeyCode, logical: &Key, state: ButtonState, text: Option<&str>) {
        let window = self.window();
        self.client().world_mut().write_message(KeyboardInput {
            key_code: code,
            logical_key: logical.clone(),
            state,
            text: text.map(Into::into),
            repeat: false,
            window,
        });
    }

    /// A key press and release, with the text it types.
    pub fn key(&mut self, code: KeyCode, logical: Key, text: Option<&str>) {
        self.key_event(code, &logical, ButtonState::Pressed, text);
        self.step();
        self.key_event(code, &logical, ButtonState::Released, None);
        self.step();
    }

    /// A key held down for `frames` frames (walking, a long jump).
    pub fn hold(&mut self, code: KeyCode, logical: Key, frames: usize) {
        self.key_event(code, &logical, ButtonState::Pressed, None);
        self.frames(frames.max(1));
        self.key_event(code, &logical, ButtonState::Released, None);
        self.step();
    }

    /// `s` arrives as one piece of text (an input method's commit).
    pub fn type_at_once(&mut self, s: &str) {
        self.key(KeyCode::KeyA, Key::Character(s.into()), Some(s));
    }

    /// A click of `button` over nothing in particular.
    pub fn click(&mut self, button: MouseButton) {
        let window = self.window();
        for state in [ButtonState::Pressed, ButtonState::Released] {
            self.client()
                .world_mut()
                .write_message(MouseButtonInput { button, state, window });
            self.step();
        }
    }

    /// The cursor moved to `at` (logical pixels), then a left click there, as the window reports them (through
    /// picking: `click` and `press` bypass it).
    pub fn click_at(&mut self, at: Vec2) {
        let window = self.window();
        let moved = CursorMoved {
            window,
            position: at,
            delta: None,
        };
        self.client().world_mut().write_message(WindowEvent::CursorMoved(moved));
        self.frames(2);
        for state in [ButtonState::Pressed, ButtonState::Released] {
            let input = MouseButtonInput {
                button: MouseButton::Left,
                state,
                window,
            };
            let w = self.client().world_mut();
            w.write_message(input);
            w.write_message(WindowEvent::MouseButtonInput(input));
            self.frames(2);
        }
    }

    /// Where the enabled buttons whose action passes `which` are on screen, logical pixels.
    pub fn rects(&mut self, which: impl Fn(&Action) -> bool) -> Vec<Rect> {
        let w = self.client().world_mut();
        let k = w.query::<&Window>().single(w).map_or(1.0, |w| 1.0 / w.scale_factor());
        w.query_filtered::<(&Act, &ComputedNode, &UiGlobalTransform, &InheritedVisibility), Without<InteractionDisabled>>()
            .iter(w)
            .filter(|(a, n, _, v)| which(&a.0) && v.get() && n.size().x > 0.0)
            .map(|(_, n, t, _)| Rect::from_center_size(t.translation * k, n.size() * k))
            .collect()
    }

    /// The mouse moved by `delta` (looking around while the cursor is captured).
    pub fn look(&mut self, delta: Vec2) {
        self.client().world_mut().write_message(MouseMotion { delta });
        self.step();
    }

    /// The text fields on screen.
    pub fn fields(&mut self) -> Vec<Field> {
        let w = self.client().world_mut();
        w.query::<(&Field, &InheritedVisibility)>()
            .iter(w)
            .filter(|(_, v)| v.get())
            .map(|(f, _)| *f)
            .collect()
    }

    /// The sliders on screen and their ranges.
    pub fn knobs(&mut self) -> Vec<(Knob, f32, f32)> {
        let w = self.client().world_mut();
        w.query::<(&Knob, &SliderRange, &InheritedVisibility)>()
            .iter(w)
            .filter(|(.., v)| v.get())
            .map(|(k, r, _)| (*k, r.start(), r.end()))
            .collect()
    }

    /// A text field has the keyboard.
    pub fn typing(&mut self) -> bool {
        let w = self.client().world_mut();
        let focus = w.resource::<InputFocus>().get();
        focus.is_some_and(|e| w.get::<EditableText>(e).is_some())
    }

    /// Types `s` into the focused field, one character a frame.
    pub fn type_text(&mut self, s: &str) {
        for c in s.chars() {
            let t = c.to_string();
            self.key(KeyCode::KeyA, Key::Character(t.as_str().into()), Some(&t));
        }
    }

    /// Empties the field, a Backspace a character (it must have the focus).
    pub fn erase(&mut self, which: Field) {
        for _ in 0..self.text(which).chars().count() {
            self.key(KeyCode::Backspace, Key::Backspace, None);
        }
        assert_eq!(self.text(which), "", "Backspace empties {which:?}");
    }

    pub fn enter(&mut self) {
        self.key(KeyCode::Enter, Key::Enter, None);
    }

    pub fn escape(&mut self) {
        self.key(KeyCode::Escape, Key::Escape, None);
    }

    /// A control message to the server, as the interface sends them.
    pub fn send(&mut self, msg: ClientMsg) {
        let sent = self.client().world_mut().run_system_once(
            move |mut senders: Query<&mut MessageSender<ClientMsg>, With<Client>>| {
                crate::session::send(&mut senders, msg.clone());
            },
        );
        sent.unwrap();
        self.step();
    }

    pub fn dev(&mut self, cmd: DevCmd) {
        self.send(ClientMsg::Dev { q: None, cmd });
    }

    pub fn res<R: Resource>(&self) -> &R {
        self.peers[self.at].app.world().resource::<R>()
    }
}

/// Pipelines that failed to build (a shader that does not compile, a layout wgpu rejects): Bevy only logs them.
impl Drop for Game {
    fn drop(&mut self) {
        if std::thread::panicking() {
            return;
        }
        let mut failed = Vec::new();
        for p in &self.peers {
            let render = p
                .app
                .get_sub_app(RenderApp)
                .expect("the render world on the main thread");
            failed.extend(
                render
                    .world()
                    .resource::<PipelineCache>()
                    .pipelines()
                    .filter_map(|p| match &p.state {
                        CachedPipelineState::Err(
                            e @ (ShaderCacheError::ProcessShaderError(_) | ShaderCacheError::CreateShaderModule(_)),
                        ) => Some(match &p.descriptor {
                            PipelineDescriptor::RenderPipelineDescriptor(d) => format!("{:?}: {e}", d.label),
                            PipelineDescriptor::ComputePipelineDescriptor(d) => format!("{:?}: {e}", d.label),
                        }),
                        _ => None,
                    }),
            );
        }
        assert!(
            failed.is_empty(),
            "pipelines that failed to build:\n{}",
            failed.join("\n")
        );
    }
}
