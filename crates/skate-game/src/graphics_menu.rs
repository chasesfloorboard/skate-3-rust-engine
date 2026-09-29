//! Native-resolution pause UI over a separately scaled 3D render target.
use crate::difficulty::Difficulty;
use bevy::{
    camera::RenderTarget,
    core_pipeline::prepass::DepthPrepass,
    image::ImageSampler,
    prelude::*,
    render::{
        experimental::occlusion_culling::OcclusionCulling,
        render_resource::{Extent3d, TextureFormat},
        renderer::RenderAdapter,
    },
    window::{MonitorSelection, PresentMode, PrimaryWindow, WindowMode},
};
use serde::{Deserialize, Serialize};
use std::{
    path::PathBuf,
    time::{Duration, Instant},
};

const RESOLUTIONS: &[(u32, u32)] = &[
    (1280, 720),
    (1280, 800),
    (1600, 900),
    (1920, 1080),
    (1920, 1200),
    (2560, 1440),
    (2560, 1600),
    (3840, 2160),
];
const SCALES: &[u32] = &[25, 50, 67, 75, 85, 100];
const DAY_SPEEDS: &[u32] = &[0, 1, 10, 30, 60, 120, 360, 720];
const LIMITS: &[u32] = &[0, 30, 60, 90, 120, 144, 165, 240, 300, 400];

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
struct GraphicsSettings {
    width: u32,
    height: u32,
    scale: u32,
    samples: u32,
    fps: u32,
    occlusion: bool,
    hour: f32,
    /// Off pins retail districts to their authored midday look.
    day_night: bool,
    day_speed: u32,
    ambient_level: Option<u32>,
    /// Index into retail_sky::SKY_PRESETS.
    sky: u32,
    /// Borderless fullscreen on the current monitor. Exclusive fullscreen is
    /// not offered: winit ignores it on Wayland, and borderless already gets
    /// direct scanout there.
    fullscreen: bool,
    /// Shadow quality: 0 off .. 4 ultra (map size, cascades and distance).
    shadows: u32,
    /// Street lamps lit at once: 0 none .. 3 many.
    lights: u32,
}
/// Shadow tiers: (map size, cascades, distance m).
const SHADOW_TIERS: [(usize, usize, f32); 5] = [(1024, 1, 0.0), (1024, 1, 30.0), (2048, 2, 50.0), (4096, 3, 80.0), (4096, 4, 100.0)];
const SHADOW_NAMES: [&str; 5] = ["Off", "Low", "Medium", "High", "Ultra"];
/// Street lamps lit at once per tier.
const LIGHT_COUNTS: [usize; 4] = [0, 8, 16, 32];
const LIGHT_NAMES: [&str; 4] = ["Off", "Few", "Some", "Many"];
/// Presets: (render scale, MSAA, shadows, lights, occlusion).
const PRESETS: [(&str, u32, u32, u32, u32, bool); 5] = [
    ("Potato", 50, 1, 0, 0, false),
    ("Low", 75, 1, 1, 1, false),
    ("Medium", 100, 2, 2, 2, false),
    ("High", 100, 4, 3, 3, false),
    ("Ultra", 100, 8, 4, 3, false),
];
impl Default for GraphicsSettings {
    fn default() -> Self {
        Self {
            width: 1280,
            height: 800,
            scale: 100,
            samples: 4,
            fps: 0,
            // GPU occlusion culling costs more CPU than it saves here.
            occlusion: false,
            hour: 12.,
            day_night: true,
            day_speed: 60,
            ambient_level: None,
            sky: 0,
            fullscreen: false,
            shadows: 3,
            lights: 3,
        }
    }
}
impl GraphicsSettings {
    fn validated(mut self) -> Self {
        self.ambient_level = self.ambient_level.map(|level| level.min(100));
        if self.sky as usize >= crate::retail_render::SKY_PRESETS.len() { self.sky = 0; }
        self.hour = if self.hour.is_finite() { self.hour.rem_euclid(24.) } else { 12. };
        if !DAY_SPEEDS.contains(&self.day_speed) { self.day_speed = 60; }
        if !RESOLUTIONS.contains(&(self.width, self.height)) {
            (self.width, self.height) = (1280, 800);
        }
        if !SCALES.contains(&self.scale) {
            self.scale = 100;
        }
        if ![1, 2, 4, 8].contains(&self.samples) {
            self.samples = 4;
        }
        if !LIMITS.contains(&self.fps) {
            self.fps = 0;
        }
        self.shadows = self.shadows.min(4);
        self.lights = self.lights.min(3);
        self
    }
    fn internal_size(&self, window: UVec2) -> UVec2 {
        (window * self.scale / 100).max(UVec2::ONE)
    }
}
/// Skate 3-style pause tabs over the menu rows, in display order.
pub(crate) const TABS: [(&str, &[usize]); 4] = [
    ("Main", &[14, 10, 12, 6, 7, 22, 23, 8, 9]),
    ("Online", &[11]),
    ("Mod Settings", &[15, 16, 13]),
    ("Options", &[21, 0, 1, 2, 19, 20, 3, 4, 17, 5, 18]),
];
/// Tab buttons are menu rows numbered from here.
pub(crate) const TAB_ROW: usize = 100;
pub(crate) fn tab_of(row: usize) -> Option<usize> {
    TABS.iter().position(|(_, rows)| rows.contains(&row))
}
/// One-line description under the selected item, as in the retail menu.
pub(crate) fn describe(row: usize) -> &'static str {
    match row {
        14 => "Explore San Vanelona and teleport to spots",
        10 => "Change your skater's look and gear",
        12 => "Skate as a pro, a special or your own model",
        6 => "Choose a district or park to load",
        7 => "Load the chosen district",
        8 => "Back to skating",
        9 => "Exit to the desktop",
        11 => "Skate with friends online or on your network",
        15 => "Turn installed mods on or off",
        16 => "Turn the day/night cycle on or off, set the time",
        13 => "Check for a newer version",
        0 => "Window size",
        1 => "Render resolution as a share of the window",
        2 => "Edge smoothing",
        3 => "Cap the frame rate",
        4 => "Skip drawing hidden scenery",
        17 => "Borderless fullscreen",
        5 => "How forgiving the physics are",
        18 => "Master, music, board and ambience volume",
        19 => "Shadow detail and distance",
        20 => "How many street lamps light the night",
        21 => "Set everything at once, from Potato to Ultra",
        22 => "Pick one of the game's movies",
        23 => "Watch the movie (any button skips)",
        _ => "",
    }
}
/// Volume percentages, saved to settings/audio.json beside graphics.json.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub(crate) struct AudioSettings {
    master: u32,
    music: u32,
    board: u32,
    ambience: u32,
}
impl Default for AudioSettings {
    fn default() -> Self {
        Self { master: 100, music: 100, board: 100, ambience: 100 }
    }
}
#[derive(Clone, Copy)]
pub(crate) enum AudioChannel {
    Music,
    Board,
    Ambience,
}
impl AudioSettings {
    fn validated(mut self) -> Self {
        for level in [&mut self.master, &mut self.music, &mut self.board, &mut self.ambience] {
            *level = (*level).min(100) / 10 * 10;
        }
        self
    }
    fn level(&mut self, row: usize) -> Option<&mut u32> {
        match row {
            0 => Some(&mut self.master),
            1 => Some(&mut self.music),
            2 => Some(&mut self.board),
            3 => Some(&mut self.ambience),
            _ => None,
        }
    }
}
#[derive(Resource)]
pub(crate) struct Menu {
    /// Movie picked in the Movies row, and one asked to play (movies.rs).
    pub(crate) movie: usize,
    pub(crate) play_movie: Option<String>,
    pub(crate) open: bool,
    selected: usize,
    settings: GraphicsSettings,
    path: PathBuf,
    supported_msaa: Vec<u32>,
    difficulty: Difficulty,
    status: String,
    maps: Vec<crate::map_library::Entry>,
    selected_map: usize,
    multiplayer: bool,
    browser: bool,
    daylight: bool,
    audio: bool,
    audio_settings: AudioSettings,
    audio_path: PathBuf,
    tab: usize,
}
impl Menu {
    pub(crate) fn day_speed(&self) -> u32 {
        self.settings.day_speed
    }
    pub(crate) fn sky_preset(&self) -> usize {
        self.settings.sky as usize
    }
    pub(crate) fn tab(&self) -> usize {
        self.tab
    }
    /// Rows whose value Left/Right cycles; the rest only react to A/Enter.
    fn adjustable(&self, row: usize) -> bool {
        if self.audio { row < 4 } else if self.daylight { row < 5 } else if self.multiplayer || self.browser { false } else { row <= 6 || row == 17 || (19..=22).contains(&row) }
    }
    /// The tabbed top level, as opposed to a submenu with its own rows.
    pub(crate) fn in_tabs(&self) -> bool {
        !self.audio && !self.daylight && !self.multiplayer && !self.browser
    }
    /// Linear gain for a channel, master included.
    /// Street lamps that may be lit at once (quality setting).
    pub(crate) fn street_light_limit(&self) -> usize {
        LIGHT_COUNTS[self.settings.lights.min(3) as usize]
    }
    pub(crate) fn audio_gain(&self, channel: AudioChannel) -> f32 {
        let a = &self.audio_settings;
        let level = match channel {
            AudioChannel::Music => a.music,
            AudioChannel::Board => a.board,
            AudioChannel::Ambience => a.ambience,
        };
        // Squared so each 10% step sounds roughly even to the ear.
        (a.master as f32 / 100.).powi(2) * (level as f32 / 100.).powi(2)
    }
    pub(crate) fn ambient_brightness(&self, automatic: f32) -> f32 {
        self.settings.ambient_level.map_or(automatic, |level| level as f32 * 10.)
    }
    /// Exposure scale from the Ambient light setting: Auto and 50% leave the
    /// map's own exposure, 0% halves it, 100% gives half as much again.
    pub(crate) fn ambient_exposure(&self) -> f32 {
        self.settings.ambient_level.map_or(1.0, |level| 0.5 + level as f32 / 100.)
    }
    /// How deep the retail night grade goes (day_cycle.rs): Auto keeps the
    /// authored night, 0% is darker still, 100% leaves night almost as bright
    /// as day.
    pub(crate) fn night_depth(&self) -> f32 {
        self.settings.ambient_level.map_or(1.0, |level| 1.25 * (1.0 - level as f32 / 100.))
    }
    pub(crate) fn advance_day(&mut self, seconds: f32) -> f32 {
        if !self.settings.day_night {
            return 12.;
        }
        if !self.open && self.settings.day_speed > 0 {
            self.settings.hour = (self.settings.hour + seconds * self.settings.day_speed as f32 / 3600.).rem_euclid(24.);
        }
        self.settings.hour
    }
    pub(crate) fn selected(&self) -> usize {
        self.selected
    }
    pub(crate) fn in_audio(&self) -> bool {
        self.audio
    }
    pub(crate) fn diagnostic_settings(&self) -> String {
        format!("{:?}", self.settings)
    }
    pub(crate) fn transition_finished(&mut self, status: String, resume: bool) {
        self.status = status;
        self.open = !resume;
    }
}
pub(crate) fn gameplay_active(menu: Option<Res<Menu>>) -> bool {
    menu.is_none_or(|m| !m.open)
}
#[derive(Resource)]
struct SceneTarget(Handle<Image>);
#[derive(Resource)]
struct FramePacer(Instant);
#[derive(Component)]
pub(crate) struct MenuRoot;
/// Title and section heading, restyled by the retail menu skin.
#[derive(Component)]
pub(crate) struct MenuHeading;
/// Container of the 19 menu rows, reordered to the current tab's order.
#[derive(Component)]
pub(crate) struct MenuRows;
/// Tab strip above the rows; each tab is a MenuRow numbered from TAB_ROW.
#[derive(Component)]
pub(crate) struct MenuTabs;
#[derive(Component)]
pub(crate) struct TabLabel(pub(crate) usize);
#[derive(Component)]
pub(crate) struct MenuRow(pub(crate) usize);
#[derive(Component)]
pub(crate) struct MenuLabel(pub(crate) usize);
#[derive(Component)]
pub(crate) struct StatusLabel;

