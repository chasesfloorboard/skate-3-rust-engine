//! Dynamic props (benches, bins, hurdles, dumpsters...) from the native-props
//! placement tables. Each prop is a rigid box:
//! - it joins the native contact solver as a proxy body (same path as
//!   vehicles), so the skater collides with it and the impulse the solver gives
//!   the proxy is fed back as the prop's own velocity;
//! - awake props integrate gravity and resolve contacts against the static
//!   world here, then sleep once still;
//! - props near the player wake once so anything authored slightly into the
//!   ground settles instead of staying stuck.
//! The render package is split per prop (skate_world::spawn_with_pieces) and
//! each piece follows its prop's pose.
//! - off the board, holding RB (or F) next to a prop grabs it (RB is the
//!   button the game's own grab-prep animation uses): it keeps its
//!   distance from the skater, so walking pushes or drags it, and letting go
//!   leaves it with its current velocity.
use bevy::prelude::*;
use skate_core::physics::{
    board_step::CollisionBody,
    board_world::BoardWorldVolume,
    contact::RetailContactMaterial,
    world_contact::ContactPrimitive,
};
use std::collections::HashMap;

/// Half height of a hurdle's top bar, its only part the skater collides with.
const HURDLE_BAR: f32 = 0.06;
/// Only props this close to the board join the skater solver each step.
const SOLVER_RANGE: f32 = 12.0;
/// Props within this range wake once to settle onto the ground.
const SETTLE_RANGE: f32 = 35.0;
const GRAVITY: Vec3 = Vec3::new(0.0, -9.81, 0.0);
const SLEEP_SPEED: f32 = 0.15;
const SLEEP_TIME: f32 = 0.4;
/// Penetration left uncorrected, so resting contacts stop feeding energy in.
const SLOP: f32 = 0.01;
/// Bulk density (kg/m^3) over the bounding box. Deliberately tiny: props are
/// meant to feel comically light to shove, drag and knock over.
const DENSITY: f32 = 18.0;
/// While riding, props hit the skater with this much more mass than their
/// feather-light on-foot weight, so slamming into one on the board knocks the
/// skater off (the native collision feedback bails on the impulse) instead of
/// bulldozing it.
const RIDING_MASS_SCALE: f32 = 8.0;
/// Grab reach from the skater's body centre to the prop's surface.
const GRAB_REACH: f32 = 0.9;
/// Slippery: props skate across the ground rather than stopping dead.
const MATERIAL: RetailContactMaterial = RetailContactMaterial { static_friction: 0.25, dynamic_friction: 0.12, restitution: 0.15 };

/// Mesh piece of prop `n` (index into the placement table).
#[derive(Component)]
pub(crate) struct PropPiece(pub usize);

/// Triangle -> prop assignment for splitting the render package.
pub(crate) struct PropPieces {
    pub centers: Vec<Vec3>,
    pub assign: Box<dyn Fn(Vec3) -> Option<usize> + Send + Sync>,
}

#[derive(Clone)]
pub(crate) struct PropBox {
    /// Authored centre and box orientation; meshes are baked in this frame.
    pub center: Vec3,
    pub rotation0: Quat,
    /// Template (shared by identical props) and a readable name for it.
    pub template: String,
    pub label: String,
    pub half: Vec3,
    pub position: Vec3,
    pub orientation: Quat,
    pub velocity: Vec3,
    pub angular: Vec3,
    pub mass: f32,
    pub awake: bool,
    pub settled: bool,
    pub still: f32,
    /// Something (skater, another prop, a grab) has pushed this prop. Until
    /// then it only settles straight down, so tall props such as signs stand
    /// where they were placed instead of toppling while they settle.
    pub disturbed: bool,
}
impl PropBox {
    fn inverse_inertia_world(&self) -> Mat3 {
        let h = self.half * 2.0;
        let m = self.mass / 12.0;
        let local = Vec3::new(1.0 / (m * (h.y * h.y + h.z * h.z)), 1.0 / (m * (h.x * h.x + h.z * h.z)), 1.0 / (m * (h.x * h.x + h.y * h.y)));
        let r = Mat3::from_quat(self.orientation);
        r * Mat3::from_diagonal(local) * r.transpose()
    }
    fn primitive(&self) -> ContactPrimitive {
        let radius = self.half.min_element().min(0.04);
        let basis = Mat3::from_quat(self.orientation);
        ContactPrimitive::RoundedBox {
            center: vector(self.position),
            basis: skate_core::math::Basis3 { columns: basis.to_cols_array_2d() },
            half_extents: vector(self.half - Vec3::splat(radius)),
            radius,
        }
    }
    pub fn wake(&mut self) {
        self.awake = true;
        self.still = 0.0;
    }
    /// The four sides of the face now pointing up, as grind edges, when the
    /// prop is a sensible height to grind (benches, barriers, planters...).
    fn top_edges(&self) -> Option<[[[f32; 3]; 2]; crate::grind_world::MOVING_EDGES]> {
        if self.mass <= 0.0 { return None; }
        let r = Mat3::from_quat(self.orientation);
        let axes = [r.x_axis, r.y_axis, r.z_axis];
        let a = (0..3).max_by(|&i, &j| axes[i].y.abs().total_cmp(&axes[j].y.abs()))?;
        if axes[a].y.abs() < 0.9 || !(0.12..=1.6).contains(&(self.half[a] * 2.0)) { return None; }
        let (i, j) = ((a + 1) % 3, (a + 2) % 3);
        if self.half[i].max(self.half[j]) < 0.2 { return None; }
        let top = self.position + axes[a] * axes[a].y.signum() * self.half[a];
        let (u, v) = (axes[i] * self.half[i], axes[j] * self.half[j]);
        let c = [top - u - v, top + u - v, top + u + v, top - u + v];
        Some(std::array::from_fn(|e| [c[e].to_array(), c[(e + 1) % 4].to_array()]))
    }
    pub fn push(&mut self) {
        self.wake();
        self.disturbed = true;
    }
}

