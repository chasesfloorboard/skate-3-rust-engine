//! Retail menu look for the game menu, from assets prepared by
//! tools/prepare_menu_skin.py: the disc's Futura Heavy bitmap font, the blue
//! selection chevron and the yellow slider pill. Without them the menu keeps
//! its plain Bevy styling.
use bevy::prelude::*;
use std::collections::HashMap;

const DIRECTORY: &str = "private/ui/menu";
const ROW_SIZE: f32 = 24.0;
const SELECTED_SIZE: f32 = 32.0;
const DESCRIPTION_SIZE: f32 = 20.0;
const BREADCRUMB_SIZE: f32 = 30.0;
/// Retail pause-menu colours: cyan selection, slate idle items.
const SELECTED: Color = Color::srgb(0.55, 0.85, 1.0);
const IDLE: Color = Color::srgb(0.52, 0.62, 0.76);
const DESCRIPTION: Color = Color::srgb(0.40, 0.72, 0.98);
const BREADCRUMB: Color = Color::srgb(0.86, 0.86, 0.84);
const GLOW: Color = Color::srgba(0.16, 0.55, 1.0, 0.16);
/// Tint of the Futura Glow halo behind selected text.
const TEXT_GLOW: Color = Color::srgba(0.30, 0.68, 1.0, 0.95);
/// Solid backing under the icon tiles; it shows through their cut-out
/// symbols (muted slate idle, near white selected, as on the retail menu).
const TILE: Color = Color::srgb(0.47, 0.56, 0.66);
const TILE_SELECTED: Color = Color::srgb(0.86, 0.93, 0.97);
/// Retail tile sides at 1080p.
const TILE_SIZE: f32 = 54.0;
const TILE_SELECTED_SIZE: f32 = 82.0;

/// Icon per menu row (graphics_menu row index); submenus show none.
fn row_icon(row: usize) -> Option<&'static str> {
    Some(match row {
        14 => "challenge_map",
        10 => "edit_skater",
        12 => "call_skater",
        6 | 7 => "district",
        8 => "free_play",
        9 => "quit",
        11 => "party_play",
        15 => "mods",
        16 => "day_night",
        13 => "updates",
        18 => "audio",
        0..=5 | 17 => "options",
        _ => return None,
    })
}
const TAB_ICONS: [&str; 4] = ["tab_main", "tab_online", "tab_extras", "tab_options"];

#[derive(serde::Deserialize)]
struct GlowFont {
    atlas: String,
    characters: HashMap<char, [f32; 7]>,
}

#[derive(serde::Deserialize)]
struct FontFile {
    atlas: String,
    size: f32,
    /// Futura Glow: same glyph layout, soft halo glyphs.
    glow: Option<GlowFont>,
    /// char -> [x, y, width, height, x_offset, y_offset (above baseline), advance]
    characters: HashMap<char, [f32; 7]>,
}

#[derive(Resource)]
pub(crate) struct MenuSkin {
    atlas: Handle<Image>,
    glow: Option<(Handle<Image>, HashMap<char, [f32; 7]>)>,
    pill: Handle<Image>,
    icons: HashMap<String, Handle<Image>>,
    size: f32,
    characters: HashMap<char, [f32; 7]>,
}
impl MenuSkin {
    /// Ascent/descent of Futura Heavy 28 (37 / 6 px), scaled.
    fn ascent(&self, size: f32) -> f32 {
        37.0 * size / self.size
    }
    fn glyph(&self, c: char) -> Option<&[f32; 7]> {
        self.characters.get(&c).or_else(|| self.characters.get(&c.to_ascii_uppercase()))
    }
}

/// Text drawn with the retail bitmap font; glyph nodes are rebuilt on change.
#[derive(Component)]
pub(crate) struct BitmapText {
    pub text: String,
    pub size: f32,
    pub color: Color,
    /// Retail halo behind the letters (Futura Glow), for highlighted text.
    pub glow: Option<Color>,
}

/// Ten-pill volume bar; removed from layout outside the audio page.
#[derive(Component)]
pub(crate) struct PillBar;

