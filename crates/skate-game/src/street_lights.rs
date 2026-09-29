//! Night lighting from the districts' light fixtures. Retail lighting is baked,
//! so lamps give no light of their own; this finds the fixture meshes (by
//! their albedo texture names), clusters their triangles into individual
//! fixtures and hangs a downward spot light under each head. Lights fade in
//! with the day/night cycle and only the nearest few stay active.
//! retail_world.wgsl adds clustered point/spot lights to the baked lighting.
use bevy::prelude::*;
use skate_data::skate_map::SkateMap;
use std::collections::HashMap;

/// Street-lamp and floodlight models (names inside the retail material
/// definitions). Deliberately narrow: texture sheets such as the wall-lamp
/// one are shared with trees and props, so a generic "light" match lights
/// palm crowns.
const FIXTURE_NAMES: [&str; 9] = ["lamppost", "lampost", "lmppst", "lampole", "lamp_post", "lightpole",
    // DownTown and Industrial street-lamp sheets (DownTown files its posts,
    // heads and diamond lamps under obj_other_…_lamp*).
    "obj_light_sp_dt_", "obj_light_sp_id_", "obj_other_sp_dt_lamp"];
/// Ground-level footlights along paths and road edges.
const FOOT_NAMES: [&str; 1] = ["footlight"];
/// Stadium floodlight banks on towers over parks: the incandescent lamp
/// sheet (MegaPark and others) and the University's floodlight poles.
const STADIUM_NAMES: [&str; 4] = ["_incand", "floodlight", "tflghtpole", "stadiumlight"];
/// Park maps whose towers carry no named lamp sheet get this many floodlights
/// in a ring over the park instead.
const PARK_RING: usize = 4;
/// Wall fixtures, not street lamps.
const NOT_LAMPS: [&str; 3] = ["wall", "sconce", "solar"];
/// Fixtures whose top is less than this above the ground are not lamps.
const MIN_HEIGHT: f32 = 2.5;
/// Bulb banks higher than this above the ground are stadium floodlights.
const TOWER_HEIGHT: f32 = 11.0;
/// Fixture triangles closer than this belong to the same fixture.
const CLUSTER: f32 = 2.5;
/// Active lights at once, nearest the camera first.
const ACTIVE_LIGHTS: usize = 32;

#[derive(Component)]
pub(crate) struct StreetLight {
    intensity: f32,
    /// Floodlights over parks stay on from further away than street lamps.
    priority: f32,
}

/// Retail material definitions carry the texture names (the portable texture
/// records do not): the first of `keys` found, unless it names an excluded kind.
fn material_name(m: &skate_data::skate_map::Material, keys: &[&str], excluded: &[&str]) -> Option<String> {
    let text = String::from_utf8_lossy(m.retail_definition.as_deref().unwrap_or(&[])).to_ascii_lowercase();
    keys.iter().find_map(|k| {
        let at = text.find(k)?;
        let start = text[..at].rfind(|c: char| !(c.is_ascii_alphanumeric() || c == '_')).map_or(0, |s| s + 1);
        let name: String = text[start..].chars().take_while(|c| c.is_ascii_alphanumeric() || *c == '_').take(64).collect();
        (!excluded.iter().any(|n| name.contains(n))).then_some(name)
    })
}

/// Footlight positions on the map.
pub(crate) fn footlights(map: &SkateMap) -> Vec<Vec3> {
    let names: Vec<Option<String>> = map.materials.iter().map(|m| material_name(m, &FOOT_NAMES, &[])).collect();
    clusters(map, &names).into_iter().map(|(low, high, _, _, _)| Vec3::new((low.x + high.x) * 0.5, high.y, (low.z + high.z) * 0.5)).collect()
}