fn vector(v: Vec3) -> skate_core::math::Vector3 {
    skate_core::math::Vector3::new(v.x, v.y, v.z)
}
fn vec3(v: skate_core::math::Vector3) -> Vec3 {
    Vec3::new(v.x, v.y, v.z)
}

#[derive(Resource, Default)]
pub(crate) struct PropColliders {
    generation: Option<u64>,
    pub boxes: Vec<PropBox>,
    /// (prop, proxy body index, velocity handed to the solver) from the last step.
    proxies: Vec<(usize, usize, Vec3, Vec3)>,
    /// Grabbed prop and the horizontal distance it is held at.
    held: Option<(usize, f32)>,
    /// World-space grip points for the skater's hands on the held prop, and
    /// how far the arms have blended onto them (0..1); read by arm_ik.
    pub hands: Option<[Vec3; 2]>,
    pub grip: f32,
    last_body: Option<Vec3>,
    /// Props already published as grind edges (physics::set_moving_grinds).
    grind_published: usize,
    /// Horizontal direction from the body the held prop is kept in; it eases
    /// toward the skater's facing so the prop swings round in front.
    aim: Vec3,
}

#[derive(serde::Deserialize)]
struct Instance {
    template_id: String,
    name: String,
    matrix: [[f32; 4]; 4],
    bounds: [[f32; 3]; 2],
}
#[derive(serde::Deserialize)]
struct Placements {
    instances: Vec<Instance>,
}

/// Rotation (columns = local axes in world) of a row-vector matrix, scale removed.
fn basis(m: &[[f32; 4]; 4]) -> Mat3 {
    Mat3::from_cols(
        Vec3::from_slice(&m[0][..3]).normalize_or_zero(),
        Vec3::from_slice(&m[1][..3]).normalize_or_zero(),
        Vec3::from_slice(&m[2][..3]).normalize_or_zero(),
    )
}

/// Local half-extents from a world AABB: aabb_half = |R| * local_half.
fn local_half(r: Mat3, aabb_half: Vec3) -> Option<Vec3> {
    let abs = Mat3::from_cols(r.x_axis.abs(), r.y_axis.abs(), r.z_axis.abs());
    (abs.determinant().abs() > 0.3).then(|| (abs.inverse() * aabb_half).max(Vec3::splat(0.02)))
}

/// How far a basis is from axis-aligned (0 = aligned).
fn skew(r: Mat3) -> f32 {
    [r.x_axis, r.y_axis, r.z_axis].iter().map(|a| 1.0 - a.abs().max_element()).sum()
}

fn placements(asset_root: &std::path::Path, map: &str) -> Option<Placements> {
    let path = asset_root.join("private/native-props").join(format!("{map}.json"));
    let bytes = std::fs::read(&path).ok()?;
    serde_json::from_slice(&bytes).map_err(|e| warn!("Props disabled for {}: {e}", path.display())).ok()
}

/// "DMO_UN_PicnicTable_1003_0x2c70..." -> "Picnic Table".
pub(crate) fn label(name: &str) -> String {
    const NOISE: [&str; 12] = ["dmo", "glbl", "un", "dt", "ind", "soho", "us", "highlod", "flatmed", "ao", "ad", "el"];
    let stem = name.split("_0x").next().unwrap_or(name);
    let mut words: Vec<String> = Vec::new();
    for token in stem.split('_') {
        let lower = token.to_ascii_lowercase();
        // Numbered instance suffixes and short lowercase layer prefixes.
        if token.is_empty() || token.chars().all(|c| c.is_ascii_digit()) || NOISE.contains(&lower.as_str())
            || (token.len() <= 2 && token.chars().all(|c| c.is_ascii_lowercase())) { continue; }
        // Split CamelCase into words.
        let mut word = String::new();
        for c in token.chars() {
            if c.is_ascii_uppercase() && !word.is_empty() && !word.chars().last().is_some_and(|l| l.is_ascii_uppercase()) {
                words.push(std::mem::take(&mut word));
            }
            word.push(c);
        }
        if !word.is_empty() { words.push(word); }
    }
    words.dedup_by(|a, b| a.eq_ignore_ascii_case(b));
    let words: Vec<String> = words.into_iter().map(|w| {
        let mut c = w.chars();
        c.next().map_or(String::new(), |f| f.to_ascii_uppercase().to_string() + c.as_str())
    }).collect();
    if words.is_empty() { "Object".into() } else { words.join(" ") }
}

fn aabb(i: &Instance) -> (Vec3, Vec3) {
    (Vec3::from(i.bounds[0]), Vec3::from(i.bounds[1]))
}

fn load(asset_root: &std::path::Path, map: &str) -> Vec<PropBox> {
    let Some(placements) = placements(asset_root, map) else { return vec![] };
    // Each template appears at several angles; its most axis-aligned placement
    // gives the true local size, avoiding oversized boxes around diagonal props.
    let mut sizes: HashMap<&str, (f32, Vec3)> = HashMap::new();
    for i in &placements.instances {
        let r = basis(&i.matrix);
        let (low, high) = aabb(i);
        if let Some(local) = local_half(r, (high - low) * 0.5) {
            let s = skew(r);
            let entry = sizes.entry(&i.template_id).or_insert((f32::MAX, local));
            if s < entry.0 { *entry = (s, local); }
        }
    }
    placements.instances.iter().map(|i| {
        let r = basis(&i.matrix);
        let (low, high) = aabb(i);
        let half = sizes.get(i.template_id.as_str()).map(|(_, h)| *h)
            .or_else(|| local_half(r, (high - low) * 0.5))
            .unwrap_or((high - low) * 0.5);
        let rotation = Quat::from_mat3(&r).normalize();
        let center = (low + high) * 0.5;
        let volume = half.x * half.y * half.z * 8.0;
        // Degenerate or huge records stay put (mass 0 marks them static).
        let usable = half.min_element() > 0.0 && half.max_element() < 8.0;
        PropBox {
            center, rotation0: rotation, half,
            template: i.template_id.clone(), label: label(&i.name),
            position: center, orientation: rotation,
            velocity: Vec3::ZERO, angular: Vec3::ZERO,
            mass: if usable { (volume * DENSITY).clamp(1.5, 40.0) } else { 0.0 },
            awake: false, settled: false, still: 0.0, disturbed: false,
        }
    }).collect()
}

