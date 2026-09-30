//! Challenge Map > Locations, laid out like the retail screen: the city map
//! filling the right of the screen, LT/RT tab strip, a bordered list panel
//! (districts, then a district's spots), a description box and button hints.
//! Data comes from tools/prepare_map_ui.py; the teleport logic lives in
//! teleport_menu.rs. Positions below are measured from retail 16:9 captures.
use bevy::prelude::*;
use std::collections::HashMap;

const DIRECTORY: &str = "private/ui/map";
const SELECTED: Color = Color::srgb(0.55, 0.85, 1.0);
const IDLE: Color = Color::srgb(0.50, 0.62, 0.76);
const GLOW: Color = Color::srgba(0.30, 0.68, 1.0, 0.95);
const HEADING: Color = Color::srgb(0.45, 0.80, 1.0);
const FRAME: Color = Color::srgba(0.55, 0.62, 0.70, 0.85);
const TAB_IDLE_TEXT: Color = Color::srgb(0.45, 0.47, 0.50);
const HINT_TEXT: Color = Color::srgb(0.93, 0.88, 0.74);
/// Retail row pitch (50 px at 1125 px screen height).
pub(crate) const ROW_VH: f32 = 4.44;
const LIST_VH: f32 = 48.0;

#[derive(serde::Deserialize, Clone, Default)]
pub(crate) struct Entry {
    pub title: String,
    /// Location photo, relative to the map directory.
    image: Option<String>,
    pub description: Option<String>,
    pub position: Option<[f32; 2]>,
}

#[derive(serde::Deserialize, Clone)]
pub(crate) struct Group {
    pub title: String,
    #[serde(default)]
    pub description: String,
    pub destinations: Vec<String>,
    /// Listed only while one of its destinations is on the loaded map.
    #[serde(default)]
    pub local_only: bool,
}

pub(crate) struct MapData {
    image: Handle<Image>,
    marker: Handle<Image>,
    ring: Handle<Image>,
    photos: HashMap<String, Handle<Image>>,
    pub entries: HashMap<String, Entry>,
    pub groups: Vec<Group>,
}

#[derive(serde::Deserialize)]
struct MapFile {
    image: String,
    #[serde(default)]
    marker: Option<String>,
    #[serde(default)]
    ring: Option<String>,
    #[serde(default)]
    groups: Vec<Group>,
    destinations: HashMap<String, Entry>,
}

impl MapData {
    /// Plain list when the challenge-map assets are not installed.
    pub(crate) fn placeholder() -> Self {
        Self { image: default(), marker: default(), ring: default(), photos: HashMap::new(), entries: HashMap::new(), groups: vec![] }
    }
}

pub(crate) fn load(asset_root: &std::path::Path, assets: &AssetServer) -> Option<MapData> {
    // The retail challenge map is Port Carverton's.
    if !crate::editions::current().shows(Some(crate::editions::Game::Skate3)) { return None; }
    let bytes = std::fs::read(asset_root.join(DIRECTORY).join("map.json")).ok()?;
    let file: MapFile = serde_json::from_slice(&bytes)
        .map_err(|e| warn!("Challenge map disabled: {e}"))
        .ok()?;
    let load = |name: Option<&String>| name.map(|n| assets.load(format!("{DIRECTORY}/{n}"))).unwrap_or_default();
    Some(MapData {
        image: assets.load(format!("{DIRECTORY}/{}", file.image)),
        marker: load(file.marker.as_ref()),
        ring: load(file.ring.as_ref()),
        photos: file.destinations.iter()
            .filter_map(|(id, e)| Some((id.clone(), assets.load(format!("{DIRECTORY}/{}", e.image.as_ref()?)))))
            .collect(),
        entries: file.destinations,
        groups: file.groups,
    })
}

