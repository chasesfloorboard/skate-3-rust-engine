//! Opt-in leak watch: SKATE_DEBUG_SOAK=<seconds> logs frame rate, entity and
//! asset counts and resident memory every interval, so growth while a map
//! stays loaded shows up in the log.
use bevy::prelude::*;
use std::sync::{Arc, atomic::{AtomicUsize, Ordering}};
use std::time::Instant;
use bevy::render::{Render, RenderApp};

/// Render-world entity count, published by the render app.
#[derive(Resource, Clone, Default)]
struct RenderEntities(Arc<AtomicUsize>, Arc<AtomicUsize>, Arc<AtomicUsize>, Arc<AtomicUsize>, Arc<std::sync::Mutex<String>>);

#[derive(Resource)]
struct Soak {
    interval: f64,
    last: Instant,
    frames: u32,
}

#[derive(Resource, Default)]
struct Archetypes(usize, usize);
fn count_archetypes(world: &mut World) {
    let value = (world.archetypes().len(), world.storages().tables.len());
    let mut out = world.resource_mut::<Archetypes>();
    out.0 = value.0;
    out.1 = value.1;
}

pub(crate) struct SoakPlugin;
impl Plugin for SoakPlugin {
    fn build(&self, app: &mut App) {
        let Some(interval) = std::env::var("SKATE_DEBUG_SOAK").ok().and_then(|v| v.parse::<f64>().ok()) else {
            return;
        };
        let shared = RenderEntities::default();
        if let Some(render) = app.get_sub_app_mut(RenderApp) {
            render.insert_resource(shared.clone()).add_systems(Render, |world: &mut World| {
                let count = world.entities().len() as usize;
                let archetypes = world.archetypes().len();
                let (slabs, size) = world.get_resource::<bevy::render::mesh::allocator::MeshAllocator>()
                    .map_or((0, 0), |m| (m.slab_count(), (m.slabs_size() >> 20) as usize));
                let materials = world.get_resource::<bevy::pbr::MaterialBindGroupAllocators>().map_or(String::new(), |a| {
                    a.values().filter(|a| a.slab_count() > 0)
                        .map(|a| format!("{}/{}{:?}", a.slab_count(), a.allocations(), a.slab_materials())).collect::<Vec<_>>().join(",")
                });
                let out = world.resource::<RenderEntities>();
                *out.4.lock().unwrap() = materials;
                out.2.store(slabs, Ordering::Relaxed);
                out.3.store(size, Ordering::Relaxed);
                out.0.store(count, Ordering::Relaxed);
                out.1.store(archetypes, Ordering::Relaxed);
            });
        }
        app.insert_resource(shared)
            .insert_resource(Soak { interval: interval.max(1.), last: Instant::now(), frames: 0 })
            .init_resource::<Archetypes>()
            .add_systems(Last, (count_archetypes, report).chain());
    }
}

fn resident_mb() -> f64 {
    std::fs::read_to_string("/proc/self/statm")
        .ok()
        .and_then(|s| s.split_whitespace().nth(1).and_then(|v| v.parse::<f64>().ok()))
        .map_or(0., |pages| pages * 4096. / 1048576.)
}

#[allow(clippy::too_many_arguments)]
fn report(
    mut soak: ResMut<Soak>,
    entities: Query<Entity>,
    meshes: Res<Assets<Mesh>>,
    images: Res<Assets<Image>>,
    standard: Res<Assets<StandardMaterial>>,
    world: Option<Res<Assets<crate::retail_render::RetailWorldMaterial>>>,
    props: Option<Res<Assets<crate::prop_material::PropMaterial>>>,
    characters: Option<Res<Assets<crate::retail_character::CharacterMaterial>>>,
    clips: Option<Res<Assets<crate::pcm_audio::PcmClip>>>,
    audio: Query<(), With<AudioPlayer<crate::pcm_audio::PcmClip>>>,
    lights: Query<(), Or<(With<PointLight>, With<SpotLight>)>>,
    ui: Query<(), With<Node>>,
    menu: Option<Res<crate::graphics_menu::Menu>>,
    render: Res<RenderEntities>,
    archetypes: Res<Archetypes>,
) {
    soak.frames += 1;
    let elapsed = soak.last.elapsed().as_secs_f64();
    if elapsed < soak.interval {
        return;
    }
    let fps = soak.frames as f64 / elapsed;
    soak.frames = 0;
    soak.last = Instant::now();
    info!(
        "SOAK fps={fps:.0} hour={:.1} rss={:.0}MB entities={} render_entities={} archetypes={} tables={} render_archetypes={} slabs={} slab_mb={} material_slabs={} ui={} lights={} audio_players={} meshes={} images={} std_mat={} world_mat={} prop_mat={} char_mat={} clips={}",
        menu.map_or(-1., |m| m.hour()),
        resident_mb(),
        entities.iter().count(),
        render.0.load(Ordering::Relaxed),
        archetypes.0,
        archetypes.1,
        render.1.load(Ordering::Relaxed),
        render.2.load(Ordering::Relaxed),
        render.3.load(Ordering::Relaxed),
        render.4.lock().unwrap(),
        ui.iter().count(),
        lights.iter().count(),
        audio.iter().count(),
        meshes.len(),
        images.len(),
        standard.len(),
        world.map_or(0, |a| a.len()),
        props.map_or(0, |a| a.len()),
        characters.map_or(0, |a| a.len()),
        clips.map_or(0, |a| a.len()),
    );
}
