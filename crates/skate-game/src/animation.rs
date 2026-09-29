//! Render the same stock skeleton that animation and physics update.
//! The GLB supplies the mesh, skin weights and hierarchy; it does not play a
//! separate idle clip or own the skater's pose.
use crate::{app::FrameSet, assets::AssetStatus, physics::SkaterRuntime, world::PlayerRoot};
use bevy::{camera::visibility::NoFrustumCulling, mesh::skinning::SkinnedMesh, prelude::*};
use skate_core::animation::output::NativeMatrix;

#[derive(Resource, Default)]
pub(crate) struct AnimationStatus {
    pub ready: bool,
    bindings: Vec<BoneBinding>,
    /// Characters that keep their own proportions (custom_models manifest
    /// "proportions"): hip height ratio to the stock rig. Stock animation
    /// rotations then play on the character's own bone lengths.
    pub keep: Option<Keep>,
    feet: Option<[usize; 2]>,
    board: Option<usize>,
    /// (hand, its *_REPARENTED grip on the board) for left and right.
    hands: Option<[(usize, usize); 2]>,
    /// (upper leg, knee, foot) for left and right: kept-proportion legs
    /// reach for the stock feet with two-bone IK.
    legs: Option<[[usize; 3]; 2]>,
}
/// Kept-proportion character (custom_models manifest "proportions").
#[derive(Clone, Copy)]
pub(crate) struct Keep {
    /// Hips-to-ankle height against the stock rig's.
    pub leg: f32,
    /// Ankle joint height over the soles, the character's and the stock rig's.
    pub ankle: f32,
    pub stock_ankle: f32,
}
struct BoneBinding {
    entity: Entity,
    bone: usize,
    parent_bone: Option<usize>,
    /// The joint's bind offset from its parent, and its role.
    rest: Vec3,
    role: Role,
}
#[derive(Clone, Copy, PartialEq)]
enum Role { Body, Hips, Board }
fn role(name: &str) -> Role {
    let name = name.to_ascii_uppercase();
    if name == "HIPS" { Role::Hips }
    else if name.starts_with("SKATEBOARD") || name.starts_with("TRUCK") || name.contains("WHEEL") || name.ends_with("_REPARENTED") { Role::Board }
    else { Role::Body }
}
fn hands(names: &[String]) -> Option<[(usize, usize); 2]> {
    let find = |n: &str| names.iter().position(|b| b.eq_ignore_ascii_case(n));
    Some([(find("LEFTHAND")?, find("LEFTHAND_REPARENTED")?), (find("RIGHTHAND")?, find("RIGHTHAND_REPARENTED")?)])
}
fn legs(names: &[String]) -> Option<[[usize; 3]; 2]> {
    let find = |n: &str| names.iter().position(|b| b.eq_ignore_ascii_case(n));
    Some([[find("LEFTUPLEG")?, find("LEFTLEG")?, find("LEFTFOOT")?], [find("RIGHTUPLEG")?, find("RIGHTLEG")?, find("RIGHTFOOT")?]])
}
fn feet(names: &[String]) -> Option<[usize; 2]> {
    let find = |n: &str| names.iter().position(|b| b.eq_ignore_ascii_case(n));
    Some([find("LEFTFOOT")?, find("RIGHTFOOT")?])
}
impl AnimationStatus {
    /// Keep the stock board joints (not those under `skip`) animating under a
    /// character that rides the player's own customised board.
    pub(crate) fn carry_board(&mut self, from: &AnimationStatus, names: &[String], skip: impl Fn(Entity) -> bool) {
        for b in &from.bindings {
            let board = names.get(b.bone).is_some_and(|n| role(n) == Role::Board);
            if board && !skip(b.entity) && !self.bindings.iter().any(|o| o.entity == b.entity) {
                self.bindings.push(BoneBinding { entity: b.entity, bone: b.bone, parent_bone: b.parent_bone, rest: b.rest, role: Role::Board });
            }
        }
    }
    /// Kept-proportion characters: body joints keep their own bone offsets,
    /// the hips drop with the leg-length ratio above the animated feet' sole
    /// plane, the board follows the animation exactly.
    fn adjust(&self, b: &BoneBinding, t: Transform, global: impl Fn(usize) -> Option<Mat4>) -> Transform {
        self.adjust_with(b, t, global, false)
    }
    /// `seated`: hips stay where the pose puts them (a kart seat), rather
    /// than following the feet.
    fn adjust_with(&self, b: &BoneBinding, mut t: Transform, global: impl Fn(usize) -> Option<Mat4>, seated: bool) -> Transform {
        let Some(keep) = self.keep else { return t };
        match b.role {
            Role::Body => {
                t.translation = b.rest;
                if !seated {
                    if let Some(rotation) = self.leg_ik(b.bone, &|i| global(i)) {
                        t.rotation = rotation;
                    }
                }
            }
            Role::Hips if seated => {}
            Role::Hips => {
                // Hips follow the animated feet with the character's leg length,
                // in every direction (leans, tucks, flips), and its soles sit
                // where the skater's do.
                // Anchor: the board when it is under the character (a pushing
                // foot swinging along the ground must not drag the body), the
                // feet otherwise; at the lower foot's height.
                let feet = self.feet.and_then(|[l, r]| Some((global(l)?.w_axis.truncate(), global(r)?.w_axis.truncate())));
                if let Some((left, right)) = feet {
                    let mut anchor = (left + right) * 0.5;
                    anchor.y = left.y.min(right.y);
                    if let Some(board) = self.board.and_then(|b| global(b)).map(|m| m.w_axis.truncate()) {
                        if board.with_y(0.0).distance(t.translation.with_y(0.0)) < 0.6 {
                            anchor = Vec3::new(board.x, anchor.y, board.z);
                        }
                    }
                    let base = anchor + Vec3::Y * (keep.ankle - keep.stock_ankle);
                    t.translation = base + (t.translation - anchor) * keep.leg;
                }
            }
            Role::Board if b.parent_bone.is_none() && !seated => {
                // A carried board (a hand on its grip point) goes with the
                // character's own hand rather than where the stock hand is.
                let Some(hands) = self.hands else { return t };
                let board = t.to_matrix();
                // Only off the board: riding, the hand grips track the hands
                // too, and the deck must stay under the feet.
                let deck = board.w_axis.truncate();
                let on_deck = self.feet.is_some_and(|[l, r]| [l, r].iter().any(|&f| global(f).is_some_and(|m| m.w_axis.truncate().distance(deck) < 0.45)));
                if on_deck { return t; }
                for (hand, grip) in hands {
                    let (Some(stock), Some(held)) = (global(hand), global(grip)) else { continue };
                    let gap = stock.w_axis.truncate().distance(held.w_axis.truncate());
                    let weight = ((0.2 - gap) / 0.1).clamp(0., 1.);
                    if weight <= 0. { continue; }
                    let Some(own) = self.character_global(hand, &|i| global(i), 0) else { continue };
                    let moved = Transform::from_matrix(own * stock.inverse() * board);
                    return crate::presentation::blend(t, moved, weight);
                }
            }
            Role::Board => {}
        }
        t
    }
    /// Two-bone IK for a kept-proportion leg joint (upper leg, knee or foot):
    /// the character's own leg bends so its ankle reaches the stock ankle
    /// (raised by the ankle-height difference), keeping the stock knee plane
    /// and foot orientation. On the board the feet go exactly where the
    /// stock feet are (on the deck); off it, the stride shrinks with the legs.
    fn leg_ik(&self, bone: usize, global: &dyn Fn(usize) -> Option<Mat4>) -> Option<Quat> {
        let keep = self.keep?;
        let [up, knee, foot] = *self.legs?.iter().find(|l| l.contains(&bone))?;
        let binding = |i: usize| self.bindings.iter().find(|b| b.bone == i && b.role != Role::Board);
        let (bu, bk, bf) = (binding(up)?, binding(knee)?, binding(foot)?);
        let rot = |m: Mat4| m.to_scale_rotation_translation().1;
        // The character's leg as posed (stock rotations on its own offsets).
        let parent = self.character_global(bu.parent_bone?, global, 0)?;
        let local = |b: &BoneBinding| -> Option<Mat4> {
            let r = rot(global(b.parent_bone?)?.inverse() * global(b.bone)?);
            Some(Mat4::from_rotation_translation(r, b.rest))
        };
        let up_g = parent * local(bu)?;
        let knee_g = up_g * local(bk)?;
        let foot_g = knee_g * local(bf)?;
        let (hip, kn, ankle) = (up_g.w_axis.truncate(), knee_g.w_axis.truncate(), foot_g.w_axis.truncate());
        // Target: the stock ankle, lifted to this character's ankle height.
        let (stock_hip, stock_knee, stock_foot) = (global(up)?.w_axis.truncate(), global(knee)?.w_axis.truncate(), global(foot)?);
        let mut target = stock_foot.w_axis.truncate() + Vec3::Y * (keep.ankle - keep.stock_ankle);
        let deck = self.board.and_then(|b| global(b)).map(|m| m.w_axis.truncate());
        let on_deck = deck.is_some_and(|d| stock_foot.w_axis.truncate().distance(d) < 0.45);
        if !on_deck {
            // Off the board: the stock stride around the hips, scaled.
            target = hip + (target - hip) * keep.leg.clamp(0.3, 1.5);
        }
        let (a, b) = ((kn - hip).length(), (ankle - kn).length());
        if a < 1e-4 || b < 1e-4 { return None; }
        let d = (target - hip).length().clamp((a - b).abs() + 1e-3, a + b - 1e-3);
        // Knee: open or close the interior angle to reach distance d.
        let (u, v) = (hip - kn, ankle - kn);
        let mut n = u.cross(v);
        if n.length_squared() < 1e-8 { n = (stock_hip - stock_knee).cross(stock_foot.w_axis.truncate() - stock_knee); }
        let n = n.normalize_or(Vec3::X);
        let current = u.angle_between(v);
        let wanted = ((a * a + b * b - d * d) / (2.0 * a * b)).clamp(-1.0, 1.0).acos();
        let bend = Quat::from_axis_angle(n, wanted - current);
        let ankle = kn + bend * (ankle - kn);
        // Hip: swing the whole leg to point at the target.
        let aim = Quat::from_rotation_arc((ankle - hip).normalize_or(Vec3::NEG_Y), (target - hip).normalize_or(Vec3::NEG_Y));
        let up_rot = aim * rot(up_g);
        let knee_rot = aim * bend * rot(knee_g);
        Some(if bone == up {
            rot(parent).inverse() * up_rot
        } else if bone == knee {
            up_rot.inverse() * knee_rot
        } else {
            // The foot keeps the stock foot's orientation.
            knee_rot.inverse() * rot(stock_foot)
        })
    }
    /// A body joint's placement on the character's own proportions: its
    /// adjusted local transforms composed up the hierarchy.
    fn character_global(&self, bone: usize, global: &dyn Fn(usize) -> Option<Mat4>, depth: usize) -> Option<Mat4> {
        if depth > 64 { return None; }
        let b = self.bindings.iter().find(|b| b.bone == bone && b.role != Role::Board)?;
        let pose = global(bone)?;
        let local = match b.parent_bone {
            Some(parent) => global(parent)?.inverse() * pose,
            None => pose,
        };
        let local = self.adjust_with(b, Transform::from_matrix(local), global, false).to_matrix();
        Some(match b.parent_bone {
            Some(parent) => self.character_global(parent, global, depth + 1)? * local,
            None => local,
        })
    }
    pub(crate) fn pose_transforms(&self, pose:&[Mat4])->Vec<(Entity,Transform)> {
        let global = |i: usize| pose.get(i).map(|m| *m * render_basis());
        self.bindings.iter().filter_map(|b|{
            let global_bone=*pose.get(b.bone)?*render_basis();
            let local=if let Some(parent)=b.parent_bone {(*pose.get(parent)?*render_basis()).inverse()*global_bone} else {global_bone};
            Some((b.entity,self.adjust(b, Transform::from_matrix(local), global)))
        }).collect()
    }

