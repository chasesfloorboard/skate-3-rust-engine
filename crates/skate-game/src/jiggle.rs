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
    /// Cap that stays on in a bail (JIGGLE_KEEP_: Mario Kart 8 racers).
    keep: bool,
    /// Rings (JIGGLE_SPIN_): hang loose around the arm and spin about it.
    spin: Option<Ring>,
    /// Tip position and velocity in the world.
    position: Vec3,
    velocity: Vec3,
    ready: bool,
    /// Cap: lift above the head (m), and the loose cap's world transform,
    /// velocity and spin while it is off in a bail.
    lift: f32,
    loose: Option<(Transform, Vec3, Vec3)>,
    floor: f32,
    /// Cap: its collision box in the joint's space (centre as translation,
    /// half extents as scale), from the importer's CAPBOX node.
    bounds: Option<Transform>,
}

/// A ring around an arm (Wendy's wrist hoops). The converter puts the joint
/// at the ring's centre with its Y axis along the arm (tools/mk8_character.py
/// centre_rings), and moves the ring in until it clears the arm whichever way
/// it hangs. Here it hangs toward gravity (less the arm's acceleration), rolls
/// round the arm as that direction turns, and spins about its own centre, so it
/// never passes through the arm. It also slides up the forearm: it floats up
/// while airborne, jolts (landings, bails) knock it up, and raised hands let it
/// slide toward the elbow. It comes back to rest on the wrist and never goes
/// past it.
struct Ring {
    /// Arm axis and the ring's distance along it, in the parent (hand) space.
    axis: Vec3,
    along: f32,
    /// How far the centre hangs off the axis, and the direction it hangs.
    hang: f32,
    dir: Vec3,
    angle: f32,
    rate: f32,
    /// Distance slid up the arm from the wrist, its speed, and the furthest
    /// it goes (short of the elbow, where the arm widens).
    slide: f32,
    slide_speed: f32,
    reach: f32,
    /// The arm point's last world position, velocity and acceleration.
    last: Option<(Vec3, Vec3, Vec3)>,
}

impl Ring {
    /// `forearm`: the hand's distance from the elbow, in the hand's parent units.
    fn new(rest: &Transform, forearm: f32) -> Self {
        let axis = (rest.rotation * Vec3::Y).normalize_or(Vec3::Y);
        let along = rest.translation.dot(axis);
        let off = rest.translation - axis * along;
        Ring { axis, along, hang: off.length(), dir: off.try_normalize().unwrap_or_else(|| axis.any_orthonormal_vector()),
               angle: 0.0, rate: 0.0, slide: 0.0, slide_speed: 0.0,
               // The rest spot's distance from the elbow, less a fifth of the
               // forearm kept clear of the elbow.
               reach: (forearm + along - 0.2 * forearm).max(0.0), last: None }
    }