/// Splits the prop render package: each triangle goes to the smallest prop
/// bounding box containing its centroid (a coarse grid keeps this fast).
pub(crate) fn pieces(asset_root: &std::path::Path, map: &str) -> Option<PropPieces> {
    let placements = placements(asset_root, map)?;
    const CELL: f32 = 4.0;
    let boxes: Vec<(Vec3, Vec3, f32)> = placements.instances.iter().map(|i| {
        let (low, high) = aabb(i);
        let pad = Vec3::splat(0.05);
        let size = high - low;
        (low - pad, high + pad, size.x * size.y * size.z)
    }).collect();
    let mut grid: HashMap<(i32, i32), Vec<usize>> = HashMap::new();
    for (n, (low, high, _)) in boxes.iter().enumerate() {
        for x in (low.x / CELL).floor() as i32..=(high.x / CELL).floor() as i32 {
            for z in (low.z / CELL).floor() as i32..=(high.z / CELL).floor() as i32 {
                grid.entry((x, z)).or_default().push(n);
            }
        }
    }
    let centers = placements.instances.iter().map(|i| { let (l, h) = aabb(i); (l + h) * 0.5 }).collect();
    Some(PropPieces {
        centers,
        assign: Box::new(move |p: Vec3| {
            let cell = ((p.x / CELL).floor() as i32, (p.z / CELL).floor() as i32);
            grid.get(&cell)?.iter()
                .filter(|&&n| { let (low, high, _) = boxes[n]; p.cmpge(low).all() && p.cmple(high).all() })
                .min_by(|&&a, &&b| boxes[a].2.total_cmp(&boxes[b].2))
                .copied()
        }),
    })
}

pub(crate) fn refresh(
    config: Res<crate::config::Config>,
    map: Res<crate::map_transition::CurrentMap>,
    mut props: ResMut<PropColliders>,
) {
    if props.generation == Some(map.generation) { return; }
    props.generation = Some(map.generation);
    props.grind_published = 0;
    props.proxies.clear();
    props.held = None;
    props.hands = None;
    props.grip = 0.0;
    let Some(stem) = map.path.as_ref().and_then(|p| p.file_stem()).and_then(|s| s.to_str()) else {
        props.boxes.clear();
        return;
    };
    props.boxes = load(&config.asset_root, stem);
    info!("Props: {} physical props on {stem}", props.boxes.iter().filter(|b| b.mass > 0.0).count());
}

/// Adds nearby props to the skater solver (called from multiplayer::prepare),
/// remembering which proxy body belongs to which prop.
pub(crate) fn append_proxies(
    props: &mut PropColliders,
    proxies: &mut crate::physics::network::Proxies,
    physics: &crate::physics::GamePhysics,
    skater: &crate::physics::SkaterRuntime,
    around: Vec3,
) {
    props.proxies.clear();
    let held = props.held.map(|(n, _)| n);
    use skate_core::player::state::PhysicalStateId as State;
    let riding = !matches!(skater.player_state.current(), State::BipedGround | State::OffBoardPushing | State::WipeoutGround);
    for (n, b) in props.boxes.iter().enumerate() {
        if b.mass <= 0.0 || b.position.distance_squared(around) > SOLVER_RANGE * SOLVER_RANGE { continue; }
        let before = proxies.bodies.len();
        // An upright hurdle blocks only with its top bar: the board rolls
        // through underneath while the skater hippie-jumps over.
        let (half, offset) = if b.label.to_ascii_lowercase().contains("hurdle") && (b.orientation * Vec3::Y).y > 0.9 {
            let bar = HURDLE_BAR.min(b.half.y);
            (Vec3::new(b.half.x, bar, b.half.z), [0.0, b.half.y - bar, 0.0])
        } else {
            (b.half, [0.0; 3])
        };
        proxies.append_vehicle(&crate::modding::vehicles::network::CollisionShape {
            position: b.position.to_array(),
            rotation: b.orientation.to_array(),
            velocity: b.velocity.to_array(),
            angular: b.angular.to_array(),
            half: half.to_array(),
            offset,
            rounding: b.half.min_element().min(0.04),
            // The held prop is near weightless so it never blocks the skater.
            mass: if held == Some(n) { 1.0 } else if riding { b.mass * RIDING_MASS_SCALE } else { b.mass },
        }, physics, skater);
        if proxies.bodies.len() > before {
            props.proxies.push((n, before, b.velocity, b.angular));
        }
    }
}

