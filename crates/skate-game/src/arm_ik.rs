//! Presentation-only arm IK: bends the skater's arms so the hands reach grip
//! points (held props, props.rs). Runs on the captured model-space pose; the
//! simulation skeleton is untouched.
use bevy::prelude::*;

/// Upper arm, forearm and hand bone names for each side.
const ARMS: [[&str; 3]; 2] = [["LEFTARM", "LEFTFOREARM", "LEFTHAND"], ["RIGHTARM", "RIGHTFOREARM", "RIGHTHAND"]];

/// Bone indices of both arm chains, if the skeleton has them.
pub(crate) fn chains<S: AsRef<str>>(names: &[S]) -> Option<[[usize; 3]; 2]> {
    let find = |n: &str| names.iter().position(|b| b.as_ref().eq_ignore_ascii_case(n));
    let chain = |c: [&str; 3]| Some([find(c[0])?, find(c[1])?, find(c[2])?]);
    Some([chain(ARMS[0])?, chain(ARMS[1])?])
}

fn descendants(parents: &[i32], root: usize) -> Vec<usize> {
    let mut out = vec![root];
    // Parents precede children in the stock hierarchy, but do not rely on it.
    let mut changed = true;
    while changed {
        changed = false;
        for (i, &p) in parents.iter().enumerate() {
            if p >= 0 && out.contains(&(p as usize)) && !out.contains(&i) {
                out.push(i);
                changed = true;
            }
        }
    }
    out
}

/// Rotates `bone` and everything under it by `rotation` about `pivot`.
fn rotate_subtree(bones: &mut [Mat4], parents: &[i32], bone: usize, pivot: Vec3, rotation: Quat) {
    let about = Mat4::from_translation(pivot) * Mat4::from_quat(rotation) * Mat4::from_translation(-pivot);
    for i in descendants(parents, bone) {
        bones[i] = about * bones[i];
    }
}

/// Two-bone IK toward `target` (model space), blended by `weight`. The elbow
/// keeps bending the way the animation already bends it.
pub(crate) fn reach(bones: &mut [Mat4], parents: &[i32], [arm, fore, hand]: [usize; 3], target: Vec3, weight: f32) {
    if weight <= 0.0 || [arm, fore, hand].iter().any(|&i| i >= bones.len()) { return; }
    let shoulder = bones[arm].w_axis.truncate();
    let elbow = bones[fore].w_axis.truncate();
    let wrist = bones[hand].w_axis.truncate();
    let target = wrist.lerp(target, weight.clamp(0.0, 1.0));
    let (a, b) = (elbow.distance(shoulder), wrist.distance(elbow));
    if a < 1e-4 || b < 1e-4 { return; }
    let to_target = target - shoulder;
    let d = to_target.length().clamp((a - b).abs() + 1e-3, (a + b) * 0.999);
    let dir = to_target.normalize_or(Vec3::NEG_Y);
    // Bend plane: the current elbow offset from the shoulder->target line,
    // falling back to "elbow down and back" when the arm is straight.
    let bend = elbow - shoulder;
    let pole = (bend - dir * bend.dot(dir)).try_normalize()
        .or_else(|| (Vec3::NEG_Y - dir * dir.y).try_normalize())
        .unwrap_or(Vec3::X);
    let cos = ((a * a + d * d - b * b) / (2.0 * a * d)).clamp(-1.0, 1.0);
    let new_elbow = shoulder + dir * (a * cos) + pole * (a * (1.0 - cos * cos).sqrt());
    let upper = Quat::from_rotation_arc((elbow - shoulder).normalize(), (new_elbow - shoulder).normalize());
    rotate_subtree(bones, parents, arm, shoulder, upper);
    let moved_wrist = bones[hand].w_axis.truncate();
    let goal = shoulder + dir * d;
    let lower = Quat::from_rotation_arc((moved_wrist - new_elbow).normalize(), (goal - new_elbow).normalize());
    rotate_subtree(bones, parents, fore, new_elbow, lower);
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn hand_reaches_target_and_bone_lengths_are_kept() {
        // Root, shoulder at (0,1.4,0), elbow 0.3 below, wrist 0.28 below, a finger.
        let parents = [-1, 0, 1, 2, 3];
        let at = |p: Vec3| Mat4::from_translation(p);
        let mut bones = vec![at(Vec3::ZERO), at(Vec3::new(0.0, 1.4, 0.0)), at(Vec3::new(0.0, 1.1, 0.05)),
            at(Vec3::new(0.0, 0.82, 0.0)), at(Vec3::new(0.0, 0.75, 0.0))];
        let before = bones[4].w_axis.truncate().distance(bones[3].w_axis.truncate());
        let target = Vec3::new(0.0, 1.1, 0.4);
        reach(&mut bones, &parents, [1, 2, 3], target, 1.0);
        let wrist = bones[3].w_axis.truncate();
        assert!(wrist.distance(target) < 0.01, "wrist {wrist}");
        assert!((bones[2].w_axis.truncate().distance(Vec3::new(0.0, 1.4, 0.0)) - 0.3041).abs() < 0.01);
        // The finger came along with the hand.
        assert!((bones[4].w_axis.truncate().distance(wrist) - before).abs() < 1e-4);
    }
}