#[derive(Component)]
pub(crate) struct Skinned {
    /// Tile container; `backing` shows through the symbol cut-out of `glyph`.
    icon: Entity,
    backing: Entity,
    glyph: Entity,
    name: Entity,
    value: Entity,
    description: Entity,
    pills: Entity,
}
/// Breadcrumb under the banner ("Main", "Options", ...).
#[derive(Component)]
pub(crate) struct Breadcrumb;
/// Icon inside a tab button.
#[derive(Component)]
pub(crate) struct TabIcon(usize);

pub(crate) fn load(mut commands: Commands, config: Res<crate::config::Config>, assets: Res<AssetServer>) {
    let path = config.asset_root.join(DIRECTORY).join("font.json");
    let Ok(bytes) = std::fs::read(&path) else { return };
    let font: FontFile = match serde_json::from_slice(&bytes) {
        Ok(font) => font,
        Err(error) => {
            warn!("Menu skin disabled: {} is invalid: {error}", path.display());
            return;
        }
    };
    commands.insert_resource(MenuSkin {
        atlas: assets.load(format!("{DIRECTORY}/{}", font.atlas)),
        glow: font.glow.map(|g| (assets.load(format!("{DIRECTORY}/{}", g.atlas)), g.characters)),
        pill: assets.load(format!("{DIRECTORY}/pill.png")),
        icons: std::fs::read_dir(config.asset_root.join(DIRECTORY).join("icons")).into_iter().flatten()
            .filter_map(|e| e.ok()?.path().file_stem()?.to_str().map(str::to_owned))
            .map(|name| { let handle = assets.load(format!("{DIRECTORY}/icons/{name}.png")); (name, handle) })
            .collect(),
        size: font.size,
        characters: font.characters,
    });
    info!("Menu skin: retail font and highlights");
}

/// Sizes are authored for 1920x1080 and follow the window, fitting its
/// narrower dimension so portrait or ultrawide windows stay usable.
fn resolution_scale(window: &Window) -> f32 {
    (window.width() / 1920.0).min(window.height() / 1080.0).max(0.5)
}

fn layout(
    skin: Option<Res<MenuSkin>>,
    window: Single<&Window, With<bevy::window::PrimaryWindow>>,
    mut texts: Query<(Entity, Ref<BitmapText>, &mut Node)>,
    mut last_scale: Local<f32>,
    mut commands: Commands,
) {
    let Some(skin) = skin else { return };
    let resolution = resolution_scale(&window);
    let rescaled = *last_scale != resolution;
    *last_scale = resolution;
    for (entity, text, mut node) in &mut texts {
        if !rescaled && !text.is_changed() {
            continue;
        }
        commands.entity(entity).despawn_related::<Children>();
        let size = text.size * resolution;
        let scale = size / skin.size;
        let ascent = skin.ascent(size);
        // '\n' starts a new line; retail paragraphs are set at ~1.3x the size
        // (the font's own 50 px line height is far looser).
        let line_height = 41.0 * scale;
        let mut glyphs = Vec::new();
        let string = text.text.replace('…', "...");
        let mut widest: f32 = 0.0;
        let mut lines = 1;
        // Halo pass (Futura Glow) under the letters when requested.
        let mut passes = Vec::new();
        if let (Some(tint), Some((atlas, table))) = (text.glow, &skin.glow) {
            passes.push((atlas.clone(), table, tint, true));
        }
        passes.push((skin.atlas.clone(), &skin.characters, text.color, false));
        for (atlas, table, tint, halo) in passes {
            let (mut pen, mut line) = (0.0, 0);
            for c in string.chars() {
                if c == '\n' {
                    widest = widest.max(pen);
                    pen = 0.0;
                    line += 1;
                    continue;
                }
                // Advances always come from Futura Heavy so both passes align.
                let Some(&[.., advance]) = skin.glyph(c) else { pen += skin.size * 0.3 * scale; continue };
                if let Some(&[x, y, w, h, x_offset, y_offset, _]) = table.get(&c).or_else(|| table.get(&c.to_ascii_uppercase())) {
                    if w > 0.0 && h > 0.0 {
                        // The halo glyph is drawn slightly enlarged around the letter.
                        let grow = if halo { 0.18 } else { 0.0 };
                        glyphs.push(commands.spawn((
                            Node {
                                position_type: PositionType::Absolute,
                                left: px(pen + (x_offset - w * grow / 2.0) * scale),
                                top: px(line as f32 * line_height + ascent - (y_offset + h * grow / 2.0) * scale),
                                width: px(w * (1.0 + grow) * scale),
                                height: px(h * (1.0 + grow) * scale),
                                ..default()
                            },
                            ImageNode { rect: Some(Rect::new(x, y, x + w, y + h)), color: tint, ..ImageNode::new(atlas.clone()) },
                            Pickable::IGNORE,
                        )).id());
                    }
                }
                pen += advance * scale;
            }
            widest = widest.max(pen);
            lines = line + 1;
        }
        // Empty text takes no room, so hidden descriptions leave no gap.
        node.width = px(widest);
        node.height = if text.text.is_empty() { px(0) } else { px((lines - 1) as f32 * line_height + ascent + 7.0 * scale) };
        commands.entity(entity).add_children(&glyphs);
    }
}