/// Vehicles (modding::vehicles) plough through props: each vehicle is an
/// immovable moving box, and props it touches take its velocity plus a bounce.
fn collide_vehicles(boxes: &mut [PropBox], vehicles: &[(Vec3, Quat, Vec3, Vec3)]) {
    use skate_core::physics::world_contact::{PrimitivePairSettings, primitive_pair_contacts};
    let settings = PrimitivePairSettings { padding_a: 0.0, padding_b: 0.0, additional_padding: 0.0,
        ..PrimitivePairSettings::skater_self_collision() };
    for &(centre, rotation, half, velocity) in vehicles {
        let radius = half.min_element().min(0.05);
        let vehicle = ContactPrimitive::RoundedBox {
            center: vector(centre),
            basis: skate_core::math::Basis3 { columns: Mat3::from_quat(rotation).to_cols_array_2d() },
            half_extents: vector(half - Vec3::splat(radius)),
            radius,
        };
        // Ramps are driven up (ramp_obstacles), not shoved around.
        for prop in boxes.iter_mut().filter(|b| b.mass > 0.0 && !is_ramp(&b.label)) {
            let reach = half.length() + prop.half.length();
            if prop.position.distance_squared(centre) > reach * reach { continue; }
            let Some(manifold) = primitive_pair_contacts(prop.primitive(), vehicle, settings) else { continue };
            let normal = vec3(manifold.normal).normalize_or_zero();
            if normal == Vec3::ZERO || manifold.count == 0 { continue; }
            let count = manifold.count.min(manifold.points.len());
            let depth = manifold.points[..count].iter()
                .map(|p| (vec3(p.b) - vec3(p.a)).dot(normal)).fold(0.0f32, f32::max);
            if depth > SLOP { prop.position += normal * (depth - SLOP); }
            let approach = (prop.velocity - velocity).dot(normal);
            if approach < 0.0 {
                // Take the vehicle's speed along the contact, plus a little pop
                // upward so props tumble rather than slide under the bumper.
                prop.velocity -= normal * approach * 1.3;
                prop.velocity.y += (-approach * 0.25).min(3.0);
                prop.angular += normal.cross(Vec3::Y) * approach * 0.8;
                prop.push();
            }
        }
    }
}

/// After the skater step: take the solver's impulses on prop proxies, then
/// simulate awake props against the static world.
/// Props against each other: native rounded-box pair contacts between every
/// awake prop and anything near it, resolved as two-body impulses (sleeping
/// props are woken when hit).
fn collide_props(boxes: &mut [PropBox]) {
    use skate_core::physics::world_contact::{PrimitivePairSettings, primitive_pair_contacts};
    let settings = PrimitivePairSettings { padding_a: 0.0, padding_b: 0.0, additional_padding: 0.0,
        ..PrimitivePairSettings::skater_self_collision() };
    let awake: Vec<usize> = (0..boxes.len()).filter(|&n| boxes[n].awake && boxes[n].mass > 0.0).collect();
    for &a in &awake {
        for b in 0..boxes.len() {
            if a == b || boxes[b].mass <= 0.0 || (boxes[b].awake && b < a) { continue; }
            let reach = boxes[a].half.length() + boxes[b].half.length();
            if boxes[a].position.distance_squared(boxes[b].position) > reach * reach { continue; }
            // Authored neighbours often overlap slightly as boxes; leave pairs
            // that are both still where the map put them.
            let home = |p: &PropBox| p.position.distance_squared(p.center) < 0.15 * 0.15;
            if home(&boxes[a]) && home(&boxes[b]) { continue; }
            let Some(manifold) = primitive_pair_contacts(boxes[a].primitive(), boxes[b].primitive(), settings) else { continue };
            // Normal points from B toward A.
            let normal = vec3(manifold.normal).normalize_or_zero();
            if normal == Vec3::ZERO || manifold.count == 0 { continue; }
            let (lo, hi) = if a < b { (a, b) } else { (b, a) };
            let (left, right) = boxes.split_at_mut(hi);
            let (pa, pb) = if a < b { (&mut left[lo], &mut right[0]) } else { (&mut right[0], &mut left[lo]) };
            let (ima, imb) = (1.0 / pa.mass, 1.0 / pb.mass);
            let count = manifold.count.min(manifold.points.len());
            for point in &manifold.points[..count] {
                let (on_a, on_b) = (vec3(point.a), vec3(point.b));
                let depth = (on_b - on_a).dot(normal);
                if depth <= -0.01 { continue; }
                // Split the overlap by mass so neither ends up inside the other.
                if depth > SLOP {
                    let push = (depth - SLOP) * 0.5 / count as f32;
                    pa.position += normal * push * ima / (ima + imb) * 2.0;
                    pb.position -= normal * push * imb / (ima + imb) * 2.0;
                }
                let (ia, ib) = (pa.inverse_inertia_world(), pb.inverse_inertia_world());
                let (ra, rb) = (on_a - pa.position, on_b - pb.position);
                let relative = (pa.velocity + pa.angular.cross(ra)) - (pb.velocity + pb.angular.cross(rb));
                let approach = relative.dot(normal);
                if approach >= 0.0 { continue; }
                let effective = ima + imb + normal.dot((ia * ra.cross(normal)).cross(ra) + (ib * rb.cross(normal)).cross(rb));
                let restitution = if approach < -1.0 { MATERIAL.restitution } else { 0.0 };
                let j = -(1.0 + restitution) * approach / effective.max(1e-6) / count as f32;
                pa.velocity += normal * j * ima;
                pa.angular += ia * ra.cross(normal * j);
                pb.velocity -= normal * j * imb;
                pb.angular -= ib * rb.cross(normal * j);
                // Friction between the two.
                let relative = (pa.velocity + pa.angular.cross(ra)) - (pb.velocity + pb.angular.cross(rb));
                let tangent = relative - normal * relative.dot(normal);
                let speed = tangent.length();
                if speed > 1e-4 {
                    let t = tangent / speed;
                    let jt = (speed / (ima + imb)).min(MATERIAL.dynamic_friction * j) / count as f32;
                    pa.velocity -= t * jt * ima;
                    pb.velocity += t * jt * imb;
                }
                if !pb.awake && (relative.length() > 0.05 || depth > SLOP) { pb.wake(); }
                if !pa.awake { pa.wake(); }
                // A real knock (not settling neighbours touching) frees both.
                if approach < -0.5 { pa.disturbed = true; pb.disturbed = true; }
            }
        }
    }
}

