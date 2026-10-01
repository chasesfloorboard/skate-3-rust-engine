//! Start-up game picker, shown before anything loads (like the retail
//! language screen): skate 2, skate 3 or freeskate. A window can only be
//! opened once per process, so the choice relaunches the game with --edition.
use crate::editions::{self, Edition};
use crate::menu_skin::{BitmapText, BREADCRUMB, DESCRIPTION, IDLE, SELECTED, TEXT_GLOW};
use bevy::{prelude::*, render::{RenderPlugin, settings::{Backends, InstanceFlags, RenderCreation, WgpuSettings}}};

/// Test and tool hooks that expect the pre-edition behaviour (everything).
const AUTOMATION_ENV: [&str; 4] = ["SKATE_VERIFY", "SKATE_PERF_REPORT", "SKATE_DEBUG", "SKATE_TRACE"];

pub(crate) enum Outcome {
    /// Carry on with this edition in this process.
    Run(Edition),
    /// The picker relaunched the game or was closed.
    Exit,
}

/// What the picker returns: an edition, or the "add skate 2" row.
#[derive(Clone, Copy, PartialEq)]
enum Choice { Edition(Edition), AddSkate2 }

/// Decide the edition: --edition, else the picker for normal launches.
/// Launches with any other arguments (development, verification, tools,
/// multiplayer) or automation variables keep every piece of content.
pub(crate) fn resolve() -> Result<Outcome, String> {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    if let Some(i) = args.iter().position(|a| a == "--edition") {
        let value = args.get(i + 1).ok_or("--edition requires skate2, skate3 or freeskate")?;
        return Ok(Outcome::Run(Edition::parse(&value.to_string_lossy())?));
    }
    let automated = std::env::vars_os().any(|(k, _)| AUTOMATION_ENV.iter().any(|p| k.to_string_lossy().starts_with(p)));
    if !args.is_empty() || automated { return Ok(Outcome::Run(Edition::Freeskate)); }
    let assets = crate::setup::asset_root()?;
    let choices = editions::installed(&assets);
    // Nothing to choose between: one game's content only shows up in Freeskate too.
    if choices.len() < 2 { return Ok(Outcome::Run(choices.first().copied().unwrap_or(Edition::Freeskate))); }
    // Without Skate 2 the picker also offers to add it (setup, then back here).
    let offer = !choices.contains(&Edition::Skate2);
    match pick(&assets, choices, offer) {
        None => Ok(Outcome::Exit),
        Some(Choice::AddSkate2) => {
            if let Err(error) = crate::setup::add_skate2() { eprintln!("{error}"); }
            relaunch(None)?;
            Ok(Outcome::Exit)
        }
        Some(Choice::Edition(edition)) => {
            editions::remember(&assets, edition);
            relaunch(Some(edition))?;
            Ok(Outcome::Exit)
        }
    }
}

fn relaunch(edition: Option<Edition>) -> Result<(), String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let mut command = std::process::Command::new(exe);
    if let Some(edition) = edition { command.arg("--edition").arg(edition.key()); }
    let title = edition.map_or("Skate Rust", Edition::window_title);
    #[cfg(unix)] {
        use std::os::unix::process::CommandExt;
        Err(format!("Could not start {title}: {}", command.exec()))
    }
    #[cfg(not(unix))] {
        command.spawn().map(drop).map_err(|e| format!("Could not start {title}: {e}"))
    }
}

#[derive(Resource)]
struct Picker { choices: Vec<Choice>, selected: usize, chosen: Option<Choice>, skinned: bool }

const ADD_SKATE2_TITLE: &str = "add skate 2...";
const ADD_SKATE2_DESCRIPTION: &str = "Own Skate 2? Choose its disc (and DLC) to add its city, parks, music and skaters.";
impl Choice {
    fn title(self) -> &'static str { match self { Choice::Edition(e) => e.title(), Choice::AddSkate2 => ADD_SKATE2_TITLE } }
    fn description(self) -> &'static str { match self { Choice::Edition(e) => e.description(), Choice::AddSkate2 => ADD_SKATE2_DESCRIPTION } }
}

#[derive(Component)]
struct Row(usize);
#[derive(Component)]
struct RowText(usize);
#[derive(Component)]
struct Description;