    pub(crate) fn online_bindings(&self) -> Vec<(Entity,usize,Option<usize>)> {
        self.bindings.iter().map(|b|(b.entity,b.bone,b.parent_bone)).collect()
    }

    /// Prepare a hidden imported scene without disturbing the live bindings.
    pub(crate) fn for_scene(
        root: Entity,
        names: &[String],
        skins: &Query<(Entity, &SkinnedMesh)>,
        nodes: &Query<(&Name, &Transform)>,
        parents: &Query<&ChildOf>,
    ) -> Result<Self, String> {
        let mut bindings = Vec::new();
        let mut seen = std::collections::HashSet::new();
        for (entity, skin) in skins.iter() {
            if !parents.iter_ancestors(entity).any(|p| p == root) { continue; }
            for &joint in &skin.joints {
                if !seen.insert(joint) { continue; }
                if !parents.iter_ancestors(joint).any(|p| p == root) {
                    return Err("Imported skin refers to a joint outside its scene".into());
                }
                let (name, rest) = nodes.get(joint).map_err(|_| "Imported joint has no name/transform")?;
                // Spring bones (hair, tails...) are left to jiggle.rs.
                if name.as_str().starts_with("JIGGLE_") { continue; }
                let (rest, joint_role) = (rest.translation, role(name.as_str()));
                let bone = names.iter().position(|n| n.eq_ignore_ascii_case(name.as_str()))
                    .ok_or_else(|| format!("Unsupported imported bone: {name}"))?;
                let mut parent_bone = None;
                for p in parents.iter_ancestors(joint) {
                    if p == root { break; }
                    if let Ok((name, transform)) = nodes.get(p) {
                        if let Some(i) = names.iter().position(|n| n.eq_ignore_ascii_case(name.as_str())) {
                            parent_bone = Some(i);
                            break;
                        }
                        if !transform.to_matrix().abs_diff_eq(Mat4::IDENTITY, 0.00001) {
                            return Err("Imported armature has an unsupported ancestor transform".into());
                        }
                    }
                }
                bindings.push(BoneBinding { entity: joint, bone, parent_bone, rest, role: joint_role });
            }
        }
        if bindings.is_empty() { return Err("Imported scene has no skinned character".into()); }
        Ok(Self { ready: true, bindings, keep: None, feet: feet(names), hands: hands(names), legs: legs(names),
            board: names.iter().position(|n| n.eq_ignore_ascii_case("SKATEBOARD_ROOT")) })
    }
}
#[cfg(test)]
mod custom_model_binding_tests {
    use super::*;
    use bevy::ecs::system::SystemState;
    #[test]
    fn custom_models_bind_only_the_candidate_and_reject_foreign_joints() {
        let mut world = World::new();
        let root = world.spawn_empty().id();
        let hips = world.spawn((Name::new("HIPS"), Transform::default(), ChildOf(root))).id();
        let head = world.spawn((Name::new("HEAD"), Transform::default(), ChildOf(hips))).id();
        for _ in 0..2 {
            world.spawn((ChildOf(root), SkinnedMesh { inverse_bindposes: default(), joints: vec![hips, head] }));
        }
        let foreign = world.spawn((Name::new("unrelated"), Transform::default())).id();
        world.spawn(SkinnedMesh { inverse_bindposes: default(), joints: vec![foreign] });
        let names = vec!["HIPS".into(), "HEAD".into()];
        let mut queries: SystemState<(Query<(Entity, &SkinnedMesh)>, Query<(&Name, &Transform)>, Query<&ChildOf>)> = SystemState::new(&mut world);
        let (skins,nodes,parents) = queries.get(&world);
        let prepared = AnimationStatus::for_scene(root,&names,&skins,&nodes,&parents).unwrap();
        assert!(prepared.ready);
        assert_eq!(prepared.bindings.len(),2);
        assert_eq!(prepared.bindings.iter().find(|b| b.entity == head).unwrap().parent_bone,Some(0));
        world.spawn((ChildOf(root), SkinnedMesh { inverse_bindposes: default(), joints: vec![foreign] }));
        let (skins,nodes,parents) = queries.get(&world);
        assert!(AnimationStatus::for_scene(root,&names,&skins,&nodes,&parents).is_err());
        assert_eq!(prepared.bindings.len(),2);
    }
}
pub(crate) struct AnimationPlugin;
impl Plugin for AnimationPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<AnimationStatus>()
            .add_systems(Update, (disable_skater_culling, bind, present).chain().in_set(FrameSet::Animation));
    }
}
fn disable_skater_culling(
    mut commands: Commands,
    skins: Query<Entity, (With<SkinnedMesh>, Without<NoFrustumCulling>)>,
    parents: Query<&ChildOf>,
    roots: Query<Entity, With<PlayerRoot>>,
) {
    // GLB primitive bounds describe the bind pose, not the animated skin.
    // Apply to every skater mesh (not just the first skin used to bind bones).
    // World meshes retain their normal culling behavior.
    for entity in &skins {
        if parents.iter_ancestors(entity).any(|e| roots.contains(e)) {
            commands.entity(entity).insert(NoFrustumCulling);
        }
    }
}