pub(crate) fn simulate(
    time: Res<Time<Fixed>>,
    mut physics: ResMut<crate::physics::GamePhysics>,
    mut props: ResMut<PropColliders>,
    mut skater: ResMut<crate::physics::SkaterRuntime>,
    input: Res<crate::input::ControllerInput>,
    keys: Res<ButtonInput<KeyCode>>,
    menu: Option<Res<crate::graphics_menu::Menu>>,
    camera: Query<&GlobalTransform, With<crate::camera::GameplayCamera>>,
    vehicles: Option<Res<crate::modding::vehicles::Vehicles>>,
) {
    let facing = camera.iter().next().map(|c| c.forward().with_y(0.0)).and_then(|f| f.try_normalize());
    let dt = time.delta_secs().clamp(1e-4, 1.0 / 30.0);
    let holding = crate::graphics_menu::gameplay_active(menu)
        && (input.raw_input().buttons & 0x0200 != 0 || keys.pressed(KeyCode::KeyF)
            || std::env::var("SKATE_DEBUG_PROPS").as_deref() == Ok("grab"));
    grab(&mut props, &skater, holding, facing, dt);
    // 1. Skater impulses: velocity change the solver applied to each proxy.
    let handed = std::mem::take(&mut props.proxies);
    for (n, index, v0, w0) in handed {
        let Some(body) = physics.network_proxies.bodies.get(index) else { continue };
        let (v, w) = (vec3(body.rates.linear_velocity), vec3(body.rates.angular_velocity));
        if (v - v0).length() > 0.03 || (w - w0).length() > 0.05 {
            let prop = &mut props.boxes[n];
            prop.velocity = v;
            prop.angular = w;
            prop.push();
        }
    }
    // Test hooks: SKATE_DEBUG_PROPS=kick shoves the nearest prop away once,
    // 3 s in; =throw sends it at the skater instead.
    let hook = std::env::var("SKATE_DEBUG_PROPS");
    if hook.as_deref() == Ok("throw") {
        let board = vec3(physics.board.bodies()[0].rates.position);
        if time.elapsed_secs() > 3.0 && time.elapsed_secs() - time.delta_secs() <= 3.0 {
            if let Some(prop) = props.boxes.iter_mut().filter(|b| b.mass > 0.0)
                .min_by(|a, b| a.position.distance(board).total_cmp(&b.position.distance(board))) {
                let toward = (board - prop.position).with_y(0.0);
                prop.position = board - toward.normalize_or(Vec3::X) * 2.5 + Vec3::Y * prop.half.y;
                prop.velocity = toward.normalize_or(Vec3::X) * 9.0;
                prop.push();
                info!("PROP_THROW from {:.1?}", prop.position);
            }
        }
    }
    if hook.as_deref() == Ok("kick") {
        let board = vec3(physics.board.bodies()[0].rates.position);
        if time.elapsed_secs() > 3.0 && time.elapsed_secs() - time.delta_secs() <= 3.0 {
            if let Some(prop) = props.boxes.iter_mut().filter(|b| b.mass > 0.0)
                .min_by(|a, b| a.position.distance(board).total_cmp(&b.position.distance(board))) {
                let away = (prop.position - board).with_y(0.0).normalize_or(Vec3::X);
                prop.velocity = away * 5.0 + Vec3::Y * 2.0;
                prop.angular = away.cross(Vec3::Y) * -6.0;
                prop.push();
                info!("PROP_KICK at {:.1?}", prop.position);
            }
        }
    }
    // 2. Settle props near the player once.
    let board = vec3(physics.board.bodies()[0].rates.position);
    for prop in props.boxes.iter_mut() {
        if !prop.settled && prop.mass > 0.0 && prop.position.distance_squared(board) < SETTLE_RANGE * SETTLE_RANGE {
            prop.settled = true;
            prop.wake();
        }
    }
    // 3. Integrate and resolve awake props against the world.
    let held = props.held.map(|(n, _)| n);
    for n in 0..props.boxes.len() {
        if !props.boxes[n].awake { continue; }
        let prop = &mut props.boxes[n];
        prop.velocity += GRAVITY * dt;
        prop.position += prop.velocity * dt;
        prop.orientation = (Quat::from_scaled_axis(prop.angular * dt) * prop.orientation).normalize();
        let volume = [BoardWorldVolume { body: CollisionBody::Attached(0), primitive: prop.primitive(), linear_velocity: vector(prop.velocity), material: MATERIAL }];
        let contacts = physics.query_world(&volume);
        let prop = &mut props.boxes[n];
        let inverse_mass = 1.0 / prop.mass;
        for c in &contacts {
            // Normals point from the world (B) toward the prop (A).
            let normal = vec3(c.contact.normal).normalize_or_zero();
            if normal == Vec3::ZERO { continue; }
            let (on_prop, on_world) = (vec3(c.contact.position_on_a), vec3(c.contact.position_on_b));
            let depth = (on_world - on_prop).dot(normal);
            // Push out firmly; anything deeper than a few centimetres is
            // lifted straight out so props never end up buried.
            if depth > 0.05 { prop.position += normal * depth; }
            else if depth > SLOP { prop.position += normal * (depth - SLOP) * 0.6; }
            let inverse_inertia = prop.inverse_inertia_world();
            let r = on_prop - prop.position;
            let point_velocity = prop.velocity + prop.angular.cross(r);
            let approach = point_velocity.dot(normal);
            if approach >= 0.0 { continue; }
            let rn = r.cross(normal);
            let effective = inverse_mass + normal.dot((inverse_inertia * rn).cross(r));
            // Bounce only on real impacts; resting contacts are inelastic.
            let restitution = if approach < -1.0 { MATERIAL.restitution } else { 0.0 };
            let j = -(1.0 + restitution) * approach / effective.max(1e-6);
            prop.velocity += normal * j * inverse_mass;
            prop.angular += inverse_inertia * r.cross(normal * j);
            // Coulomb friction along the sliding direction.
            let point_velocity = prop.velocity + prop.angular.cross(r);
            let tangent_velocity = point_velocity - normal * point_velocity.dot(normal);
            let speed = tangent_velocity.length();
            if speed > 1e-4 {
                let t = tangent_velocity / speed;
                let rt = r.cross(t);
                let effective_t = inverse_mass + t.dot((inverse_inertia * rt).cross(r));
                let jt = (speed / effective_t.max(1e-6)).min(MATERIAL.dynamic_friction * j);
                prop.velocity -= t * jt * inverse_mass;
                prop.angular -= inverse_inertia * r.cross(t * jt);
            }
        }
        // Rolling resistance while touching something, light air drag otherwise;
        // then sleep once still for a moment.
        let damping = if contacts.is_empty() { 0.3 } else { 3.0 };
        prop.angular *= 1.0 - (damping * dt).min(0.5);
        if !contacts.is_empty() { prop.velocity *= 1.0 - (0.4 * dt).min(0.5); }
        // Untouched props settle straight down in their placed orientation.
        if !prop.disturbed {
            prop.angular = Vec3::ZERO;
            prop.velocity = prop.velocity.with_x(0.0).with_z(0.0);
        }
        // Held props stay upright: spring the authored up axis back to world up.
        if held == Some(n) {
            let up = prop.orientation * (prop.rotation0.inverse() * Vec3::Y);
            let correction = up.cross(Vec3::Y);
            prop.angular = prop.angular * (1.0 - (6.0 * dt).min(1.0)) + correction * 60.0 * dt;
        }
        if prop.velocity.length() < SLEEP_SPEED && prop.angular.length() < SLEEP_SPEED * 2.0 && !contacts.is_empty() {
            prop.still += dt;
            if prop.still > SLEEP_TIME {
                prop.awake = false;
                prop.velocity = Vec3::ZERO;
                prop.angular = Vec3::ZERO;
            }
        } else {
            prop.still = 0.0;
        }
        // Safety net: anything that fell out of the world returns to its spot.
        if prop.position.y < prop.center.y - 50.0 {
            prop.position = prop.center;
            prop.orientation = prop.rotation0;
            prop.velocity = Vec3::ZERO;
            prop.angular = Vec3::ZERO;
        }
    }
    // 4. Props against each other and against vehicles.
    collide_props(&mut props.boxes);
    if let Some(vehicles) = vehicles.as_ref().map(|v| v.boxes()).filter(|b| !b.is_empty()) {
        collide_vehicles(&mut props.boxes, &vehicles);
    }
    // 5. Grind edges follow props that moved (and new drops).
    if props.grind_published != props.boxes.len() || props.boxes.iter().any(|b| b.awake) {
        let edges: Vec<_> = props.boxes.iter().map(PropBox::top_edges).collect();
        physics.set_moving_grinds(&mut skater, &edges);
        props.grind_published = props.boxes.len();
    }
}

