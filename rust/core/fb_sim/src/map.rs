use crate::bots::BotBrain;
use crate::builder::Builder;
use crate::math::V3;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Genre {
    Race,
    Survival,
    Points,
}

#[derive(Clone, Copy, Debug)]
pub struct GameMeta {
    pub id: &'static str,
    pub title: &'static str,
    pub genre: Genre,
    pub desc: &'static str,
    pub goal: &'static str,
    pub duration: f64,
}

pub struct MapCtx<'a> {
    pub server: bool,
    pub seed: u32,
    pub participants: &'a [u32],
}

#[derive(Clone, Copy, Debug)]
pub struct Checkpoint {
    /// Progress threshold (z unless the map defines `progress`).
    pub z: f64,
    pub p: V3,
}

/// Crossing z (above y − 2, within |x| ≤ half_width when given) finishes the race.
#[derive(Clone, Copy, Debug)]
pub struct Finish {
    pub z: f64,
    pub y: f64,
    pub half_width: Option<f64>,
}

pub type PosTest = Box<dyn Fn(V3) -> bool + Send + Sync>;

#[derive(Default)]
pub struct MapSpec {
    pub spawns: Vec<V3>,
    pub kill_y: f64,
    pub is_out: Option<PosTest>,
    pub checkpoints: Vec<Checkpoint>,
    /// Progress along the course (default: z); checkpoint thresholds and race ranking use it.
    pub progress: Option<Box<dyn Fn(V3) -> f64 + Send + Sync>>,
    pub finish: Option<Finish>,
    /// Out-of-course places (on top of frames, behind walls): standing there counts as a shortcut.
    pub forbidden: Option<PosTest>,
    /// What a fall is blamed on when no hazard or player was involved (default "fall").
    pub fall_cause: Option<&'static str>,
    /// Point the camera looks at in arenas.
    pub view: Option<V3>,
    pub face_center: bool,
    pub bot: Option<BotBrain>,
}

pub trait MapDef: Sync {
    fn meta(&self) -> &'static GameMeta;
    fn build(&self, b: &mut Builder, ctx: &MapCtx) -> MapSpec;
    fn looks(&self) -> &'static [&'static str] {
        &[]
    }
}

/// Problems with a built map that would break a round.
pub fn spec_problems(spec: &MapSpec) -> Vec<&'static str> {
    let mut out = Vec::new();
    if spec.spawns.is_empty() {
        out.push("no spawns");
    }
    if spec.spawns.iter().any(|p| !p.is_finite()) {
        out.push("a spawn is not a finite position");
    }
    if !spec.kill_y.is_finite() {
        out.push("killY is not a number");
    } else if spec.spawns.iter().any(|p| p.y <= spec.kill_y) {
        out.push("a spawn is below killY");
    }
    if spec.checkpoints.iter().any(|c| !c.p.is_finite() || !c.z.is_finite()) {
        out.push("a checkpoint is not finite");
    }
    if spec.finish.is_some_and(|f| !(f.z + f.y).is_finite()) {
        out.push("the finish is not finite");
    }
    out
}