fn bitmap(text: &str, size: f32, color: Color) -> impl Bundle {
    (BitmapText { text: text.into(), size, color, glow: None }, Node::default(), Pickable::IGNORE)
}

fn set(texts: &mut Query<&mut BitmapText>, entity: Entity, text: &str, color: Color) {
    if let Ok(mut current) = texts.get_mut(entity) {
        if current.text != text || current.color != color {
            current.text = text.into();
            current.color = color;
        }
    }
}

/// Splits "Resolution          1280 x 800" at its padding into name and value.
fn split(label: &str) -> (String, String) {
    match label.find("  ") {
        Some(i) => (label[..i].trim().to_owned(), label[i..].trim().to_owned()),
        None => (label.trim().to_owned(), String::new()),
    }
}

fn icon(skin: &MenuSkin, name: &str) -> Handle<Image> {
    skin.icons.get(name).cloned().unwrap_or_default()
}

/// Screen furniture: open layout over a dimmed game, banner and breadcrumb in
/// place of the plain headings, and icon tabs.
pub(crate) fn chrome(
    skin: Option<Res<MenuSkin>>,
    menu: Option<Res<crate::graphics_menu::Menu>>,
    root: Single<(Entity, &mut Node, &mut BackgroundColor, &Children), With<crate::graphics_menu::MenuRoot>>,
    mut panels: Query<(&mut Node, &mut BackgroundColor), (Without<crate::graphics_menu::MenuRoot>, Without<crate::graphics_menu::MenuRow>, Without<TabIcon>, Without<crate::graphics_menu::TabLabel>)>,
    headings: Query<(Entity, &ChildOf), With<crate::graphics_menu::MenuHeading>>,
    parents: Query<&Children>,
    mut tabs: Query<(Entity, &crate::graphics_menu::MenuRow, &Children, &mut BackgroundColor), (Without<crate::graphics_menu::MenuRoot>, Without<TabIcon>)>,
    mut tab_labels: Query<&mut Node, (With<crate::graphics_menu::TabLabel>, Without<crate::graphics_menu::MenuRoot>, Without<TabIcon>, Without<crate::graphics_menu::MenuRow>)>,
    mut tab_icons: Query<(&TabIcon, &mut ImageNode, &mut Node), (Without<crate::graphics_menu::MenuRoot>, Without<crate::graphics_menu::TabLabel>, Without<crate::graphics_menu::MenuRow>)>,
    mut breadcrumb: Query<&mut BitmapText, With<Breadcrumb>>,
    mut done: Local<bool>,
    window: Single<&Window, With<bevy::window::PrimaryWindow>>,
    mut commands: Commands,
) {
    let (Some(skin), Some(menu)) = (skin, menu) else { return };
    let resolution = resolution_scale(&window);
    let (_, mut root_node, mut root_color, root_children) = root.into_inner();
    if !*done {
        *done = true;
        // Retail layout: left column over the dimmed game, no boxed panel.
        root_node.justify_content = JustifyContent::FlexStart;
        root_node.align_items = AlignItems::FlexStart;
        root_node.padding = UiRect { left: vw(17), top: vh(6), ..default() };
        // UI blends in linear light: 0.94 reads as the retail ~75% darkening.
        root_color.0 = Color::srgba(0.0, 0.0, 0.0, 0.94);
        for panel in root_children {
            if let Ok((mut node, mut color)) = panels.get_mut(*panel) {
                node.width = vw(46);
                node.padding = UiRect::ZERO;
                node.row_gap = vh(0.8);
                color.0 = Color::NONE;
            }
        }
        for (i, (entity, parent)) in headings.iter().enumerate() {
            let at = parents.get(parent.parent()).map_or(0, |c| c.iter().position(|e| e == entity).unwrap_or(0));
            let replacement = if i == 0 {
                commands.spawn((
                    Node { width: vw(38), aspect_ratio: Some(8.0), border_radius: BorderRadius::all(px(4)),
                        margin: UiRect::bottom(vh(2)), ..default() },
                    ImageNode::new(icon(&skin, "banner")),
                    BoxShadow::new(Color::srgba(0.0, 0.0, 0.0, 0.6), px(0), px(2), px(0), px(8)),
                )).id()
            } else {
                commands.spawn(Node { flex_direction: FlexDirection::Column, row_gap: px(3), ..default() }).with_children(|crumb| {
                    crumb.spawn((Breadcrumb, BitmapText { text: String::new(), size: BREADCRUMB_SIZE, color: BREADCRUMB, glow: None }, Node::default()));
                    // Fading underline, as under "Career > Main".
                    crumb.spawn(Node { width: vw(20), height: px(2), ..default() }).with_children(|line| {
                        for step in 0..10 {
                            line.spawn((Node { flex_grow: 1.0, ..default() },
                                BackgroundColor(BREADCRUMB.with_alpha(0.7 * (1.0 - step as f32 / 10.0)))));
                        }
                    });
                }).id()
            };
            commands.entity(parent.parent()).insert_children(at, &[replacement]);
            commands.entity(entity).despawn();
        }
    }
    let (tab_name, _) = crate::graphics_menu::TABS[menu.tab()];
    let crumb = if menu.in_tabs() { format!("Skate > {tab_name}") } else { "Skate > Settings".into() };
    for mut text in &mut breadcrumb {
        if text.text != crumb { text.text = crumb.clone(); }
    }
    for (button, row, children, mut color) in &mut tabs {
        let Some(tab) = row.0.checked_sub(crate::graphics_menu::TAB_ROW) else { continue };
        color.0 = Color::NONE;
        if !children.iter().any(|c| tab_icons.contains(c)) {
            // Replace the text tab with the retail icon, once.
            for child in children {
                if let Ok(mut node) = tab_labels.get_mut(*child) { node.display = Display::None; }
            }
            let icon_entity = commands.spawn((TabIcon(tab), Node::default(), ImageNode::default(), Pickable::IGNORE)).id();
            commands.entity(button).add_child(icon_entity);
        }
    }
    for (tab, mut image, mut node) in &mut tab_icons {
        let selected = tab.0 == menu.tab();
        let name = if selected { TAB_ICONS[tab.0].to_owned() } else { format!("{}_dim", TAB_ICONS[tab.0]) };
        let handle = icon(&skin, &name);
        if image.image != handle { image.image = handle; }
        let size = px(if selected { 80.0 } else { 46.0 } * resolution);
        if node.width != size {
            node.width = size;
            node.height = size;
            node.align_self = AlignSelf::FlexEnd;
        }
    }
}