/// Off-board grabbing: pick the nearest reachable prop when the button goes
/// down, then steer it to stay at the grabbed distance from the body.
fn grab(props: &mut PropColliders, skater: &crate::physics::SkaterRuntime, holding: bool, facing: Option<Vec3>, dt: f32) {
    use skate_core::player::state::PhysicalStateId as State;
    let on_foot = matches!(skater.player_state.current(), State::BipedGround | State::OffBoardPushing);
    // The animated root (feet level) is live in every on-foot state; the
    // ragdoll bodies only simulate during bails.
    let root = skater.animated_skeleton.roots.animation_to_world[3];
    let body = Vec3::new(root[0], root[1] + 1.0, root[2]);
    // The prop stays in front of the skater (the way they face, turned with
    // the left stick); the right stick only looks around.
    let forward = skater.animated_skeleton.roots.animation_to_world[2];
    let facing = Vec3::new(forward[0], 0.0, forward[2]).try_normalize().or(facing);
    let body_velocity = props.last_body.map_or(Vec3::ZERO, |last| (body - last) / dt);
    // A teleport or respawn is not a walk.
    let body_velocity = if body_velocity.length() > 20.0 { Vec3::ZERO } else { body_velocity };
    props.last_body = Some(body);
    steer_held(props, holding && on_foot, body, body_velocity, facing, dt);
}