/// Imported locations (custom_locations.rs): a "Custom Locations" group with
/// one row per location, plus each location's own spots while you are there.
pub(crate) fn add_custom(data: &mut Option<MapData>, asset_root: &std::path::Path, assets: &AssetServer) {
    let locations = crate::custom_locations::all(asset_root);
    if locations.is_empty() { return; }
    let data = data.get_or_insert_with(MapData::placeholder);
    let mut starts = Vec::new();
    for loc in &locations {
        let l = &loc.location;
        let photo = l.image.as_ref().map(|i| assets.load(loc.asset(i)));
        let id = loc.start_id();
        data.entries.insert(id.clone(), Entry { title: l.title.clone(), image: None,
            description: Some(l.description.clone()).filter(|d| !d.is_empty()), position: None });
        if let Some(p) = &photo { data.photos.insert(id.clone(), p.clone()); }
        starts.push(id);
        let mut spots = Vec::new();
        for s in &l.destinations {
            let id = loc.spot_id(s);
            data.entries.insert(id.clone(), Entry { title: format!("{} - {}", l.title, s.name), image: None, description: s.description.clone(), position: None });
            if let Some(p) = s.image.as_ref().map(|i| assets.load(loc.asset(i))).or_else(|| photo.clone()) {
                data.photos.insert(id.clone(), p);
            }
            spots.push(id);
        }
        starts.extend(spots.into_iter().skip(1));
    }
    data.groups.push(Group { title: "Custom Spots".into(),
        description: "Maps imported from other games. Choosing one loads it.".into(), destinations: starts, local_only: false });
}

/// A row of the list panel, by list position.
#[derive(Component)]
pub(crate) struct ListItem(pub usize);
/// A map marker, by list position; its child ring shows the selection.
#[derive(Component)]
pub(crate) struct Marker(pub usize);
#[derive(Component)]
pub(crate) struct SelectionRing;
#[derive(Component)]
pub(crate) struct ListScroll;
#[derive(Component)]
pub(crate) struct Description;

/// What the screen shows at the current level.
pub(crate) struct View {
    /// District title above the spots (level 1), none for the district list.
    pub heading: Option<String>,
    /// Label and optional map position for each list row.
    pub items: Vec<(String, Option<[f32; 2]>)>,
    /// Destination id per row (level 1), for its photo icon.
    pub ids: Vec<Option<String>>,
}

/// Retail Futura bitmap text when the menu skin is installed, plain text otherwise.
fn text<'a>(parent: &'a mut ChildSpawnerCommands, skin: bool, value: &str, size: f32, color: Color) -> EntityCommands<'a> {
    if skin {
        parent.spawn((crate::menu_skin::BitmapText { text: value.into(), size, color, glow: None }, Node::default(), Pickable::IGNORE))
    } else {
        parent.spawn((Text::new(value), TextFont { font_size: size * 0.85, ..default() }, TextColor(color), Pickable::IGNORE))
    }
}

/// Frame edges per side, so panels stay open to the map on their right.
fn frame(left: bool, top: bool, bottom: bool) -> UiRect {
    UiRect {
        left: if left { px(2) } else { px(0) },
        top: if top { px(2) } else { px(0) },
        bottom: if bottom { px(2) } else { px(0) },
        right: px(0),
    }
}

fn badge(parent: &mut ChildSpawnerCommands, skin: bool, letter: &str, color: Color, label: &str) {
    parent.spawn(Node { align_items: AlignItems::Center, column_gap: vw(0.5), ..default() }).with_children(|hint| {
        hint.spawn((
            Node { width: vh(3.3), height: vh(3.3), border_radius: BorderRadius::MAX, justify_content: JustifyContent::Center,
                align_items: AlignItems::Center, border: UiRect::all(px(2)), ..default() },
            BackgroundColor(color),
            BorderColor::all(Color::srgba(1.0, 1.0, 1.0, 0.35)),
        )).with_children(|b| { text(b, skin, letter, 22.0, Color::WHITE); });
        text(hint, skin, label, 30.0, HINT_TEXT);
    });
}

