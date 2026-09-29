//! Optional arcade assists, not recovered Burnout constants. Apply angular
//! impulses through the rigid body; never overwrite its transform or add lift.
use crate::{Vehicle, rapier3d::prelude::*};
use crate::rapier3d::utils::AngularInertiaOps;

/// Airborne with the landing assist on: the normal of the surface the vehicle
/// is heading for (along its fall, up to 40 m), else world up.
pub(crate) fn landing(v: &Vehicle, world: &PhysicsWorld) -> Option<Vector> {
    if v.definition.landing_assist <= 0. || !v.occupied
        || v.controller.wheels().iter().any(|w| w.raycast_info().is_in_contact) {
        return None;
    }
    let body = &world.bodies[v.body];
    let velocity = body.linvel();
    // Look ahead along the trajectory: mostly down, leaning with the motion.
    let direction = (velocity * 0.35 + Vector::NEG_Y * velocity.length().max(4.)).normalize_or(Vector::NEG_Y);
    let ray = Ray::new(body.translation(), direction);
    let filter = QueryFilter::default().exclude_rigid_body(v.body);
    Some(world.cast_ray_and_get_normal(&ray, 40., true, filter)
        .map(|(_, hit)| hit.normal)
        .filter(|n| n.y > 0.2)
        .unwrap_or(Vector::Y))
}

pub(crate) fn apply(v: &Vehicle, bodies: &mut RigidBodySet, dt: f32, landing: Option<Vector>) {
    let mut contacts = 0;
    let mut normal = Vector::ZERO;
    for wheel in v.controller.wheels() {
        if wheel.raycast_info().is_in_contact {
            contacts += 1;
            normal += wheel.raycast_info().contact_normal_ws;
        }
    }
    let body = &mut bodies[v.body];
    let rotation = *body.rotation();
    let local_omega = rotation.inverse() * body.angvel();
    let mut acceleration = Vector::ZERO;
    if contacts >= 2 && v.definition.ground_stability > 0. {
        let normal = normal.normalize_or_zero();
        let up = rotation * Vector::Y;
        if up.dot(normal) > 0.25 {
            // Follow banked support, not global up. Stabilize roll without steering
            // the car or fighting the pitch needed to climb a ramp.
            let error = rotation.inverse() * up.cross(normal);
            acceleration.z = (error.z * 24. - local_omega.z * 8.)
                * v.definition.ground_stability;
        }
    } else if contacts == 0 && v.occupied && v.definition.air_control > 0. {
        // Rate target keeps repeated input controllable. Neutral stick damps only
        // pitch/roll; yaw and linear momentum remain with the physical simulation.
        // Driver-left is +X: a negative local-Z rotation leans the roof left.
        let target = Vector::new(v.controls.pitch * 2.5, 0., -v.controls.steering * 2.5);
        let strength = v.definition.air_control;
        acceleration.x = ((target.x - local_omega.x) * 3.).clamp(-strength, strength);
        acceleration.z = ((target.z - local_omega.z) * 3.).clamp(-strength, strength);
    }
    if let Some(up) = landing {
        // Board-style landing: wheels down onto the surface ahead, nose along
        // the direction of travel. The stick still turns the car, the assist
        // backs off while it is held.
        let velocity = body.linvel();
        let mut forward = velocity - up * velocity.dot(up);
        if forward.length_squared() < 1.0 {
            let current = rotation * Vector::Z;
            forward = current - up * current.dot(up);
        }
        let forward = forward.normalize_or(Vector::Z);
        let right = up.cross(forward).normalize_or(Vector::X);
        let wanted = Rotation::from_mat3(&glamx::Mat3::from_cols(right, up, right.cross(up)));
        let error = rotation.inverse() * (wanted * rotation.inverse()).to_scaled_axis();
        let input = v.controls.pitch.abs().max(v.controls.steering.abs());
        let assist = v.definition.landing_assist * (1.0 - 0.7 * input);
        acceleration += (error * 10. - local_omega * 4.) * assist;
    }
    if acceleration.length_squared() > 0. {
        let angular_impulse = body.mass_properties().effective_world_inv_inertia.inverse()
            * (rotation * acceleration * dt);
        body.apply_torque_impulse(angular_impulse, true);
    }
}
