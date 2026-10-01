//! Soundtrack prepared by tools/prepare_audio.py from the disc's iPod playlist
//! (Skate 2 edition: Skate 2's, Freeskate: both games').
//! Plays the playlist shuffled, pauses with the menu. N skips, M mutes.
use bevy::{
    audio::{AudioSinkPlayback, Volume},
    prelude::*,
};

const DIRECTORY: &str = "private/audio/music";
const SKATE2_DIRECTORY: &str = "private/audio/music-skate2";

fn directories() -> &'static [&'static str] {
    use crate::editions::Edition;
    match crate::editions::current() {
        Edition::Skate2 => &[SKATE2_DIRECTORY],
        Edition::Skate3 => &[DIRECTORY],
        Edition::Freeskate => &[DIRECTORY, SKATE2_DIRECTORY],
    }
}
const VOLUME: f32 = 0.45;

#[derive(serde::Deserialize, Clone)]
struct Track {
    file: String,
    title: String,
    #[serde(default = "full")]
    volume: f32,
}
fn full() -> f32 {
    1.0
}

#[derive(Resource)]
struct Playlist {
    tracks: Vec<Track>,
    order: Vec<usize>,
    position: usize,
    muted: bool,
}

/// Per-track playlist gain; the menu's music volume is applied on top.
#[derive(Component)]
struct Song(f32);

pub(crate) struct MusicPlugin;
impl Plugin for MusicPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, load).add_systems(Update, (controls, advance, pause_with_menu).chain());
    }
}

fn load(mut commands: Commands, config: Res<crate::config::Config>) {
    let mut tracks = Vec::new();
    for directory in directories() {
        let path = config.asset_root.join(directory).join("playlist.json");
        let Ok(bytes) = std::fs::read(&path) else {
            info!("Soundtrack not installed ({})", path.display());
            continue;
        };
        let list: Vec<Track> = match serde_json::from_slice(&bytes) {
            Ok(list) => list,
            Err(error) => {
                warn!("Soundtrack skipped: {} is invalid: {error}", path.display());
                continue;
            }
        };
        tracks.extend(list.into_iter()
            .map(|t| Track { file: format!("{directory}/{}", t.file), ..t })
            .filter(|t| config.asset_root.join(&t.file).is_file()));
    }
    if tracks.is_empty() {
        return;
    }
    // Fisher-Yates with a time seed; a new order every launch.
    let mut seed = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(1, |d| d.as_nanos() as u64)
        | 1;
    let mut order: Vec<usize> = (0..tracks.len()).collect();
    for i in (1..order.len()).rev() {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        order.swap(i, (seed % (i as u64 + 1)) as usize);
    }
    info!("Soundtrack: {} tracks (N next, M mute)", tracks.len());
    commands.insert_resource(Playlist { tracks, order, position: 0, muted: false });
}

fn controls(
    keys: Res<ButtonInput<KeyCode>>,
    playlist: Option<ResMut<Playlist>>,
    songs: Query<(Entity, &mut AudioSink), With<Song>>,
    mut commands: Commands,
) {
    let Some(mut playlist) = playlist else { return };
    if keys.just_pressed(KeyCode::KeyN) {
        for (entity, sink) in &songs {
            sink.stop();
            commands.entity(entity).despawn();
        }
    }
    if keys.just_pressed(KeyCode::KeyM) {
        playlist.muted = !playlist.muted;
        info!("Music {}", if playlist.muted { "muted" } else { "on" });
        for (_, mut sink) in songs {
            if playlist.muted { sink.mute() } else { sink.unmute() }
        }
    }
}

fn advance(
    playlist: Option<ResMut<Playlist>>,
    songs: Query<(), With<Song>>,
    assets: Res<AssetServer>,
    mut commands: Commands,
) {
    let Some(mut playlist) = playlist else { return };
    if !songs.is_empty() {
        return;
    }
    let track = playlist.tracks[playlist.order[playlist.position]].clone();
    playlist.position = (playlist.position + 1) % playlist.order.len();
    info!("Now playing: {}", track.title);
    // Starts silent; pause_with_menu applies the menu volume on the next frame.
    let mut settings = PlaybackSettings::DESPAWN.with_volume(Volume::Linear(0.0));
    settings.muted = playlist.muted;
    commands.spawn((Song(VOLUME * track.volume), AudioPlayer::new(assets.load::<AudioSource>(track.file.clone())), settings));
}

fn pause_with_menu(menu: Option<Res<crate::graphics_menu::Menu>>, mut songs: Query<(&Song, &mut AudioSink)>) {
    let gain = menu.as_ref().map_or(1.0, |m| m.audio_gain(crate::graphics_menu::AudioChannel::Music));
    let active = crate::graphics_menu::gameplay_active(menu);
    for (song, mut sink) in &mut songs {
        sink.set_volume(Volume::Linear(song.0 * gain));
        if active && sink.is_paused() {
            sink.play();
        } else if !active && !sink.is_paused() {
            sink.pause();
        }
    }
}
