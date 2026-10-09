//! `--check-assets`: loads every model through Bevy's glTF loader, reports and quits.
use bevy::asset::LoadState;
use bevy::prelude::*;
use fb_sim::scene::Model;

pub struct CheckAssetsPlugin;

impl Plugin for CheckAssetsPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, load_models);
        app.add_systems(Update, report_models);
    }
}

#[derive(Resource)]
struct Models(Vec<(Model, Handle<Gltf>)>);

fn load_models(mut commands: Commands, assets: Res<AssetServer>) {
    let list = Model::ALL
        .into_iter()
        .map(|n| (n, assets.load(format!("models/{n}.glb"))))
        .collect();
    commands.insert_resource(Models(list));
}

fn report_models(
    models: Res<Models>,
    assets: Res<AssetServer>,
    gltfs: Res<Assets<Gltf>>,
    meshes: Res<Assets<bevy::gltf::GltfMesh>>,
    time: Res<Time>,
    mut done: Local<Option<(f32, u8)>>,
    mut exit: MessageWriter<AppExit>,
) {
    // Quitting while the driver still compiles pipelines crashes it (NVIDIA 615): let it finish first.
    if let Some((at, code)) = *done {
        if time.elapsed_secs() - at > 3.0 {
            exit.write(if code == 0 {
                AppExit::Success
            } else {
                AppExit::from_code(code)
            });
        }
        return;
    }
    let states: Vec<_> = models.0.iter().map(|(n, h)| (*n, assets.load_state(h))).collect();
    if states
        .iter()
        .any(|(_, s)| matches!(s, LoadState::Loading | LoadState::NotLoaded))
    {
        return;
    }
    let mut failed = 0;
    for ((name, state), (_, h)) in states.iter().zip(&models.0) {
        match state {
            LoadState::Loaded => {
                let Some(g) = gltfs.get(h) else {
                    failed += 1;
                    println!("FAILED  {name:<9} loaded, but not in Assets<Gltf>");
                    continue;
                };
                let prims: usize = g
                    .meshes
                    .iter()
                    .filter_map(|m| meshes.get(m))
                    .map(|m| m.primitives.len())
                    .sum();
                println!(
                    "ok      {name:<9} scenes {} nodes {:>3} meshes {:>3} primitives {:>3} materials {:>2}",
                    g.scenes.len(),
                    g.nodes.len(),
                    g.meshes.len(),
                    prims,
                    g.materials.len()
                );
            }
            LoadState::Failed(e) => {
                failed += 1;
                println!("FAILED  {name:<9} {e}");
            }
            _ => {}
        }
    }
    println!("{} of {} models load in Bevy", states.len() - failed, states.len());
    *done = Some((time.elapsed_secs(), u8::from(failed > 0)));
}
