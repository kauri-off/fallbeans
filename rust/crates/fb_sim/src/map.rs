use crate::builder::Builder;
use crate::math::V3;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Genre {
    Race,
    Survival,
    Team,
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

#[derive(Clone, Debug)]
pub struct MapSpec {
    pub spawns: Vec<V3>,
    pub kill_y: f64,
    pub face_center: bool,
    /// Point the camera looks at in arenas.
    pub view: Option<V3>,
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
    out
}
