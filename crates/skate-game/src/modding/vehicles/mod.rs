//! Native vehicle host: isolated Rapier world, mod ownership and driver lifecycle.
mod animations;
pub(crate) mod network;
mod audio;
mod engine_sound;
mod interpolation;
use bevy::prelude::*;
use serde_json::{Value, json};
use skate_mods::Command;
use skate_vehicles::{Controls, Simulation, VehicleDefinition};
use std::collections::BTreeMap;
#[derive(Component, Clone)]
struct WheelVisual {
    vehicle: u64,
    index: usize,
    rest: Transform,
}
struct Instance {
    id: u64,
    entity: Entity,
    scene: Handle<Scene>,
    clips: animations::Clips,
    last_control: f32,
    definition_path: String,
    /// Seated driver model (parts.driver), shown in place of the player.
    driver_model: Option<Entity>,
    /// Bike layout (parts.layout): the body and rider lean into turns.
    bike: bool,
    /// Ridden astride (bikes, ATVs): straddle_ rider poses.
    straddled: bool,
    /// Current visual lean (radians about the forward axis, + leans left).
    lean: f32,
    /// Bike wheelie (radians, nose up about the rear axle) and that axle's
    /// position along the body.
    wheelie: f32,
    rear_z: f32,
}
struct Driver {
    owner: String,
    key: String,
    phase: &'static str,
    time: f32,
    return_position: [f32; 3],
    return_heading: f32,
}
#[derive(Resource, Default)]
pub(crate) struct Vehicles {
    simulation: Simulation,
    remote: BTreeMap<u64, network::Target>,
    skaters: BTreeMap<(u64,usize),skate_vehicles::rapier3d::prelude::RigidBodyHandle>,
    owned: BTreeMap<(String, String), Instance>,
    driver: Option<Driver>,
    pub(super) events: Vec<Value>,
    clock: f32,
    hidden: Vec<(Entity, Visibility)>,
    pub(crate) pose: Option<Vec<Mat4>>,
    pub(crate) network_pose: Option<skate_net::packed::PoseState>,
    last_visual: Vec<(Entity, Transform)>,
    blend_from: Vec<(Entity, Transform)>,
    visual_phase: String,
    blend_time: f32,
    steering_visual: f32,
    crash_handoff: bool,
    previous_motion: BTreeMap<u64, interpolation::Motion>,
    rendered_motion: BTreeMap<u64, interpolation::Motion>,
}
impl Vehicles {
    /// Every simulated vehicle as a box: (centre, rotation, half extents,
    /// velocity). props.rs knocks props with these.
    pub(crate) fn boxes(&self) -> Vec<(Vec3, Quat, Vec3, Vec3)> {
        self.simulation.vehicles.values().map(|v| {
            let body = &self.simulation.world.bodies[v.body];
            (Vec3::from_array(body.translation().to_array()), Quat::from_array(body.rotation().to_array()),
                Vec3::from_array(v.definition.half_extents), Vec3::from_array(body.linvel().to_array()))
        }).collect()
    }
    pub(crate) fn has_vehicles(&self) -> bool { !self.owned.is_empty() }
    pub(crate) fn has_obstacle(&self, key: u64) -> bool { self.simulation.has_obstacle(key) }
    pub(crate) fn add_obstacle(&mut self, key: u64, points: &[[f32; 3]]) -> Result<(), String> {
        self.simulation.add_obstacle(key, points)
    }
    pub(crate) fn move_obstacle(&mut self, key: u64, position: [f32; 3], rotation: [f32; 4]) {
        self.simulation.move_obstacle(key, position, rotation);
    }
    pub(crate) fn occupied(&self) -> bool {
        self.driver.is_some()
    }
    /// The driven kart's body position and heading, while seated in it
    /// (session markers are placed and returned to from the kart).
    pub(crate) fn driving(&self) -> Option<(Vec3, f32)> {
        let d = self.driver.as_ref().filter(|d| d.phase == "driving")?;
        let i = self.owned.get(&(d.owner.clone(), d.key.clone()))?;
        let (p, q) = self.simulation.pose(i.id)?;
        let forward = Quat::from_array(q) * Vec3::Z;
        Some((Vec3::from_array(p), forward.x.atan2(forward.z)))
    }
    /// Moves the driven kart (a session marker return).
    pub(crate) fn relocate_driven(&mut self, position: Vec3, heading: f32) -> Result<(), String> {
        let d = self.driver.as_ref().filter(|d| d.phase == "driving").ok_or("Not driving")?;
        let id = self.owned.get(&(d.owner.clone(), d.key.clone())).ok_or("Unknown vehicle")?.id;
        self.simulation.reset(id, position.to_array(), heading)?;
        self.previous_motion.remove(&id);
        self.rendered_motion.remove(&id);
        Ok(())
    }
    pub(super) fn player_pose(&self) -> Option<([f32; 3], f32)> {
        let d = self.driver.as_ref()?;
        let i = self.owned.get(&(d.owner.clone(), d.key.clone()))?;
        let (p, q) = self.simulation.pose(i.id)?;
        let q = Quat::from_array(q);
        let forward = q * Vec3::Z;
        let seat = Vec3::from_array(self.simulation.vehicles[&i.id].definition.seat);
        Some((
            (Vec3::from_array(p) + q * seat).to_array(),
            forward.x.atan2(forward.z),
        ))
    }
    pub(crate) fn camera(&self) -> Option<Transform> {
        let d = self.driver.as_ref()?;
        let i = self.owned.get(&(d.owner.clone(), d.key.clone()))?;
        let pose = self.rendered_motion.get(&i.id)?.body;
        let q = pose.rotation;
        let def = &self.simulation.vehicles[&i.id].definition;
        let center = pose.translation + Vec3::Y * 0.5;
        let forward = (q * Vec3::Z).with_y(0.).normalize_or_zero();
        Some(
            Transform::from_translation(
                center - forward * def.camera_distance + Vec3::Y * def.camera_height,
            )
            .looking_at(center + forward, Vec3::Y),
        )
    }
}
pub(super) fn install(app: &mut App) {
    audio::install(app);
    app.init_resource::<Vehicles>()
        .add_systems(
            FixedUpdate,
            tick.after(super::fixed)
                .run_if(network::simulation_active),
        )
        .add_systems(
            Update,
            present
                .after(crate::app::FrameSet::Animation)
                .before(crate::camera::present),
        );
}
pub(super) fn snapshot(world: &World) -> Value {
    let v = world.resource::<Vehicles>();
    let mut out = serde_json::Map::new();
    for ((owner, key), instance) in &v.owned {
        let Some((p, q)) = v.simulation.pose(instance.id) else {
            continue;
        };
        let car = &v.simulation.vehicles[&instance.id];
        let phase = v
            .driver
            .as_ref()
            .filter(|d| d.owner == *owner && d.key == *key)
            .map_or("parked", |d| d.phase);
        out.entry(owner.clone()).or_insert(json!({})).as_object_mut().unwrap().insert(key.clone(),json!({"position":p,"rotation":q,"heading":({let f=Quat::from_array(q)*Vec3::Z;f.x.atan2(f.z)}),"speed":car.controller.current_vehicle_speed,"phase":phase,"occupied":phase!="parked","ready":world.resource::<AssetServer>().is_loaded_with_dependencies(instance.scene.id())}));
    }
    Value::Object(out)
}
/// A bike's chassis transform tilted by its lean about the ground line
/// under its centre.
fn leaned(body: Transform, i: &Instance, def: &VehicleDefinition) -> Transform {
    if !i.bike || (i.lean == 0.0 && i.wheelie == 0.0) {
        return body;
    }
    let ground = -(def.half_extents[1] + def.suspension_length);
    let pivot = Vec3::Y * ground;
    let tilt = Transform::from_translation(pivot)
        * Transform::from_rotation(Quat::from_rotation_z(-i.lean))
        * Transform::from_translation(-pivot);
    // Wheelie: nose up about the rear tyre's contact with the ground.
    let axle = Vec3::new(0.0, ground, i.rear_z);
    let wheelie = Transform::from_translation(axle)
        * Transform::from_rotation(Quat::from_rotation_x(-i.wheelie))
        * Transform::from_translation(-axle);
    body * tilt * wheelie
}