fn bind(
    status: Res<AssetStatus>,
    skater: Res<SkaterRuntime>,
    mut animation: ResMut<AnimationStatus>,
    skins: Query<(Entity, &SkinnedMesh)>,
    nodes: Query<(&Name, &Transform)>,
    parents: Query<&ChildOf>,
    visibility: Query<&Visibility>,
    roots: Query<Entity, With<PlayerRoot>>,
    mut exit: MessageWriter<AppExit>,
) {
    if *status != AssetStatus::Ready || animation.ready {
        return;
    }
    let names = &skater.animation.evaluator.frames.bone_names;
    let result = (|| -> Result<Option<Vec<BoneBinding>>, String> {
        let mut bindings = Vec::new();
        let mut seen = std::collections::HashSet::new();
        for (entity, skin) in &skins {
            if !parents.iter_ancestors(entity).any(|e| roots.contains(e)) {
                continue;
            }
            if parents.iter_ancestors(entity).any(|e| visibility.get(e).is_ok_and(|v| *v == Visibility::Hidden)) { continue; }
            let mut binding = Vec::with_capacity(skin.joints.len());
            for &joint in &skin.joints {
                if !seen.insert(joint) { continue; }
                let (name, _) = nodes
                    .get(joint)
                    .map_err(|_| "Skater skin joint is missing its name or transform")?;
                let bone = names
                    .iter()
                    .position(|n| n.eq_ignore_ascii_case(name.as_str()))
                    .ok_or_else(|| {
                        format!("Skater skin bone {name} is absent from the stock rig")
                    })?;
                let mut parent_bone = None;
                for parent in parents.iter_ancestors(joint) {
                    if roots.contains(parent) {
                        break;
                    }
                    if let Ok((name, transform)) = nodes.get(parent) {
                        if let Some(index) = names
                            .iter()
                            .position(|n| n.eq_ignore_ascii_case(name.as_str()))
                        {
                            parent_bone = Some(index);
                            break;
                        }
                        // The supplied GLB's armature node is identity. Reject
                        // another import convention rather than double a pose.
                        if !transform.to_matrix().abs_diff_eq(Mat4::IDENTITY, 0.00001) {
                            return Err(format!(
                                "Skater armature ancestor {name} has an unsupported transform"
                            ));
                        }
                    }
                }
                binding.push(BoneBinding {
                    entity: joint,
                    bone,
                    parent_bone,
                    rest: Vec3::ZERO,
                    role: Role::Body,
                });
            }
            // Bind each visible modular rig, retaining its authored inverse binds.
            bindings.extend(binding);
        }
        Ok(if bindings.is_empty() { None } else { Some(bindings) })
    })();
    match result {
        Ok(Some(binding)) => {
            animation.bindings = binding;
            animation.ready = true;
            info!(
                "GAME_CHARACTER_READY bones={} source=physical_stock_pose",
                animation.bindings.len()
            );
        }
        Ok(None) => (),
        Err(message) => {
            error!("{message}");
            exit.write(AppExit::error());
        }
    }
}
fn present(
    history: Res<crate::presentation::Presentation>,
    skater: Res<SkaterRuntime>,
    replay: Res<crate::replay::Replay>,
    time: Res<Time<Fixed>>,
    animation: Res<AnimationStatus>,
    mut nodes: Query<&mut Transform>,
) {
    if !animation.ready {
        return;
    }
    // Existing GLB was exported through Blender: its bone-local axes are
    // rotated -90 degrees about X relative to the native frames. Both files'
    // world positions are Y-up. This is a skin basis change, not a physics turn.
    let basis = render_basis();
    let Some((previous, current, alpha)) = history.view(&replay, time.overstep_fraction()) else {
        let pose:Vec<_>=skater.render_pose.iter().copied().map(native_matrix).collect();
        for (entity,transform) in animation.pose_transforms(&pose) {if let Ok(mut node)=nodes.get_mut(entity){*node=transform;}}
        return;
    };
    for binding in &animation.bindings {
        // Blend bone-local rotations, not matrix entries or independent world
        // positions: joints retain their hierarchy while limbs turn.
        let local = |snapshot: &crate::presentation::Snapshot| {
            let global = snapshot.bones[binding.bone] * basis;
            let t = Transform::from_matrix(if let Some(parent) = binding.parent_bone {
                (snapshot.bones[parent] * basis).inverse() * global
            } else { global });
            animation.adjust(binding, t, |i| snapshot.bones.get(i).map(|m| *m * basis))
        };
        if let Ok(mut transform) = nodes.get_mut(binding.entity) {
            *transform = crate::presentation::blend(local(previous), local(current), alpha);
        }
    }
}
fn render_basis() -> Mat4 {
    Mat4::from_cols(Vec4::X, -Vec4::Z, Vec4::Y, Vec4::W)
}
pub(crate) fn native_matrix(matrix: NativeMatrix) -> Mat4 {
    Mat4::from_cols(
        Vec3::from_array(matrix[0][..3].try_into().unwrap()).extend(0.0),
        Vec3::from_array(matrix[1][..3].try_into().unwrap()).extend(0.0),
        Vec3::from_array(matrix[2][..3].try_into().unwrap()).extend(0.0),
        Vec3::from_array(matrix[3][..3].try_into().unwrap()).extend(1.0),
    )
}