pub(crate) fn apply(
    skin: Option<Res<MenuSkin>>,
    menu: Option<Res<crate::graphics_menu::Menu>>,
    rows: Query<(Entity, &crate::graphics_menu::MenuRow, &Interaction, Option<&Skinned>, &Children)>,
    mut labels: Query<(&crate::graphics_menu::MenuLabel, &Text, &mut Node), (Without<crate::graphics_menu::MenuRow>, Without<PillBar>)>,
    mut bars: Query<&mut Node, (With<PillBar>, Without<crate::graphics_menu::MenuLabel>, Without<crate::graphics_menu::MenuRow>)>,
    mut styles: Query<(&mut BackgroundColor, &mut BoxShadow, &mut Node), (With<crate::graphics_menu::MenuRow>, Without<crate::graphics_menu::MenuLabel>, Without<PillBar>)>,
    mut icons: Query<(&mut ImageNode, &mut Node, &mut Visibility, Option<&mut BackgroundColor>, Option<&mut BoxShadow>), (Without<crate::graphics_menu::MenuRow>, Without<crate::graphics_menu::MenuLabel>, Without<PillBar>, Without<BitmapText>)>,
    parents: Query<&Children>,
    mut texts: Query<(&mut BitmapText, &mut BoxShadow), Without<crate::graphics_menu::MenuRow>>,
    mut visibility: Query<&mut Visibility, (Without<ImageNode>, Without<crate::graphics_menu::MenuRow>)>,
    window: Single<&Window, With<bevy::window::PrimaryWindow>>,
    mut commands: Commands,
) {
    let (Some(skin), Some(menu)) = (skin, menu) else { return };
    let resolution = resolution_scale(&window);
    for (row, index, interaction, skinned, children) in &rows {
        if index.0 >= crate::graphics_menu::TAB_ROW { continue; }
        let Some(skinned) = skinned else {
            // First sight of this row: hide the plain label, add skin nodes.
            for child in children {
                if let Ok((_, _, mut node)) = labels.get_mut(*child) {
                    node.display = Display::None;
                }
            }
            // The texture has a transparent margin around its tile, so the
            // backing is inset to fill only the symbol, never a rim.
            let backing = commands.spawn((Node { position_type: PositionType::Absolute, left: percent(7), right: percent(7),
                top: percent(7), bottom: percent(7), ..default() }, ImageNode { color: Color::NONE, ..default() }, BackgroundColor(Color::NONE), Pickable::IGNORE)).id();
            let glyph = commands.spawn((Node { position_type: PositionType::Absolute, width: percent(100), height: percent(100), ..default() },
                ImageNode::default(), Pickable::IGNORE)).id();
            let icon = commands.spawn((Node::default(), ImageNode { color: Color::NONE, ..default() }, Visibility::Hidden, Pickable::IGNORE))
                .add_children(&[backing, glyph]).id();
            let name = commands.spawn((bitmap("", ROW_SIZE, IDLE), BoxShadow::default())).id();
            let value = commands.spawn((bitmap("", ROW_SIZE, IDLE), BoxShadow::default())).id();
            let description = commands.spawn((bitmap("", DESCRIPTION_SIZE, DESCRIPTION), BoxShadow::default(), Visibility::Hidden)).id();
            let pills = commands.spawn((
                PillBar,
                Node { display: Display::None, column_gap: px(3), margin: UiRect::left(px(10)), align_self: AlignSelf::Center, ..default() },
                Pickable::IGNORE,
            )).with_children(|bar| {
                for _ in 0..10 {
                    bar.spawn((Node { width: px(12), height: px(9), ..default() }, ImageNode::new(skin.pill.clone()), Pickable::IGNORE));
                }
            }).id();
            let line = commands.spawn((Node { column_gap: vw(1.2), align_items: AlignItems::FlexEnd, ..default() }, Pickable::IGNORE))
                .add_children(&[name, value, pills]).id();
            let text = commands.spawn((Node { flex_direction: FlexDirection::Column, ..default() }, Pickable::IGNORE))
                .add_children(&[line, description]).id();
            commands.entity(row).add_children(&[icon, text])
                .insert((Skinned { icon, backing, glyph, name, value, description, pills }, BoxShadow::default()));
            continue;
        };
        let Some(label) = children.iter().find_map(|c| labels.get(c).ok().filter(|(l, ..)| l.0 == index.0)) else { continue };
        let (name, mut value) = split(&label.1.0);
        let selected = index.0 == menu.selected() || *interaction == Interaction::Hovered;
        // Audio levels: the plain "#####     50%" bar becomes ten pills.
        let level = (menu.in_audio() && value.ends_with('%'))
            .then(|| value.trim_start_matches('#').trim().trim_end_matches('%').parse::<u32>().ok())
            .flatten();
        if let Some(level) = level {
            value = format!("{level}%");
            if let Ok(bar) = parents.get(skinned.pills) {
                for (i, pill) in bar.iter().enumerate() {
                    if let Ok((_, _, mut v, _, _)) = icons.get_mut(pill) {
                        v.set_if_neq(if (i as u32) < level / 10 { Visibility::Inherited } else { Visibility::Hidden });
                    }
                }
            }
        }
        if let Ok(mut bar) = bars.get_mut(skinned.pills) {
            let display = if level.is_some() { Display::Flex } else { Display::None };
            if bar.display != display { bar.display = display; }
        }
        let size = if selected { SELECTED_SIZE } else { ROW_SIZE };
        let color = if selected { SELECTED } else { IDLE };
        for (entity, text, colour, glow) in [(skinned.name, name.as_str(), color, selected), (skinned.value, value.as_str(), color, selected)] {
            if let Ok((mut bitmap, mut shadow)) = texts.get_mut(entity) {
                let halo = glow.then_some(TEXT_GLOW);
                if bitmap.text != text || bitmap.color != colour || bitmap.size != size || bitmap.glow != halo {
                    bitmap.text = text.into();
                    bitmap.color = colour;
                    bitmap.size = size;
                    bitmap.glow = halo;
                }
                // Faint wash behind the selected title, under the glyph halo.
                shadow.set_if_neq(if glow && entity == skinned.name { BoxShadow::new(GLOW, px(0), px(0), px(4), px(22)) } else { BoxShadow::default() });
            }
        }
        let description = if selected && menu.in_tabs() { crate::graphics_menu::describe(index.0) } else { "" };
        if let Ok((mut bitmap, _)) = texts.get_mut(skinned.description) {
            if bitmap.text != description { bitmap.text = description.into(); }
        }
        if let Ok(mut v) = visibility.get_mut(skinned.description) {
            v.set_if_neq(if description.is_empty() { Visibility::Hidden } else { Visibility::Inherited });
        }
        // Retail tiles: bigger for the selection; submenu rows have none.
        let row_icon = menu.in_tabs().then(|| row_icon(index.0)).flatten();
        let tile = if row_icon.is_none() { Color::NONE } else if selected { TILE_SELECTED } else { TILE };
        if let Ok((_, _, _, Some(mut backing), _)) = icons.get_mut(skinned.backing) {
            backing.set_if_neq(BackgroundColor(tile));
        }
        if let Ok((mut glyph, ..)) = icons.get_mut(skinned.glyph) {
            let handle = row_icon.map(|n| icon(&skin, n)).unwrap_or_default();
            if glyph.image != handle { glyph.image = handle; }
        }
        if let Ok((_, mut node, mut v, ..)) = icons.get_mut(skinned.icon) {
            // Sized like the text (resolution_scale) so portrait windows keep
            // the retail proportions; idle tiles centre under the selected one.
            let side = if row_icon.is_none() { 0.0 } else if selected { TILE_SELECTED_SIZE } else { TILE_SIZE } * resolution;
            if node.width != px(side) {
                node.width = px(side);
                node.height = px(side);
                node.margin = UiRect::horizontal(px(if selected || row_icon.is_none() { 0.0 } else { (TILE_SELECTED_SIZE - TILE_SIZE) / 2.0 * resolution }));
            }
            v.set_if_neq(if row_icon.is_some() { Visibility::Inherited } else { Visibility::Hidden });
        }
        if let Ok((mut background, mut shadow, mut node)) = styles.get_mut(row) {
            background.0 = Color::NONE;
            shadow.set_if_neq(BoxShadow::default());
            if node.column_gap != vw(1.0) {
                node.column_gap = vw(1.0);
                node.padding = UiRect::vertical(vh(0.4));
            }
        }
    }
}

pub(crate) struct MenuSkinPlugin;
impl Plugin for MenuSkinPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, load).add_systems(PostUpdate, layout.before(bevy::ui::UiSystems::Layout));
    }
}
