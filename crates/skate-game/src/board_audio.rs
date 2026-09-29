//! Skateboard sounds from the disc banks prepared by tools/prepare_audio.py.
//! `board.json` maps events to clips. Loops run continuously and are faded by
//! board state and speed; one-shots fire on physical state transitions.
use bevy::{
    audio::{AudioSinkPlayback, Volume},
    prelude::*,
};
use skate_core::player::state::PhysicalStateId as State;
use std::collections::HashMap;

const DIRECTORY: &str = "private/audio/board";
/// Board speed (m/s) at which rolling reaches full volume.
const FULL_SPEED: f32 = 9.0;
/// Seconds between powerslide skid grains.
const SKID_INTERVAL: f32 = 0.07;

#[derive(serde::Deserialize)]
struct Sound {
    clips: Vec<String>,
    #[serde(default = "full")]
    volume: f32,
}
fn full() -> f32 {
    1.0
}

#[derive(Resource)]
struct Bank {
    sounds: HashMap<String, (Vec<Handle<AudioSource>>, f32)>,
    /// Every clip decoded in memory (pcm_audio.rs). Each compressed play built
    /// a new decoder on the main thread (a landing's five sounds cost a frame
    /// ~9 ms), and the wheel spin-downs, started partway in, first decoded and
    /// discarded everything before their start.
    decoded: HashMap<AssetId<AudioSource>, crate::pcm_audio::Slot>,
    seed: u64,
    /// Menu board volume, refreshed every frame.
    gain: f32,
}
impl Bank {
    fn random(&mut self) -> f32 {
        // xorshift64: variety for clip choice and pitch, no crate needed.
        self.seed ^= self.seed << 13;
        self.seed ^= self.seed >> 7;
        self.seed ^= self.seed << 17;
        (self.seed >> 40) as f32 / (1u64 << 24) as f32
    }
    /// A wheel spin-down from `start` into its clip, faded by the caller.
    fn play_from(&mut self, commands: &mut Commands, clips_in_memory: &mut Assets<crate::pcm_audio::PcmClip>,
                 name: &str, gain: f32, start: std::time::Duration) -> Option<Entity> {
        let (clips, volume) = self.sounds.get(name)?;
        let clip = clips.first()?.clone();
        let volume = volume * gain;
        let settings = PlaybackSettings::DESPAWN.with_volume(Volume::Linear(volume * self.gain));
        if let Some(pcm) = self.decoded.get(&clip.id()).and_then(crate::pcm_audio::ready) {
            let player = AudioPlayer(clips_in_memory.add(pcm.from(start)));
            return Some(commands.spawn((WheelSpin { fade: None, volume }, player, settings)).id());
        }
        // Still decoding: seek the compressed clip.
        let mut settings = settings;
        settings.start_position = Some(start);
        Some(commands.spawn((WheelSpin { fade: None, volume }, AudioPlayer::new(clip), settings)).id())
    }
    fn play(&mut self, commands: &mut Commands, clips_in_memory: &mut Assets<crate::pcm_audio::PcmClip>, name: &str, gain: f32) {
        let gain = gain * self.gain;
        let Some((clips, volume)) = self.sounds.get(name) else { return };
        if clips.is_empty() {
            return;
        }
        let (clips, volume) = (clips.clone(), *volume);
        let clip = clips[(self.random() * clips.len() as f32) as usize % clips.len()].clone();
        if std::env::var("SKATE_DEBUG_AUDIO").is_ok() { info!("BOARD_SOUND {name} gain={gain:.2}"); }
        let speed = 0.94 + self.random() * 0.12;
        let settings = PlaybackSettings::DESPAWN.with_volume(Volume::Linear(volume * gain)).with_speed(speed);
        if let Some(pcm) = self.decoded.get(&clip.id()).and_then(crate::pcm_audio::ready) {
            commands.spawn((OneShot, AudioPlayer(clips_in_memory.add(pcm.from(std::time::Duration::ZERO))), settings));
        } else {
            commands.spawn((OneShot, AudioPlayer::new(clip), settings));
        }
    }
}

#[derive(Component)]
struct OneShot;

#[derive(Component)]
struct Loop {
    name: &'static str,
    volume: f32,
    level: f32,
}

const LOOPS: [&str; 6] = ["rattle", "grind_trucks", "grind_board", "wind", "body_slide", "body_slide_soft"];