#[derive(SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) struct MenuInput;

/// The presentation camera must exist before overlays select their UI target.
#[derive(SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) struct PresentationSetup;

pub(crate) struct GraphicsMenuPlugin;
impl Plugin for GraphicsMenuPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(FramePacer(Instant::now()))
            .add_systems(PostStartup, setup.in_set(PresentationSetup))
            .add_systems(PreUpdate, interact.in_set(MenuInput).after(bevy::input::InputSystems))
            .add_systems(Update, (crate::map_render::advance_day, apply, labels, order_rows, crate::menu_skin::chrome, crate::menu_skin::apply).chain())
            .add_systems(Update, apply_shadows)
            .add_systems(PostUpdate, crate::map_render::position_celestial_bodies.before(bevy::transform::TransformSystems::Propagate))
            .add_systems(Last, pace);
    }
}
fn setup(
    mut commands: Commands,
    config: Res<crate::config::Config>,
    mut images: ResMut<Assets<Image>>,
    mut window: Single<&mut Window, With<PrimaryWindow>>,
    cameras: Query<Entity, With<Camera3d>>,
    adapter: Res<RenderAdapter>,
    mut time: ResMut<Time<Virtual>>,
) {
    let path = config
        .asset_root
        .parent()
        .unwrap_or(&config.asset_root)
        .join("settings/graphics.json");
    let settings = match std::fs::read(&path) {
        Ok(bytes) => serde_json::from_slice::<GraphicsSettings>(&bytes).unwrap_or_else(|e| {
            warn!("Graphics settings: {e}");
            GraphicsSettings::default()
        }),
        Err(_) => GraphicsSettings::default(),
    }
    .validated();
    let audio_path = path.with_file_name("audio.json");
    let audio_settings = std::fs::read(&audio_path).ok()
        .and_then(|bytes| serde_json::from_slice::<AudioSettings>(&bytes).map_err(|e| warn!("Audio settings: {e}")).ok())
        .unwrap_or_default()
        .validated();
    let supported_msaa: Vec<_> = [1, 2, 4, 8]
        .into_iter()
        .filter(|&samples| {
            [
                TextureFormat::Rgba16Float,
                TextureFormat::Rgba8UnormSrgb,
                TextureFormat::Depth32Float,
            ]
            .into_iter()
            .all(|format| {
                adapter
                    .get_texture_format_features(format)
                    .flags
                    .sample_count_supported(samples)
            })
        })
        .collect();
    let mut settings = settings;
    // Reproducible A/B override; normal launches use the saved menu setting.
    match std::env::var("SKATE_OCCLUSION").as_deref() {
        Ok("0") => settings.occlusion = false,
        Ok("1") => settings.occlusion = true,
        _ => {}
    }
    if !supported_msaa.contains(&settings.samples) {
        settings.samples = 1;
    }
    window
        .resolution
        .set_physical_resolution(settings.width, settings.height);
    window.present_mode = PresentMode::AutoNoVsync;
    let size = settings.internal_size(window.physical_size());
    let mut image = Image::new_target_texture(size.x, size.y, TextureFormat::Rgba8UnormSrgb, None);
    image.sampler = ImageSampler::linear();
    let target = images.add(image);
    for camera in &cameras {
        commands.entity(camera).insert((
            RenderTarget::Image(target.clone().into()),
            msaa(settings.samples),
        ));
        // Render the world before the presentation camera consumes its image.
        commands.entity(camera).insert(Camera {
            order: -1,
            ..default()
        });
    }
    let output = commands
        .spawn((Camera2d, Msaa::Off, IsDefaultUiCamera))
        .id();
    commands.spawn((
        Node {
            width: percent(100),
            height: percent(100),
            position_type: PositionType::Absolute,
            ..default()
        },
        ImageNode::new(target.clone()),
        UiTargetCamera(output),
    ));
    commands.spawn((MenuRoot, UiTargetCamera(output), GlobalZIndex(10), Node {
        display: Display::None, width:percent(100), height:percent(100), align_items:AlignItems::Center,
        justify_content:JustifyContent::Center, position_type:PositionType::Absolute, ..default()
    }, BackgroundColor(Color::srgba(0.015,0.025,0.04,0.88)))).with_children(|root| {
        root.spawn((Node { width:vmin(60),min_width:px(560),max_width:percent(95),padding:UiRect::all(px(18)),flex_direction:FlexDirection::Column,row_gap:px(4),border_radius:BorderRadius::all(px(12)),..default() },
            BackgroundColor(Color::srgb(0.035,0.055,0.08)))).with_children(|panel| {
            panel.spawn((MenuHeading,Text::new("GAME MENU"),TextFont {font_size:32.,..default()},TextColor(Color::WHITE)));
            panel.spawn((MenuHeading,Text::new("GAMEPLAY & GRAPHICS"),TextFont {font_size:16.,..default()},TextColor(Color::srgb(0.4,0.85,0.85))));
            panel.spawn((MenuTabs, Node { column_gap: px(6), margin: UiRect::vertical(px(4)), ..default() })).with_children(|tabs| {
                for (i, (name, _)) in TABS.iter().enumerate() {
                    tabs.spawn((Button, MenuRow(TAB_ROW + i), Node { padding: UiRect::axes(px(10), px(4)), align_items: AlignItems::Center, border_radius: BorderRadius::all(px(5)), ..default() },
                        BackgroundColor(Color::srgb(0.08,0.11,0.15))))
                        .with_child((TabLabel(i), Text::new(name.to_uppercase()), TextFont { font_size: 15., ..default() }, TextColor(Color::WHITE)));
                }
            });
            panel.spawn((MenuRows, Node { width: percent(100), flex_direction: FlexDirection::Column, row_gap: px(4), ..default() })).with_children(|rows| {
                for i in 0..24 {
                    rows.spawn((Button, MenuRow(i), Node {width:percent(100),min_height:px(26),padding:UiRect::all(px(3)),align_items:AlignItems::Center,border_radius:BorderRadius::all(px(5)),..default()},
                        BackgroundColor(Color::srgb(0.08,0.11,0.15)))).with_children(|row| {
                        row.spawn((MenuLabel(i),Text::new(""),TextFont {font_size:18.,..default()},TextColor(Color::WHITE)));
                    });
                }
            });
            panel.spawn((StatusLabel,Text::new(""),TextFont {font_size:15.,..default()},TextColor(Color::srgb(0.65,0.75,0.8))));
            panel.spawn((Text::new("LB/RB or Q/E switch tabs | Up/Down select | Left/Right change\nEsc resume | Changes save automatically"),TextFont {font_size:14.,..default()},TextColor(Color::srgb(0.65,0.75,0.8))));
        });
    });
    commands.insert_resource(SceneTarget(target));
    let maps = crate::map_library::discover(&config.asset_root);
    let selected_map = maps.iter().position(|m| m.path.as_ref() == config.map_path.as_ref()).unwrap_or(0);
    // Test hook: open the menu on a tab for screenshots.
    let debug_tab = std::env::var("SKATE_DEBUG_MENU").ok().and_then(|t| t.parse::<usize>().ok()).filter(|t| *t < TABS.len());
    if config.start_paused || debug_tab.is_some() { time.pause(); }
    commands.insert_resource(Menu {
        open: config.start_paused || debug_tab.is_some(),
        selected: TABS[debug_tab.unwrap_or(0)].1[0],
        settings,
        path,
        supported_msaa,
        difficulty: config.difficulty,
        status: String::new(),
        maps,
        selected_map,
        movie: 0,
        play_movie: None,
        multiplayer: false,
        browser: false,
        daylight: false,
        audio: false,
        audio_settings,
        audio_path,
        tab: debug_tab.unwrap_or(0),
    });
}
fn msaa(samples: u32) -> Msaa {
    match samples {
        2 => Msaa::Sample2,
        4 => Msaa::Sample4,
        8 => Msaa::Sample8,
        _ => Msaa::Off,
    }
}
fn cycle<T: PartialEq + Copy>(values: &[T], value: T, direction: i32) -> T {
    let index = values.iter().position(|x| *x == value).unwrap_or(0) as i32;
    values[(index + direction).rem_euclid(values.len() as i32) as usize]
}
pub(crate) fn interact(
    mut config: ResMut<crate::config::Config>,
    mut transition: ResMut<crate::map_transition::MapTransition>,
    mut customiser: ResMut<crate::customiser::Customiser>,
    mut custom_models: ResMut<crate::custom_models::CustomModels>,
    (mut mods, panel): (ResMut<crate::modding::ModMenu>, Res<crate::modding::EnabledPanel>),
    nav: Res<crate::customiser::Navigation>,
    mut physics: ResMut<crate::physics::GamePhysics>,
    keys: Res<ButtonInput<KeyCode>>,
    mut menu: ResMut<Menu>,
    mut time: ResMut<Time<Virtual>>,
    buttons: Query<(&Interaction, &MenuRow), Changed<Interaction>>,
    mut exit: MessageWriter<AppExit>,
    mut net: ResMut<crate::multiplayer::Multiplayer>,
    mut typing: MessageReader<bevy::input::keyboard::KeyboardInput>,
    mut updater: ResMut<crate::updater::Updater>,
    mut travel: ResMut<crate::teleport_menu::Travel>,
) {
    if transition.busy() {
        menu.open = true;
        time.pause();
        return;
    }
    if mods.open || travel.open || travel.closed_this_frame || customiser.open || custom_models.open { return; }
    if keys.just_pressed(KeyCode::Escape) || nav.pressed & 0x10 != 0 {
        menu.open = !menu.open;
        if menu.open && menu.in_tabs() {
            menu.tab = 0;
            menu.selected = TABS[0].1[0];
        }
    }
    let mut action = None;
    for event in typing.read() {
        if !menu.open
            || !menu.multiplayer
            || menu.browser
            || menu.selected != 3
            || !event.state.is_pressed()
        {
            continue;
        }
        if event.key_code == KeyCode::Backspace {
            net.join_code.pop();
        }
        if let Some(text) = &event.text {
            for ch in text.chars().filter(|c| c.is_ascii_hexdigit() || *c == '-') {
                if net.join_code.len() < 40 {
                    net.join_code.push(ch);
                }
            }
        }
    }
    if menu.open {
        let rows = if menu.audio { 5 } else if menu.daylight { 6 } else if menu.multiplayer { 11 } else { 19 };
        if menu.in_tabs() {
            // Coming back from a submenu lands on the tab that owns its row.
            if !TABS[menu.tab].1.contains(&menu.selected) {
                menu.tab = tab_of(menu.selected).unwrap_or(0);
                menu.selected = if TABS[menu.tab].1.contains(&menu.selected) { menu.selected } else { TABS[menu.tab].1[0] };
            }
        }
        if !panel.focused {
        let step = |menu: &mut Menu, delta: i32| {
            let list = TABS[menu.tab].1;
            let at = list.iter().position(|r| *r == menu.selected).unwrap_or(0) as i32;
            menu.selected = list[(at + delta).rem_euclid(list.len() as i32) as usize];
        };
        if menu.in_tabs() {
            let tab_step = if keys.just_pressed(KeyCode::KeyE) || nav.pressed & 0x200 != 0 { 1 }
                else if keys.just_pressed(KeyCode::KeyQ) || nav.pressed & 0x100 != 0 { -1 } else { 0 };
            if tab_step != 0 {
                menu.tab = (menu.tab as i32 + tab_step).rem_euclid(TABS.len() as i32) as usize;
                menu.selected = TABS[menu.tab].1[0];
            }
        }
        if keys.just_pressed(KeyCode::ArrowUp) || nav.pressed & 1 != 0 {
            if menu.in_tabs() { step(&mut menu, -1) } else { menu.selected = (menu.selected + rows - 1) % rows; }
        }
        if keys.just_pressed(KeyCode::ArrowDown) || nav.pressed & 2 != 0 {
            if menu.in_tabs() { step(&mut menu, 1) } else { menu.selected = (menu.selected + 1) % rows; }
        }
        // Left/Right only change values; rows that open a submenu or run
        // something need A/Enter, as on the retail menu.
        let adjustable = menu.adjustable(menu.selected);
        if (keys.just_pressed(KeyCode::ArrowLeft) || nav.pressed & 4 != 0) && adjustable {
            action = Some((menu.selected, -1));
        }
        if (keys.just_pressed(KeyCode::ArrowRight) || nav.pressed & 8 != 0) && adjustable {
            action = Some((menu.selected, 1));
        }
        if keys.just_pressed(KeyCode::Enter) || nav.pressed & 0x1000 != 0 {
            action = Some((menu.selected, 1));
        }
        }
        for (interaction, row) in &buttons {
            if panel.dragging() { continue; }
            if *interaction == Interaction::Pressed {
                menu.selected = row.0;
                action = Some((row.0, 1));
            }
        }
    }
    if let Some(tab) = action.and_then(|(row, _)| row.checked_sub(TAB_ROW)) {
        menu.tab = tab;
        menu.selected = TABS[tab].1[0];
        action = None;
    }
    if let Some((row, direction)) = action {
        let day_action = menu.daylight;
        let audio_action = menu.audio;
        if menu.audio {
            if let Some(level) = menu.audio_settings.level(row) {
                *level = (*level as i32 + direction * 10).clamp(0, 100) as u32;
                let save = (|| -> Result<(), String> {
                    std::fs::create_dir_all(menu.audio_path.parent().unwrap()).map_err(|e| e.to_string())?;
                    std::fs::write(&menu.audio_path, serde_json::to_vec_pretty(&menu.audio_settings).map_err(|e| e.to_string())?)
                        .map_err(|e| e.to_string())
                })();
                menu.status = match save {
                    Ok(()) => "Saved".into(),
                    Err(e) => format!("Could not save: {e}"),
                };
            } else {
                menu.audio = false;
                menu.selected = 18;
            }
        } else if menu.daylight {
            match row {
                0 => menu.settings.day_night = !menu.settings.day_night,
                1 => menu.settings.hour = ((menu.settings.hour * 4.).round() + direction as f32).rem_euclid(96.) / 4.,
                2 => menu.settings.day_speed = cycle(DAY_SPEEDS, menu.settings.day_speed, direction),
                3 => {
                    // Auto, 0%, 5%, ... 100%, then Auto again.
                    let index = menu.settings.ambient_level.map_or(0, |level| level as i32 / 5 + 1);
                    let next = (index + direction).rem_euclid(22);
                    menu.settings.ambient_level = if next == 0 { None } else { Some((next as u32 - 1) * 5) };
                }
                4 => {
                    let count = crate::retail_render::SKY_PRESETS.len() as i32;
                    menu.settings.sky = (menu.settings.sky as i32 + direction).rem_euclid(count) as u32;
                }
                _ => { menu.daylight = false; menu.selected = 16; }
            }
        } else if menu.browser {
            match row {
                0 => net.browse(0),
                1..=5 => net.join_row(row - 1),
                6 => {
                    let page = net.browser_page.saturating_sub(1);
                    net.browse(page);
                }
                7 => {
                    let page = net.browser_page + 1;
                    if page * 5 < net.browser_total {
                        net.browse(page);
                    }
                }
                8 => menu.open = false,
                9 => {
                    exit.write(AppExit::Success);
                }
                10 => {
                    menu.browser = false;
                    menu.selected = 6;
                }
                _ => {}
            }
        } else if menu.multiplayer {
            match row {
                0 => net.local(true),
                1 => net.local(false),
                2 => net.steam(true),
                3 => {}
                4 => net.steam(false),
                5 => net.leave(),
                6 => {
                    net.browse(0);
                    menu.browser = true;
                    menu.selected = 0;
                }
                8 => menu.open = false,
                9 => {
                    exit.write(AppExit::Success);
                }
                10 => {
                    menu.multiplayer = false;
                    menu.selected = 11;
                }
                _ => {}
            }
        } else {
            match row {
                0 => {
                    let size = cycle(
                        RESOLUTIONS,
                        (menu.settings.width, menu.settings.height),
                        direction,
                    );
                    (menu.settings.width, menu.settings.height) = size;
                }
                1 => menu.settings.scale = cycle(SCALES, menu.settings.scale, direction),
                2 => {
                    menu.settings.samples =
                        cycle(&menu.supported_msaa, menu.settings.samples, direction)
                }
                3 => menu.settings.fps = cycle(LIMITS, menu.settings.fps, direction),
                4 => menu.settings.occlusion = !menu.settings.occlusion,
                19 => menu.settings.shadows = (menu.settings.shadows as i32 + direction).clamp(0, 4) as u32,
                22 => menu.movie = (menu.movie as i32 + direction).rem_euclid(crate::movies::MOVIES.len() as i32) as usize,
                23 => menu.play_movie = Some(crate::movies::MOVIES[menu.movie].0.into()),
                20 => menu.settings.lights = (menu.settings.lights as i32 + direction).clamp(0, 3) as u32,
                21 => {
                    let current = PRESETS.iter().position(|p| menu.settings.scale == p.1 && menu.settings.samples == p.2
                        && menu.settings.shadows == p.3 && menu.settings.lights == p.4).unwrap_or(2);
                    let (_, scale, samples, shadows, lights, occlusion) = PRESETS[(current as i32 + direction).clamp(0, 4) as usize];
                    menu.settings.scale = scale;
                    menu.settings.samples = if menu.supported_msaa.contains(&samples) { samples } else { 1 };
                    menu.settings.shadows = shadows;
                    menu.settings.lights = lights;
                    menu.settings.occlusion = occlusion;
                }
                5 => {
                    menu.difficulty = cycle(&Difficulty::ALL, menu.difficulty, direction);
                    physics.set_difficulty(menu.difficulty);
                    config.difficulty = menu.difficulty;
                    menu.status = match menu.difficulty.save(&config.asset_root) {
                        Ok(()) => "Difficulty saved".into(),
                        Err(e) => format!("Applied, but could not save: {e}"),
                    };
                }
                6 => {
                    menu.selected_map = (menu.selected_map as i32 + direction)
                        .rem_euclid(menu.maps.len() as i32)
                        as usize;
                    menu.status = "Choose Load map to switch".into();
                }
                7 => {
                    if net.active() {
                        menu.status = "Leave multiplayer before switching maps".into();
                    } else {
                        net.leave();
                        transition.request(menu.maps[menu.selected_map].clone());
                        menu.status = "Loading map...".into();
                    }
                }
                8 => menu.open = false,
                9 => {
                    exit.write(AppExit::Success);
                }
                10 => { custom_models.request_stock(); customiser.begin(); },
                11 => {
                    menu.multiplayer = true;
                    menu.selected = 0;
                }
                12 => custom_models.begin(),
                13 => menu.status = updater.open(false),
                14 => travel.open = true,
                15 => mods.begin(),
                17 => menu.settings.fullscreen = !menu.settings.fullscreen,
                18 => { menu.audio = true; menu.selected = 0; menu.status = "Left/Right adjusts. Music: N next song, M mute.".into(); },
                16 => { menu.daylight = true; menu.selected = 0; menu.status = "Time of day and cycle speed. Retail districts darken and warm with the sun.".into(); },
                _ => {}
            }
        }
        if !audio_action && (((row < 5 || row == 17) && !menu.multiplayer && !menu.daylight && !menu.audio && !day_action) || (day_action && row < 5)) {
            let save = (|| -> Result<(), String> {
                std::fs::create_dir_all(menu.path.parent().unwrap()).map_err(|e| e.to_string())?;
                std::fs::write(
                    &menu.path,
                    serde_json::to_vec_pretty(&menu.settings).map_err(|e| e.to_string())?,
                )
                .map_err(|e| e.to_string())
            })();
            menu.status = match save {
                Ok(()) => "Saved".into(),
                Err(e) => format!("Could not save: {e}"),
            };
        }
    }
    if menu.open && !net.active() {
        time.pause();
    } else {
        time.unpause();
    }
}
/// Shadow quality: the shadow map size, and every sun-like light's cascades
/// (the player-only receiver light just switches on and off), reapplied when
/// the setting changes or a map spawns new lights.
fn apply_shadows(
    menu: Res<Menu>,
    mut map: ResMut<bevy::light::DirectionalLightShadowMap>,
    mut lights: Query<(&mut DirectionalLight, &mut bevy::light::CascadeShadowConfig, Option<&bevy::camera::visibility::RenderLayers>)>,
    mut previous: Local<Option<(u32, usize)>>,
) {
    let tier = menu.settings.shadows.min(4);
    // Reapply when the setting changes or lights come and go (map loads).
    let state = (tier, lights.iter().len());
    if *previous == Some(state) { return; }
    *previous = Some(state);
    let (size, cascades, distance) = SHADOW_TIERS[tier as usize];
    if map.size != size { map.size = size; }
    for (mut light, mut config, layers) in &mut lights {
        light.shadows_enabled = tier > 0;
        let player_only = layers.is_some_and(|l| l.intersects(&bevy::camera::visibility::RenderLayers::layer(28))
            && !l.intersects(&bevy::camera::visibility::RenderLayers::layer(0)));
        if tier > 0 && !player_only {
            // Retail maps' world-caster light only shades characters and
            // props (the world's own shadows are baked), and those are near
            // the player: every cascade re-tests and redraws the whole map, so
            // it stops at two cascades and 40 m.
            let character_only = light.illuminance == 0.0 && !light.affects_lightmapped_mesh_diffuse;
            let (cascades, distance) = if character_only { (cascades.min(2), distance.min(40.0)) } else { (cascades, distance) };
            *config = bevy::light::CascadeShadowConfigBuilder {
                num_cascades: cascades,
                maximum_distance: distance,
                first_cascade_far_bound: (distance / 12.0).clamp(4.0, 10.0),
                ..default()
            }.build();
        }
    }
    info!("Shadows: {} ({size}px, {cascades} cascades, {distance} m)", SHADOW_NAMES[tier as usize]);
}

