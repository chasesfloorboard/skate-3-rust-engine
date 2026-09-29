//! Per-map ambience loop prepared by tools/prepare_ambience.py. `maps.json`
//! maps a `.skate` file stem to a loop name, with "*" as the fallback.
use bevy::{
    audio::{AudioSinkPlayback, Volume},
    prelude::*,
};
use std::collections::HashMap;

const DIRECTORY: &str = "private/audio/ambience";
const VOLUME: f32 = 0.6;

#[derive(Component)]
struct Ambience;

pub(crate) struct AmbiencePlugin;
impl Plugin for AmbiencePlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, start)
            .add_systems(Update, pause_with_menu);
    }
}

fn select(config: &crate::config::Config) -> Option<String> {
    let map = config.map_path.as_ref()?.file_stem()?.to_str()?;
    let bytes = std::fs::read(config.asset_root.join(DIRECTORY).join("maps.json")).ok()?;
    let table: HashMap<String, String> = serde_json::from_slice(&bytes).ok()?;
    table.get(map).or_else(|| table.get("*")).cloned()
}

fn start(mut commands: Commands, config: Res<crate::config::Config>, assets: Res<AssetServer>) {
    let Some(name) = select(&config) else {
        return;
    };
    let path = format!("{DIRECTORY}/{name}.ogg");
    if !config.asset_root.join(&path).is_file() {
        warn!("Ambience loop missing: {path}");
        return;
    }
    info!("Ambience: {name}");
    commands.spawn((
        Ambience,
        AudioPlayer::new(assets.load(path)),
        PlaybackSettings::LOOP.with_volume(Volume::Linear(VOLUME)),
    ));
}

fn pause_with_menu(
    menu: Option<Res<crate::graphics_menu::Menu>>,
    mut sinks: Query<&mut AudioSink, With<Ambience>>,
) {
    let gain = menu.as_ref().map_or(1.0, |m| m.audio_gain(crate::graphics_menu::AudioChannel::Ambience));
    let active = crate::graphics_menu::gameplay_active(menu);
    for mut sink in &mut sinks {
        sink.set_volume(Volume::Linear(VOLUME * gain));
        if active && sink.is_paused() {
            sink.play();
        } else if !active && !sink.is_paused() {
            sink.pause();
        }
    }
}