/// One rolling loop per surface family, cross-faded as the wheels move from one
/// surface to another. `board.json` may name its own clips for these; the
/// defaults are the bank's six rolling loops matched by ear (loudness,
/// brightness, bumpiness), with the plain "roll" entry as smooth concrete.
const SURFACE_LOOPS: [(&str, &str); 7] = [
    ("roll_smooth", "PatchBank_Rolling_Surfaces/2.ogg"),
    ("roll_rough", "PatchBank_Rolling_Surfaces/3.ogg"),
    ("roll_bumpy", "PatchBank_Rolling_Surfaces/4.ogg"),
    ("roll_wood", "PatchBank_Rolling_Surfaces/5.ogg"),
    ("roll_dirt", "PatchBank_Rolling_Surfaces/6.ogg"),
    ("roll_grass", "PatchBank_Rolling_Surfaces/7.ogg"),
    ("roll_metal", "PatchBank_Rolling_Surfaces/1.ogg"),
];
/// The retail rolling layers (grains.big, tools/prepare_audio.py
/// prepare_rolling): a slow ("soft") and fast ("hard") recording per surface,
/// cross-faded by speed. They replace SURFACE_LOOPS' clips when installed;
/// the "_hard" loop is its family's fast layer.
const GRAIN_LOOPS: [(&str, &str, &str, &str); 5] = [
    ("roll_smooth", "roll_smooth_hard", "Grains/asphalt_smooth_soft.ogg", "Grains/asphalt_smooth_hard.ogg"),
    ("roll_rough", "roll_rough_hard", "Grains/concrete_rough_soft.ogg", "Grains/concrete_rough_hard.ogg"),
    ("roll_bumpy", "roll_bumpy_hard", "Grains/asphalt_rough_soft.ogg", "Grains/asphalt_rough_hard.ogg"),
    ("roll_wood", "roll_wood_hard", "Grains/wood_ramp_soft.ogg", "Grains/wood_ramp_hard.ogg"),
    ("roll_metal", "roll_metal_hard", "Grains/metal_smooth_hard.ogg", "Grains/metal_smooth_hard.ogg"),
];
/// Free-spinning wheels winding down (wheels.big): after leaving the ground,
/// and the lifted pair in a manual. Played from the point matching the
/// wheels' speed.
const WHEEL_SPINS: [(&str, &str); 2] = [("spin_jump", "Wheel_Spins/jump.ogg"), ("spin_manual", "Wheel_Spins/manual.ogg")];
/// Seconds of a wheel-spin clip that span full speed to stopped.
const SPIN_SPAN: f32 = 12.0;

#[derive(Component)]
struct WheelSpin {
    /// Fading out (seconds of fade left), else playing.
    fade: Option<f32>,
    volume: f32,
}

/// Body-on-ground impacts (tools/prepare_audio.py BODY_MAP), chosen by how
/// hard a ragdoll body part hits: (name, clip numbers, volume).
const BODY_IMPACTS: [(&str, &str, &[u32], f32); 14] = [
    ("body_light", "Body_Impacts", &[1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12], 0.9),
    ("body_medium", "Body_Impacts", &[16, 17, 18, 20, 21, 23, 24, 26, 27, 28, 29, 30], 1.0),
    ("body_heavy", "Body_Impacts", &[36, 37, 38, 39, 43, 44, 45, 46, 47, 48], 1.0),
    ("body_flesh", "Body_Flesh", &[1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 13, 14, 15, 16, 18, 20, 21, 23, 25,
        26, 27, 30, 31, 32, 33, 34, 35, 36, 37, 38, 39, 40, 41, 44, 47, 52, 53, 55], 0.7),
    ("body_metal", "Body_Metal", &[5, 6, 7, 29, 30, 51, 52, 54, 129, 189, 260, 267, 268, 273], 0.9),
    // Hall of Meat bone cracks (the sharpest, brightest non-voiced hits).
    ("body_bone", "Body_Bones", &[8, 14, 15, 30, 37, 49, 52], 0.9),
    // Board landings by surface, from the "land" thunks: the deep, hollow
    // booms are ramp landings; only the two bright slaps suit the street.
    ("land_wood", "PatchBank_Rolling_Surfaces", &[8, 9, 10, 11, 12, 13, 14], 1.0),
    ("land_hard", "PatchBank_Rolling_Surfaces", &[15, 16], 1.0),
    // Tail snaps and scrapes by surface: the board_scrapes bank's bright half
    // (concrete, metal) and darker half (wood, dirt).
    ("pop_hard", "board_scrapes", &[1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15], 1.0),
    ("pop_wood", "board_scrapes", &[16, 17, 18, 19, 20, 21, 22, 23, 24, 25, 26, 27, 28, 29, 30], 1.0),
    // Wheel skids by surface (retail wheelskid_smth/ruff/wood/dirt_grass),
    // grouped from the bank by brightness.
    ("skid_smooth", "WHEEL_SKID_BANK", &[57, 58, 59, 60, 61, 62, 63, 64, 65, 66, 67, 68, 69, 70, 71, 72], 0.9),
    ("skid_rough", "WHEEL_SKID_BANK", &[1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20], 0.9),
    ("skid_wood", "WHEEL_SKID_BANK", &[73, 74, 75, 76, 77, 78, 79, 80, 81, 82, 83, 84, 85, 86, 87, 88], 0.9),
    ("skid_soft", "WHEEL_SKID_BANK", &[49, 50, 51, 52, 53, 54, 55, 56], 0.8),
];
/// Extra retail banks (tools/prepare_audio.py BOARD_BANKS) with no board.json
/// entry in older installations: (event, bank directory, clip range, volume).
const EXTRA_SOUNDS: [(&str, &str, std::ops::RangeInclusive<u32>, f32); 9] = [
    ("seam", "Seams_Bank", 1..=60, 0.35),
    ("squeak", "Brd_Squeaks", 1..=12, 0.35),
    ("cloth", "Foley_Cloth", 1..=16, 0.35),
    ("footstep", "fstep_skateshoe1_sm", 1..=28, 0.6),
    ("wind", "sense_of_speed", 1..=1, 0.5),
    // Short swishes, one per board revolution during flip tricks.
    ("flip_spin", "Sk8_Air_Flip_Tricks", 5..=14, 0.5),
    // A body sliding along the ground in a bail: the Bodyslide bank's steady
    // loops (bright concrete, darker dirt), and its short gritty scrapes.
    ("body_slide", "Bodyslide", 1..=1, 0.8),
    ("body_slide_soft", "Bodyslide", 11..=11, 0.7),
    ("body_scrape", "Bodyslide", 2..=5, 0.6),
];
/// Pavement joint spacing (m): one seam clack per this much travel on concrete.
const SEAM_SPACING: f32 = 2.2;
/// Board speed (m/s) where rushing wind starts to be heard.
const WIND_SPEED: f32 = 6.0;
/// Ragdoll part 1 is the head (skeleton record order).
const HEAD_PART: usize = 1;
/// Seconds the wheels take to spin down after leaving the ground.
const WHEEL_SPIN_DOWN: f32 = 1.5;
/// Falling speed (m/s) that earns the big-drop whoosh: about a 3 m fall.
const DROP_WHOOSH_SPEED: f32 = 7.5;
/// Board spin (rad/s) in the air that counts as a flip trick starting.
const FLIP_SPIN: f32 = 10.0;
/// Speed (m/s) a body part must lose for a bone crack on hard ground.
const BONE_SPEED: f32 = 8.0;
/// Speed (m/s) a body part must lose in one step to count as hitting something.
const IMPACT_SPEED: f32 = 1.8;