pub(crate) fn spawn(commands: &mut Commands, root: impl Bundle, data: &MapData, view: &View, skin: bool) {
    commands.spawn(root).with_children(|screen| {
        // City map, zoomed and offset as the retail screen frames it.
        screen.spawn((
            Node { position_type: PositionType::Absolute, left: vw(27.55), top: vh(-3.9), width: vh(122.8), height: vh(122.8), ..default() },
            ImageNode::new(data.image.clone()),
        )).with_children(|map| {
            // Retail shows markers only inside a district.
            if view.heading.is_none() { return; }
            for (i, (_, position)) in view.items.iter().enumerate() {
                let Some([u, v]) = *position else { continue };
                map.spawn((
                    Button, Marker(i),
                    Node { position_type: PositionType::Absolute, left: percent(u * 100.0), top: percent(v * 100.0),
                        width: vh(3.4), height: vh(3.4), margin: UiRect { left: vh(-1.7), top: vh(-1.7), ..default() }, ..default() },
                    ImageNode::new(data.marker.clone()),
                )).with_children(|marker| {
                    // Four mirrored copies of the retail arc form the selection ring.
                    marker.spawn((SelectionRing, Visibility::Hidden, Pickable::IGNORE,
                        Node { position_type: PositionType::Absolute, left: vh(-2.1), top: vh(-2.1), width: vh(7.6), height: vh(7.6),
                            flex_wrap: FlexWrap::Wrap, ..default() }))
                        .with_children(|ring| {
                            // The arc texture is the top-left quarter; mirror it into the others.
                            for (flip_x, flip_y) in [(false, false), (true, false), (false, true), (true, true)] {
                                ring.spawn((Node { width: percent(50), height: percent(50), ..default() },
                                    ImageNode { flip_x, flip_y, ..ImageNode::new(data.ring.clone()) }, Pickable::IGNORE));
                            }
                        });
                });
            }
        });
        // Darken the left of the screen behind the panels.
        screen.spawn((Node { position_type: PositionType::Absolute, left: px(0), top: px(0), width: vw(50), height: percent(100), ..default() }, Pickable::IGNORE))
            .with_children(|shade| {
                for step in 0..20 {
                    let alpha = 0.92 * (1.0 - (step as f32 / 20.0).powf(1.6));
                    shade.spawn((Node { flex_grow: 1.0, height: percent(100), ..default() }, BackgroundColor(Color::srgba(0.0, 0.0, 0.0, alpha)), Pickable::IGNORE));
                }
            });
        // LT  Challenges  Completed  Locations  RT
        screen.spawn(Node { position_type: PositionType::Absolute, left: vw(19.0), top: vh(7.8), height: vh(3.9), column_gap: vw(0.6), align_items: AlignItems::FlexEnd, ..default() })
            .with_children(|tabs| {
                let bumper = |tabs: &mut ChildSpawnerCommands, label: &str| {
                    tabs.spawn((Node { width: vw(2.7), height: vh(3.3), border_radius: BorderRadius::all(px(4)), justify_content: JustifyContent::Center,
                        align_items: AlignItems::Center, margin: UiRect::bottom(vh(0.4)), ..default() },
                        BackgroundColor(Color::srgb(0.82, 0.82, 0.80)))).with_children(|b| { text(b, skin, label, 20.0, Color::srgb(0.12, 0.12, 0.12)); });
                };
                bumper(tabs, "LT");
                for (label, width, active) in [("Challenges", 18.4, false), ("Completed", 18.25, false), ("Locations", 17.25, true)] {
                    tabs.spawn((
                        Node { width: vw(width), height: vh(3.9), justify_content: JustifyContent::Center, align_items: AlignItems::Center,
                            border: UiRect { left: px(2), right: px(2), top: px(2), bottom: px(0) },
                            border_radius: BorderRadius::top(px(4)), ..default() },
                        BackgroundColor(if active { Color::srgba(0.02, 0.03, 0.04, 0.95) } else { Color::srgba(0.13, 0.14, 0.15, 0.92) }),
                        BorderColor::all(if active { FRAME } else { Color::srgba(0.30, 0.33, 0.36, 0.9) }),
                    )).with_children(|t| { text(t, skin, label, 30.0, if active { SELECTED } else { TAB_IDLE_TEXT }); });
                }
                bumper(tabs, "RT");
            });
        // Tab baseline running over the map.
        screen.spawn((Node { position_type: PositionType::Absolute, left: vw(18.1), top: vh(11.6), width: vw(63.9), height: px(2), ..default() },
            BackgroundColor(FRAME), Pickable::IGNORE));
        // List panel.
        screen.spawn((
            Node { position_type: PositionType::Absolute, left: vw(18.1), top: vh(11.6), width: vw(21.0), height: vh(56.7),
                border: frame(true, false, true), flex_direction: FlexDirection::Column, padding: UiRect { left: vw(0.8), top: vh(2.2), ..default() }, ..default() },
            BorderColor::all(FRAME),
            BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.35)),
        )).with_children(|panel| {
            if let Some(heading) = &view.heading {
                panel.spawn(Node { flex_direction: FlexDirection::Column, align_self: AlignSelf::FlexStart, margin: UiRect::bottom(vh(1.2)), ..default() })
                    .with_children(|h| {
                        text(h, skin, heading, 34.0, HEADING);
                        h.spawn((Node { width: percent(100), height: px(2), ..default() }, BackgroundColor(HEADING)));
                    });
            }
            panel.spawn(Node { flex_direction: FlexDirection::Row, ..default() }).with_children(|body| {
                if view.heading.is_some() {
                    // Scroll rail along the spot list.
                    body.spawn((Node { width: px(3), height: vh(LIST_VH - 2.0), margin: UiRect::right(vw(0.8)), ..default() },
                        BackgroundColor(Color::srgba(0.45, 0.55, 0.65, 0.5))));
                }
                body.spawn((ListScroll, ScrollPosition::default(),
                    Node { height: vh(if view.heading.is_some() { LIST_VH - 2.0 } else { LIST_VH }), flex_grow: 1.0,
                        overflow: Overflow::scroll_y(), flex_direction: FlexDirection::Column, padding: UiRect::left(vw(1.2)), ..default() }))
                    .with_children(|list| {
                        for (i, (label, _)) in view.items.iter().enumerate() {
                            let photo = view.ids.get(i).cloned().flatten().and_then(|id| data.photos.get(&id).cloned());
                            list.spawn((Button, ListItem(i), Node { height: vh(ROW_VH), flex_shrink: 0.0, align_items: AlignItems::Center,
                                column_gap: vw(0.6), ..default() }))
                                .with_children(|row| {
                                    // Small location photo (or the map marker for districts) so
                                    // the destination is recognisable at a glance.
                                    let (image, width) = match photo { Some(p) => (p, vh(6.0)), None => (data.marker.clone(), vh(3.0)) };
                                    row.spawn((Node { width, height: vh(3.0), flex_shrink: 0.0, border_radius: BorderRadius::all(px(3)), ..default() },
                                        ImageNode::new(image), Pickable::IGNORE));
                                    text(row, skin, label, 32.0, IDLE);
                                });
                        }
                    });
            });
        });
        // Description box.
        screen.spawn((
            Node { position_type: PositionType::Absolute, left: vw(18.1), top: vh(70.0), width: vw(22.5), height: vh(15.5),
                border: frame(true, true, true), padding: UiRect { left: vw(0.6), top: vh(1.0), right: vw(0.6), ..default() },
                overflow: Overflow::clip(), ..default() },
            BorderColor::all(FRAME),
            BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.35)),
        )).with_children(|box_| { text(box_, skin, "", 22.0, IDLE).insert(Description); });
        // Button hints.
        screen.spawn(Node { position_type: PositionType::Absolute, left: vw(18.8), top: vh(87.0), column_gap: vw(1.4), align_items: AlignItems::Center, ..default() })
            .with_children(|hints| {
                badge(hints, skin, "A", Color::srgb(0.36, 0.58, 0.10), "Select");
                badge(hints, skin, "B", Color::srgb(0.74, 0.10, 0.08), "Back");
            });
    });
}