    /// The ring's local transform this frame under a parent turned by `parent`
    /// whose origin is at `origin`.
    fn step(&mut self, rest: &Transform, parent: Quat, origin: Vec3, airborne: bool, dt: f32) -> Transform {
        let (velocity, accel, jolt) = match self.last {
            Some((last, last_velocity, last_accel)) => {
                // Smoothed: frame-time jitter at speed reads as sharp jolts.
                let velocity = last_velocity.lerp((origin - last) / dt, (30.0 * dt).min(1.0));
                let accel = ((velocity - last_velocity) / dt).clamp_length_max(80.0);
                (velocity, accel, accel.distance(last_accel))
            }
            None => (Vec3::ZERO, Vec3::ZERO, 0.0),
        };
        // A jump (teleport, respawn) starts the ring afresh.
        let restart = velocity.length() > 40.0;
        self.last = Some((origin, if restart { Vec3::ZERO } else { velocity }, if restart { Vec3::ZERO } else { accel }));
        let jolt = if restart { 0.0 } else { jolt };
        let axis_world = parent * self.axis;
        let pull = Vec3::NEG_Y * 9.8 - accel;
        let across = pull - axis_world * pull.dot(axis_world);
        let before = self.dir;
        if across.length() > 1.0 {
            let want = parent.inverse() * across.normalize();
            let blend = self.dir.lerp(want, (12.0 * dt).min(1.0));
            self.dir = (blend - self.axis * blend.dot(self.axis)).try_normalize().unwrap_or(self.dir);
        }
        // Rolling round the arm turns the ring the other way; arm movement
        // flicks it into a spin that friction slows again.
        let rolled = before.cross(self.dir).dot(self.axis).clamp(-1.0, 1.0).asin();
        self.rate += velocity.length().min(10.0) * 2.5 * dt;
        self.rate *= (1.0 - 1.2 * dt).max(0.0);
        self.rate = self.rate.clamp(0.3, 20.0);
        self.angle = (self.angle + self.rate * dt - rolled * 0.6) % std::f32::consts::TAU;
        // Up the arm: toward the elbow (the axis runs elbow to hand). Airborne,
        // the ring drifts up most of the way; on the ground a spring settles it
        // on the wrist. Gravity along the arm slides it when the hand is raised,
        // and sharp changes in the arm's motion knock it up.
        let up_arm = -axis_world;
        let target = if airborne { 0.7 * self.reach } else { 0.0 };
        // In the air the ring is weightless, so only the spring moves it.
        let gravity = if airborne { 0.0 } else { 0.5 * 9.8 * Vec3::NEG_Y.dot(up_arm) };
        let pull = 40.0 * (target - self.slide) + gravity - 6.0 * self.slide_speed;
        self.slide_speed += pull * dt;
        if jolt > 20.0 {
            self.slide_speed += ((jolt - 20.0) * 0.004).min(1.0);
        }
        self.slide += self.slide_speed * dt;
        // The wrist and the elbow end stop it, with a small bounce.
        if self.slide < 0.0 {
            self.slide = 0.0;
            self.slide_speed = (-self.slide_speed * 0.25).max(0.0);
        } else if self.slide > self.reach {
            self.slide = self.reach;
            self.slide_speed = (-self.slide_speed * 0.25).min(0.0);
        }
        Transform {
            translation: self.axis * (self.along - self.slide) + self.dir * self.hang,
            rotation: (Quat::from_axis_angle(self.axis, self.angle) * rest.rotation).normalize(),
            scale: rest.scale,
        }
    }
}

fn tuning(name: &str) -> (f32, f32, f32, f32) {
    let name = name.to_ascii_lowercase();
    // (stiffness, damping ratio, gravity share, max swing radians)
    if name.contains("cap") { (220.0, 0.45, 0.15, 0.3) } // loose enough to wobble while riding
    else if name.contains("mustache") { (260.0, 0.35, 0.2, 0.5) }
    else if name.contains("tail") { (70.0, 0.25, 0.3, 1.0) }
    else if name.contains("skirt") { (160.0, 0.45, 0.3, 0.6) }
    else if name.contains("ear") { (140.0, 0.3, 0.2, 0.7) }
    // Tongues sit in the mouth: a small stiff wobble, no sag out of it.
    else if name.contains("tongue") { (320.0, 0.6, 0.0, 0.25) }
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
        let bounds = children.and_then(|c| c.iter().find_map(|c| names.get(c).ok()
            .filter(|(n, _)| n.as_str() == "CAPBOX").map(|(_, t)| *t)));
        commands.entity(entity).insert(Jiggle {
            rest: *transform, tip, depth, stiffness, damping, gravity, limit,
            cap: name.as_str().to_ascii_lowercase().contains("cap"),
            keep: name.as_str().starts_with("JIGGLE_KEEP_"),
            spin: name.as_str().starts_with("JIGGLE_SPIN_").then(|| {
                // The ring's parent is the hand: its offset is the forearm.
                let forearm = parents.get(entity).ok().and_then(|p| names.get(p.parent()).ok())
                    .map_or(0.0, |(_, hand)| hand.translation.length());
                Ring::new(transform, forearm)
            }),
            position: Vec3::ZERO, velocity: Vec3::ZERO, ready: false,
            lift: 0.0, loose: None, floor: 0.0, bounds,
        });
    }
}