/// Rolling family for a retail audio surface (the low 7 bits of a collision
/// triangle's packed surface; names from the map tools' surface table).
fn surface_family(audio: u16) -> &'static str {
    match audio {
        2 | 65 | 66 => "roll_bumpy",              // rough asphalt, brick
        4 | 5 | 53 | 54 => "roll_rough",          // rough/aggregate concrete, curbs
        6 | 7 | 41..=46 | 90 => "roll_wood",      // ramps, plywood, wood
        8 | 55 | 56 | 70 | 71 => "roll_dirt",     // dirt, leaves, snow
        10 | 77 => "roll_grass",
        9 | 11..=40 | 67..=69 | 85 | 89 | 91 => "roll_metal",
        _ => "roll_smooth",                        // smooth asphalt, polished concrete, tile
    }
}

#[derive(Default)]
struct Tracker {
    state: Option<State>,
    fall_speed: f32,
    skid_timer: f32,
    /// Each ragdoll body part's velocity last step, and per-part and overall
    /// cooldowns so one landing is not a burst of thuds.
    body_velocities: Vec<Vec3>,
    part_cooldowns: Vec<f32>,
    thud_cooldown: f32,
    /// Board was already spinning fast last step (one whoosh per flip).
    spinning: bool,
    /// The wheel spin-down playing (entity, kind), and how long only one
    /// pair of wheels has been touching (a manual).
    spin: Option<(Entity, &'static str)>,
    manual_time: f32,
    /// Time until the next deck scrape or board knock.
    drag_timer: f32,
    /// The big-drop whoosh has played this air.
    drop_whoosh: bool,
    /// Board rotation (rad) since the last flip swish, and swishes this air.
    flip_turn: f32,
    flip_swishes: u32,
    /// Seconds until another bone crack may play.
    bone_cooldown: f32,
    /// Fastest ground-contact slide of a ragdoll part (m/s), its surface
    /// family, and time to the next scrape burst.
    body_slide: f32,
    slide_family: &'static str,
    scrape_timer: f32,
    /// Wheel speed carried into the air (0..1), spinning down while airborne.
    wheel_spin: f32,
    /// Distance rolled since the last pavement seam, and walked since the
    /// last footstep.
    seam_travel: f32,
    step_travel: f32,
    last_walk: Option<Vec3>,
    /// Rolling family under the wheels (kept while airborne), and a
    /// challenger with how long it has been winning.
    surface: &'static str,
    pending: (&'static str, f32),
}

pub(crate) struct BoardAudioPlugin;
impl Plugin for BoardAudioPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, load).add_systems(Update, update);
    }
}