fn steer_held(props: &mut PropColliders, holding: bool, body: Vec3, body_velocity: Vec3, facing: Option<Vec3>, dt: f32) {
    // Arms ease onto the grip and back off again.
    let grip_target = if holding && props.held.is_some() { 1.0 } else { 0.0 };
    props.grip += (grip_target - props.grip).clamp(-6.0 * dt, 8.0 * dt);
    if !holding {
        props.held = None;
        if props.grip <= 0.0 { props.hands = None; }
        return;
    }
    if props.held.is_none() {
        // Distance from the body to the prop's box surface, horizontally.
        let reach = |b: &PropBox| {
            let local = b.orientation.inverse() * (body - b.position);
            let outside = (local.abs() - b.half).max(Vec3::ZERO);
            Vec2::new(outside.x, outside.z).length().max(outside.length() - 0.8)
        };
        props.held = props.boxes.iter().enumerate()
            .filter(|(_, b)| b.mass > 0.0 && reach(b) < GRAB_REACH)
            .min_by(|a, b| reach(a.1).total_cmp(&reach(b.1)))
            .map(|(n, b)| (n, (b.position - body).with_y(0.0).length()));
        if let Some((n, _)) = props.held {
            props.aim = (props.boxes[n].position - body).with_y(0.0).normalize_or(Vec3::X);
        }
    }
    let Some((n, distance)) = props.held else { return };
    // Swing the aim toward the skater's facing at a limited rate, turning the
    // prop with it so the same side stays toward the skater.
    let wanted = facing.unwrap_or(props.aim);
    let turn = props.aim.cross(wanted).y.atan2(props.aim.dot(wanted)).clamp(-4.0 * dt, 4.0 * dt);
    let swing = Quat::from_rotation_y(turn);
    props.aim = (swing * props.aim).normalize_or(Vec3::X);
    let aim = props.aim;
    let prop = &mut props.boxes[n];
    prop.push();
    prop.orientation = (swing * prop.orientation).normalize();
    let direction = (prop.position - body).with_y(0.0).normalize_or(aim);
    // Feather light: the prop tracks its spot in front and the body's motion
    // almost immediately, whatever it weighs. Only horizontal velocity is
    // steered, so gravity, bumps and tipping still play out physically.
    let target = body_velocity.with_y(0.0) + (body + aim * distance - prop.position).with_y(0.0) * 10.0;
    let horizontal = prop.velocity.with_y(0.0);
    prop.velocity += (target - horizontal) * (18.0 * dt).min(1.0);
    // Grip points: the prop surface nearest each hand, a shoulder width
    // apart and around chest height, so they slide as the skater moves.
    let side = Vec3::Y.cross(direction).normalize_or(Vec3::Z);
    let inverse = prop.orientation.inverse();
    let grip = |shift: f32| {
        let wanted = body + side * shift + Vec3::Y * 0.1;
        let local = (inverse * (wanted - prop.position)).clamp(-prop.half, prop.half);
        prop.position + prop.orientation * local
    };
    props.hands = Some([grip(0.22), grip(-0.22)]);
}

/// SKATE_DEBUG_PROPS=1: once a second, awake count and largest drift.
pub(crate) fn debug(time: Res<Time<Real>>, props: Res<PropColliders>) {
    if std::env::var("SKATE_DEBUG_PROPS").is_err() || (time.elapsed_secs() % 1.0) >= time.delta_secs() { return; }
    let awake = props.boxes.iter().filter(|b| b.awake).count();
    let drift = props.boxes.iter().filter(|b| b.mass > 0.0).map(|b| (b.position - b.center).length())
        .fold(0.0f32, f32::max);
    let tilt = props.boxes.iter().filter(|b| b.mass > 0.0).map(|b| b.orientation.angle_between(b.rotation0).to_degrees())
        .fold(0.0f32, f32::max);
    info!("PROPS awake={awake} max_drift={drift:.2}m max_tilt={tilt:.0}deg proxies={}", props.proxies.len());
    for (n, b) in props.boxes.iter().enumerate().filter(|(_, b)| b.awake) {
        info!("PROP_AWAKE #{n} half={:.2?} tilt={:.0} v={:.2?} w={:.2?}", b.half, b.orientation.angle_between(b.rotation0).to_degrees(), b.velocity, b.angular);
    }
}

/// Moves each prop's mesh pieces with its body.
/// Ramp-like props (wedges, kickers, quarter pipes, banks) that vehicles
/// should climb rather than knock over.
pub(crate) fn is_ramp(label: &str) -> bool {
    let label = label.to_ascii_lowercase();
    ["ramp", "kicker", "quarter", "wedge", "funbox", "bank"].iter().any(|k| label.contains(k))
}

/// Vehicles drive up ramp props: each ramp mesh piece becomes a convex
/// obstacle in the vehicle physics, posed with its prop every frame.
pub(crate) fn ramp_obstacles(
    props: Res<PropColliders>,
    pieces: Query<(Entity, &PropPiece, &Mesh3d)>,
    meshes: Res<Assets<Mesh>>,
    vehicles: Option<ResMut<crate::modding::vehicles::Vehicles>>,
) {
    let Some(mut vehicles) = vehicles else { return };
    if !vehicles.has_vehicles() { return; }
    for (entity, piece, mesh) in &pieces {
        let Some(prop) = props.boxes.get(piece.0) else { continue };
        if !is_ramp(&prop.label) { continue; }
        let key = entity.to_bits();
        if !vehicles.has_obstacle(key) {
            let Some(positions) = meshes.get(&mesh.0).and_then(|m| m.try_attribute(Mesh::ATTRIBUTE_POSITION).ok()).and_then(|a| a.as_float3()) else { continue };
            if let Err(e) = vehicles.add_obstacle(key, positions) { warn!("Ramp obstacle: {e}"); continue; }
        }
        let rotation = prop.orientation * prop.rotation0.inverse();
        vehicles.move_obstacle(key, prop.position.to_array(), rotation.to_array());
    }
}