/// Seconds for the convertible-style hop into or out of the seat.
const HOP: f32 = 0.3;

/// Where the driver lands when hopping out: the definition's exit offset
/// dropped to the floor (else the spot they got in from).
fn exit_spot(v: &Vehicles, id: u64, fallback: ([f32; 3], f32)) -> ([f32; 3], f32) {
    let Some((p, q)) = v.simulation.pose(id) else { return fallback };
    let q = Quat::from_array(q);
    let candidate = (Vec3::from_array(p) + q * Vec3::from_array(v.simulation.vehicles[&id].definition.exit)).to_array();
    match v.simulation.floor(candidate) {
        Some(floor) => {
            let forward = q * Vec3::Z;
            ([floor[0], floor[1] + 0.15, floor[2]], forward.x.atan2(forward.z))
        }
        None => fallback,
    }
}

fn event(v: &mut Vehicles, owner: &str, key: &str, name: &str) {
    v.events.push(json!({"name":name,"owner":owner,"key":key}));
}
fn exit_now(world: &mut World, v: &mut Vehicles, forced: bool) -> Result<(), String> {
    let Some(driver) = v.driver.take() else {
        return Ok(());
    };
    let mut position = driver.return_position;
    let mut heading = driver.return_heading;
    if !forced {
        if let Some(i) = v.owned.get(&(driver.owner.clone(), driver.key.clone())) {
            (position, heading) = exit_spot(v, i.id, (position, heading));
        }
    }
    if let Some(i)=v.owned.get(&(driver.owner.clone(),driver.key.clone())) {v.simulation.set_occupied(i.id,false);}
    v.pose = None;
    for (entity, visibility) in v.hidden.drain(..) {
        if let Some(mut current) = world.get_mut::<Visibility>(entity) {
            *current = visibility;
        }
    }
    let (sin, cos) = heading.sin_cos();
    let matrix = [
        [cos, 0., -sin, 0.],
        [0., 1., 0., 0.],
        [sin, 0., cos, 0.],
        [position[0], position[1], position[2], 0.],
    ];
    let mut skater = world.resource_mut::<crate::physics::SkaterRuntime>();
    if skater.player_input.pending_teleport().is_none() {
        skater
            .player_input
            .request_teleport(matrix)
            .map_err(|e| e.to_string())?;
        skater.teleport_state.request_manual(matrix, false);
    }
    event(v, &driver.owner, &driver.key, "vehicle_exited");
    Ok(())
}
fn eject_now(world: &mut World, v: &mut Vehicles, ejection: skate_vehicles::Ejection) -> Result<(), String> {
    let Some(driver) = v.driver.as_ref() else {return Ok(());};
    let owner=driver.owner.clone(); let key=driver.key.clone();
    let id=v.owned[&(owner.clone(),key.clone())].id;
    let (_,rotation)=v.simulation.pose(id).ok_or("Missing crash vehicle")?;
    let forward=Quat::from_array(rotation)*Vec3::Z;
    let heading=forward.x.atan2(forward.z);
    let (sin,cos)=heading.sin_cos();
    // The native reset initializes a full upright body. Start clear of the seat,
    // then its ordinary ragdoll/contact solver takes over immediately.
    let p=Vec3::from_array(ejection.position)+Vec3::Y*0.25;
    let matrix=[[cos,0.,-sin,0.],[0.,1.,0.,0.],[sin,0.,cos,0.],[p.x,p.y,p.z,0.]];
    {
        let mut skater=world.resource_mut::<crate::physics::SkaterRuntime>();
        skater.player_input.request_teleport(matrix).map_err(|e|e.to_string())?;
        skater.teleport_state.request_vehicle_ejection(matrix,ejection.velocity,ejection.angular_velocity);
    }
    v.simulation.set_occupied(id,false);
    v.simulation.vehicles.get_mut(&id).unwrap().controls=Controls{brake:0.2,..Default::default()};
    v.driver=None;v.pose=None;v.crash_handoff=true;
    for (entity,visibility) in v.hidden.drain(..) {
        if let Some(mut current)=world.get_mut::<Visibility>(entity) {*current=visibility;}
    }
    v.events.push(json!({"name":"vehicle_bailed","owner":owner,"key":key,
        "reason":ejection.reason,"position":ejection.position,"velocity":ejection.velocity,
        "angular_velocity":ejection.angular_velocity}));
    Ok(())
}
pub(super) fn retire(world: &mut World, owner: &str) {
    world.resource_scope(|world, mut v: Mut<Vehicles>| {
        if v.driver.as_ref().is_some_and(|d| d.owner == owner) {
            let _ = exit_now(world, &mut v, true);
        }
        let keys: Vec<_> = v
            .owned
            .keys()
            .filter(|(o, _)| o == owner)
            .cloned()
            .collect();
        for key in keys {
            if let Some(i) = v.owned.remove(&key) {
                v.simulation.remove(i.id);
                v.previous_motion.remove(&i.id);
                v.rendered_motion.remove(&i.id);
                v.remote.remove(&i.id);
                world.despawn(i.entity);
            }
        }
    });
}
pub(super) fn clear(world: &mut World) {
    // A map transition has already installed a new skater: never teleport it back to the old map.
    world.resource_scope(|world, mut v: Mut<Vehicles>| {
        for (_, i) in std::mem::take(&mut v.owned) {
            world.despawn(i.entity);
        }
        for (id, visibility) in v.hidden.drain(..) {
            if let Some(mut current) = world.get_mut::<Visibility>(id) {
                *current = visibility;
            }
        }
        v.driver = None;
        v.pose = None;
        v.last_visual.clear();v.blend_from.clear();v.visual_phase.clear();v.steering_visual=0.;
        v.crash_handoff=false;
        v.previous_motion.clear();
        v.rendered_motion.clear();
        v.remote.clear();
        v.skaters.clear();
        v.network_pose=None;
        v.simulation = Simulation::default();
        v.events.clear();
    });
}
fn ensure_ground(world: &World, v: &mut Vehicles) -> Result<(), String> {
    if v.simulation.world.colliders.is_empty() {
        v.simulation.ground(
            world
                .resource::<crate::physics::GamePhysics>()
                .world_triangles()
                .iter()
                .map(|t| t.triangle.vertices.map(|p| [p.x, p.y, p.z])),
        )?;
    }
    Ok(())
}
pub(super) fn command(
    world: &mut World,
    root: &std::path::Path,
    owner: &str,
    command: Command,
) -> Result<(), String> {
    world.resource_scope(|world, mut v: Mut<Vehicles>| {
        let key = match &command {
            Command::VehicleTune { key, .. }
            | Command::VehicleSpawn { key, .. }
            | Command::VehicleRemove { key }
            | Command::VehicleEnter { key }
            | Command::VehicleExit { key }
            | Command::VehicleReset { key, .. }
            | Command::VehicleControl { key, .. } => key.clone(),
            _ => unreachable!(),
        };
        let owned_key = (owner.to_owned(), key.clone());
        match command {
            Command::VehicleTune { tuning, .. } => {
                let id = v.owned.get(&owned_key).ok_or("Unknown vehicle")?.id;
                let car = v.simulation.vehicles.get_mut(&id).unwrap();
                car.definition = tuning.apply(&car.definition)?;
            }
            Command::VehicleSpawn {
                definition,
                position,
                heading,
                parts,
                ..
            } => {
                if v.owned.contains_key(&owned_key) {
                    return Err("Vehicle key already spawned; remove it before respawning".into());
                }
                if v.owned.keys().filter(|(o, _)| o == owner).count() >= 8 || v.owned.keys().filter(|(o,_)|o.starts_with('@')==owner.starts_with('@')).count() >= if owner.starts_with('@') {288} else {32} {
                    return Err("Vehicle limit: 8 per mod, 32 total".into());
                }
                let bytes = skate_mods::read_bounded(root, &definition, 128 * 1024)?;
                let mut def: VehicleDefinition =
                    serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
                def.validate()?;
                let parts = parts.unwrap_or_default();
                // A body's own cockpit replaces the definition's seat.
                if let Some(seat) = parts.seat {
                    def.seat = seat;
                }
                let body_path = parts.body.clone().unwrap_or_else(|| def.model.clone());
                let bytes = skate_mods::read_bounded(root, &body_path, 32 * 1024 * 1024)?;
                validate_glb(&bytes)?;
                let clips = animations::Clips::load(
                    root,
                    &def,
                    &world
                        .resource::<crate::physics::SkaterRuntime>()
                        .animation
                        .evaluator
                        .frames
                        .bone_names,
                )?;
                // Package-relative model -> asset scene, confined to the mod packages.
                let package_root = super::package_root()
                    .canonicalize()
                    .map_err(|e| e.to_string())?;
                let load_scene = |world: &World, path: &str| -> Result<Handle<Scene>, String> {
                    let model = root.join(path).canonicalize().map_err(|e| format!("{path}: {e}"))?;
                    let relative = model
                        .strip_prefix(&package_root)
                        .map_err(|_| "Model outside mod packages")?
                        .to_string_lossy()
                        .replace('\\', "/");
                    Ok(world.resource::<AssetServer>().load(GltfAssetLabel::Scene(0).from_asset(format!("mods://{relative}"))))
                };
                let scene = load_scene(world, &body_path)?;
                let wheel_scene = parts.wheels.as_deref().map(|p| load_scene(world, p)).transpose()?;
                let driver_scene = parts.driver.as_deref().map(|p| load_scene(world, p)).transpose()?;
                ensure_ground(world, &mut v)?;
                let id = v.simulation.spawn(def.clone(), position, heading)?;
                let entity = world
                    .spawn((
                        Transform::from_translation(Vec3::from_array(position))
                            .with_rotation(Quat::from_rotation_y(heading)),
                        Visibility::default(),
                        crate::prop_material::ProbeLit,
                    ))
                    .id();
                // Headlights at the front corners (+Z is forward), dipped a
                // little so they pool on the road ahead.
                let half = Vec3::from_array(def.half_extents);
                for side in [-1.0f32, 1.0] {
                    world.spawn((
                        SpotLight {
                            color: Color::srgb(1.0, 0.96, 0.85),
                            intensity: 3.0e6,
                            range: 35.0,
                            radius: 0.05,
                            shadows_enabled: false,
                            inner_angle: 0.25,
                            outer_angle: 0.5,
                            ..default()
                        },
                        Transform::from_xyz(side * half.x * 0.6, half.y + 0.25, half.z + 0.1)
                            .looking_to(Vec3::new(0.0, -0.12, 1.0), Vec3::Y),
                        ChildOf(entity),
                    ));
                }
                world.spawn((
                    SceneRoot(scene.clone()),
                    Transform::from_translation(Vec3::from_array(def.model_offset))
                        .with_rotation(Quat::from_rotation_y(def.model_yaw))
                        .with_scale(Vec3::splat(def.model_scale)),
                    ChildOf(entity),
                ));
                // Chosen tyres on every named wheel, at the suspension's rest
                // height, scaled to the physics radius; right-side hubs face out.
                if let Some(wheel_scene) = &wheel_scene {
                    let model_radius = parts.wheel_radius.unwrap_or(0.33);
                    // Bikes: one tyre on the centre line per axle, driven by
                    // that axle's first physics wheel (which it is named after).
                    let mounts: Vec<_> = if parts.bike() {
                        [true, false].into_iter().enumerate().filter_map(|(end, front)| {
                            let axle: Vec<_> = def.wheels.iter().filter(|w| w.steering == front).collect();
                            let first = axle.first()?;
                            let mut position = axle.iter().map(|w| Vec3::from_array(w.position)).sum::<Vec3>() / axle.len() as f32;
                            position.x = 0.0;
                            // The body's own wheel gaps, when the parts name them.
                            if let Some(z) = parts.wheel_z { position.z = z[end]; }
                            Some((first.node.clone(), position.to_array(), first.radius))
                        }).collect()
                    } else {
                        def.wheels.iter().map(|w| (w.node.clone(), w.position, w.radius)).collect()
                    };
                    for (node, position, radius) in mounts {
                        let Some(node) = &node else { continue };
                        let scale = parts.wheel_scale.unwrap_or(1.0);
                        let mount = Vec3::from_array(position) - Vec3::Y * (def.suspension_length + radius * (1.0 - scale));
                        let yaw = if position[0] < 0.0 { std::f32::consts::PI } else { 0.0 };
                        let hub = world.spawn((
                            Name::new(node.clone()),
                            Transform::from_translation(mount),
                            Visibility::default(),
                            ChildOf(entity),
                        )).id();
                        world.spawn((
                            SceneRoot(wheel_scene.clone()),
                            Transform::from_rotation(Quat::from_rotation_y(yaw))
                                .with_scale(Vec3::splat(radius * scale / model_radius)),
                            ChildOf(hub),
                        ));
                    }
                }
                // A seated driver model rides in place of the player.
                let driver_model = driver_scene.map(|driver| world.spawn((
                    SceneRoot(driver),
                    Transform::from_translation(Vec3::from_array(def.seat)),
                    Visibility::Hidden,
                    ChildOf(entity),
                )).id());
                let clock = v.clock;
                v.owned.insert(
                    owned_key,
                    Instance {
                        id,
                        entity,
                        scene,
                        clips,
                        last_control: clock,
                        definition_path: definition,
                        driver_model,
                        bike: parts.bike(),
                        straddled: parts.straddled(),
                        lean: 0.0,
                        wheelie: 0.0,
                        rear_z: parts.wheel_z.map_or_else(|| {
                            let rear: Vec<_> = def.wheels.iter().filter(|w| !w.steering).map(|w| w.position[2]).collect();
                            if rear.is_empty() { -0.6 } else { rear.iter().sum::<f32>() / rear.len() as f32 }
                        }, |z| z[1]),
                    },
                );
                event(&mut v, owner, &key, "vehicle_spawned");
            }
            Command::VehicleRemove { .. } => {
                if v.driver
                    .as_ref()
                    .is_some_and(|d| d.owner == owner && d.key == key)
                {
                    exit_now(world, &mut v, true)?;
                }
                if let Some(i) = v.owned.remove(&owned_key) {
                    v.simulation.remove(i.id);
                v.previous_motion.remove(&i.id);
                v.rendered_motion.remove(&i.id);
                v.remote.remove(&i.id);
                    world.despawn(i.entity);
                    event(&mut v, owner, &key, "vehicle_removed");
                }
            }
            Command::VehicleEnter { .. } => {
                if v.driver.is_some()
                    || world.resource::<crate::replay::Replay>().active
                    || world
                        .resource::<crate::map_transition::MapTransition>()
                        .busy()
                {
                    return Ok(());
                }
                let i = v.owned.get(&owned_key).ok_or("Unknown vehicle")?;
                if !world
                    .resource::<AssetServer>()
                    .is_loaded_with_dependencies(i.scene.id())
                {
                    return Ok(());
                }
                if !world.resource::<crate::animation::AnimationStatus>().ready {
                    return Ok(());
                }
                let skater = world.resource::<crate::physics::SkaterRuntime>();
                if skater.player_input.pending_teleport().is_some()
                    || world
                        .resource::<crate::physics::GamePhysics>()
                        .board_wiping_out
                {
                    return Ok(());
                }
                let m = skater.animated_skeleton.roots.animation_to_world;
                let p = [m[3][0], m[3][1], m[3][2]];
                let car = v.simulation.pose(i.id).ok_or("Missing vehicle body")?.0;
                if Vec3::from_array(p).distance(Vec3::from_array(car)) > 4.
                    || v.simulation.vehicles[&i.id]
                        .controller
                        .current_vehicle_speed
                        .abs()
                        > 3.
                {
                    return Ok(());
                }
                v.driver = Some(Driver {
                    owner: owner.into(),
                    key: key.clone(),
                    phase: "entering",
                    time: 0.,
                    return_position: p,
                    return_heading: m[2][0].atan2(m[2][2]),
                });
                event(&mut v, owner, &key, "vehicle_entering");
            }
            Command::VehicleExit { .. } => {
                if let Some(d) = &v.driver {
                    if d.owner != owner || d.key != key {
                        return Ok(());
                    }
                    if d.phase != "driving" { return Ok(()); }
                }
                // Jumping out at speed: thrown clear into a bail, carrying the
                // kart's momentum (the crash ejection path).
                if let Some(i) = v.owned.get(&owned_key) {
                    let id = i.id;
                    if v.driver.is_some() && v.simulation.vehicles[&id].controller.current_vehicle_speed.abs() > 3. {
                        let body = &v.simulation.world.bodies[v.simulation.vehicles[&id].body];
                        let q = Quat::from_array(body.rotation().to_array());
                        let seat = Vec3::from_array(body.translation().to_array()) + q * Vec3::from_array(v.simulation.vehicles[&id].definition.seat);
                        let velocity = Vec3::from_array(body.linvel().to_array()) + Vec3::Y * 2.5 + q * Vec3::X * 1.5;
                        let ejection = skate_vehicles::Ejection {
                            position: (seat + Vec3::Y * 0.3).to_array(),
                            velocity: velocity.to_array(),
                            angular_velocity: (q * Vec3::new(0.0, 0.0, -3.0)).to_array(),
                            reason: "jumped_out",
                        };
                        return eject_now(world, &mut v, ejection);
                    }
                }
                if let Some(d) = &mut v.driver {
                    d.phase = "exiting";
                    d.time = 0.;
                }
            }
            Command::VehicleControl { controls, .. } => {
                let clock = v.clock;
                let i = v.owned.get_mut(&owned_key).ok_or("Unknown vehicle")?;
                i.last_control = clock;
                let id = i.id;
                v.simulation.vehicles.get_mut(&id).unwrap().controls = controls;
            }
            Command::VehicleReset {
                position, heading, ..
            } => {
                let i = v.owned.get(&owned_key).ok_or("Unknown vehicle")?;
                let id = i.id;
                v.simulation.reset(id, position, heading)?;
                v.previous_motion.remove(&id);
                v.rendered_motion.remove(&id);
                event(&mut v, owner, &key, "vehicle_reset");
            }
            _ => unreachable!(),
        }
        Ok(())
    })
}
fn validate_glb(bytes: &[u8]) -> Result<(), String> {
    if bytes.len() < 20
        || &bytes[..4] != b"glTF"
        || u32::from_le_bytes(bytes[4..8].try_into().unwrap()) != 2
    {
        return Err("Vehicle model must be GLB 2".into());
    }
    let n = u32::from_le_bytes(bytes[12..16].try_into().unwrap()) as usize;
    let json: Value = serde_json::from_slice(bytes.get(20..20 + n).ok_or("Truncated GLB")?)
        .map_err(|e| e.to_string())?;
    for field in ["buffers", "images"] {
        for entry in json[field].as_array().into_iter().flatten() {
            if entry.get("uri").is_some() {
                return Err("Vehicle GLB must embed all buffers and textures".into());
            }
        }
    }
    Ok(())
}
fn tick(world: &mut World) {
    if world.resource::<crate::replay::Replay>().active {
        return;
    }
    let dt = world.resource::<Time<Fixed>>().delta_secs();
    world.resource_scope(|world, mut v: Mut<Vehicles>| {
        v.clock += dt;
        network::skater_proxies(world,&mut v);
        network::advance(&mut v, dt);
        let clock = v.clock;
        let parked: Vec<_> = v
            .owned
            .values()
            .filter(|i| clock - i.last_control > 0.25)
            .map(|i| i.id)
            .collect();
        for id in parked {
            v.simulation.vehicles.get_mut(&id).unwrap().controls = Controls {
                brake: 0.2,
                ..Default::default()
            };
        }
        let driver_info = v
            .driver
            .as_ref()
            .map(|d| (d.owner.clone(), d.key.clone(), d.phase));
        if let Some((owner, key, phase)) = &driver_info {
            if *phase != "driving" {
                if let Some(i) = v.owned.get(&(owner.clone(), key.clone())) {
                    let id = i.id;
                    v.simulation.vehicles.get_mut(&id).unwrap().controls = Controls {
                        brake: 1.,
                        ..Default::default()
                    };
                }
            }
        }
        let occupied_id=v.driver.as_ref().and_then(|d|v.owned.get(&(d.owner.clone(),d.key.clone()))).map(|i|i.id);
        let ids:Vec<_>=v.simulation.vehicles.keys().copied().collect();
        for id in ids { if !v.remote.contains_key(&id) { v.simulation.set_occupied(id,Some(id)==occupied_id); }}
        if !v.owned.is_empty() {
            v.previous_motion = v.simulation.vehicles.keys().filter_map(|&id|
                network::motion(&v, id).map(|m| (id, m))).collect();
            v.simulation.step(dt);
        }
        if let Some(id)=occupied_id {
            if let Some(ejection)=v.simulation.take_ejection(id) {
                if let Err(error)=eject_now(world,&mut v,ejection) {warn!("Vehicle ejection: {error}");}
                return;
            }
        }
        if let Some((owner, key, phase)) = driver_info {
            if v.owned.contains_key(&(owner.clone(), key.clone())) {
                // A quick hop over the side replaces long enter/exit clips.
                let duration = HOP;
                if let Some(d) = &mut v.driver {
                    d.time += dt;
                }
                if phase != "driving" && v.driver.as_ref().unwrap().time >= duration {
                    if phase == "exiting" {
                        if let Err(e) = exit_now(world, &mut v, false) {
                            warn!("Vehicle exit: {e}");
                        }
                    } else {
                        let d = v.driver.as_mut().unwrap();
                        d.phase = "driving";
                        d.time = 0.;
                        event(&mut v, &owner, &key, "vehicle_entered");
                    }
                }
            }
        }
    });
}
pub(crate) fn present(world: &mut World) {
    let dt=world.resource::<Time<Virtual>>().delta_secs();
    let alpha=world.resource::<Time<Fixed>>().overstep_fraction();
    let phase=world.resource::<Vehicles>().driver.as_ref().map_or("vanilla",|d|d.phase).to_owned();
    let current=crate::animation::capture_vehicle_visual(world);
    {
        let mut v=world.resource_mut::<Vehicles>();
        if phase!=v.visual_phase {
            v.blend_from=if v.last_visual.is_empty() {current} else {v.last_visual.clone()};
            v.visual_phase=phase;v.blend_time=0.;
        }
        v.blend_time+=dt;
    }
    world.resource_scope(|world, mut v: Mut<Vehicles>| {
        let failures: Vec<_> = v
            .owned
            .iter()
            .filter_map(|((owner, key), i)| {
                if let Some(bevy::asset::LoadState::Failed(error)) =
                    world.resource::<AssetServer>().get_load_state(i.scene.id())
                {
                    Some((
                        owner.clone(),
                        format!("Vehicle {key} model failed: {error}"),
                    ))
                } else {
                    None
                }
            })
            .collect();
        for (owner, error) in failures {
            world
                .resource_mut::<super::Mods>()
                .manager
                .fail(&owner, error);
        }
        // Chassis, wheels, rider and camera share one fixed-step render sample.
        v.rendered_motion = v.simulation.vehicles.keys().filter_map(|&id| {
            let current = network::motion(&v, id)?;
            let sample = v.previous_motion.get(&id).map_or_else(|| current.clone(),
                |previous| previous.sample(&current, alpha));
            Some((id, sample))
        }).collect();
        // Right stick held down: wheelie on a moving bike.
        let wheelie_held = world.resource::<crate::input::ControllerInput>().raw_input().right[1] < -0.5
            || world.resource::<ButtonInput<KeyCode>>().pressed(KeyCode::ControlLeft);
        let driven = v.driver.as_ref().filter(|d| d.phase == "driving").map(|d| (d.owner.clone(), d.key.clone()));
        let v = &mut *v;
        for (key, i) in v.owned.iter_mut() {
            if let Some(sample) = v.rendered_motion.get(&i.id) {
                if i.bike {
                    // Lean into turns with speed (visual only; the camera and
                    // physics keep the upright chassis).
                    let car = &v.simulation.vehicles[&i.id];
                    let speed = (car.controller.current_vehicle_speed / 8.0).clamp(0.0, 1.0);
                    let target = 0.45 * car.controls.steering * speed;
                    i.lean += (target - i.lean) * (1.0 - (-6.0 * dt).exp());
                    let up = if wheelie_held && driven.as_ref() == Some(key) && car.controller.current_vehicle_speed > 2.0 { 0.4 } else { 0.0 };
                    i.wheelie += (up - i.wheelie) * (1.0 - (-(if up > i.wheelie { 3.0 } else { 6.0 }) * dt).exp());
                }
                if let Some(mut t) = world.get_mut::<Transform>(i.entity) {
                    *t = leaned(sample.body, i, &v.simulation.vehicles[&i.id].definition);
                }
            }
        }
        let mut wheels = Vec::new();
        for instance in v.owned.values() {
            for (index, wheel) in v.simulation.vehicles[&instance.id]
                .definition
                .wheels
                .iter()
                .enumerate()
            {
                let Some(name) = &wheel.node else {
                    continue;
                };
                let matches: Vec<_> = world
                    .query_filtered::<(Entity, &Name, &Transform), Without<WheelVisual>>()
                    .iter(world)
                    .filter(|(_, n, _)| n.as_str() == name)
                    .map(|(e, _, t)| (e, *t))
                    .collect();
                for (entity, rest) in matches {
                    let mut ancestor = entity;
                    let mut owned = false;
                    for _ in 0..128 {
                        let Some(parent) = world.get::<ChildOf>(ancestor) else {
                            break;
                        };
                        ancestor = parent.parent();
                        if ancestor == instance.entity {
                            owned = true;
                            break;
                        }
                    }
                    if owned {
                        wheels.push((
                            entity,
                            WheelVisual {
                                vehicle: instance.id,
                                index,
                                rest,
                            },
                        ));
                    }
                }
            }
        }
        for (entity, wheel) in wheels {
            world.entity_mut(entity).insert(wheel);
        }
        for (wheel, mut transform) in world
            .query::<(&WheelVisual, &mut Transform)>()
            .iter_mut(world)
        {
            if let Some(sample) = v.rendered_motion.get(&wheel.vehicle) {
                if let Some(offset) = sample.wheels.get(wheel.index) {
                    *transform = wheel.rest;
                    transform.translation += offset.translation;
                    transform.rotation = wheel.rest.rotation * offset.rotation;
                }
            }
        }
        v.pose = None;
        // Seated driver models show only while their kart is being driven.
        let driven = v.driver.as_ref().map(|d| (d.owner.clone(), d.key.clone()));
        for (key, instance) in &v.owned {
            let Some(model) = instance.driver_model else { continue };
            let show = driven.as_ref() == Some(key) && v.driver.as_ref().is_some_and(|d| d.phase == "driving");
            if let Some(mut vis) = world.get_mut::<Visibility>(model) {
                vis.set_if_neq(if show { Visibility::Inherited } else { Visibility::Hidden });
            }
        }
        let Some(d) = &v.driver else {
            return;
        };
        let Some(i) = v.owned.get(&(d.owner.clone(), d.key.clone())) else {
            return;
        };
        let replaced = i.driver_model.is_some() && d.phase == "driving";
        let car = &v.simulation.vehicles[&i.id];
        let a = &car.definition.animations;
        let hopping = d.phase != "driving";
        let name = match d.phase {
            _ if hopping => a.idle.as_ref().or(a.drive.as_ref()),
            _ => {
                if car.controls.brake > 0.2 {
                    a.brake.as_ref()
                } else if car.controller.current_vehicle_speed < -0.5 {
                    a.reverse.as_ref()
                } else if car.controller.current_vehicle_speed.abs() > 0.5 {
                    a.drive.as_ref()
                } else {
                    a.idle.as_ref()
                }
            }
        }
        .or(a.drive.as_ref());
        let target_steering=car.controls.steering;
        let steering=v.steering_visual+(target_steering-v.steering_visual)*(1.-(-12.*dt).exp());
        let name = i.clips.variant(name, i.straddled);
        let pose = i.clips.pose(name.as_ref(), if hopping { 0. } else { d.time }, !hopping);
        let turn_name = i.clips.variant(if steering>=0. {a.steer_left.as_ref()} else {a.steer_right.as_ref()}, i.straddled);
        let turn=if d.phase=="driving" {i.clips.pose(turn_name.as_ref(),d.time,true)} else {None};
        let body = leaned(v.rendered_motion[&i.id].body, i, &car.definition);
        let q = body.rotation;
        let seat = body.translation + q * Vec3::from_array(car.definition.seat);
        // Hop: the seated pose arcs between the standing spot and the seat.
        let (seat, q) = if hopping {
            let (spot, heading) = if d.phase == "entering" {
                (d.return_position, d.return_heading)
            } else {
                exit_spot(&v, i.id, (d.return_position, d.return_heading))
            };
            let t = (d.time / HOP).clamp(0., 1.);
            let s = if d.phase == "entering" { t } else { 1. - t };
            let e = s * s * (3. - 2. * s);
            let ground = Vec3::from_array(spot) - Vec3::Y * 0.15;
            let position = ground.lerp(seat, e) + Vec3::Y * 0.7 * 4. * s * (1. - s);
            (position, Quat::from_rotation_y(heading).slerp(q, e))
        } else {
            (seat, q)
        };
        v.steering_visual=steering;
        let roots: Vec<_> = world
            .query_filtered::<Entity, With<crate::world::PlayerRoot>>()
            .iter(world)
            .collect();
        for entity in roots {
            if !v.hidden.iter().any(|(id, _)| *id == entity) {
                if let Some(vis) = world.get::<Visibility>(entity) {
                    v.hidden.push((entity, *vis));
                }
            }
            if let Some(mut vis) = world.get_mut::<Visibility>(entity) {
                *vis = if pose.is_some() && !replaced {
                    Visibility::Inherited
                } else {
                    Visibility::Hidden
                };
            }
            if let Some(mut transform) = world.get_mut::<Transform>(entity) {
                *transform = Transform::from_translation(seat).with_rotation(q);
            }
        }
        if let Some(pose) = &pose {
            crate::animation::vehicle_pose(world, pose, turn.as_deref().map(|p|(p,steering.abs())));
        }
        v.pose = pose;
    });
    world.resource_scope(|world,mut v:Mut<Vehicles>| {
        let duration=match v.visual_phase.as_str() {"vanilla" if v.crash_handoff=>0.12,"vanilla"=>0.25,"driving"=>0.05,_=>0.1};
        if v.blend_time<duration {
            let t=(v.blend_time/duration).clamp(0.,1.);
            crate::animation::blend_vehicle_visual(world,&v.blend_from,t*t*(3.-2.*t));
        } else {v.blend_from.clear();v.crash_handoff=false;}
        v.last_visual=crate::animation::capture_vehicle_visual(world);
        v.network_pose=if v.driver.is_some() || !v.blend_from.is_empty() {Some(crate::animation::network_visual(world))} else {None};
    });
}

pub(super) fn input(world: &World) -> Value {
    let keys = world.resource::<ButtonInput<KeyCode>>();
    let pad = world
        .resource::<crate::input::ControllerInput>()
        .raw_input();
    let pressed = |key| if keys.pressed(key) { 1. } else { 0. };
    let pitch = (pressed(KeyCode::ArrowUp) - pressed(KeyCode::ArrowDown)
        + if pad.left[1].abs() > 0.15 { pad.left[1] } else { 0. }).clamp(-1., 1.);
    json!({"pitch":pitch,"throttle":(pressed(KeyCode::KeyW)-pressed(KeyCode::KeyS)+pad.triggers[1]-pad.triggers[0]).clamp(-1.,1.),"steering":(pressed(KeyCode::KeyA)-pressed(KeyCode::KeyD)-if pad.left[0].abs()>0.15 {pad.left[0]} else {0.}).clamp(-1.,1.),"brake":if pad.buttons & 0x1000 != 0 {1.} else {pressed(KeyCode::Space)},"handbrake":keys.pressed(KeyCode::ShiftLeft) || pad.buttons & 0x2000 != 0,"interact":keys.pressed(KeyCode::KeyE) || pad.buttons & 0x8000 != 0,"pad_buttons":pad.buttons})
}