fn apply(
    mut commands: Commands,
    menu: Res<Menu>,
    mut window: Single<&mut Window, With<PrimaryWindow>>,
    target: Res<SceneTarget>,
    mut images: ResMut<Assets<Image>>,
    mut cameras: Query<(Entity, &mut Msaa), With<Camera3d>>,
    mut previous: Local<Option<GraphicsSettings>>,
) {
    if previous
        .as_ref()
        .is_none_or(|p| p.fullscreen != menu.settings.fullscreen)
    {
        window.mode = if menu.settings.fullscreen {
            WindowMode::BorderlessFullscreen(MonitorSelection::Current)
        } else {
            WindowMode::Windowed
        };
    }
    // Leaving fullscreen restores the chosen windowed resolution.
    if previous.as_ref().is_none_or(|p| {
        p.width != menu.settings.width
            || p.height != menu.settings.height
            || (p.fullscreen && !menu.settings.fullscreen)
    }) {
        window
            .resolution
            .set_physical_resolution(menu.settings.width, menu.settings.height);
    }
    if previous
        .as_ref()
        .is_none_or(|p| p.samples != menu.settings.samples)
    {
        for (_, mut samples) in &mut cameras {
            *samples = msaa(menu.settings.samples);
        }
    }
    if previous
        .as_ref()
        .is_none_or(|p| p.occlusion != menu.settings.occlusion)
    {
        for (entity, _) in &cameras {
            if menu.settings.occlusion {
                commands
                    .entity(entity)
                    .insert((DepthPrepass, OcclusionCulling));
            } else {
                commands
                    .entity(entity)
                    .remove::<(DepthPrepass, OcclusionCulling)>();
            }
        }
        info!("GPU occlusion culling: {}", menu.settings.occlusion);
    }
    let size = menu.settings.internal_size(window.physical_size());
    if let Some(image) = images.get(&target.0) {
        if image.size() != size {
            images.get_mut(&target.0).unwrap().resize(Extent3d {
                width: size.x,
                height: size.y,
                depth_or_array_layers: 1,
            });
        }
    }
    *previous = Some(menu.settings.clone());
}
fn labels(
    menu: Res<Menu>,
    transition: Res<crate::map_transition::MapTransition>,
    time: Res<Time<Real>>,
    customiser: Res<crate::customiser::Customiser>,
    custom_models: Res<crate::custom_models::CustomModels>,
    travel: Res<crate::teleport_menu::Travel>,
    mods: Res<crate::modding::ModMenu>,
    net: Res<crate::multiplayer::Multiplayer>,
    window: Single<&Window, With<PrimaryWindow>>,
    mut root: Single<&mut Node, With<MenuRoot>>,
    mut labels: Query<(&MenuLabel, &mut Text), Without<StatusLabel>>,
    mut status: Single<&mut Text, With<StatusLabel>>,
    mut buttons: Query<(&MenuRow, &Interaction, &mut BackgroundColor, &mut Node), Without<MenuRoot>>,
) {
    root.display = if menu.open && !mods.open && !travel.open && !customiser.open && !custom_models.open {
        Display::Flex
    } else {
        Display::None
    };
    if !menu.open {
        return;
    }
    let s = &menu.settings;
    let size = s.internal_size(window.physical_size());
    for (label, mut text) in &mut labels {
        **text = if menu.audio {
            let a = &menu.audio_settings;
            let bar = |level: u32| format!("{:<10} {level:>3}%", "#".repeat(level as usize / 10));
            match label.0 {
                0 => format!("Master volume        {}", bar(a.master)),
                1 => format!("Music                {}", bar(a.music)),
                2 => format!("Board sounds         {}", bar(a.board)),
                3 => format!("Ambience             {}", bar(a.ambience)),
                4 => "Back".into(),
                _ => String::new(),
            }
        } else if menu.daylight {
            match label.0 {
                0 => format!("Day/night cycle      {}", if s.day_night { "On" } else { "Off (always day)" }),
                1 => { let minutes = (s.hour * 60.).floor() as u32 % 1440; format!("Time of day          {:02}:{:02}", minutes / 60, minutes % 60) },
                2 => if s.day_speed == 0 { "Cycle speed          Frozen".into() } else { format!("Cycle speed          {}x ({} min/day)", s.day_speed, 1440 / s.day_speed) },
                3 => match s.ambient_level {
                    Some(level) => format!("Ambient light        {level}%"),
                    None => "Ambient light        Auto (day/night)".into(),
                },
                4 => format!("Sky colour           {}", crate::retail_render::SKY_PRESETS[s.sky as usize].0),
                5 => "Back".into(),
                _ => String::new(),
            }
        } else if menu.browser {
            match label.0 {
                0 => "Refresh public Steam lobbies".into(),
                1..=5 => net
                    .browser_rows
                    .get(label.0 - 1)
                    .map(|r| {
                        format!(
                            "{} | {}/{} | #{}{}",
                            r.map,
                            r.players,
                            r.capacity,
                            r.id % 100000,
                            if r.compatible {
                                ""
                            } else {
                                " | incompatible physics/protocol"
                            }
                        )
                    })
                    .unwrap_or_else(|| "--".into()),
                6 => "Previous page".into(),
                7 => "Next page".into(),
                8 => "Resume".into(),
                9 => "Quit game".into(),
                _ => "Back to multiplayer".into(),
            }
        } else if menu.multiplayer {
            match label.0 {
                0 => "Host local test (no Steam)".into(),
                1 => "Join local test (no Steam)".into(),
                2 => "Host via Steam / Spacewar".into(),
                3 => format!(
                    "Join code: {}{}",
                    net.join_code,
                    if menu.selected == 3 { "_" } else { "" }
                ),
                4 => "Join via Steam / Spacewar".into(),
                5 => "Leave multiplayer".into(),
                6 => "Browse public Steam lobbies".into(),
                7 => "Solo / local play does not require Steam".into(),
                8 => "Resume".into(),
                9 => "Quit game".into(),
                _ => "Back to gameplay & graphics".into(),
            }
        } else {
            match label.0 {
                0 => format!("Resolution          {} x {}", s.width, s.height),
                1 => format!(
                    "Internal resolution   {}%  ({} x {})",
                    s.scale, size.x, size.y
                ),
                2 => format!(
                    "MSAA                {}",
                    if s.samples == 1 {
                        "Off".into()
                    } else {
                        format!("{}x", s.samples)
                    }
                ),
                3 => format!(
                    "FPS limit             {}",
                    if s.fps == 0 {
                        "Unlimited".into()
                    } else {
                        s.fps.to_string()
                    }
                ),
                4 => format!(
                    "Occlusion culling     {}",
                    if s.occlusion { "On" } else { "Off" }
                ),
                5 => format!("Difficulty            {}", menu.difficulty.label()),
                19 => format!("Shadows               {}", SHADOW_NAMES[s.shadows.min(4) as usize]),
                22 => format!("Movie                 {}", crate::movies::MOVIES[menu.movie].1),
                23 => "Watch Movie".into(),
                20 => format!("Street lights         {}", LIGHT_NAMES[s.lights.min(3) as usize]),
                21 => format!("Quality preset        {}", PRESETS.iter().find(|p| s.scale == p.1 && s.samples == p.2
                    && s.shadows == p.3 && s.lights == p.4).map_or("Custom", |p| p.0)),
                6 => format!(
                    "District              {}",
                    menu.maps[menu.selected_map].label
                ),
                7 => if transition.busy() { "Loading District...".into() } else { "Load District".into() },
                8 => "Free Play".into(),
                9 => "Quit Game".into(),
                10 => "Edit Skater".into(),
                12 => "Call Skater".into(),
                13 => "Updates".into(),
                15 => "Mods".into(),
                14 => "Challenge Map".into(),
                16 => "Day & Night".into(),
                18 => "Audio".into(),
                17 => format!(
                    "Fullscreen            {}",
                    if s.fullscreen { "On (borderless)" } else { "Off" }
                ),
                _ => "Party Play".into(),
            }
        };
    }
    ***status = if transition.busy() {
        format!("{} {}\nGameplay is paused. Please wait.", ["|", "/", "-", "\\"][(time.elapsed_secs() * 4.) as usize % 4], transition.label())
    } else if menu.browser {
        net.browser_status.clone()
    } else if menu.multiplayer {
        format!(
            "{}{}",
            net.status,
            if net.host_code.is_empty() {
                String::new()
            } else {
                format!("\nYour connection code: {}", net.host_code)
            }
        )
    } else {
        menu.status.clone()
    };
    for (row, interaction, mut color, mut node) in &mut buttons {
        let hidden = if row.0 >= TAB_ROW {
            !menu.in_tabs()
        } else if menu.in_tabs() {
            !TABS[menu.tab].1.contains(&row.0)
        } else {
            (menu.audio && row.0 >= 5) || (menu.daylight && row.0 >= 6) || (menu.multiplayer && row.0 >= 11)
        };
        node.display = if hidden { Display::None } else { Display::Flex };
        if row.0 >= TAB_ROW {
            color.0 = if row.0 - TAB_ROW == menu.tab { Color::srgb(0.10, 0.30, 0.34) } else { Color::srgb(0.08, 0.11, 0.15) };
            continue;
        }
        color.0 = if row.0 == menu.selected || *interaction == Interaction::Hovered {
            Color::srgb(0.10, 0.30, 0.34)
        } else {
            Color::srgb(0.08, 0.11, 0.15)
        };
    }
}
/// Shows the current tab's rows in its authored order (e.g. Challenge map
/// first); submenus use plain row order.
fn order_rows(
    menu: Res<Menu>,
    container: Single<(Entity, &Children), With<MenuRows>>,
    rows: Query<&MenuRow>,
    mut commands: Commands,
) {
    let (entity, children) = *container;
    let index = |e: &Entity| rows.get(*e).map_or(usize::MAX, |r| r.0);
    let mut wanted: Vec<Entity> = children.iter().collect();
    let rank = |row: usize| if menu.in_tabs() {
        TABS[menu.tab].1.iter().position(|r| *r == row).unwrap_or(usize::MAX / 2 + row)
    } else {
        row
    };
    wanted.sort_by_key(|e| rank(index(e)));
    if !wanted.iter().eq(children.iter().collect::<Vec<_>>().iter()) {
        commands.entity(entity).replace_children(&wanted);
    }
}
fn pace(menu: Option<Res<Menu>>, mut pacer: ResMut<FramePacer>) {
    let Some(menu) = menu else {
        return;
    };
    if menu.settings.fps > 0 {
        // Sleep overshoots by a millisecond or more, which made capped frames
        // land unevenly (visible hitches): sleep to just short of the frame's
        // slot, then spin the rest. Slots advance by whole periods so one late
        // frame does not push every later one back.
        let period = Duration::from_secs_f64(1. / f64::from(menu.settings.fps));
        let deadline = pacer.0 + period;
        let now = Instant::now();
        if deadline > now {
            let wait = deadline - now;
            if wait > Duration::from_micros(1500) {
                std::thread::sleep(wait - Duration::from_micros(1200));
            }
            while Instant::now() < deadline { std::hint::spin_loop(); }
            pacer.0 = deadline;
        } else {
            // Running behind: start the next slot from now.
            pacer.0 = now;
        }
        return;
    }
    pacer.0 = Instant::now();
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn culling_can_toggle_with_msaa_and_render_scale_changes() {
        let mut app = App::new();
        let mut images = Assets::<Image>::default();
        let target = images.add(Image::new_target_texture(
            1280,
            800,
            TextureFormat::Rgba8UnormSrgb,
            None,
        ));
        app.insert_resource(SceneTarget(target.clone()))
            .insert_resource(images)
            .insert_resource(Menu {
                movie: 0,
                play_movie: None,
                open: false, selected: 0, settings: GraphicsSettings::default(),
                difficulty: Difficulty::Easy, path: PathBuf::new(), supported_msaa: vec![1, 2, 4, 8], status: String::new(),
                multiplayer: false, browser: false, daylight: false,
                audio: false, audio_settings: AudioSettings::default(), audio_path: PathBuf::new(), tab: 0,
                maps: vec![crate::map_library::Entry { label: "Test world".into(), path: None }], selected_map: 0,
            })
            .add_systems(Update, apply);
        {
            let mut menu = app.world_mut().resource_mut::<Menu>();
            menu.settings.hour = 23.5;
            menu.settings.day_speed = 60;
            assert!((menu.advance_day(60.) - 0.5).abs() < 0.0001);
            menu.open = true;
            assert_eq!(menu.advance_day(60.), 0.5);
            menu.open = false;
            menu.settings.day_speed = 0;
            assert_eq!(menu.advance_day(60.), 0.5);
        }
        app.world_mut().spawn((Window::default(), PrimaryWindow));
        let camera = app.world_mut().spawn((Camera3d::default(), Msaa::Off)).id();
        app.update();
        // Off by default: GPU occlusion culling costs more CPU than it saves.
        assert!(!app.world().entity(camera).contains::<OcclusionCulling>());
        app.world_mut().resource_mut::<Menu>().settings.occlusion = true;
        app.update();
        assert!(app.world().entity(camera).contains::<OcclusionCulling>());
        assert!(app.world().entity(camera).contains::<DepthPrepass>());
        {
            let mut menu = app.world_mut().resource_mut::<Menu>();
            menu.settings.occlusion = false;
            menu.settings.samples = 1;
            menu.settings.scale = 67;
        }
        app.update();
        assert!(!app.world().entity(camera).contains::<OcclusionCulling>());
        assert!(!app.world().entity(camera).contains::<DepthPrepass>());
        assert_eq!(*app.world().get::<Msaa>(camera).unwrap(), Msaa::Off);
        assert_eq!(
            app.world()
                .resource::<Assets<Image>>()
                .get(&target)
                .unwrap()
                .size(),
            UVec2::new(857, 536)
        );
        {
            let mut menu = app.world_mut().resource_mut::<Menu>();
            menu.settings.occlusion = true;
            menu.settings.samples = 8;
        }
        app.update();
        assert!(app.world().entity(camera).contains::<OcclusionCulling>());
        assert!(app.world().entity(camera).contains::<DepthPrepass>());
        assert_eq!(*app.world().get::<Msaa>(camera).unwrap(), Msaa::Sample8);
    }
    #[test]
    fn invalid_saved_values_fall_back() {
        let settings: GraphicsSettings =
            serde_json::from_str(r#"{"width":0,"height":999999,"scale":0,"samples":3,"fps":1}"#)
                .unwrap();
        assert_eq!(settings.validated(), GraphicsSettings::default());
    }
    #[test]
    fn scaled_target_and_cycle_boundaries() {
        let s = GraphicsSettings {
            scale: 50,
            ..default()
        };
        assert_eq!(
            s.internal_size(UVec2::new(1920, 1080)),
            UVec2::new(960, 540)
        );
        assert_eq!(s.internal_size(UVec2::ZERO), UVec2::ONE);
        assert_eq!(cycle(LIMITS, 0, -1), 400);
        assert_eq!(cycle(LIMITS, 400, 1), 0);
    }
}
