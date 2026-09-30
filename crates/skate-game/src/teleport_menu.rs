//! Authored FE travel destinations; private content is produced by the extractor.
use std::path::Path;
use serde::Deserialize;

#[derive(Clone, Deserialize)]
pub(crate) struct Destination {
    pub id: String,
    pub name: String,
    pub map: String,
    pub matrix: Option<[[f32; 4]; 4]>,
    #[serde(default)]
    pub unavailable_reason: Option<String>,
}
#[derive(Deserialize)]
struct Catalog { version: u32, destinations: Vec<Destination> }

pub(crate) fn load(assets: &Path) -> Result<Vec<Destination>, String> {
    let path = assets.join("private/teleports.json");
    let bytes = std::fs::read(&path).map_err(|e| format!("Teleport locations: {e}"))?;
    let catalog: Catalog = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
    if catalog.version != 1 { return Err("Unsupported teleport catalog version".into()); }
    let mut ids = std::collections::HashSet::new();
    for d in &catalog.destinations {
        if d.name.is_empty() || d.id.is_empty() || !ids.insert(&d.id)
            || d.map.is_empty() || d.map.contains(['/', '\\', ':']) || matches!(d.map.as_str(), "." | "..") {
            return Err("Invalid teleport destination identity".into());
        }
        if let Some(m) = d.matrix {
            if !valid_matrix(m) { return Err(format!("Invalid teleport transform: {}", d.name)); }
        }
    }
    Ok(catalog.destinations)
}

/// Retail destinations (when installed) plus imported custom locations: a
/// row for each location (its start) followed by its spots.
fn load_all(assets: &Path) -> Vec<Destination> {
    let mut rows = load(assets).unwrap_or_else(|e| { bevy::log::warn!("Travel destinations: {e}"); vec![] });
    let mut ids: std::collections::HashSet<String> = rows.iter().map(|d| d.id.clone()).collect();
    for loc in crate::custom_locations::all(assets) {
        let map = loc.map_name();
        let start = (loc.start_id(), loc.location.title.clone(), loc.location.destinations[0].matrix);
        let spots = loc.location.destinations.iter().map(|s| (loc.spot_id(s), s.name.clone(), s.matrix));
        for (id, name, matrix) in std::iter::once(start).chain(spots) {
            if name.is_empty() || !valid_matrix(matrix) || !ids.insert(id.clone()) {
                bevy::log::warn!("Custom location {}: skipping spot {id}", loc.key);
                continue;
            }
            rows.push(Destination { id, name, map: map.clone(), matrix: Some(matrix), unavailable_reason: None });
        }
    }
    rows
}

fn valid_matrix(m: [[f32; 4]; 4]) -> bool {
    m.iter().flatten().all(|v| v.is_finite())
        && (0..3).all(|i| m[i][3].abs() < 1e-5)
        && (m[3][3] - 1.).abs() < 1e-5
        && m[2][0] * m[2][0] + m[2][2] * m[2][2] > 1e-6
}

pub(crate) fn same_map(path: &Path, name: &str) -> bool {
    path.file_stem().is_some_and(|stem| stem.to_string_lossy().eq_ignore_ascii_case(name))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejects_invalid_destinations_and_preserves_authored_heading() {
        let mut m = skate_core::physics::skeleton_animation_record::IDENTITY;
        m[2] = [1., 0., 0., 0.];
        m[3] = [350.5, 140.2, -725.3, 1.];
        assert!(valid_matrix(m));
        m[3][1] = f32::NAN;
        assert!(!valid_matrix(m));
        assert!(same_map(Path::new("maps/DownTown.skate"), "Downtown"));
        assert!(!same_map(Path::new("maps/MegaPark.skate"), "University"));
    }
}