/// Street-lamp heads (position, height above ground) and stadium floodlight
/// banks on the map. Bulb sheets are shared by lamp posts and tower banks, so
/// the two are told apart by how high the fixture stands over the ground.
pub(crate) fn fixtures(map: &SkateMap) -> (Vec<(Vec3, f32)>, Vec<Vec3>) {
    let keys: Vec<&str> = FIXTURE_NAMES.iter().chain(&STADIUM_NAMES).copied().collect();
    let names: Vec<Option<String>> = map.materials.iter().map(|m| material_name(m, &keys, &NOT_LAMPS)).collect();
    debug_names(map, &names);
    let ground = ground_below(map);
    let (mut heads, mut floods) = (Vec::new(), Vec::new());
    for (low, high, down, stadium_sheet, _) in clusters(map, &names) {
        // Ground from collision around (not under) the fixture, whose own pole
        // has collision; a pole sharing the lamp sheet already gives its height.
        let centre = Vec3::new((low.x + high.x) * 0.5, low.y, (low.z + high.z) * 0.5);
        let reach = (high - low).with_y(0.0).length() * 0.5 + 1.0;
        let above = (high.y - ground(centre, reach)).max(high.y - low.y);
        if above < MIN_HEIGHT { continue; }
        if stadium_sheet && above > TOWER_HEIGHT {
            floods.push((low + high) * 0.5);
            continue;
        }
        let height = high.y - low.y;
        let mut groups: Vec<(Vec3, f32)> = Vec::new();
        for p in down.into_iter().filter(|p| p.y > low.y + height * 0.6) {
            match groups.iter_mut().find(|(c, n)| (*c / *n).distance(p) < 0.8) {
                Some((c, n)) => { *c += p; *n += 1.0; }
                None => groups.push((p, 1.0)),
            }
        }
        // No lens or shade found facing down: light the top of the fixture.
        if groups.is_empty() { groups.push((Vec3::new(centre.x, high.y - 0.3, centre.z), 1.0)); }
        heads.extend(groups.into_iter().map(|(c, n)| (c / n, above)));
    }
    if let Ok(path) = std::env::var("SKATE_DEBUG_LIGHTS_DUMP") {
        // Audit: every light-like cluster, and what was placed.
        let broad: Vec<Option<String>> = map.materials.iter()
            .map(|m| material_name(m, &["light", "lght", "lamp", "lmp"], &["lightmap"])).collect();
        let all: Vec<_> = clusters(map, &broad).into_iter().map(|(l, h, _, _, m)| {
            let name = broad[m as usize].clone().unwrap_or_default();
            serde_json::json!({"low": l.to_array(), "high": h.to_array(), "name": name})
        }).collect();
        let dump = serde_json::json!({"map": map.name, "all": all,
            "heads": heads.iter().map(|(h, _)| h.to_array()).collect::<Vec<_>>(),
            "floods": floods.iter().map(|f| f.to_array()).collect::<Vec<_>>()});
        // Backdrop copies of a district share its name; keep the real one.
        if !all.is_empty() { let _ = std::fs::write(format!("{path}-{}.json", map.name), dump.to_string()); }
    }
    (heads, floods)
}

/// Height of the highest collision surface below a point, from collision
/// vertices within a few metres but further than `skip` horizontally (the
/// point's own level when there is none).
fn ground_below(map: &SkateMap) -> impl Fn(Vec3, f32) -> f32 + '_ {
    const CELL: f32 = 4.0;
    let mut grid: HashMap<(i32, i32), Vec<Vec3>> = HashMap::new();
    let mut add = |p: [f32; 3]| grid.entry(((p[0] / CELL).floor() as i32, (p[2] / CELL).floor() as i32)).or_default().push(Vec3::from_array(p));
    for c in &map.geometry.collision { c.points.into_iter().for_each(&mut add); }
    // Retail districts keep their collision in the embedded archive.
    if let Ok(Some(archive)) = crate::skate_world::retail_archive(map) {
        let _ = skate_data::retail_collision::visit_clusters(archive, |_, cluster| {
            for triangle in cluster { triangle.points.into_iter().for_each(&mut add); }
            Ok(())
        });
    }
    move |at: Vec3, skip: f32| {
        let (x, z) = ((at.x / CELL).floor() as i32, (at.z / CELL).floor() as i32);
        (-2..=2).flat_map(|dx| (-2..=2).map(move |dz| (x + dx, z + dz)))
            .filter_map(|cell| grid.get(&cell))
            .flatten()
            .filter(|p| p.y < at.y - 0.5 && p.with_y(0.0).distance(at.with_y(0.0)) > skip)
            .map(|p| p.y)
            .fold(f32::MIN, f32::max)
            .max(at.y - 60.0)
    }
}

