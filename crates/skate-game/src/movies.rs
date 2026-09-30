//! The disc's FMV movies (tools/prepare_movies.py): the EA logo at start-up,
//! the story intro on first launch, and any movie from the pause menu.
//!
//! Video is EA VP6 decoded by ffmpeg into raw frames on a reader thread; the
//! soundtrack plays from the Ogg extracted beside it. Any key or button skips.
use bevy::{asset::RenderAssetUsages, prelude::*, render::render_resource::{Extent3d, TextureDimension, TextureFormat}};
use std::{io::Read, path::PathBuf, process::{Child, Command, Stdio}, sync::mpsc, time::Instant};

/// (file stem, menu title) in the order the menu lists them.
const SKATE3: [(&str, &str); 11] = [
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
/// Skate 2's disc movies (prepared into private/skate2/movies).
const SKATE2: [(&str, &str); 40] = [
    ("intro_movie", "Story intro"),
    ("Skate2_EA_Intro", "EA intro"),
    ("Blackbox_bumper", "EA Black Box"),
    ("RedaCarRide", "Reda's car ride"),
    ("Service_BB01", "Big Black"),
    ("Service_Drain01", "Service: drain"),
    ("Service_DrainDam01", "Service: drain dam"),
    ("Service_Uncap01", "Service: uncapped"),
    ("HOM", "Hall of Meat"),
    ("Unlock_DannyMegaPark", "Unlock: Danny Way's Mega Park"),
    ("Unlock_GVR", "Unlock: GVR"),
    ("Unlock_KingOfMountain", "Unlock: King of the Mountain"),
    ("Unlock_PublicSkatePark", "Unlock: Public skatepark"),
    ("Unlock_Stadium", "Unlock: Stadium"),
    ("Unlock_TopOfDam", "Unlock: Top of the dam"),
    ("Tutorial_Skate01", "Tutorial 1"),
    ("Tutorial_Skate02", "Tutorial 2"),
    ("Tutorial_Skate03", "Tutorial 3"),
    ("Tutorial_Skate04", "Tutorial 4"),
    ("Tutorial_Skate05", "Tutorial 5"),
    ("Tutorial_Skate06", "Tutorial 6"),
    ("Tutorial_Map", "Tutorial: map"),
    ("Tutorial_CreateASpot", "Tutorial: create a spot"),
    ("Tutorial_BasicReplay", "Tutorial: replay editor"),
    ("SponsorTraining01", "Sponsor training 1"),
    ("SponsorTraining02", "Sponsor training 2"),
    ("SponsorTraining03", "Sponsor training 3"),
    ("SponsorTraining04", "Sponsor training 4"),
    ("SponsorTraining05", "Sponsor training 5"),
    ("SponsorTraining06", "Sponsor training 6"),
    ("SponsorTraining07", "Sponsor training 7"),
    ("SponsorTraining08", "Sponsor training 8"),
    ("SponsorTraining09", "Sponsor training 9"),
    ("SponsorTraining10", "Sponsor training 10"),
    ("SponsorTraining11", "Sponsor training 11"),
    ("SponsorTraining12", "Sponsor training 12"),
    ("SponsorTraining13", "Sponsor training 13"),
    ("SponsorTraining14", "Sponsor training 14"),
    ("Credits", "Credits"),
    ("Attract", "Attract loop"),
];

/// A movie of the running edition: (file stem, menu title, source game).
#[derive(Clone)]
pub(crate) struct Movie { pub name: &'static str, pub title: String, skate2: bool }

/// Movies for the pause menu. Freeskate lists both games' movies.
pub(crate) fn list() -> Vec<Movie> {
    use crate::editions::{current, Game};
    let edition = current();
    let mut movies = Vec::new();
    if edition.shows(Some(Game::Skate3)) {
        movies.extend(SKATE3.iter().map(|&(name, title)| Movie { name, title: title.into(), skate2: false }));
    }
    if edition.shows(Some(Game::Skate2)) {
        let suffix = edition == crate::editions::Edition::Freeskate;
        movies.extend(SKATE2.iter().map(|&(name, title)| Movie { name,
            title: if suffix { format!("{title} (Skate 2)") } else { title.into() }, skate2: true }));
    }
    movies
}
impl Movie {
    /// Identifier the menu queues: Skate 2 movies carry a "skate2/" prefix.
    pub fn id(&self) -> String { if self.skate2 { format!("skate2/{}", self.name) } else { self.name.into() } }
}

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

/// Asset path (under the asset root, without extension) of a queued movie id.
fn asset(id: &str) -> String {
    match id.strip_prefix("skate2/") {
        Some(name) => format!("private/skate2/movies/{name}"),
        None => format!("private/movies/{id}"),
    }
}
fn seen_path(config: &crate::config::Config, skate2: bool) -> PathBuf {
    let settings = config.asset_root.parent().unwrap_or(&config.asset_root).join("settings");
    settings.join(if skate2 { "movies-seen-skate2" } else { "movies-seen" })
}

/// Start-up: the EA logo, then the story intro once per installation, from
/// the running edition's disc (Freeskate plays Skate 3's).
fn boot(mut player: ResMut<Player>, config: Res<crate::config::Config>) {
    // Test hook: SKATE_DEBUG_MOVIE=<name> plays that movie, even under verification.
    if let Ok(name) = std::env::var("SKATE_DEBUG_MOVIE") { player.queue.push_back(name); return; }
    if std::env::var_os("SKATE_VERIFY_DELAY").is_some() || std::env::var_os("SKATE_PERF_REPORT").is_some() { return; }
    let skate2 = crate::editions::current() == crate::editions::Edition::Skate2;
    let (logos, intro): (&[&str], _) = if skate2 { (&["skate2/Skate2_EA_Intro", "skate2/Blackbox_bumper"], "skate2/intro_movie") }
        else { (&["EA_Blackbox"], "intro_movie") };
    if !config.asset_root.join(format!("{}.vp6", asset(logos[0]))).is_file() { return; }
    player.queue.extend(logos.iter().map(|&logo| logo.to_owned()));
    if !seen_path(&config, skate2).is_file() {
        player.queue.push_back(intro.into());
        let _ = std::fs::write(seen_path(&config, skate2), b"intro\n");
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
    let video = config.asset_root.join(format!("{}.vp6", asset(&name)));
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
    let soundtrack = config.asset_root.join(format!("{}.ogg", asset(&name)));
    let audio = soundtrack.is_file().then(|| commands.spawn((
        AudioPlayer::new(assets.load::<AudioSource>(format!("{}.ogg", asset(&name)))),
        PlaybackSettings::DESPAWN,
    )).id());
    let resume = !time.is_paused();
    time.pause();
    player.current = Some(Playback { child, frames: std::sync::Mutex::new(frames), image, root, audio, start: Instant::now(), shown: 0, resume });
}