/// Word-wraps retail text for the description box (bitmap text does not wrap).
pub(crate) fn wrap(text: &str, width: usize) -> String {
    let mut lines: Vec<String> = vec![String::new()];
    for word in text.split_whitespace() {
        let line = lines.last_mut().unwrap();
        if !line.is_empty() && line.len() + 1 + word.len() > width {
            lines.push(word.to_owned());
        } else {
            if !line.is_empty() { line.push(' '); }
            line.push_str(word);
        }
    }
    lines.join("\n")
}

/// Per-frame presentation: selection colours and glow, marker rings, list
/// scrolling, the description, and clicks handed back to the teleport logic.
pub(crate) fn update(
    mut travel: ResMut<crate::teleport_menu::Travel>,
    window: Single<&Window, With<bevy::window::PrimaryWindow>>,
    items: Query<(&ListItem, &Interaction, &Children)>,
    markers: Query<(&Marker, &Interaction, &Children)>,
    mut rings: Query<&mut Visibility, With<SelectionRing>>,
    mut bitmaps: Query<&mut crate::menu_skin::BitmapText, Without<Description>>,
    mut texts: Query<&mut TextColor, Without<Description>>,
    mut description: Query<(Option<&mut crate::menu_skin::BitmapText>, Option<&mut Text>), With<Description>>,
    mut scroll: Query<(&mut ScrollPosition, &ComputedNode, &Children), With<ListScroll>>,
    rows: Query<&ComputedNode, With<ListItem>>,
    mut wheel: MessageReader<bevy::input::mouse::MouseWheel>,
) {
    let wheel_delta: f32 = wheel.read().map(|e| e.y * if e.unit == bevy::input::mouse::MouseScrollUnit::Line { 36. } else { 1. }).sum();
    let Some((cursor, count, body, follow)) = travel.presentation() else { return };
    if let Some((item, ..)) = items.iter().find(|(_, i, _)| **i == Interaction::Pressed) {
        travel.clicked = Some(item.0);
    }
    if let Some((marker, ..)) = markers.iter().find(|(_, i, _)| **i == Interaction::Pressed) {
        travel.clicked = Some(marker.0);
    }
    for (item, interaction, children) in &items {
        let selected = item.0 == cursor || *interaction == Interaction::Hovered;
        for child in children {
            if let Ok(mut bitmap) = bitmaps.get_mut(*child) {
                let (color, glow) = if selected { (SELECTED, Some(GLOW)) } else { (IDLE, None) };
                if bitmap.color != color || bitmap.glow != glow { bitmap.color = color; bitmap.glow = glow; }
            } else if let Ok(mut color) = texts.get_mut(*child) {
                color.0 = if selected { SELECTED } else { IDLE };
            }
        }
    }
    for (marker, interaction, children) in &markers {
        let show = marker.0 == cursor || *interaction == Interaction::Hovered;
        for child in children {
            if let Ok(mut v) = rings.get_mut(*child) {
                v.set_if_neq(if show { Visibility::Inherited } else { Visibility::Hidden });
            }
        }
    }
    if let Ok((bitmap, plain)) = description.single_mut() {
        if let Some(mut b) = bitmap {
            let wrapped = wrap(&body, 33);
            if b.text != wrapped { b.text = wrapped; }
        } else if let Some(mut t) = plain {
            if t.0 != body { t.0 = body; }
        }
    }
    for (mut p, node, children) in &mut scroll {
        // Measure the laid-out list (logical px, like ScrollPosition) rather
        // than assuming its height: the heading and padding shrink it.
        let scale = node.inverse_scale_factor();
        let row = children.iter().find_map(|c| rows.get(c).ok()).map_or(window.height() * ROW_VH / 100.0, |r| r.size().y * scale);
        let height = node.size().y * scale - (node.padding().min_inset.y + node.padding().max_inset.y) * scale;
        if height <= 0.0 { continue; }
        p.y = (p.y - wheel_delta).clamp(0.0, (count as f32 * row - height).max(0.0));
        if follow {
            let y = cursor as f32 * row;
            p.y = p.y.min(y).max(y + row - height).max(0.0);
        }
    }
    travel.followed();
}
