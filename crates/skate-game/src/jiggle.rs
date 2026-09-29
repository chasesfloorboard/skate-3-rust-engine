//! Spring ("jiggle") bones for imported characters: hair, tails, skirts,
//! ears, mustaches... The character converter keeps those joints as
//! `JIGGLE_<name>` (tools/mk8_convert.py); here each one's tip lags behind the
//! animated body on a damped spring and the joint turns toward it.
//!
//! Caps are special: they lift off the head on a long fall and settle back on
//! landing, and come off in a bail, tumbling to the ground until the skater is
//! back up.
use bevy::prelude::*;
use std::collections::HashMap;

pub(crate) struct JigglePlugin;
impl Plugin for JigglePlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(
            PostUpdate,
            (adopt, simulate, under_cap).chain().after(bevy::transform::TransformSystems::Propagate),
        );
    }
}

#[derive(Component)]
struct Jiggle {
    /// Bind-pose local transform, and the tip point in the joint's space.
    rest: Transform,
    tip: Vec3,
    /// Hierarchy depth among jiggle joints (parents simulate first).
    depth: usize,
    /// Spring stiffness (1/s^2), damping ratio, gravity share, largest swing.
    stiffness: f32,
    damping: f32,
    gravity: f32,
    limit: f32,
    cap: bool,
    /// Rings (JIGGLE_SPIN_): turn about their bone, driven by its motion.
    spin: Option<(Vec3, f32, f32)>,
    /// Tip position and velocity in the world.
    position: Vec3,
    velocity: Vec3,
    ready: bool,
    /// Cap: lift above the head (m), and the loose cap's world transform,
    /// velocity and spin while it is off in a bail.
    lift: f32,
    loose: Option<(Transform, Vec3, Vec3)>,
    floor: f32,
}

fn tuning(name: &str) -> (f32, f32, f32, f32) {
    let name = name.to_ascii_lowercase();
    // (stiffness, damping ratio, gravity share, max swing radians)
    if name.contains("cap") { (220.0, 0.45, 0.15, 0.5) } // loose enough to wobble while riding
    else if name.contains("mustache") { (260.0, 0.35, 0.2, 0.5) }
    else if name.contains("tail") { (70.0, 0.25, 0.3, 1.0) }
    else if name.contains("skirt") { (160.0, 0.45, 0.3, 0.6) }
    else if name.contains("ear") { (140.0, 0.3, 0.2, 0.7) }
    else { (100.0, 0.3, 0.4, 0.8) } // hair, sleeves, tongues, ribbons
}

fn adopt(
    mut commands: Commands,
    new: Query<(Entity, &Name, &Transform, Option<&Children>), Without<Jiggle>>,
    names: Query<(&Name, &Transform)>,
    parents: Query<&ChildOf>,
) {
    for (entity, name, transform, children) in &new {
        if !name.as_str().starts_with("JIGGLE_") { continue; }
        // Tip: the first jiggle child's offset, else the joint continued
        // along its own bone (or hanging down) by 12 cm.
        let child = children.and_then(|c| c.iter().find_map(|c| names.get(c).ok()
            .filter(|(n, _)| n.as_str().starts_with("JIGGLE_")).map(|(_, t)| t.translation)));
        let tip = child.filter(|t| t.length() > 0.01).unwrap_or_else(|| {
            let along = transform.rotation.inverse() * transform.translation;
            along.try_normalize().unwrap_or(Vec3::NEG_Y) * 0.12
        });
        let depth = parents.iter_ancestors(entity)
            .filter(|&a| names.get(a).is_ok_and(|(n, _)| n.as_str().starts_with("JIGGLE_"))).count();
        let (stiffness, damping, gravity, limit) = tuning(name.as_str());
        commands.entity(entity).insert(Jiggle {
            rest: *transform, tip, depth, stiffness, damping, gravity, limit,
            cap: name.as_str().to_ascii_lowercase().contains("cap"),
            // Spin axis: along the bone the ring sits on (its offset from it).
            spin: name.as_str().starts_with("JIGGLE_SPIN_").then(|| (transform.translation.try_normalize().unwrap_or(Vec3::Y), 0.0, 0.0)),
            position: Vec3::ZERO, velocity: Vec3::ZERO, ready: false,
            lift: 0.0, loose: None, floor: 0.0,
        });
    }
}

