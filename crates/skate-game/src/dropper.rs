//! Object Dropper: pick any movable prop type found in the current district
//! and drop a copy in front of the camera. Copies are ordinary props
//! (props.rs): they fall, settle, get knocked around and can be grabbed.
//! LB + B (or O) opens it; Left/Right choose, A/Enter drop, B/O close.
use bevy::prelude::*;

/// Oldest drops are recycled beyond this many.
const MAX_DROPS: usize = 30;
const LB: u16 = 0x0100;
const A: u16 = 0x1000;
const B: u16 = 0x2000;
const LEFT: u16 = 0x0004;
const RIGHT: u16 = 0x0008;

#[derive(Resource, Default)]
pub(crate) struct Dropper {
    pub open: bool,
    cursor: usize,
    /// (label, source prop index) per distinct object type.
    catalogue: Vec<(String, usize)>,
    generation: Option<u64>,
    /// Prop indices created by the dropper, oldest first.
    drops: Vec<usize>,
    previous_buttons: u16,
}

/// Mesh copies made by the dropper (cleared on district change).
#[derive(Component)]
pub(crate) struct Dropped;
#[derive(Component)]
struct DropperUi;
#[derive(Component)]
struct DropperLabel;

pub(crate) struct DropperPlugin;
impl Plugin for DropperPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Dropper>()
            .add_systems(Startup, spawn_ui)
            .add_systems(Update, update);
    }
}

fn spawn_ui(mut commands: Commands) {
    commands.spawn((
        DropperUi,
        Node {
            position_type: PositionType::Absolute, bottom: vh(10.0), left: percent(0), right: percent(0),
            justify_content: JustifyContent::Center, ..default()
        },
        Visibility::Hidden,
        GlobalZIndex(40),
    )).with_children(|root| {
        root.spawn((
            Node { padding: UiRect::axes(vw(1.6), vh(1.2)), border_radius: BorderRadius::all(vh(0.9)), flex_direction: FlexDirection::Column, align_items: AlignItems::Center, ..default() },
            BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.8)),
        )).with_children(|panel| {
            panel.spawn((Text::new("OBJECT DROPPER"), TextFont { font_size: 16.0, ..default() }, TextColor(Color::srgb(0.4, 0.85, 0.85))));
            panel.spawn((DropperLabel, Text::new(""), TextFont { font_size: 30.0, ..default() }, TextColor(Color::WHITE)));
            panel.spawn((Text::new("Left/Right choose   A / Enter drop   B / O close"), TextFont { font_size: 14.0, ..default() }, TextColor(Color::srgb(0.65, 0.75, 0.8))));
        });
    });
}

/// One entry per template, labelled; duplicate labels get numbered.
fn catalogue(props: &crate::props::PropColliders, skip: &[usize]) -> Vec<(String, usize)> {
    let mut seen = std::collections::HashSet::new();
    let mut entries: Vec<(String, usize)> = props.boxes.iter().enumerate()
        .filter(|(n, b)| b.mass > 0.0 && !skip.contains(n) && seen.insert(b.template.clone()))
        .map(|(n, b)| (b.label.clone(), n))
        .collect();
    entries.sort_by(|a, b| a.0.cmp(&b.0));
    let mut counts = std::collections::HashMap::<String, usize>::new();
    for (label, _) in &entries { *counts.entry(label.clone()).or_default() += 1; }
    let mut seen_labels = std::collections::HashMap::<String, usize>::new();
    for (label, _) in &mut entries {
        if counts[label.as_str()] > 1 {
            let k = seen_labels.entry(label.clone()).or_default();
            *k += 1;
            *label = format!("{label} {k}");
        }
    }
    entries
}