fn load(mut commands: Commands, config: Res<crate::config::Config>, assets: Res<AssetServer>) {
    let path = config.asset_root.join(DIRECTORY).join("board.json");
    let Ok(bytes) = std::fs::read(&path) else {
        info!("Board sounds not installed ({})", path.display());
        return;
    };
    let table: HashMap<String, Sound> = match serde_json::from_slice(&bytes) {
        Ok(table) => table,
        Err(error) => {
            warn!("Board sounds disabled: {} is invalid: {error}", path.display());
            return;
        }
    };
    let mut sounds = HashMap::new();
    // board.json names a player may have remapped (grains then stay out).
    let authored: std::collections::HashSet<String> = table.keys().cloned().collect();
    for (name, sound) in table {
        let clips: Vec<_> = sound
            .clips
            .iter()
            .filter(|clip| config.asset_root.join(DIRECTORY).join(clip).is_file())
            .map(|clip| assets.load::<AudioSource>(format!("{DIRECTORY}/{clip}")))
            .collect();
        if clips.len() < sound.clips.len() {
            warn!("Board sound {name}: {} clip(s) missing", sound.clips.len() - clips.len());
        }
        sounds.insert(name, (clips, sound.volume));
    }
    // Older installations only have "roll"; it doubles as smooth concrete and
    // lends its volume to the other families.
    let roll = sounds.remove("roll");
    let roll_volume = roll.as_ref().map_or(0.9, |(_, volume)| *volume);
    if let Some(roll) = roll.filter(|(clips, _)| !clips.is_empty()) {
        sounds.entry("roll_smooth".into()).or_insert(roll);
    }
    for (name, clip) in SURFACE_LOOPS {
        if !sounds.contains_key(name) && config.asset_root.join(DIRECTORY).join(clip).is_file() {
            sounds.insert(name.into(), (vec![assets.load::<AudioSource>(format!("{DIRECTORY}/{clip}"))], roll_volume));
        }
    }
    // Installations set up before body impacts existed have the clips but no
    // board.json entries; fall back to the setup's own layout.
    for (name, directory, numbers, volume) in BODY_IMPACTS {
        if sounds.contains_key(name) { continue; }
        let clips: Vec<_> = numbers.iter().map(|n| format!("{directory}/{n}.ogg"))
            .filter(|clip| config.asset_root.join(DIRECTORY).join(clip).is_file())
            .map(|clip| assets.load::<AudioSource>(format!("{DIRECTORY}/{clip}")))
            .collect();
        if !clips.is_empty() { sounds.insert(name.into(), (clips, volume)); }
    }
    for (name, directory, numbers, volume) in EXTRA_SOUNDS {
        if sounds.contains_key(name) { continue; }
        let clips: Vec<_> = numbers.map(|n| format!("{directory}/{n}.ogg"))
            .filter(|clip| config.asset_root.join(DIRECTORY).join(clip).is_file())
            .map(|clip| assets.load::<AudioSource>(format!("{DIRECTORY}/{clip}")))
            .collect();
        if !clips.is_empty() { sounds.insert(name.into(), (clips, volume)); }
    }
    // Short board rattles on pop (the AEMS "Ollie_Rattles" class).
    if !sounds.contains_key("ollie_rattle") {
        let clips: Vec<_> = (1..=4).map(|n| format!("Sk8_Air_Flip_Tricks/{n}.ogg"))
            .filter(|clip| config.asset_root.join(DIRECTORY).join(clip).is_file())
            .map(|clip| assets.load::<AudioSource>(format!("{DIRECTORY}/{clip}")))
            .collect();
        if !clips.is_empty() { sounds.insert("ollie_rattle".into(), (clips, 0.5)); }
    }
    // Retail grains replace the rolling clips where installed.
    for (soft, hard, soft_clip, hard_clip) in GRAIN_LOOPS {
        if authored.contains(soft) { continue; }
        let (a, b) = (config.asset_root.join(DIRECTORY).join(soft_clip), config.asset_root.join(DIRECTORY).join(hard_clip));
        if a.is_file() && b.is_file() {
            sounds.insert(soft.into(), (vec![assets.load::<AudioSource>(format!("{DIRECTORY}/{soft_clip}"))], roll_volume));
            sounds.insert(hard.into(), (vec![assets.load::<AudioSource>(format!("{DIRECTORY}/{hard_clip}"))], roll_volume));
        }
    }
    for (name, clip) in WHEEL_SPINS {
        if !sounds.contains_key(name) && config.asset_root.join(DIRECTORY).join(clip).is_file() {
            sounds.insert(name.into(), (vec![assets.load::<AudioSource>(format!("{DIRECTORY}/{clip}"))], 0.8));
        }
    }
    for name in LOOPS.into_iter().chain(SURFACE_LOOPS.map(|(name, _)| name)).chain(GRAIN_LOOPS.map(|(_, hard, _, _)| hard)) {
        if let Some((clips, volume)) = sounds.get(name).filter(|(clips, _)| !clips.is_empty()) {
            commands.spawn((
                Loop { name, volume: *volume, level: 0.0 },
                AudioPlayer::new(clips[0].clone()),
                PlaybackSettings::LOOP.with_volume(Volume::Linear(0.0)),
            ));
        }
    }
    info!("Board sounds: {} events", sounds.len());
    let seed = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0x9E37_79B9_7F4A_7C15, |d| d.as_nanos() as u64)
        | 1;
    // One-shots and wheel spins; the loops start once and never again.
    let continuous: Vec<&str> = LOOPS.into_iter().chain(SURFACE_LOOPS.map(|(name, _)| name))
        .chain(GRAIN_LOOPS.map(|(soft, _, _, _)| soft)).chain(GRAIN_LOOPS.map(|(_, hard, _, _)| hard)).collect();
    let mut clips: Vec<(AssetId<AudioSource>, std::path::PathBuf)> = sounds.iter()
        .filter(|(name, _)| !continuous.contains(&name.as_str()))
        .flat_map(|(_, (handles, _))| handles.iter())
        .filter_map(|h| Some((h.id(), config.asset_root.join(h.path()?.path()))))
        .collect();
    clips.sort_by(|a, b| a.1.cmp(&b.1));
    clips.dedup_by(|a, b| a.0 == b.0);
    let slots = crate::pcm_audio::decode_in_background(clips.iter().map(|(_, path)| path.clone()).collect());
    let decoded = clips.into_iter().map(|(id, _)| id).zip(slots).collect();
    commands.insert_resource(Bank { sounds, decoded, seed, gain: 1.0 });
}

/// The fast rolling layer of a surface family, if it has one.
fn hard_of(surface: &str) -> Option<&'static str> {
    GRAIN_LOOPS.iter().find(|(soft, ..)| *soft == surface).map(|(_, hard, ..)| *hard)
}

fn rolling(state: State) -> bool {
    matches!(state, State::PhysicsGround | State::RevertGround | State::GroundAnimation | State::SlideGround)
}
fn airborne(state: State) -> bool {
    matches!(state, State::PhysicsAir | State::KnownAir | State::PhysicsAirSecondary)
}
fn board_slide(state: State) -> bool {
    matches!(state, State::GrindBoardslide | State::GrindTipslide | State::GrindDarkslide)
}

