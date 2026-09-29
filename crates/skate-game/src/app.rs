use crate::{
    animation, assets, camera,
    config::Config,
    graph_runtime::StockGraphs,
    input,
    physics::{GamePhysics, PhysicsPlugin, SkaterRuntime},
    verification, world,
};
use bevy::{
    prelude::*,
    render::{
        RenderPlugin,
        settings::{Backends, InstanceFlags, RenderCreation, WgpuSettings},
    },
};
use skate_data::GameAssets;

#[derive(SystemSet, Debug, Hash, PartialEq, Eq, Clone)]
pub(crate) enum FrameSet {
    Assets,
    Physics,
    Animation,
    Verification,
}

#[derive(SystemSet, Debug, Hash, PartialEq, Eq, Clone)]
pub(crate) enum SimulationSet {
    Input,
    Controls,
    Physics,
}

pub(crate) fn build(
    config: Config,
    manifest: GameAssets,
    graphs: StockGraphs,
    physics: GamePhysics,
    skater: SkaterRuntime,
) -> App {
    let retail_scene = config.map.as_ref().is_some_and(|map| crate::retail_render::RetailScene::for_map(map));
    let mut app = App::new();
    crate::custom_models::register_source(&mut app);
    app.register_asset_source("mods", bevy::asset::io::AssetSourceBuilder::platform_default(
        &crate::modding::package_root().to_string_lossy(), None));
    app.add_plugins(
        DefaultPlugins
            .set(AssetPlugin {
                file_path: config.asset_root.to_string_lossy().into_owned(),
                ..default()
            })
            .set(WindowPlugin {
                primary_window: Some(Window {
                    title: config.multiplayer.title.clone().unwrap_or_else(||"Skate 3 Rust Engine".into()),
                    resolution: (1280, 800).into(),
                    ..default()
                }),
                ..default()
            })
            .set(RenderPlugin {
                render_creation: RenderCreation::Automatic(WgpuSettings {
                    backends: Some(Backends::VULKAN),
                    // Existing machine's validation layer rejects wgpu atomic shaders.
                    // This workaround belongs only to the rendering adapter.
                    instance_flags: InstanceFlags::empty(),
                    ..default()
                }),
                ..default()
            }).build().disable::<bevy::log::LogPlugin>()
            // Gameplay and menu navigation both use raw XInput. No game system
            // consumes Bevy gamepad events/rumble; its second device backend can
            // stall PreUpdate (70.68 ms in the University capture).
            .disable::<bevy::gilrs::GilrsPlugin>(),
    )
    .insert_resource(bevy::winit::WinitSettings {focused_mode:bevy::winit::UpdateMode::Continuous,unfocused_mode:bevy::winit::UpdateMode::Continuous})
    .insert_resource(config)
    .insert_resource(crate::retail_render::RetailScene(retail_scene))
    .insert_resource(assets::AssetManifest(manifest))
    .insert_resource(graphs)
    .insert_resource(physics)
    .insert_resource(skater)
    .configure_sets(
        FixedUpdate,
        (
            SimulationSet::Input,
            SimulationSet::Controls,
            SimulationSet::Physics,
        )
            .chain(),
    )
    .configure_sets(
        Update,
        (
            FrameSet::Assets,
            FrameSet::Physics,
            FrameSet::Animation,
            FrameSet::Verification,
        )
            .chain(),
    )
    .add_plugins(crate::fps_overlay::FpsOverlayPlugin)
    .add_plugins((crate::ambience::AmbiencePlugin, crate::board_audio::BoardAudioPlugin, crate::pcm_audio::PcmAudioPlugin, crate::music::MusicPlugin, crate::menu_skin::MenuSkinPlugin))
    .add_plugins(crate::prop_material::PropMaterialPlugin)
    .add_plugins(crate::dropper::DropperPlugin)
    .init_resource::<crate::props::PropColliders>()
    .add_systems(PreUpdate, crate::props::refresh)
    .add_systems(FixedUpdate, crate::props::simulate.after(crate::app::SimulationSet::Physics))
    .add_systems(PostUpdate, crate::props::sync_pieces.before(bevy::transform::TransformSystems::Propagate))
    .add_systems(Update, (crate::props::debug, crate::props::ramp_obstacles))
    .add_systems(Update, crate::street_lights::update)
    .add_systems(Update, crate::day_cycle::advance.after(crate::map_render::advance_day))
    // Skinned character parts keep their bind-pose bounds while animating, so
    // frustum culling could hide a head or shoe at some camera angles.
    .add_systems(PostUpdate, |mut commands: Commands, skins: Query<Entity, (With<bevy::mesh::skinning::SkinnedMesh>, Without<bevy::camera::visibility::NoFrustumCulling>)>| {
        for entity in &skins { commands.entity(entity).insert(bevy::camera::visibility::NoFrustumCulling); }
    })
    .add_plugins((
        crate::retail_render::RetailRenderPlugin,
        input::InputPlugin,
        PhysicsPlugin,
        crate::presentation::PresentationPlugin,
        crate::replay::ReplayPlugin,
        assets::GameAssetsPlugin,
        animation::AnimationPlugin,
        world::WorldPlugin,
        crate::grind_world::GrindGeometryPlugin,
        camera::CameraPlugin,
        crate::graphics_menu::GraphicsMenuPlugin,
        crate::map_transition::MapTransitionPlugin,
        crate::render_capacity::RenderCapacityPlugin,
        verification::VerificationPlugin,
        crate::performance::PerformancePlugin,
    ));
    app.add_plugins((crate::session_marker::SessionMarkerPlugin, crate::customiser::CustomiserPlugin));
    app.add_plugins(crate::custom_models::CustomModelsPlugin);
    app.add_plugins(crate::jiggle::JigglePlugin);
    app.add_plugins(crate::movies::MoviesPlugin);
    app.add_plugins(crate::modding::ModdingPlugin);
    crate::teleport_menu::install(&mut app);
    app.add_plugins(crate::updater::UpdaterPlugin);
    app.add_plugins(crate::multiplayer::MultiplayerPlugin);
    app.add_plugins(crate::scoring_hud::ScoringHudPlugin);
    app.add_systems(Last, crate::crash_context::sample);
    crate::profiling::install(&mut app);
    app
}
