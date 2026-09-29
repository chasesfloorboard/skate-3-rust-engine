//! The disc's FMV movies (tools/prepare_movies.py): the EA logo at start-up,
//! the story intro on first launch, and any movie from the pause menu.
//!
//! Video is EA VP6 decoded by ffmpeg into raw frames on a reader thread; the
//! soundtrack plays from the Ogg extracted beside it. Any key or button skips.
use bevy::{asset::RenderAssetUsages, prelude::*, render::render_resource::{Extent3d, TextureDimension, TextureFormat}};
use std::{io::Read, path::PathBuf, process::{Child, Command, Stdio}, sync::mpsc, time::Instant};

/// (file stem, menu title) in the order the menu lists them.
pub(crate) const MOVIES: [(&str, &str); 11] = [
    ("intro_movie", "Story intro"),
    ("Coach_frank_intro", "Coach Frank"),
    ("skate_park", "skate.Park"),
    ("create_plaza_intro", "Plaza park intro"),
    ("create_industrial_intro", "Industrial park intro"),
    ("create_stadium_intro", "Stadium park intro"),
    ("film_ender_outro", "Film ender"),
    ("Attract", "Attract loop"),
    ("EA_Blackbox", "EA Black Box"),
    ("cameraHigh", "Camera: high"),
    ("cameraLow", "Camera: low"),
];
const WIDTH: u32 = 1280;
const HEIGHT: u32 = 720;
const FPS: f32 = 29.97;

pub(crate) struct MoviesPlugin;
impl Plugin for MoviesPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Player>().add_systems(Startup, boot).add_systems(Update, (request, play).chain());
    }
}

#[derive(Resource, Default)]
struct Player {
    queue: std::collections::VecDeque<String>,
    current: Option<Playback>,
}
struct Playback {
    child: Child,
    frames: std::sync::Mutex<mpsc::Receiver<Vec<u8>>>,
    image: Handle<Image>,
    root: Entity,
    audio: Option<Entity>,
    start: Instant,
    shown: u64,
    /// Virtual time was running before the movie paused it.
    resume: bool,
}
impl Drop for Playback {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn directory(config: &crate::config::Config) -> PathBuf {
    config.asset_root.join("private/movies")
}
fn seen_path(config: &crate::config::Config) -> PathBuf {
    config.asset_root.parent().unwrap_or(&config.asset_root).join("settings/movies-seen")
}

/// Start-up: the EA logo, then the story intro once per installation.
fn boot(mut player: ResMut<Player>, config: Res<crate::config::Config>) {
    // Test hook: SKATE_DEBUG_MOVIE=<name> plays that movie, even under verification.
    if let Ok(name) = std::env::var("SKATE_DEBUG_MOVIE") { player.queue.push_back(name); return; }
    if std::env::var_os("SKATE_VERIFY_DELAY").is_some() || std::env::var_os("SKATE_PERF_REPORT").is_some() { return; }
    if !directory(&config).join("EA_Blackbox.vp6").is_file() { return; }
    player.queue.push_back("EA_Blackbox".into());
    if !seen_path(&config).is_file() {
        player.queue.push_back("intro_movie".into());
        let _ = std::fs::write(seen_path(&config), b"intro\n");
    }
}

fn request(mut menu: ResMut<crate::graphics_menu::Menu>, mut player: ResMut<Player>) {
    if let Some(name) = menu.play_movie.take() {
        player.queue.push_back(name);
        menu.open = false;
    }
}

#[allow(clippy::too_many_arguments)]
fn play(
    mut commands: Commands,
    mut player: ResMut<Player>,
    config: Res<crate::config::Config>,
    mut images: ResMut<Assets<Image>>,
    assets: Res<AssetServer>,
    keys: Res<ButtonInput<KeyCode>>,
    nav: Option<Res<crate::customiser::Navigation>>,
    mut time: ResMut<Time<Virtual>>,
) {
    let skip = keys.get_just_pressed().next().is_some() || nav.is_some_and(|n| n.pressed != 0);
    if let Some(playback) = player.current.as_mut() {
        // Show the frame due now, dropping any the decoder delivered late.
        let due = (playback.start.elapsed().as_secs_f32() * FPS) as u64;
        let mut latest = None;
        let mut finished = false;
        while playback.shown < due {
            match playback.frames.lock().unwrap().try_recv() {
                Ok(frame) => { latest = Some(frame); playback.shown += 1; }
                Err(mpsc::TryRecvError::Empty) => break,
                Err(mpsc::TryRecvError::Disconnected) => { finished = true; break; }
            }
        }
        if let (Some(frame), Some(image)) = (latest, images.get_mut(&playback.image)) {
            image.data = Some(frame);
        }
        if skip || finished {
            let done = player.current.take().unwrap();
            commands.entity(done.root).despawn();
            if let Some(audio) = done.audio { commands.entity(audio).despawn(); }
            if done.resume && player.queue.is_empty() { time.unpause(); }
            if skip { player.queue.clear(); if done.resume { time.unpause(); } }
        }
        return;
    }
    let Some(name) = player.queue.pop_front() else { return };
    let video = directory(&config).join(format!("{name}.vp6"));
    if !video.is_file() { return; }
    let child = Command::new("ffmpeg")
        .args(["-v", "error", "-i"]).arg(&video)
        .args(["-vf", &format!("scale={WIDTH}:{HEIGHT}"), "-f", "rawvideo", "-pix_fmt", "rgba", "-"])
        .stdout(Stdio::piped()).stderr(Stdio::null()).spawn();
    let Ok(mut child) = child else {
        warn!("Movies need ffmpeg installed to play {name}");
        player.queue.clear();
        return;
    };
    let mut stdout = child.stdout.take().unwrap();
    let (sender, frames) = mpsc::sync_channel::<Vec<u8>>(6);
    std::thread::spawn(move || {
        let size = (WIDTH * HEIGHT * 4) as usize;
        loop {
            let mut frame = vec![0u8; size];
            if stdout.read_exact(&mut frame).is_err() || sender.send(frame).is_err() { break; }
        }
    });
    let image = images.add(Image::new_fill(
        Extent3d { width: WIDTH, height: HEIGHT, depth_or_array_layers: 1 },
        TextureDimension::D2, &[0, 0, 0, 255], TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::MAIN_WORLD | RenderAssetUsages::RENDER_WORLD,
    ));
    let root = commands.spawn((
        GlobalZIndex(1000),
        Node { position_type: PositionType::Absolute, width: percent(100), height: percent(100),
            align_items: AlignItems::Center, justify_content: JustifyContent::Center, ..default() },
        BackgroundColor(Color::BLACK),
    )).with_children(|screen| {
        screen.spawn((ImageNode::new(image.clone()), Node { width: percent(100), aspect_ratio: Some(16.0 / 9.0), max_height: percent(100), ..default() }));
    }).id();
    let soundtrack = directory(&config).join(format!("{name}.ogg"));
    let audio = soundtrack.is_file().then(|| commands.spawn((
        AudioPlayer::new(assets.load::<AudioSource>(format!("private/movies/{name}.ogg"))),
        PlaybackSettings::DESPAWN,
    )).id());
    let resume = !time.is_paused();
    time.pause();
    player.current = Some(Playback { child, frames: std::sync::Mutex::new(frames), image, root, audio, start: Instant::now(), shown: 0, resume });
}
