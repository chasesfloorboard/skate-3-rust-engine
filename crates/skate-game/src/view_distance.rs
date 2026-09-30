//! World draw distance. skate_world.rs splits map batches that span more than
//! one CELL into per-cell meshes and tags every world mesh with its bounds;
//! here meshes whose nearest point is beyond the View distance setting are
//! hidden. Collision, grinds and baked shadows are unaffected.
use bevy::prelude::*;

/// Grid size for splitting large batches. Smaller cells cull tighter but add
/// draw calls, and this renderer is CPU-bound on draw calls.
pub(crate) const CELL: f32 = 128.0;
/// Auto limits only maps wider than this (New San Vanelona, not the districts).
pub(crate) const AUTO_EXTENT: f32 = 2000.0;
pub(crate) const AUTO_DISTANCE: f32 = 600.0;
/// Re-test once the camera has moved this far (or the setting changed).
const RETEST: f32 = 4.0;

/// World-space bounds of a world mesh.
#[derive(Component)]
pub(crate) struct WorldCell {
    pub min: Vec3,
    pub max: Vec3,
}

/// Width of the loaded world's ground (largest horizontal side), in metres.
#[derive(Resource)]
pub(crate) struct WorldExtent(pub f32);

pub(crate) fn install(app: &mut App) {
    app.add_systems(PostUpdate, cull.before(bevy::camera::visibility::VisibilitySystems::CheckVisibility));
    app.add_systems(Update, fog);
}

fn cull(
    menu: Res<crate::graphics_menu::Menu>,
    extent: Option<Res<WorldExtent>>,
    camera: Query<&GlobalTransform, With<crate::camera::GameplayCamera>>,
    mut cells: Query<(&WorldCell, &mut Visibility)>,
    added: Query<(), Added<WorldCell>>,
    mut last: Local<Option<(Option<f32>, Vec3)>>,
) {
    let Some(extent) = extent else { return };
    let Ok(camera) = camera.single() else { return };
    let mut limit = menu.view_distance(extent.0);
    // Test hook: SKATE_DEBUG_VIEW_DISTANCE=<metres> (0 = unlimited) overrides the setting.
    if let Some(m) = std::env::var("SKATE_DEBUG_VIEW_DISTANCE").ok().and_then(|v| v.parse::<f32>().ok()) {
        limit = (m > 0.0).then_some(m);
    }
    let eye = camera.translation();
    // New meshes (a map load) always get tested.
    let spawned = !added.is_empty();
    if !spawned && last.is_some_and(|(l, at)| l == limit && at.distance(eye) < RETEST) { return; }
    *last = Some((limit, eye));
    for (cell, mut visibility) in &mut cells {
        let near = eye.clamp(cell.min, cell.max);
        let wanted = match limit {
            Some(limit) if near.distance(eye) > limit => Visibility::Hidden,
            _ => Visibility::Inherited,
        };
        visibility.set_if_neq(wanted);
    }
}

/// Optional Distance fog: the retail world fog is re-aimed to start at half the
/// view distance and turn solid at the cutoff, in a haze that follows the time
/// of day. The authored fog comes back when it is off.
fn fog(
    menu: Res<crate::graphics_menu::Menu>,
    extent: Option<Res<WorldExtent>>,
    mut materials: ResMut<Assets<crate::retail_render::RetailWorldMaterial>>,
    mut frame: ResMut<crate::retail_render::ShadowState>,
    cameras: Query<(Entity, Option<&DistanceFog>), With<crate::camera::GameplayCamera>>,
    mut commands: Commands,
    mut authored: Local<Option<(Vec4, Vec4)>>,
    mut applied: Local<Option<(Vec4, Vec4)>>,
) {
    // Test hook: SKATE_DEBUG_FOG=1 forces the fog on (with SKATE_DEBUG_VIEW_DISTANCE for the limit).
    let debug = std::env::var("SKATE_DEBUG_VIEW_DISTANCE").ok().and_then(|v| v.parse::<f32>().ok()).filter(|m| *m > 0.0);
    let limit = debug.or_else(|| extent.and_then(|e| menu.view_distance(e.0)))
        .filter(|_| menu.view_fog() || std::env::var_os("SKATE_DEBUG_FOG").is_some());
    let target = limit.map(|limit| {
        let sun = ((menu.hour() - 6.0) / 12.0 * std::f32::consts::PI).sin();
        let day = (sun * 2.0 + 0.3).clamp(0.0, 1.0);
        let dark = 1.0 - (1.0 - day) * menu.night_depth().min(1.0) * 0.92;
        let haze = Vec3::new(0.030, 0.038, 0.060).lerp(Vec3::new(0.15, 0.19, 0.26), day) * dark.max(day);
        // f = saturate(d * x + y): 0 at half the limit, 1 at the limit.
        (Vec4::new(2.0 / limit, -1.0, 1.0, 0.0), haze.extend(-1.0))
    });
    // Any material not showing what it should is fixed: new ones (a map load,
    // late foliage) carry the authored fog, which is remembered for Off.
    let stale: Vec<_> = materials.iter().filter_map(|(id, m)| {
        let have = (m.params.fog_ramp, m.params.fog_color);
        if applied.is_none_or(|a| a != have) && target.is_none_or(|t| t != have) { *authored = Some(have); }
        let want = target.or(*authored).unwrap_or(have);
        (have != want && (target.is_some() || applied.is_some_and(|a| a == have))).then_some((id, have, want))
    }).collect();
    // The haze drifts with the hour: materials already fogged skip tiny colour changes.
    let drift = target.zip(*applied).is_some_and(|(t, a)| t.0 == a.0 && t.1.distance(a.1) < 0.002);
    for (id, have, want) in stale {
        if drift && Some(have) == *applied { continue; }
        if let Some(m) = materials.get_mut(id) { m.params.fog_ramp = want.0; m.params.fog_color = want.1; }
    }
    if !drift { *applied = target; }
    // Standard-material meshes (unconverted parts, props before adoption) take
    // Bevy's own fog: same start and end, same haze at the world's exposure.
    for (camera, fog) in &cameras {
        match (limit, target, fog) {
            (Some(limit), Some((_, haze)), _) => {
                let wanted = DistanceFog {
                    color: Color::linear_rgb(haze.x * 2.5, haze.y * 2.5, haze.z * 2.5),
                    falloff: FogFalloff::Linear { start: limit * 0.5, end: limit },
                    ..default()
                };
                if fog.is_none_or(|f| f.color != wanted.color || !matches!(f.falloff, FogFalloff::Linear { end, .. } if end == limit)) {
                    commands.entity(camera).insert(wanted);
                }
            }
            (_, _, Some(_)) => { commands.entity(camera).remove::<DistanceFog>(); }
            _ => {}
        }
    }
    let shown = target.or(*authored);
    if let Some(f) = shown && frame.5 != [f.0, f.1] { frame.5 = [f.0, f.1]; }
}