fn simulate(
    time: Res<Time>,
    skater: Res<crate::physics::SkaterRuntime>,
    physics: Res<crate::physics::GamePhysics>,
    mut last_root: Local<Option<Vec3>>,
    mut settled: Local<f32>,
    mut jiggles: Query<(Entity, &mut Jiggle, &mut Transform, &mut GlobalTransform, &ChildOf)>,
    globals: Query<&GlobalTransform, Without<Jiggle>>,
) {
    let dt = time.delta_secs().clamp(1e-4, 1.0 / 20.0);
    let root = skater.animated_skeleton.roots.animation_to_world[3];
    let root = Vec3::new(root[0], root[1], root[2]);
    let root_velocity = last_root.map_or(Vec3::ZERO, |last| (root - last) / dt);
    let root_velocity = if root_velocity.length() > 40.0 { Vec3::ZERO } else { root_velocity };
    *last_root = Some(root);
    use skate_core::player::state::PhysicalStateId as State;
    let state = skater.player_state.current();
    let bailing = physics.board_wiping_out || state == State::WipeoutGround;
    let airborne = matches!(state, State::PhysicsAir | State::KnownAir | State::PhysicsAirSecondary | State::BipedAir);
    // The ground a loose cap lands on: just under the lowest ragdoll part
    // (the body lies on it through a bail), else the feet.
    let lowest = skater.skeleton.bodies().iter().map(|b| b.rates.position.y).fold(f32::INFINITY, f32::min);
    let ground = if lowest.is_finite() { lowest - 0.1 } else { root.y };
    // Back on the feet for a moment before a lost cap returns.
    *settled = if bailing { 0.0 } else { *settled + dt };

    let mut order: Vec<_> = jiggles.iter().map(|(e, j, ..)| (j.depth, e)).collect();
    order.sort();
    let mut updated: HashMap<Entity, GlobalTransform> = HashMap::new();
    for (_, entity) in order {
        let Ok((_, mut j, mut transform, mut global, parent)) = jiggles.get_mut(entity) else { continue };
        let Some(parent_global) = updated.get(&parent.parent()).copied()
            .or_else(|| globals.get(parent.parent()).ok().copied()) else { continue };
        let parent_affine = parent_global.affine();
        let rest_world = parent_affine * j.rest.compute_affine();
        let origin: Vec3 = rest_world.translation.into();
        let target = rest_world.transform_point3(j.tip);

        if j.cap {
            // Off in a bail: a loose cap falls, bounces and rests on the floor
            // the skater bailed on, until they are back up.
            if bailing && j.loose.is_none() && j.ready {
                let start = Transform::from_matrix(Mat4::from(parent_affine * transform.compute_affine()));
                let kick = root_velocity * 0.6 + Vec3::Y * 2.5;
                j.loose = Some((start, kick, Vec3::new(3.0, 1.0, -2.0)));
                j.floor = ground;
            }
            if let Some((mut world, mut velocity, spin)) = j.loose {
                if *settled > 0.6 {
                    j.loose = None;
                    j.ready = false;
                } else {
                    // The body keeps falling and sliding: follow its ground down.
                    j.floor = j.floor.min(ground);
                    velocity.y -= 9.8 * dt;
                    world.translation += velocity * dt;
                    let resting = world.translation.y <= j.floor + 0.04;
                    if resting {
                        world.translation.y = j.floor + 0.04;
                        velocity = Vec3::new(velocity.x * 0.6, (-velocity.y * 0.25).max(0.0), velocity.z * 0.6);
                        if velocity.length() < 0.3 { velocity = Vec3::ZERO; }
                    } else {
                        world.rotate(Quat::from_scaled_axis(spin * dt));
                    }
                    j.loose = Some((world, velocity, if resting { spin * 0.5 } else { spin }));
                    let local = Transform::from_matrix(parent_global.to_matrix().inverse() * world.to_matrix());
                    *transform = local;
                    *global = GlobalTransform::from(world);
                    updated.insert(entity, *global);
                    continue;
                }
            }
            // A long fall lifts the cap off the head; landing sets it back.
            let falling = airborne && !bailing && root_velocity.y < -6.0;
            let wanted = if falling { ((-root_velocity.y - 6.0) / 8.0).clamp(0.0, 1.0) * 0.18 } else { 0.0 };
            let rate = if wanted > j.lift { 1.5 } else { 12.0 };
            j.lift += (wanted - j.lift) * (rate * dt).min(1.0);
        }

        if !j.ready || j.position.distance(target) > 2.0 {
            j.position = target;
            j.velocity = Vec3::ZERO;
            j.ready = true;
        }
        let damping = 2.0 * j.damping * j.stiffness.sqrt();
        let accel = (target - j.position) * j.stiffness - j.velocity * damping + Vec3::NEG_Y * 9.8 * j.gravity;
        j.velocity += accel * dt;
        let step = j.velocity * dt;
        j.position += step;
        // Keep the bone length.
        let length = j.tip.length().max(1e-3);
        j.position = origin + (j.position - origin).try_normalize().unwrap_or(target - origin) * length;
        // Turn toward the lagging tip, within the joint's swing limit.
        let from = (target - origin).normalize_or(Vec3::Y);
        let to = (j.position - origin).normalize_or(from);
        let mut swing = Quat::from_rotation_arc(from, to);
        let (axis, angle) = swing.to_axis_angle();
        if angle > j.limit {
            swing = Quat::from_axis_angle(axis, j.limit);
            j.position = origin + swing * (target - origin);
        }
        let world_rotation = swing * Quat::from_affine3(&rest_world);
        let mut local = j.rest;
        local.rotation = (parent_global.rotation().inverse() * world_rotation).normalize();
        let flick = j.velocity.length();
        if let Some((axis, angle, rate)) = j.spin.as_mut() {
            // The arm's swing flicks the ring round; friction slows it again.
            *rate += flick * 6.0 * dt;
            *rate *= (1.0 - 1.5 * dt).max(0.0);
            *rate = rate.clamp(0.4, 25.0);
            *angle = (*angle + *rate * dt) % std::f32::consts::TAU;
            local.rotation = (Quat::from_axis_angle(*axis, *angle) * local.rotation).normalize();
        }
        if j.cap {
            local.translation += parent_global.rotation().inverse() * Vec3::Y * j.lift / parent_global.scale().y.max(1e-3);
        }
        *transform = local;
        *global = parent_global.mul_transform(local);
        updated.insert(entity, *global);
    }
}

/// Hair the cap covers (material "…_UnderCap", tools/smd_to_mixamo.py) shows
/// only while a cap is off: otherwise it pokes through the cap.
fn under_cap(
    caps: Query<&Jiggle>,
    mut hair: Query<(&bevy::gltf::GltfMaterialName, &mut Visibility)>,
) {
    let off = caps.iter().any(|j| j.cap && j.loose.is_some());
    for (name, mut visibility) in &mut hair {
        if name.0.contains("UnderCap") {
            visibility.set_if_neq(if off { Visibility::Inherited } else { Visibility::Hidden });
        }
    }
}