fn pick(assets: &std::path::Path, editions: Vec<Edition>, offer_skate2: bool) -> Option<Choice> {
    let mut choices: Vec<Choice> = editions.into_iter().map(Choice::Edition).collect();
    if offer_skate2 { choices.push(Choice::AddSkate2); }
    let selected = editions::last(assets).and_then(|last| choices.iter().position(|&c| c == Choice::Edition(last))).unwrap_or(0);
    let chosen = std::sync::Arc::new(std::sync::Mutex::new(None));
    let mut app = App::new();
    app.add_plugins(DefaultPlugins
        .set(AssetPlugin { file_path: assets.to_string_lossy().into_owned(), ..default() })
        .set(WindowPlugin {
            primary_window: Some(Window { title: "Skate Rust".into(), resolution: (1280, 800).into(), ..default() }),
            ..default()
        })
        .set(RenderPlugin {
            // Same adapter settings as the game (app.rs).
            render_creation: RenderCreation::Automatic(WgpuSettings {
                backends: Some(Backends::VULKAN), instance_flags: InstanceFlags::empty(), ..default()
            }),
            ..default()
        }).build().disable::<bevy::log::LogPlugin>().disable::<bevy::gilrs::GilrsPlugin>())
        .add_plugins(crate::menu_skin::MenuSkinPlugin)
        .insert_resource(crate::menu_skin::SkinRoot(assets.to_path_buf()))
        .insert_resource(ClearColor(Color::BLACK))
        .insert_resource(Picker { choices, selected, chosen: None, skinned: assets.join("private/ui/menu/font.json").is_file() })
        .init_resource::<crate::customiser::Navigation>()
        .add_systems(Startup, spawn)
        .add_systems(Update, (crate::customiser::navigation, interact, draw).chain());
    let result = chosen.clone();
    app.add_systems(Last, move |picker: Res<Picker>, mut exit: MessageWriter<AppExit>| {
        if let Some(choice) = picker.chosen {
            *result.lock().unwrap() = Some(choice);
            exit.write(AppExit::Success);
        }
    });
    app.run();
    let choice = *chosen.lock().unwrap();
    choice
}

/// Bitmap retail font when the menu skin is installed, else Bevy's text.
fn text(entity: &mut EntityCommands, skinned: bool, value: &str, size: f32, color: Color) {
    if skinned { entity.insert(BitmapText { text: value.into(), size, color, glow: None }); }
    else { entity.insert((Text::new(value), TextFont { font_size: size, ..default() }, TextColor(color))); }
}

fn spawn(mut commands: Commands, picker: Res<Picker>) {
    commands.spawn(Camera2d);
    let skinned = picker.skinned;
    commands.spawn((Node {
        width: percent(100), height: percent(100), flex_direction: FlexDirection::Column,
        align_items: AlignItems::Center, justify_content: JustifyContent::Center, row_gap: px(18), ..default()
    }, BackgroundColor(Color::BLACK))).with_children(|screen| {
        let mut title = screen.spawn((Node { margin: UiRect::bottom(px(24)), ..default() }, Pickable::IGNORE));
        text(&mut title, skinned, "select game", 40.0, BREADCRUMB);
        for (i, edition) in picker.choices.iter().enumerate() {
            screen.spawn((Button, Row(i), Node { padding: UiRect::axes(px(32), px(6)), ..default() }))
                .with_children(|row| { text(&mut row.spawn((Node::default(), Pickable::IGNORE, RowText(i))), skinned, edition.title(), 28.0, IDLE); });
        }
        let mut description = screen.spawn((Node { margin: UiRect::top(px(24)), ..default() }, Pickable::IGNORE, Description));
        text(&mut description, skinned, "", 20.0, DESCRIPTION);
    });
}

fn interact(
    mut picker: ResMut<Picker>,
    nav: Res<crate::customiser::Navigation>,
    keys: Res<ButtonInput<KeyCode>>,
    rows: Query<(&Row, &Interaction), Changed<Interaction>>,
    mut moved: MessageReader<bevy::window::CursorMoved>,
    mut exit: MessageWriter<AppExit>,
) {
    // A resting pointer over a row must not steal the remembered choice.
    let pointer = moved.read().count() > 0;
    let count = picker.choices.len();
    if nav.pressed & 1 != 0 { picker.selected = (picker.selected + count - 1) % count; }
    if nav.pressed & 2 != 0 { picker.selected = (picker.selected + 1) % count; }
    let mut confirm = keys.just_pressed(KeyCode::Enter) || keys.just_pressed(KeyCode::Space) || nav.pressed & 0x1000 != 0;
    for (row, interaction) in &rows {
        match interaction {
            Interaction::Hovered if pointer => picker.selected = row.0,
            Interaction::Hovered => {}
            Interaction::Pressed => { picker.selected = row.0; confirm = true; }
            Interaction::None => {}
        }
    }
    if confirm { picker.chosen = Some(picker.choices[picker.selected]); }
    if keys.just_pressed(KeyCode::Escape) { exit.write(AppExit::Success); }
}

fn draw(
    picker: Res<Picker>,
    mut bitmaps: Query<(&mut BitmapText, Option<&RowText>, Has<Description>)>,
    mut plain: Query<(&mut Text, &mut TextColor, &mut TextFont, Option<&RowText>, Has<Description>)>,
) {
    if !picker.is_changed() { return; }
    let description = picker.choices[picker.selected].description();
    let style = |i: usize| if i == picker.selected { (SELECTED, 34.0, true) } else { (IDLE, 28.0, false) };
    for (mut bitmap, row, is_description) in &mut bitmaps {
        if is_description { bitmap.text = description.into(); }
        if let Some(RowText(i)) = row {
            let (tint, size, on) = style(*i);
            bitmap.color = tint; bitmap.size = size; bitmap.glow = on.then_some(TEXT_GLOW);
        }
    }
    for (mut text, mut color, mut font, row, is_description) in &mut plain {
        if is_description { text.0 = description.into(); }
        if let Some(RowText(i)) = row {
            let (tint, size, _) = style(*i);
            color.0 = tint; font.font_size = size;
        }
    }
}
