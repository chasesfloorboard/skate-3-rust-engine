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