fn update(
    // Real time: the menu pauses virtual time, which would freeze the fades.
    time: Res<Time<Real>>,
    bank: Option<ResMut<Bank>>,
    physics: Res<crate::physics::GamePhysics>,
    skater: Res<crate::physics::SkaterRuntime>,
    menu: Option<Res<crate::graphics_menu::Menu>>,
    mut loops: Query<(&mut Loop, &mut AudioSink)>,
    one_shots: Query<&AudioSink, (With<OneShot>, Without<Loop>, Without<WheelSpin>)>,
    mut spins: Query<(Entity, &mut WheelSpin, &mut AudioSink), Without<Loop>>,
    mut commands: Commands,
    mut tracker: Local<Tracker>,
    mut clips_in_memory: ResMut<Assets<crate::pcm_audio::PcmClip>>,
) {
    let Some(mut bank) = bank else { return };
    let dt = time.delta_secs();
    bank.gain = menu.as_ref().map_or(1.0, |m| m.audio_gain(crate::graphics_menu::AudioChannel::Board));
    let active = crate::graphics_menu::gameplay_active(menu);
    let state = skater.player_state.current();
    let bodies = physics.board.bodies();
    let velocity = bodies
        .iter()
        .fold(Vec3::ZERO, |sum, b| {
            let v = b.rates.linear_velocity;
            sum + Vec3::new(v.x, v.y, v.z)
        })
        / bodies.len() as f32;
    let speed = velocity.length();
    let spin = bodies.iter().map(|b| { let w = b.rates.angular_velocity; Vec3::new(w.x, w.y, w.z).length() })
        .fold(0.0f32, f32::max);
    let intensity = (speed / FULL_SPEED).clamp(0.0, 1.0);
    // During a wipeout the board flies off on its own; bail audio follows the
    // skater's body instead.
    let body = skater.skeleton.bodies();
    // Most common audio surface among the board's contacts this step.
    let mut counts: HashMap<&'static str, usize> = HashMap::new();
    for report in physics.board.contact_reports() {
        *counts.entry(surface_family(report.other_surface & 0x7f)).or_default() += 1;
    }
    // What the wheels touch this very step (landings sound of it, not of the
    // surface kept through the air).
    let touched = counts.iter().max_by_key(|(_, n)| **n).map(|(f, _)| *f);
    // Ties keep the current surface, so a board straddling two surfaces
    // does not flip between loops every step.
    let current = counts.get(tracker.surface).copied().unwrap_or(0);
    match counts.into_iter().filter(|(_, n)| *n > current).max_by_key(|(_, n)| *n) {
        Some((family, _)) if tracker.surface.is_empty() => tracker.surface = family,
        Some((family, _)) => {
            if tracker.pending.0 != family { tracker.pending = (family, 0.0); }
            tracker.pending.1 += dt;
            // A surface must win for a moment before the loops cross-fade.
            if tracker.pending.1 > 0.15 {
                if std::env::var("SKATE_DEBUG_SURFACE").is_ok() { info!("BOARD_SURFACE {family}"); }
                tracker.surface = family;
            }
        }
        None => tracker.pending.1 = 0.0,
    }
    let surface = if tracker.surface.is_empty() { "roll_smooth" } else { tracker.surface };

    if active {
        if let Some(previous) = tracker.state.filter(|&previous| previous != state) {
            if airborne(state) && (rolling(previous) || previous.is_grind()) {
                let under = touched.unwrap_or(surface);
                bank.play(&mut commands, &mut clips_in_memory, if matches!(under, "roll_wood" | "roll_dirt" | "roll_grass") { "pop_wood" } else { "pop_hard" }, 1.0);
                bank.play(&mut commands, &mut clips_in_memory, "ollie_rattle", 0.4 + 0.6 * intensity);
                bank.play(&mut commands, &mut clips_in_memory, "cloth", 0.5 + 0.5 * intensity);
                tracker.wheel_spin = intensity;
            } else if airborne(previous) && rolling(state) {
                // Harder landings are louder; 6 m/s downward is a big drop.
                let gain = (0.45 + tracker.fall_speed / 6.0).min(1.3);
                let under = touched.unwrap_or(surface);
                match under {
                    "roll_wood" => bank.play(&mut commands, &mut clips_in_memory, "land_wood", gain),
                    "roll_grass" | "roll_dirt" => bank.play(&mut commands, &mut clips_in_memory, "land_wood", gain * 0.5),
                    _ => {
                        // Street and sidewalk: a slap and the tail's clack, not a ramp boom.
                        bank.play(&mut commands, &mut clips_in_memory, "land_hard", gain * 0.8);
                        bank.play(&mut commands, &mut clips_in_memory, "pop_hard", gain * 0.5);
                    }
                }
                // Metal rings under the thunk.
                if under == "roll_metal" { bank.play(&mut commands, &mut clips_in_memory, "body_metal", gain * 0.4); }
                // Some landings flex the deck enough to squeak.
                if tracker.fall_speed > 3.0 && bank.random() < 0.5 { bank.play(&mut commands, &mut clips_in_memory, "squeak", 0.6); }
            } else if state.is_grind() && !previous.is_grind() {
                bank.play(&mut commands, &mut clips_in_memory, "grind_enter", 1.0);
            }
            // No generic bail sound: a wipeout is heard only through the body
            // hitting things (below).
        }
        // Flip tricks: a short swish (and cloth) as the board starts spinning.
        let spinning = airborne(state) && spin > FLIP_SPIN;
        if spinning && !tracker.spinning && !physics.board_wiping_out {
            bank.play(&mut commands, &mut clips_in_memory, "flip_spin", (0.4 + (spin - FLIP_SPIN) / 30.0).min(0.9));
            bank.play(&mut commands, &mut clips_in_memory, "cloth", 0.6);
        }
        // Big drops: the deep whoosh of falling fast, once per air, just
        // before the landing (retail's pre-land whoosh class).
        if !airborne(state) {
            tracker.drop_whoosh = false;
        } else if !tracker.drop_whoosh && tracker.fall_speed > DROP_WHOOSH_SPEED {
            tracker.drop_whoosh = true;
            bank.play(&mut commands, &mut clips_in_memory, "flip", (0.5 + (tracker.fall_speed - DROP_WHOOSH_SPEED) / 8.0).min(1.2));
        }
        // A swish every full turn of the board while it keeps spinning.
        if !airborne(state) { tracker.flip_swishes = 0; }
        if spinning && !physics.board_wiping_out && tracker.flip_swishes < 3 {
            tracker.flip_turn += spin * dt;
            if tracker.flip_turn > std::f32::consts::TAU {
                tracker.flip_swishes += 1;
                tracker.flip_turn -= std::f32::consts::TAU;
                bank.play(&mut commands, &mut clips_in_memory, "flip_spin", (0.35 + (spin - FLIP_SPIN) / 40.0).min(0.8));
            }
        } else {
            tracker.flip_turn = 0.0;
        }
        tracker.spinning = spinning || (airborne(state) && tracker.spinning && spin > FLIP_SPIN * 0.6);
        if state == State::SlideGround && speed > 1.0 {
            tracker.skid_timer -= dt;
            if tracker.skid_timer <= 0.0 {
                tracker.skid_timer = SKID_INTERVAL;
                let skid = match surface {
                    "roll_smooth" | "roll_metal" => "skid_smooth",
                    "roll_wood" => "skid_wood",
                    "roll_grass" | "roll_dirt" => "skid_soft",
                    _ => "skid_rough",
                };
                bank.play(&mut commands, &mut clips_in_memory, skid, 0.4 + 0.6 * intensity);
            }
        }
        // Body impacts: any ragdoll part that suddenly loses speed along the
        // way it was moving has hit the ground (or a wall). Per part, not the
        // average, which smooths a head or shoulder slam away.
        tracker.thud_cooldown -= dt;
        tracker.bone_cooldown -= dt;
        tracker.part_cooldowns.resize(body.len(), 0.0);
        // What each ragdoll part is touching this step: the audio surface of
        // its world contacts in the shared solve (as the board's own reports).
        let base = skate_core::physics::board_step::ATTACHED_REACTION_BASE as u32;
        let mut touching: HashMap<usize, u16> = HashMap::new();
        for contact in physics.board.solved_contacts() {
            let words = contact.words();
            let part = match (words[31], words[43]) {
                (a, u32::MAX) if a >= base && ((a - base) as usize) < body.len() => ((a - base) as usize, words[55] as u16),
                (u32::MAX, b) if b >= base && ((b - base) as usize) < body.len() => ((b - base) as usize, (words[55] >> 16) as u16),
                _ => continue,
            };
            touching.insert(part.0, part.1 & 0x7f);
        }
        let mut hardest: Option<(f32, usize)> = None;
        for (part, b) in body.iter().enumerate() {
            let v = b.rates.linear_velocity;
            let v = Vec3::new(v.x, v.y, v.z);
            tracker.part_cooldowns[part] -= dt;
            let Some(&before) = tracker.body_velocities.get(part) else { continue };
            let lost = before.length() - v.dot(before.normalize_or_zero());
            if state == State::WipeoutGround && lost > IMPACT_SPEED && tracker.part_cooldowns[part] <= 0.0 {
                // A head hit counts for a bit more.
                let lost = if part == HEAD_PART { lost * 1.3 } else { lost };
                if hardest.is_none_or(|(h, _)| lost > h) { hardest = Some((lost, part)); }
                tracker.part_cooldowns[part] = 0.3;
            }
        }
        // Sliding: the fastest part in ground contact, moving along it.
        tracker.body_slide = 0.0;
        if state == State::WipeoutGround {
            for (&part, &audio) in &touching {
                let Some(b) = body.get(part) else { continue };
                let v = b.rates.linear_velocity;
                let along = Vec3::new(v.x, 0.0, v.z).length();
                if along > tracker.body_slide {
                    tracker.body_slide = along;
                    tracker.slide_family = surface_family(audio);
                }
            }
        }
        if tracker.body_slide > 1.5 {
            tracker.scrape_timer -= dt;
            if tracker.scrape_timer <= 0.0 {
                tracker.scrape_timer = 0.25 + 0.3 * bank.random();
                bank.play(&mut commands, &mut clips_in_memory, "body_scrape", ((tracker.body_slide - 1.5) / 4.0).clamp(0.2, 0.9));
            }
        }
        tracker.body_velocities = body.iter().map(|b| { let v = b.rates.linear_velocity; Vec3::new(v.x, v.y, v.z) }).collect();
        if let (Some((hardest, part)), true) = (hardest, tracker.thud_cooldown <= 0.0) {
            // Surface under that part, else what the board last rolled on.
            let family = touching.get(&part).map_or(surface, |&s| surface_family(s));
            let soft = matches!(family, "roll_grass" | "roll_dirt");
            let (name, gain) = if hardest > 7.0 { ("body_heavy", 1.2) }
                else if hardest > 4.0 { ("body_medium", 1.0) } else { ("body_light", 0.6 + hardest / 10.0) };
            // Soft ground muffles the hit a step down.
            let name = if soft && name == "body_heavy" { "body_medium" } else if soft { "body_light" } else { name };
            bank.play(&mut commands, &mut clips_in_memory, name, if soft { gain * 0.7 } else { gain });
            if family == "roll_metal" {
                bank.play(&mut commands, &mut clips_in_memory, "body_metal", (0.5 + hardest / 10.0).min(1.2));
            }
            // Hard hits get the meat; soft ground muffles it.
            if hardest > 3.0 {
                let meat = (0.4 + (hardest - 3.0) / 8.0).min(1.0);
                bank.play(&mut commands, &mut clips_in_memory, "body_flesh", if soft { meat * 0.6 } else { meat });
            }
            // Brutal slams on hard ground crack bones now and then.
            if hardest > BONE_SPEED && !soft && tracker.bone_cooldown <= 0.0 && bank.random() < 0.6 {
                bank.play(&mut commands, &mut clips_in_memory, "body_bone", (0.6 + (hardest - BONE_SPEED) / 8.0).min(1.1));
                tracker.bone_cooldown = 1.2;
            }
            if std::env::var("SKATE_DEBUG_AUDIO").is_ok() {
                info!("BODY_IMPACT part={part} speed={hardest:.1} surface={family}");
            }
            tracker.thud_cooldown = 0.08;
        }
        // The deck itself on the ground: tail/nose drags while riding (a run
        // of scrapes), and the loose board clattering after a bail.
        use skate_core::physics::board::BodyId;
        let mut drag: f32 = 0.0;
        let mut knock: f32 = 0.0;
        let mut knock_surface = surface;
        for report in physics.board.contact_reports() {
            let v = report.relative_linear_velocity;
            let v = Vec3::new(v.x, v.y, v.z);
            let n = Vec3::new(report.normal.x, report.normal.y, report.normal.z);
            if report.part == BodyId::Deck { drag = drag.max((v - n * v.dot(n)).length()); }
            if physics.board_wiping_out && v.dot(n).abs() > knock {
                knock = v.dot(n).abs();
                knock_surface = surface_family(report.other_surface & 0x7f);
            }
        }
        tracker.drag_timer -= dt;
        let wood = |f: &str| matches!(f, "roll_wood" | "roll_dirt" | "roll_grass");
        if !physics.board_wiping_out && rolling(state) && drag > 1.0 && tracker.drag_timer <= 0.0 {
            tracker.drag_timer = 0.09;
            bank.play(&mut commands, &mut clips_in_memory, if wood(surface) { "pop_wood" } else { "pop_hard" }, (0.25 + drag / 12.0).min(0.6));
        }
        if knock > 1.2 && tracker.drag_timer <= 0.0 {
            tracker.drag_timer = 0.12;
            let gain = (knock / 6.0).clamp(0.2, 0.8);
            bank.play(&mut commands, &mut clips_in_memory, if wood(knock_surface) { "land_wood" } else { "land_hard" }, gain);
            bank.play(&mut commands, &mut clips_in_memory, "ollie_rattle", gain);
        }
        // Pavement seams: a clack every joint rolled over on concrete.
        if rolling(state) && matches!(surface, "roll_smooth" | "roll_rough") {
            tracker.seam_travel += speed * dt;
            if tracker.seam_travel > SEAM_SPACING * (0.7 + 0.6 * bank.random()) {
                tracker.seam_travel = 0.0;
                bank.play(&mut commands, &mut clips_in_memory, "seam", 0.4 + 0.6 * intensity);
            }
        } else {
            tracker.seam_travel = 0.0;
        }
        // Footsteps off the board, spaced by stride (longer when running).
        if state == State::BipedGround {
            let root = skater.animated_skeleton.roots.animation_to_world[3];
            let walk = Vec3::new(root[0], 0.0, root[2]);
            let moved = tracker.last_walk.map_or(0.0, |last| (walk - last).length());
            tracker.last_walk = Some(walk);
            let pace = moved / dt.max(1e-4);
            if moved < 1.0 && pace > 0.4 {
                tracker.step_travel += moved;
                let stride = if pace > 3.0 { 1.3 } else { 0.75 };
                if tracker.step_travel > stride {
                    tracker.step_travel = 0.0;
                    bank.play(&mut commands, &mut clips_in_memory, "footstep", if pace > 3.0 { 0.9 } else { 0.6 });
                }
            }
        } else {
            tracker.last_walk = None;
            tracker.step_travel = 0.0;
        }
        if airborne(state) { tracker.wheel_spin *= 1.0 - (dt / WHEEL_SPIN_DOWN).min(1.0); }
        else if rolling(state) { tracker.wheel_spin = intensity; } else { tracker.wheel_spin = 0.0; }
        tracker.state = Some(state);
        tracker.fall_speed = if airborne(state) { (-velocity.y).max(0.0) } else { 0.0 };
    }

    // Everything board-related holds still while the menu is open.
    for sink in &one_shots {
        if active == sink.is_paused() {
            if active { sink.play() } else { sink.pause() }
        }
    }
    // Rolling layers: slow grain into fast grain with speed.
    let slide = if state == State::SlideGround { 0.5 } else { 1.0 };
    let fast_layer = hard_of(surface).is_some_and(|h| bank.sounds.contains_key(h));
    let mix = { let t = ((intensity - 0.25) / 0.5).clamp(0.0, 1.0); t * t * (3.0 - 2.0 * t) };
    // Wheel spins: in the air, or the lifted pair in a manual (only the front
    // or only the back wheels touching for a moment).
    let has_spins = bank.sounds.contains_key("spin_jump");
    use skate_core::physics::board::BodyId as Part;
    let (mut front, mut back) = (false, false);
    for report in physics.board.contact_reports() {
        match report.part {
            Part::RightFrontWheel | Part::LeftFrontWheel => front = true,
            Part::RightBackWheel | Part::LeftBackWheel => back = true,
            _ => {}
        }
    }
    tracker.manual_time = if rolling(state) && front != back { tracker.manual_time + dt } else { 0.0 };
    let wanted: Option<&'static str> = if !has_spins || !active || physics.board_wiping_out { None }
        else if airborne(state) { Some("spin_jump") }
        else if tracker.manual_time > 0.25 { Some("spin_manual") }
        else { None };
    if wanted != tracker.spin.map(|(_, kind)| kind) {
        if let Some((entity, _)) = tracker.spin.take() {
            if let Ok((_, mut spin, _)) = spins.get_mut(entity) { spin.fade.get_or_insert(0.12); }
        }
        if let Some(kind) = wanted {
            let speed = if kind == "spin_jump" { tracker.wheel_spin } else { intensity };
            if speed > 0.1 {
                let start = std::time::Duration::from_secs_f32((1.0 - speed.min(1.0)) * SPIN_SPAN);
                if std::env::var("SKATE_DEBUG_AUDIO").is_ok() { info!("WHEEL_SPIN {kind} from {start:?}"); }
                if let Some(entity) = bank.play_from(&mut commands, &mut clips_in_memory, kind, 0.9, start) {
                    tracker.spin = Some((entity, kind));
                }
            }
        }
    }
    for (entity, mut spin, mut sink) in &mut spins {
        if let Some(fade) = spin.fade {
            let fade = fade - dt;
            spin.fade = Some(fade);
            if fade <= 0.0 { commands.entity(entity).despawn(); continue; }
            sink.set_volume(Volume::Linear(spin.volume * bank.gain * (fade / 0.12)));
        } else {
            sink.set_volume(Volume::Linear(spin.volume * bank.gain));
        }
    }
    for (mut sound, mut sink) in &mut loops {
        if active == sink.is_paused() {
            if active { sink.play() } else { sink.pause() }
        }
        let target = if !active {
            0.0
        } else {
            match sound.name {
                // Rolling: the surface's slow grain fading into its fast one
                // with speed (one loop where no fast grain is installed).
                name if name == surface && rolling(state) => intensity.sqrt() * slide * if fast_layer { 1.0 - mix } else { 1.0 },
                name if Some(name) == hard_of(surface) && rolling(state) => intensity.sqrt() * slide * mix,
                "rattle" if rolling(state) => intensity,
                "grind_trucks" if state.is_grind() && !board_slide(state) => 0.5 + 0.5 * intensity,
                "grind_board" if board_slide(state) => 0.5 + 0.5 * intensity,
                // Free-spinning wheels in the air: the retail spin-down clip
                // (spin_jump) where installed, else the surface loop pitched up.
                name if name == surface && airborne(state) && !has_spins => 0.6 * tracker.wheel_spin.sqrt(),
                "body_slide" if !matches!(tracker.slide_family, "roll_grass" | "roll_dirt") => ((tracker.body_slide - 0.8) / 4.0).clamp(0.0, 1.0),
                "body_slide_soft" if matches!(tracker.slide_family, "roll_grass" | "roll_dirt") => ((tracker.body_slide - 0.8) / 4.0).clamp(0.0, 1.0),
                // Rushing air at speed, on the board or in the air.
                "wind" if rolling(state) || airborne(state) || state.is_grind() => ((speed - WIND_SPEED) / 10.0).clamp(0.0, 1.0),
                _ => 0.0,
            }
        };
        if std::env::var("SKATE_DEBUG_AUDIO").is_ok() && sound.level > 0.02 && (time.elapsed_secs() * 2.0).fract() < dt * 2.0 {
            info!("LOOP {} level={:.2} vol={:.2} state={state:?}", sound.name, sound.level, sound.level * sound.volume);
        }
        // Fast attack, slower release, so loops neither click nor linger.
        let rate = if target > sound.level { 30.0 } else { 10.0 };
        sound.level += (target - sound.level) * (rate * dt).min(1.0);
        sink.set_volume(Volume::Linear(sound.level * sound.volume * bank.gain));
        if sound.name.starts_with("roll") || sound.name == "rattle" {
            // Airborne wheels whirr higher: no road load on the bearings. The
            // grains are recorded at speed, so they bend less.
            let spin = if airborne(state) { 1.1 + 0.45 * tracker.wheel_spin }
                else if fast_layer { 0.92 + 0.16 * intensity } else { 0.8 + 0.45 * intensity };
            sink.set_speed(spin);
        }
    }
}