/// Vehicle clips use the same native model-space skeleton matrices as the SDK.
pub(crate) fn vehicle_pose(world:&mut World, pose:&[Mat4], steering:Option<(&[Mat4],f32)>) {
 let board=world.resource::<crate::physics::SkaterRuntime>().animation.evaluator.frames.bone_names.iter().position(|n|n=="SKATEBOARD_ROOT");
 world.resource_scope(|world,animation:Mut<AnimationStatus>| {
  let basis=render_basis();
  for binding in &animation.bindings {
   let Some(global)=pose.get(binding.bone).map(|m|*m*basis) else {continue;};
   let local=if let Some(parent)=binding.parent_bone {let Some(parent)=pose.get(parent) else {continue;};(*parent*basis).inverse()*global} else {global};
   let mut target=Transform::from_matrix(local);
   if let Some((turn,weight))=steering {
    if let Some(global)=turn.get(binding.bone).map(|m|*m*basis) {
     let local=if let Some(parent)=binding.parent_bone {turn.get(parent).map(|p|(*p*basis).inverse()*global).unwrap_or(global)} else {global};
     target=crate::presentation::blend(target,Transform::from_matrix(local),weight.clamp(0.,1.));
    }
   }
   // Kept-proportion characters keep their own bone lengths in the seat too.
   target=animation.adjust_with(binding,target,|i|pose.get(i).map(|m|*m*basis),true);
   if let Some(mut transform)=world.get_mut::<Transform>(binding.entity) {*transform=target;if Some(binding.bone)==board {transform.scale=Vec3::splat(0.001);}}
  }
 });
}