#[allow(clippy::too_many_arguments)]
fn update(
    mut dropper: ResMut<Dropper>,
    mut input: ResMut<crate::input::ControllerInput>,
    keys: Res<ButtonInput<KeyCode>>,
    menu: Option<Res<crate::graphics_menu::Menu>>,
    map: Res<crate::map_transition::CurrentMap>,
    camera: Res<crate::camera::CameraRuntime>,
    skater: Res<crate::physics::SkaterRuntime>,
    mut props: ResMut<crate::props::PropColliders>,
    pieces: Query<(Entity, &crate::props::PropPiece)>,
    dropped: Query<Entity, With<Dropped>>,
    mut ui: Query<&mut Visibility, With<DropperUi>>,
    mut label: Query<&mut Text, With<DropperLabel>>,
    mut commands: Commands,
    time: Res<Time<Real>>,
) {
    // Test hook: SKATE_DEBUG_PROPS=drop opens the dropper and drops the first
    // three catalogue entries, one every second from 4 s.
    let auto = std::env::var("SKATE_DEBUG_PROPS").as_deref() == Ok("drop")
        && (4.0..7.5).contains(&time.elapsed_secs()) && (time.elapsed_secs() % 1.0) < time.delta_secs();
    if auto && !dropper.catalogue.is_empty() {
        dropper.open = true;
        dropper.cursor = dropper.drops.len() % dropper.catalogue.len();
    }
    if dropper.generation != Some(map.generation) {
        dropper.generation = Some(map.generation);
        dropper.catalogue.clear();
        dropper.drops.clear();
        dropper.open = false;
        for entity in &dropped { commands.entity(entity).try_despawn(); }
    }
    if dropper.catalogue.is_empty() && !props.boxes.is_empty() {
        let skip = dropper.drops.clone();
        dropper.catalogue = catalogue(&props, &skip);
    }
    let raw = input.raw_input();
    let pressed = raw.buttons & !dropper.previous_buttons;
    dropper.previous_buttons = raw.buttons;
    let menu_open = menu.as_ref().is_some_and(|m| m.open);
    if menu_open { dropper.open = false; }
    let toggle = keys.just_pressed(KeyCode::KeyO) || (raw.buttons & LB != 0 && pressed & B != 0);
    if !menu_open && toggle && !dropper.catalogue.is_empty() {
        dropper.open = !dropper.open;
    } else if dropper.open && pressed & B != 0 && raw.buttons & LB == 0 {
        dropper.open = false;
    }
    if let Ok(mut v) = ui.single_mut() {
        v.set_if_neq(if dropper.open { Visibility::Inherited } else { Visibility::Hidden });
    }
    if !dropper.open { return; }
    // The dropper owns the pad while open (as replay does).
    input.discard_gameplay();
    let count = dropper.catalogue.len();
    if pressed & LEFT != 0 || keys.just_pressed(KeyCode::ArrowLeft) { dropper.cursor = (dropper.cursor + count - 1) % count; }
    if pressed & RIGHT != 0 || keys.just_pressed(KeyCode::ArrowRight) { dropper.cursor = (dropper.cursor + 1) % count; }
    dropper.cursor %= count;
    let (name, source) = dropper.catalogue[dropper.cursor].clone();
    if let Ok(mut text) = label.single_mut() {
        let shown = format!("<  {name}  >   {}/{count}", dropper.cursor + 1);
        if text.0 != shown { text.0 = shown; }
    }
    if pressed & A == 0 && !keys.just_pressed(KeyCode::Enter) && !auto { return; }
    // In front of where the camera looks, dropped from a little above so it
    // lands and settles instead of spawning inside anything.
    let root = skater.animated_skeleton.roots.animation_to_world[3];
    let root = Vec3::new(root[0], root[1], root[2]);
    let forward = camera.frame.as_ref()
        .map(|f| Vec3::from_array(f.basis.columns[2]).with_y(0.0).normalize_or(Vec3::Z))
        .unwrap_or(Vec3::Z);
    let Some(template) = props.boxes.get(source) else { return };
    let reach = 1.6 + template.half.x.max(template.half.z);
    let yaw = Quat::from_rotation_arc(Vec3::Z, forward);
    let mut copy = crate::props::PropBox {
        position: root + forward * reach + Vec3::Y * (template.half.y + 0.8),
        orientation: yaw * template.rotation0,
        velocity: Vec3::ZERO,
        angular: Vec3::ZERO,
        awake: true,
        settled: true,
        still: 0.0,
        disturbed: false,
        ..template.clone()
    };
    copy.center = copy.position;
    copy.rotation0 = template.rotation0;
    // Recycle the oldest drop past the cap.
    let index = if dropper.drops.len() >= MAX_DROPS {
        let old = dropper.drops.remove(0);
        for (entity, piece) in &pieces { if piece.0 == old { commands.entity(entity).try_despawn(); } }
        props.boxes[old] = copy;
        old
    } else {
        props.boxes.push(copy);
        props.boxes.len() - 1
    };
    dropper.drops.push(index);
    for (entity, piece) in &pieces {
        if piece.0 == source {
            commands.entity(entity).clone_and_spawn().insert((crate::props::PropPiece(index), Dropped));
        }
    }
    info!("Object Dropper: {name} (prop {index})");
}