fn simulate(
    time: Res<Time>,
    skater: Res<crate::physics::SkaterRuntime>,
    mut physics: ResMut<crate::physics::GamePhysics>,
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
        if let Some(mut ring) = j.spin.take() {
            let local = ring.step(&j.rest, parent_global.rotation(), parent_global.translation(), airborne && !bailing, dt);
            j.spin = Some(ring);
            *transform = local;
            *global = parent_global.mul_transform(local);
            updated.insert(entity, *global);
            continue;
        }

        if j.cap {
            // Off in a bail: a loose cap falls, bounces and rests on the floor
            // the skater bailed on, until they are back up.
            if bailing && j.loose.is_none() && j.ready && !j.keep {
                let start = Transform::from_matrix(Mat4::from(parent_affine * transform.compute_affine()));
                let kick = root_velocity * 0.6 + Vec3::Y * 2.5;
                j.loose = Some((start, kick, Vec3::new(3.0, 1.0, -2.0)));
                j.floor = ground;
            }
            if let (Some((world, velocity, spin)), Some(bounds)) = (j.loose, j.bounds) {
                if *settled > 0.6 {
                    j.loose = None;
                    j.ready = false;
                } else {
                    // A rigid box against the world, like the props.
                    let (world, velocity, spin) = tumble(&mut physics, world, velocity, spin, &bounds, dt);
                    j.loose = Some((world, velocity, spin));
                    *transform = Transform::from_matrix(parent_global.to_matrix().inverse() * world.to_matrix());
                    *global = GlobalTransform::from(world);
                    updated.insert(entity, *global);
                    continue;
                }
            }
            if let Some((mut world, mut velocity, spin)) = j.loose {
                if *settled > 0.6 {
                    j.loose = None;
                    j.ready = false;
                } else {
                    // No collision box (an older import): fall to the floor the
                    // skater bailed on.
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
        if j.cap {
            local.translation += parent_global.rotation().inverse() * Vec3::Y * j.lift / parent_global.scale().y.max(1e-3);
        }
        *transform = local;
        *global = parent_global.mul_transform(local);
        updated.insert(entity, *global);
    }
}

/// One step of a cap loose in a bail: a light box under gravity, bounced off
/// and slid along the world geometry (kerbs, ramps, stairs) with the props'
/// contact response. `world` is the cap joint; `bounds` its box in joint space.
fn tumble(
    physics: &mut crate::physics::GamePhysics,
    mut world: Transform,
    mut velocity: Vec3,
    mut angular: Vec3,
    bounds: &Transform,
    dt: f32,
) -> (Transform, Vec3, Vec3) {
    use skate_core::physics::{board_step::CollisionBody, board_world::BoardWorldVolume,
                              contact::RetailContactMaterial, world_contact::ContactPrimitive};
    const MASS: f32 = 0.15;
    const MATERIAL: RetailContactMaterial = RetailContactMaterial { static_friction: 0.6, dynamic_friction: 0.45, restitution: 0.3 };
    let v3 = |v: Vec3| skate_core::math::Vector3::new(v.x, v.y, v.z);
    let from = |v: skate_core::math::Vector3| Vec3::new(v.x, v.y, v.z);
    velocity.y -= 9.8 * dt;
    // Sub-steps: a thin fast cap would otherwise pass through kerbs.
    let steps = ((velocity.length() * dt / 0.03).ceil() as usize).clamp(1, 4);
    let h = dt / steps as f32;
    let half = (world.scale * bounds.scale).abs().max(Vec3::splat(0.02));
    let inverse_mass = 1.0 / MASS;
    let (a, b, c) = (half * 2.0).into();
    let local_inertia = Vec3::new(1.0 / (MASS / 12.0 * (b * b + c * c)), 1.0 / (MASS / 12.0 * (a * a + c * c)), 1.0 / (MASS / 12.0 * (a * a + b * b)));
    let mut touching = false;
    for _ in 0..steps {
        world.translation += velocity * h;
        world.rotation = (Quat::from_scaled_axis(angular * h) * world.rotation).normalize();
        let centre = world.transform_point(bounds.translation);
        let basis = Mat3::from_quat(world.rotation);
        let radius = half.min_element().min(0.02);
        let volume = [BoardWorldVolume {
            body: CollisionBody::Attached(0),
            primitive: ContactPrimitive::RoundedBox {
                center: v3(centre),
                basis: skate_core::math::Basis3 { columns: basis.to_cols_array_2d() },
                half_extents: v3(half - Vec3::splat(radius)),
                radius,
            },
            linear_velocity: v3(velocity),
            material: MATERIAL,
        }];
        let inverse_inertia = basis * Mat3::from_diagonal(local_inertia) * basis.transpose();
        for contact in physics.query_world(&volume) {
            // Normals point from the world toward the cap.
            let normal = from(contact.contact.normal).normalize_or_zero();
            if normal == Vec3::ZERO { continue; }
            touching = true;
            let (on_cap, on_world) = (from(contact.contact.position_on_a), from(contact.contact.position_on_b));
            let depth = (on_world - on_cap).dot(normal);
            if depth > 0.0 { world.translation += normal * depth; }
            let r = on_cap - centre;
            let approach = (velocity + angular.cross(r)).dot(normal);
            if approach >= 0.0 { continue; }
            let effective = inverse_mass + normal.dot((inverse_inertia * r.cross(normal)).cross(r));
            let restitution = if approach < -1.0 { MATERIAL.restitution } else { 0.0 };
            let j = -(1.0 + restitution) * approach / effective.max(1e-6);
            velocity += normal * j * inverse_mass;
            angular += inverse_inertia * r.cross(normal * j);
            let point = velocity + angular.cross(r);
            let slide = point - normal * point.dot(normal);
            let speed = slide.length();
            if speed > 1e-4 {
                let t = slide / speed;
                let effective_t = inverse_mass + t.dot((inverse_inertia * r.cross(t)).cross(r));
                let jt = (speed / effective_t.max(1e-6)).min(MATERIAL.dynamic_friction * j);
                velocity -= t * jt * inverse_mass;
                angular -= inverse_inertia * r.cross(t * jt);
            }
        }
    }
    // Rolling resistance on the ground, a little air drag otherwise; come to
    // rest rather than creep.
    angular *= 1.0 - ((if touching { 4.0 } else { 0.3 }) * dt).min(0.5);
    if touching {
        velocity *= 1.0 - (1.5 * dt).min(0.5);
        if velocity.length() < 0.08 && angular.length() < 0.3 {
            velocity = Vec3::ZERO;
            angular = Vec3::ZERO;
        }
    }
    (world, velocity, angular)
}

/// Hair the cap covers comes twice (tools/cap_fit.py): "…_InCap", pulled in
/// to fit inside the cap, while it sits on the head, and "…_UnderCap", the
/// hair as modelled, once it lifts on a fall or comes off in a bail. Neither
/// pokes through the cap, and the head is never bare under it.
fn under_cap(
    caps: Query<&Jiggle>,
    mut hair: Query<(&bevy::gltf::GltfMaterialName, &mut Visibility)>,
) {
    let off = caps.iter().any(|j| j.cap && (j.loose.is_some() || j.lift > 0.03));
    for (name, mut visibility) in &mut hair {
        let show = if name.0.contains("UnderCap") { off } else if name.0.contains("InCap") { !off } else { continue };
        visibility.set_if_neq(if show { Visibility::Inherited } else { Visibility::Hidden });
    }
}