use bevy::prelude::*;
/// A list-panel group: a district (or park set) and its destination rows.
struct Group {
    title: String,
    description: String,
    rows: Vec<usize>,
}
#[derive(Resource, Default)]
pub(crate) struct Travel {
    pub open: bool,
    pub closed_this_frame: bool,
    shown: bool,
    rows: Vec<Destination>,
    /// Whether each row is on the loaded district; others load it first.
    local: Vec<bool>,
    debug_done: bool,
    debug_clock: Option<std::time::Instant>,
    generation: Option<u64>,
    map: Option<crate::challenge_map::MapData>,
    /// Destination to reach once a requested district has loaded.
    pending: Option<String>,
    groups: Vec<Group>,
    /// 0: district list, 1: the chosen district's spots.
    level: u8,
    group: usize,
    /// Rows (level 1) or groups (level 0) currently listed.
    items: Vec<usize>,
    cursor: usize,
    /// List position clicked last frame (row or map marker).
    pub(crate) clicked: Option<usize>,
    follow: bool,
}
impl Travel {
    fn title(&self, i: usize) -> String {
        let d = &self.rows[i];
        self.map.as_ref().and_then(|m| m.entries.get(&d.id)).map_or_else(|| d.name.clone(), |e| e.title.clone())
    }
    fn entry(&self, i: usize) -> Option<&crate::challenge_map::Entry> {
        self.map.as_ref()?.entries.get(&self.rows[i].id)
    }
    fn view(&self) -> crate::challenge_map::View {
        if self.level == 0 {
            crate::challenge_map::View {
                heading: None,
                items: self.groups.iter().map(|g| (format!("{} ({})", g.title, g.rows.len()), None)).collect(),
                ids: vec![],
            }
        } else {
            crate::challenge_map::View {
                heading: Some(self.groups[self.group].title.clone()),
                items: self.items.iter().map(|&i| (self.title(i), self.entry(i).and_then(|e| e.position))).collect(),
                ids: self.items.iter().map(|&i| Some(self.rows[i].id.clone())).collect(),
            }
        }
    }
    /// (cursor, row count, description, scroll to cursor) while the map is shown.
    pub(crate) fn presentation(&self) -> Option<(usize, usize, String, bool)> {
        if !(self.open && self.shown) || self.items.is_empty() { return None; }
        let body = if self.level == 0 {
            self.groups[self.items[self.cursor]].description.clone()
        } else {
            let row = self.items[self.cursor];
            let mut body = self.entry(row).and_then(|e| e.description.clone()).unwrap_or_default();
            if !self.local[row] {
                body = format!("{body} (Loads {} first.)", district_label(&self.rows[row].map));
            }
            body
        };
        Some((self.cursor, self.items.len(), body, self.follow))
    }
    pub(crate) fn followed(&mut self) { self.follow = false; }
    fn build_groups(&mut self) {
        let mut used = vec![false; self.rows.len()];
        let mut groups = Vec::new();
        if let Some(map) = &self.map {
            for g in &map.groups {
                let rows: Vec<usize> = g.destinations.iter()
                    .filter_map(|id| self.rows.iter().position(|d| &d.id == id)).collect();
                for &r in &rows { used[r] = true; }
                // A custom location's own spots are listed only while you are there.
                if g.local_only && !rows.iter().any(|&r| self.local[r]) { continue; }
                if !rows.is_empty() {
                    groups.push(Group { title: g.title.clone(), description: g.description.clone(), rows });
                }
            }
        }
        let rest: Vec<usize> = (0..self.rows.len()).filter(|&r| !used[r]).collect();
        if !rest.is_empty() {
            groups.push(Group { title: if groups.is_empty() { "Locations".into() } else { "Other".into() }, description: String::new(), rows: rest });
        }
        self.groups = groups;
    }
    fn show_level(&mut self, level: u8, group: usize, cursor: usize) {
        self.level = level;
        self.group = group;
        self.items = if level == 0 { (0..self.groups.len()).collect() } else { self.groups[group].rows.clone() };
        self.cursor = cursor.min(self.items.len().saturating_sub(1));
        self.shown = false; // rebuild the screen
        self.follow = true;
    }
}
#[derive(Component)] struct TravelRoot;
pub(crate) fn install(app: &mut App) {
    app.init_resource::<Travel>().add_systems(PreUpdate,
        interact.after(crate::customiser::navigation).before(crate::graphics_menu::MenuInput))
        .add_systems(Update, crate::challenge_map::update);
}
/// Display name for a map/district id ("DownTown" -> "Downtown").
pub(crate) fn district_label(map: &str) -> String {
    match map {
        "DownTown" => "Downtown".into(),
        "SkateSchool" => "skate.School".into(),
        "StartPark" => "skate.Park".into(),
        other => other.chars().enumerate().flat_map(|(i, c)| {
            (i > 0 && c.is_uppercase()).then_some(' ').into_iter().chain(std::iter::once(c))
        }).collect(),
    }
}
fn interact(
    mut commands: Commands, mut travel: ResMut<Travel>,
    config: Res<crate::config::Config>, map: Res<crate::map_transition::CurrentMap>,
    keys: Res<ButtonInput<KeyCode>>, nav: Res<crate::customiser::Navigation>,
    mut menu: ResMut<crate::graphics_menu::Menu>, mut skater: ResMut<crate::physics::SkaterRuntime>,
    roots: Query<Entity, With<TravelRoot>>,
    mut transition: ResMut<crate::map_transition::MapTransition>,
    assets: Res<AssetServer>,
    skin: Option<Res<crate::menu_skin::MenuSkin>>,
    mut vehicles: Option<ResMut<crate::modding::vehicles::Vehicles>>,
) {
    travel.closed_this_frame = false;
    // Test hook: SKATE_DEBUG_TRAVEL=<destination id> travels there once, as
    // choosing it on the challenge map does, 8 s in.
    if let Ok(id) = std::env::var("SKATE_DEBUG_TRAVEL") {
        let started = *travel.debug_clock.get_or_insert_with(std::time::Instant::now);
        if !travel.debug_done && !travel.rows.is_empty() && started.elapsed().as_secs_f32() > 8.0 {
            travel.debug_done = true;
            if let Some(row) = travel.rows.iter().position(|d| d.id == id) {
                let d = travel.rows[row].clone();
                info!("SKATE_DEBUG_TRAVEL {id} local={} target={:?}", travel.local[row], d.matrix.map(|m| m[3]));
                if let (true, Some(m)) = (travel.local[row], d.matrix) {
                    if let Err(e) = skater.travel_to(m) { warn!("Travel: {e}"); }
                }
            }
        }
    }
    if travel.generation != Some(map.generation) {
        travel.generation = Some(map.generation); travel.open = false; travel.shown = false;
        for e in &roots { commands.entity(e).despawn(); }
        travel.map = crate::challenge_map::load(&config.asset_root, &assets);
        crate::challenge_map::add_custom(&mut travel.map, &config.asset_root, &assets);
        let mut rows = load_all(&config.asset_root);
        rows.retain(|d| d.matrix.is_some());
        travel.local = rows.iter().map(|d| map.path.as_ref().is_some_and(|p| same_map(p, &d.map))).collect();
        travel.rows = rows;
        travel.build_groups();
        // Opt-in visual check of the challenge map, like SKATE_VERIFY_REPLAY.
        if map.generation == 0 && matches!(std::env::var("SKATE_VERIFY_TRAVEL").as_deref(), Ok("1" | "2")) {
            travel.open = true;
            menu.open = true;
        }
        let target = travel.pending.take().or_else(|| (map.generation == 0).then(|| config.teleport.clone()).flatten());
        if let Some(id) = target {
            let mut found = travel.rows.iter().zip(&travel.local).find(|(d, local)| d.id == id && **local).and_then(|(d, _)| d.matrix);
            // Test hook: SKATE_DEBUG_POS="x,y,z" moves the start to exact coordinates.
            if let (Some(m), Ok(pos)) = (found.as_mut(), std::env::var("SKATE_DEBUG_POS")) {
                let v: Vec<f32> = pos.split(',').filter_map(|c| c.trim().parse().ok()).collect();
                if v.len() == 3 { m[3] = [v[0], v[1], v[2], 1.0]; }
            }
            if let Some(m) = found { if let Err(e) = skater.travel_to(m) { warn!("Travel: {e}"); } }
        }
    }
    if travel.open && travel.items.is_empty() && travel.level == 0 && !travel.groups.is_empty() && !travel.shown {
        // Opening: start on the district you are in, like the retail list.
        let here = travel.groups.iter().position(|g| g.rows.iter().any(|&r| travel.local[r])).unwrap_or(0);
        travel.show_level(0, 0, here);
        if std::env::var("SKATE_VERIFY_TRAVEL").as_deref() == Ok("2") {
            travel.show_level(1, here, 0);
        }
    }
    if travel.open && !travel.shown {
        for e in &roots { commands.entity(e).despawn(); }
        travel.shown = true;
        let root = (TravelRoot, GlobalZIndex(11), Node { position_type: PositionType::Absolute, width: percent(100), height: percent(100), overflow: Overflow::clip(), ..default() },
            BackgroundColor(Color::BLACK));
        let view = travel.view();
        let empty = crate::challenge_map::MapData::placeholder();
        let data = travel.map.as_ref().unwrap_or(&empty);
        crate::challenge_map::spawn(&mut commands, root, data, &view, skin.is_some());
        return;
    }
    if travel.open {
        let count = travel.items.len().max(1);
        if keys.just_pressed(KeyCode::ArrowUp) || nav.pressed & 1 != 0 { travel.cursor = (travel.cursor + count - 1) % count; travel.follow = true; }
        if keys.just_pressed(KeyCode::ArrowDown) || nav.pressed & 2 != 0 { travel.cursor = (travel.cursor + 1) % count; travel.follow = true; }
        let mut accept = keys.just_pressed(KeyCode::Enter) || nav.pressed & 0x1000 != 0;
        // A click selects; clicking the selected row or marker again accepts.
        if let Some(i) = travel.clicked.take() {
            if i == travel.cursor { accept = true; } else { travel.cursor = i; }
        }
        let back = keys.just_pressed(KeyCode::Escape) || nav.pressed & (0x2000 | 0x10) != 0;
        if back {
            if travel.level == 1 {
                let group = travel.group;
                travel.show_level(0, 0, group);
            } else {
                travel.closed_this_frame = menu.open;
                travel.open = false;
                travel.items.clear();
            }
        } else if accept && !travel.items.is_empty() {
            if travel.level == 0 {
                let group = travel.items[travel.cursor];
                travel.show_level(1, group, 0);
            } else {
                let row = travel.items[travel.cursor];
                let d = travel.rows[row].clone();
                if travel.local[row] {
                    if let Some(m) = d.matrix {
                        // Driving: the kart goes there with its driver (moving
                        // only the skater left them seated where they were).
                        let driving = vehicles.as_ref().is_some_and(|v| v.driving().is_some());
                        let result = if driving {
                            let heading = m[2][0].atan2(m[2][2]);
                            vehicles.as_mut().unwrap().relocate_driven(Vec3::new(m[3][0], m[3][1] + 0.5, m[3][2]), heading)
                        } else {
                            skater.travel_to(m)
                        };
                        match result { Ok(()) => menu.open = false, Err(e) => warn!("Travel: {e}") }
                    }
                } else {
                    // Load the district, then teleport when its world is published.
                    match crate::map_library::discover(&config.asset_root).into_iter()
                        .find(|e| e.path.as_ref().is_some_and(|p| same_map(p, &d.map))) {
                        Some(entry) => { travel.pending = Some(d.id.clone()); transition.request(entry); }
                        None => warn!("Travel: district {} is not installed", d.map),
                    }
                }
                travel.closed_this_frame = menu.open;
                travel.open = false;
                travel.items.clear();
                travel.level = 0;
            }
        }
    }
    if !travel.open && travel.shown { for e in &roots { commands.entity(e).despawn(); } travel.shown = false; }
}
