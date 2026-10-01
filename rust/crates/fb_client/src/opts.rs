use core::time::Duration;

use bevy::prelude::*;
use lightyear::prelude::LinkConditionerConfig;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Transport {
    Auto,
    Udp,
    Ws,
}

#[derive(Resource, Clone, Debug)]
pub struct Opts {
    pub server: String,
    pub udp_port: u16,
    pub ws_port: u16,
    pub ws_url: Option<String>,
    pub transport: Transport,
    pub id: Option<u64>,
    pub conditioner: Option<LinkConditionerConfig>,
    pub backend: Option<String>,
    pub screenshot: Option<String>,
    pub exit_after: Option<f32>,
    pub autopilot: bool,
    pub check_assets: bool,
    pub title: String,
}

pub fn parse() -> Opts {
    let mut o = Opts {
        server: "127.0.0.1".into(),
        udp_port: fb_net::UDP_PORT,
        ws_port: fb_net::WS_PORT,
        ws_url: None,
        transport: Transport::Auto,
        id: None,
        conditioner: None,
        backend: None,
        screenshot: None,
        exit_after: None,
        autopilot: false,
        check_assets: false,
        title: "Fall Beans".into(),
    };
    let (mut lag, mut jitter, mut loss) = (0u64, 0u64, 0f32);
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut i = 0;
    while i < args.len() {
        let v = args.get(i + 1).cloned().unwrap_or_default();
        match args[i].as_str() {
            "--server" => o.server = v,
            "--udp-port" => o.udp_port = v.parse().expect("--udp-port"),
            "--ws-port" => o.ws_port = v.parse().expect("--ws-port"),
            "--ws-url" => o.ws_url = Some(v),
            "--transport" => {
                o.transport = match v.as_str() {
                    "udp" => Transport::Udp,
                    "ws" => Transport::Ws,
                    _ => Transport::Auto,
                }
            }
            "--id" => o.id = Some(v.parse().expect("--id")),
            "--lag" => lag = v.parse().expect("--lag ms (one way)"),
            "--jitter" => jitter = v.parse().expect("--jitter ms"),
            "--loss" => loss = v.parse().expect("--loss 0..1"),
            "--backend" => o.backend = Some(v),
            "--screenshot" => o.screenshot = Some(v),
            "--exit-after" => o.exit_after = Some(v.parse().expect("--exit-after s")),
            "--title" => o.title = v,
            "--autopilot" => {
                o.autopilot = true;
                i += 1;
                continue;
            }
            "--check-assets" => {
                o.check_assets = true;
                i += 1;
                continue;
            }
            other => panic!("unknown flag {other}"),
        }
        i += 2;
    }
    if lag > 0 || jitter > 0 || loss > 0.0 {
        o.conditioner = Some(
            LinkConditionerConfig::default()
                .with_incoming_latency(Duration::from_millis(lag))
                .with_incoming_jitter(Duration::from_millis(jitter))
                .with_fixed_loss(loss),
        );
    }
    o
}