pub(crate) fn capture_vehicle_visual(world: &mut World) -> Vec<(Entity, Transform)> {
 let mut ids:Vec<_>=world.resource::<AnimationStatus>().bindings.iter().map(|b|b.entity).collect();
 ids.extend(world.query_filtered::<Entity,With<crate::world::PlayerRoot>>().iter(world));
 ids.into_iter().filter_map(|id|world.get::<Transform>(id).copied().map(|t|(id,t))).collect()
}
pub(crate) fn blend_vehicle_visual(world:&mut World, from:&[(Entity,Transform)], alpha:f32) {
 for &(id,previous) in from {if let Some(mut current)=world.get_mut::<Transform>(id) {*current=crate::presentation::blend(previous,*current,alpha);}}
}

/// Read the final displayed hierarchy, including vehicle steering and stance blends.
pub(crate) fn network_visual(world: &mut World) -> skate_net::packed::PoseState {
 let root=world.query_filtered::<&Transform,With<PlayerRoot>>().iter(world).next().copied().unwrap_or_default();
 let status=world.resource::<AnimationStatus>();
 let mut locals=std::collections::BTreeMap::new();
 for b in &status.bindings {if let Some(t)=world.get::<Transform>(b.entity) {locals.entry(b.bone).or_insert((b.parent_bone,t.to_matrix()));}}
 fn global(i:usize,locals:&std::collections::BTreeMap<usize,(Option<usize>,Mat4)>,depth:usize)->Mat4 {
  if depth>128 {return Mat4::IDENTITY;}
  let Some(&(parent,m))=locals.get(&i) else {return Mat4::IDENTITY;};
  parent.map_or(m,|p|global(p,locals,depth+1)*m)
 }
 let anchors=crate::physics::network::anchors(world.resource::<SkaterRuntime>());
 let basis=render_basis().inverse();
 skate_net::packed::PoseState{root:crate::physics::network::pose(root.to_matrix()),bones:anchors.into_iter().map(|i|skate_net::Bone{index:i as u16,pose:crate::physics::network::pose(if locals.contains_key(&i) {global(i,&locals,0)*basis} else {native_matrix(world.resource::<SkaterRuntime>().render_pose[i])})}).collect()}
}
#[cfg(test)]
mod online_swap_tests {
    use super::*;
    use bevy::{ecs::system::SystemState,mesh::skinning::SkinnedMesh};
    #[test]
    fn online_appearance_swap_seeds_new_rig_and_keeps_animating() {
        let mut world=World::new();
        let mut roots=vec![];
        for _ in 0..2 {
            let root=world.spawn_empty().id();
            let hip=world.spawn((Name::new("HIPS"),Transform::default(),ChildOf(root))).id();
            let head=world.spawn((Name::new("HEAD"),Transform::default(),ChildOf(hip))).id();
            world.spawn((ChildOf(root),SkinnedMesh{inverse_bindposes:default(),joints:vec![hip,head]}));
            roots.push((root,hip,head));
        }
        let names=vec!["HIPS".to_owned(),"HEAD".to_owned()];
        let bind=|world:&mut World,root| {
            let mut query:SystemState<(Query<(Entity,&SkinnedMesh)>,Query<(&Name,&Transform)>,Query<&ChildOf>)>=SystemState::new(world);
            let (skins,nodes,parents)=query.get(world);
            AnimationStatus::for_scene(root,&names,&skins,&nodes,&parents).unwrap()
        };
        let old=bind(&mut world,roots[0].0);
        let pose=[Mat4::from_translation(Vec3::new(1.,2.,3.)),Mat4::from_rotation_z(0.7)];
        for (entity,t) in old.pose_transforms(&pose){*world.get_mut::<Transform>(entity).unwrap()=t;}
        let replacement=bind(&mut world,roots[1].0);
        assert_eq!(*world.get::<Transform>(roots[1].1).unwrap(),Transform::default());
        for (entity,t) in replacement.pose_transforms(&pose){*world.get_mut::<Transform>(entity).unwrap()=t;}
        assert_eq!(world.get::<Transform>(roots[0].1),world.get::<Transform>(roots[1].1));
        assert_eq!(world.get::<Transform>(roots[0].2),world.get::<Transform>(roots[1].2));
        let next=[Mat4::from_translation(Vec3::new(2.,3.,4.)),Mat4::from_rotation_z(1.2)];
        for (entity,t) in replacement.pose_transforms(&next){*world.get_mut::<Transform>(entity).unwrap()=t;}
        assert_ne!(world.get::<Transform>(roots[0].1),world.get::<Transform>(roots[1].1));
        assert_ne!(world.get::<Transform>(roots[0].2),world.get::<Transform>(roots[1].2));
    }
}