pub(crate) fn sync_pieces(props: Res<PropColliders>, mut pieces: Query<(&PropPiece, &mut Transform)>) {
    for (piece, mut transform) in &mut pieces {
        let Some(prop) = props.boxes.get(piece.0) else { continue };
        let rotation = prop.orientation * prop.rotation0.inverse();
        if transform.translation != prop.position || transform.rotation != rotation {
            transform.translation = prop.position;
            transform.rotation = rotation;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn crate_box(position: Vec3) -> PropBox {
        PropBox {
            center: position, rotation0: Quat::IDENTITY, half: Vec3::splat(0.3), position,
            template: String::new(), label: String::new(),
            orientation: Quat::IDENTITY, velocity: Vec3::ZERO, angular: Vec3::ZERO, mass: 20.0,
            awake: false, settled: true, still: 0.0, disturbed: false,
        }
    }
    #[test]
    fn a_moving_prop_knocks_a_resting_one() {
        let mut boxes = vec![crate_box(Vec3::new(0.0, 0.3, 0.0)), crate_box(Vec3::new(0.55, 0.3, 0.0))];
        boxes[0].center = Vec3::new(-3.0, 0.3, 0.0);
        boxes[0].velocity = Vec3::new(3.0, 0.0, 0.0);
        boxes[0].awake = true;
        collide_props(&mut boxes);
        assert!(boxes[1].awake);
        assert!(boxes[1].velocity.x > 0.5, "hit prop moves: {:?}", boxes[1].velocity);
        assert!(boxes[0].velocity.x < 3.0);
    }
    #[test]
    fn prop_labels_are_readable() {
        assert_eq!(label("DMO_UN_PicnicTable_1003_0x2c70170500030012:0x2c"), "Picnic Table");
        assert_eq!(label("AD_Bench_DMO_1007_0x0000042103e38705"), "Bench");
        assert_eq!(label("el_DMO_glbl_OilDrum_1002_0x0000041c"), "Oil Drum");
        assert_eq!(label("DMO_Glbl_ba_DMO_Glbl_RailFancyMed_FlatMed_1009_0x00"), "Rail Fancy Med");
        assert_eq!(label("Construction_Barrier__DMO_1002_0x2c"), "Construction Barrier");
    }
    #[test]
    fn grabbed_prop_follows_the_walking_skater_and_is_released() {
        let mut props = PropColliders { boxes: vec![crate_box(Vec3::new(1.0, 0.3, 0.0)), crate_box(Vec3::new(9.0, 0.3, 0.0))], ..default() };
        let walk = Vec3::new(-1.5, 0.0, 0.0);
        let mut body = Vec3::new(0.0, 0.9, 0.0);
        for _ in 0..120 {
            steer_held(&mut props, true, body, walk, None, 1.0 / 60.0);
            let prop = &mut props.boxes[0];
            prop.position += prop.velocity / 60.0;
            body += walk / 60.0;
        }
        assert_eq!(props.held.map(|h| h.0), Some(0));
        // Dragged along two seconds of walking, still about a metre away.
        let gap = (props.boxes[0].position - body).with_y(0.0).length();
        assert!((gap - 1.0).abs() < 0.15, "gap {gap}");
        assert!(props.boxes[1].velocity == Vec3::ZERO);
        steer_held(&mut props, false, body, walk, None, 1.0 / 60.0);
        assert!(props.held.is_none());
    }
    #[test]
    fn upright_props_offer_their_top_edges_for_grinding() {
        let mut bench = crate_box(Vec3::new(0.0, 0.3, 0.0));
        bench.half = Vec3::new(1.0, 0.3, 0.25);
        let edges = bench.top_edges().expect("bench is grindable");
        assert!(edges.iter().all(|e| e[0][1] == 0.6 && e[1][1] == 0.6), "top at 0.6 m: {edges:?}");
        // Tipped onto its back it offers the new top face (0.25 m half depth).
        bench.orientation = Quat::from_rotation_x(std::f32::consts::FRAC_PI_2);
        let back = bench.top_edges().expect("still a box with an upward face");
        assert!(back.iter().all(|e| (e[0][1] - 0.55).abs() < 1e-4), "{back:?}");
        // Stood on its end it is 2 m tall: not a grind.
        bench.orientation = Quat::from_rotation_z(std::f32::consts::FRAC_PI_2);
        assert!(bench.top_edges().is_none());
        // Tilted mid-fall: nothing to grind.
        bench.orientation = Quat::from_rotation_z(0.6);
        assert!(bench.top_edges().is_none());
        // Too tall (a sign post).
        let mut post = crate_box(Vec3::ZERO);
        post.half = Vec3::new(0.3, 1.5, 0.3);
        assert!(post.top_edges().is_none());
    }
    #[test]
    fn held_prop_swings_round_to_the_facing() {
        let mut props = PropColliders { boxes: vec![crate_box(Vec3::new(1.0, 0.3, 0.0))], ..default() };
        let body = Vec3::new(0.0, 0.9, 0.0);
        // Camera turned to look down +Z; the prop starts off to the +X side.
        for _ in 0..90 {
            steer_held(&mut props, true, body, Vec3::ZERO, Some(Vec3::Z), 1.0 / 60.0);
            let prop = &mut props.boxes[0];
            prop.position += prop.velocity / 60.0;
        }
        let offset = (props.boxes[0].position - body).with_y(0.0);
        assert!(offset.normalize().dot(Vec3::Z) > 0.95, "prop in front: {offset:?}");
        assert!((offset.length() - 1.0).abs() < 0.15, "distance kept: {}", offset.length());
        // It turned with its orbit (a quarter turn about Y).
        assert!(props.boxes[0].orientation.angle_between(Quat::IDENTITY) > 1.3);
    }
    #[test]
    fn diagonal_placement_recovers_template_size_from_aligned_one() {
        let yaw = |deg: f32| {
            let (s, c) = deg.to_radians().sin_cos();
            [[c, 0., -s, 0.], [0., 1., 0., 0.], [s, 0., c, 0.], [0., 0., 0., 1.]]
        };
        let aligned = basis(&yaw(0.0));
        assert!((local_half(aligned, Vec3::new(2.0, 0.5, 0.3)).unwrap() - Vec3::new(2.0, 0.5, 0.3)).length() < 1e-4);
        // At exactly 45 degrees the AABB alone cannot separate the axes.
        assert!(local_half(basis(&yaw(45.0)), Vec3::splat(1.0)).is_none());
        let r = basis(&yaw(30.0));
        let abs = Mat3::from_cols(r.x_axis.abs(), r.y_axis.abs(), r.z_axis.abs());
        let local = Vec3::new(2.0, 0.5, 0.3);
        assert!((local_half(r, abs * local).unwrap() - local).length() < 1e-3);
    }
}