/// SKATE_DEBUG_LIGHTS: the matched fixture names (=all: every light-like name).
fn debug_names(map: &SkateMap, names: &[Option<String>]) {
    let Ok(mode) = std::env::var("SKATE_DEBUG_LIGHTS") else { return };
    if mode == "all" {
        let mut all: Vec<String> = map.materials.iter()
            .filter_map(|m| material_name(m, &["light", "lght", "flood", "stadium", "lamp"], &["lightmap"])).collect();
        all.sort(); all.dedup();
        info!("LIGHT_ALL_NAMES {all:?}");
    }
    let mut seen: Vec<&String> = names.iter().flatten().collect();
    seen.sort(); seen.dedup();
    info!("LIGHT_FIXTURE_NAMES {seen:?}");
}

/// Triangles of the named materials grouped into fixtures (horizontally
/// close triangles join): bounds, plus the centres of downward-facing faces
/// (lenses, shades).
fn clusters(map: &SkateMap, names: &[Option<String>]) -> Vec<(Vec3, Vec3, Vec<Vec3>, bool, u32)> {
    let v = &map.geometry.vertices;
    // Fixture triangle centroids, bucketed on a grid for neighbour search.
    let mut points: Vec<(Vec3, Vec3, Vec3, bool)> = Vec::new();
    let mut sheets: Vec<bool> = Vec::new();
    let mut materials: Vec<u32> = Vec::new();
    for t in map.geometry.indices.chunks_exact(3) {
        let [a, b, c] = [t[0], t[1], t[2]].map(|i| &v[i as usize]);
        let Some(Some(name)) = names.get(a.material as usize) else { continue };
        sheets.push(STADIUM_NAMES.iter().any(|k| name.contains(k.trim_start_matches('_'))));
        materials.push(a.material);
        let p = [a, b, c].map(|v| Vec3::from_array(v.position));
        let low = p[0].min(p[1]).min(p[2]);
        let high = p[0].max(p[1]).max(p[2]);
        let down = (p[1] - p[0]).cross(p[2] - p[0]).normalize_or_zero().y < -0.6;
        points.push(((low + high) * 0.5, low, high, down));
    }
    let cell = |p: Vec3| ((p.x / CLUSTER).floor() as i32, (p.z / CLUSTER).floor() as i32);
    let mut grid: HashMap<(i32, i32), Vec<usize>> = HashMap::new();
    for (n, (c, _, _, _)) in points.iter().enumerate() { grid.entry(cell(*c)).or_default().push(n); }
    // Union-find over horizontally close triangles.
    let mut parent: Vec<usize> = (0..points.len()).collect();
    fn root(parent: &mut [usize], mut i: usize) -> usize {
        while parent[i] != i { parent[i] = parent[parent[i]]; i = parent[i]; }
        i
    }
    for n in 0..points.len() {
        let (x, z) = cell(points[n].0);
        for dx in -1..=1 {
            for dz in -1..=1 {
                for &m in grid.get(&(x + dx, z + dz)).map_or(&[][..], |v| v.as_slice()) {
                    if m <= n || points[n].0.with_y(0.0).distance(points[m].0.with_y(0.0)) > CLUSTER { continue; }
                    let (a, b) = (root(&mut parent, n), root(&mut parent, m));
                    if a != b { parent[a] = b; }
                }
            }
        }
    }
    let mut clusters: HashMap<usize, (Vec3, Vec3, Vec<Vec3>, bool, u32)> = HashMap::new();
    for n in 0..points.len() {
        let r = root(&mut parent, n);
        let e = clusters.entry(r).or_insert((Vec3::MAX, Vec3::MIN, Vec::new(), false, materials[n]));
        e.0 = e.0.min(points[n].1);
        e.1 = e.1.max(points[n].2);
        e.3 |= sheets[n];
        if points[n].3 { e.2.push(points[n].0); }
    }
    clusters.into_values().collect()
}

pub(crate) fn spawn(map: &SkateMap, commands: &mut crate::map_render::SceneCommands) {
    let (fixtures, mut floods) = fixtures(map);
    let feet = footlights(map);
    info!("Street lights: {} fixtures, {} floodlight banks, {} footlights on {}", fixtures.len(), floods.len(), feet.len(), map.name);
    for foot in feet {
        // A small warm pool around each footlight.
        commands.spawn((
            Name::new("Footlight"),
            StreetLight { intensity: 3.0e4, priority: 0.5 },
            SpotLight { color: Color::srgb(1.0, 0.86, 0.64), intensity: 0.0, range: 5.0, radius: 0.1, shadows_enabled: false,
                inner_angle: 0.6, outer_angle: 1.45, ..default() },
            Transform::from_translation(foot + Vec3::Y * 0.3).looking_to(Vec3::NEG_Y, Vec3::X),
            Visibility::Hidden,
        ));
    }
    for (head, height) in fixtures {
        // A warm sodium glow spreading wide under each lamp head.
        let (intensity, range) = (3.5e5, (height * 2.0 + 6.0).min(20.0));
        commands.spawn((
            Name::new("Street light"),
            StreetLight { intensity, priority: 1.0 },
            SpotLight { color: Color::srgb(1.0, 0.88, 0.66), intensity: 0.0, range, radius: 0.3, shadows_enabled: false,
                inner_angle: 0.35, outer_angle: 1.35, ..default() },
            Transform::from_translation(head - Vec3::Y * 0.15).looking_to(Vec3::NEG_Y, Vec3::X),
            Visibility::Hidden,
        ));
    }
    // Parks lit only by unnamed tower banks: a ring of floodlights over the
    // spawn area stands in for them.
    let spawn = Vec3::from_array(map.spawn);
    let park = ["park", "school", "maloof", "blackbox"].iter().any(|k| map.name.to_ascii_lowercase().contains(k));
    if floods.is_empty() && park {
        floods = (0..PARK_RING).map(|i| {
            let a = i as f32 / PARK_RING as f32 * std::f32::consts::TAU + 0.4;
            spawn + Vec3::new(a.cos() * 30.0, 22.0, a.sin() * 30.0)
        }).collect();
    }
    // Floodlights aim down and in, toward the middle of what they surround.
    let centre = if floods.is_empty() { spawn } else { floods.iter().copied().sum::<Vec3>() / floods.len() as f32 };
    let ground = spawn.y.min(centre.y - 10.0);
    for bank in floods {
        let target = Vec3::new(centre.x, ground, centre.z);
        let aim = (target - bank).normalize_or(Vec3::NEG_Y);
        let aim = (aim + Vec3::NEG_Y * 0.6).normalize();
        commands.spawn((
            Name::new("Floodlight"),
            StreetLight { intensity: 6.0e6, priority: 4.0 },
            SpotLight { color: Color::srgb(1.0, 0.92, 0.75), intensity: 0.0, range: 90.0, radius: 1.0, shadows_enabled: false,
                inner_angle: 0.3, outer_angle: 1.2, ..default() },
            Transform::from_translation(bank).looking_to(aim, Vec3::Y),
            Visibility::Hidden,
        ));
    }
}

/// Fade lights with the night and keep only the nearest ones active.
pub(crate) fn update(
    state: Res<crate::retail_render::ShadowState>,
    camera: Query<&GlobalTransform, With<crate::camera::GameplayCamera>>,
    mut lights: Query<(Entity, &StreetLight, &GlobalTransform, &mut SpotLight, &mut Visibility)>,
    mut order: Local<Vec<(f32, Entity)>>,
    menu: Option<Res<crate::graphics_menu::Menu>>,
) {
    let limit = menu.as_ref().map_or(ACTIVE_LIGHTS, |m| m.street_light_limit().min(ACTIVE_LIGHTS));
    let night = state.1.z.clamp(0.0, 1.0);
    let on = ((night - 0.2) / 0.4).clamp(0.0, 1.0);
    let Some(eye) = camera.iter().next().map(GlobalTransform::translation) else { return };
    order.clear();
    if on > 0.0 {
        order.extend(lights.iter().map(|(e, light, t, _, _)| (t.translation().distance_squared(eye) / (light.priority * light.priority), e)));
        order.sort_by(|a, b| a.0.total_cmp(&b.0));
        order.truncate(limit);
    }
    let active: std::collections::HashSet<Entity> = order.iter().map(|(_, e)| *e).collect();
    if std::env::var("SKATE_DEBUG_LIGHTS").is_ok() && !order.is_empty() {
        let first = order[0].1;
        let at = lights.get(first).map(|(_, _, t, s, v)| (t.translation(), s.intensity, *v)).ok();
        info!("LIGHTS_ACTIVE n={} on={on:.2} nearest={:.1}m eye={eye:.1?} first={at:.1?}", order.len(), order[0].0.sqrt());
    }
    for (e, light, _, mut spot, mut visibility) in &mut lights {
        let on_now = active.contains(&e);
        visibility.set_if_neq(if on_now { Visibility::Inherited } else { Visibility::Hidden });
        let intensity = if on_now { light.intensity * on } else { 0.0 };
        if spot.intensity != intensity { spot.intensity = intensity; }
    }
}
