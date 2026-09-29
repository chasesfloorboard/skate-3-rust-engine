#define_import_path skate_dynamic_lights
// Street lamps, stadium floodlights and headlights (street_lights.rs, vehicle
// lights) on characters and props, as retail_world.wgsl lights the streets:
// diffuse only, inverse square with Bevy's range falloff, in the world's
// lighting units. Night grades the image down in the tone pass; lamp light is
// pre-divided by the grade so it keeps its strength, and `lamp_share` tells
// the tone pass how much of the pixel to leave in colour (1 - alpha).
#import bevy_pbr::mesh_view_bindings as frame
#import bevy_pbr::clustered_forward as clustering
#import bevy_pbr::mesh_view_types::POINT_LIGHT_FLAGS_SPOT_LIGHT_Y_NEGATIVE

const DYNAMIC_LIGHT_SCALE: f32 = 2.0e-4;
const LUMA: vec3<f32> = vec3<f32>(0.2126, 0.7152, 0.0722);

// Summed lamp irradiance at a surface point, found through the light clusters.
fn dynamic_lights(world: vec3<f32>, n: vec3<f32>) -> vec3<f32> {
    let clip = frame::view.clip_from_world * vec4<f32>(world, 1.0);
    let ndc = clip.xy / clip.w;
    let frag = frame::view.viewport.xy + (vec2<f32>(ndc.x, -ndc.y) * 0.5 + 0.5) * frame::view.viewport.zw;
    let view_z = (frame::view.view_from_world * vec4<f32>(world, 1.0)).z;
    let cluster = clustering::fragment_cluster_index(frag, view_z, false);
    let ranges = clustering::unpack_clusterable_object_index_ranges(cluster);
    var total = vec3<f32>(0.0);
    for (var i = ranges.first_point_light_index_offset; i < ranges.first_reflection_probe_index_offset; i += 1u) {
        let light = &frame::clusterable_objects.data[clustering::get_clusterable_object_id(i)];
        let to_light = (*light).position_radius.xyz - world;
        let d2 = max(dot(to_light, to_light), 1.0);
        let l = to_light * inverseSqrt(d2);
        let factor = d2 * (*light).color_inverse_square_range.w;
        let range = saturate(1.0 - factor * factor);
        var cone = 1.0;
        if i >= ranges.first_spot_light_index_offset {
            var dir = vec3<f32>((*light).light_custom_data.x, 0.0, (*light).light_custom_data.y);
            dir.y = sqrt(max(0.0, 1.0 - dir.x * dir.x - dir.z * dir.z));
            if ((*light).flags & POINT_LIGHT_FLAGS_SPOT_LIGHT_Y_NEGATIVE) != 0u { dir.y = -dir.y; }
            let c = saturate(dot(-dir, l) * (*light).light_custom_data.z + (*light).light_custom_data.w);
            cone = c * c;
        }
        // Wrapped: a lamp beside or above still reaches a body's vertical
        // sides a little, so figures under a lamp read as lit, not black.
        let wrap = saturate((dot(n, l) + 0.35) / 1.35);
        total += (*light).color_inverse_square_range.rgb * wrap * range * range / d2 * cone;
    }
    return total * DYNAMIC_LIGHT_SCALE;
}

// The tone pass's night brightness (retail_tone.wgsl) for a night amount.
fn night_grade(night: f32) -> f32 {
    return dot(pow(vec3<f32>(0.010, 0.016, 0.045), vec3<f32>(max(night, 0.0))), LUMA);
}

// Lamp light for albedo `d` in the same units as the surface's other light,
// pale in hot cores as retail_world.wgsl does. `exposure` is what the caller
// multiplies its light by before output.
fn lamp_light(d: vec3<f32>, world: vec3<f32>, n: vec3<f32>, night: f32, exposure: f32) -> vec3<f32> {
    let grade = night_grade(night);
    var lamp = d * dynamic_lights(world, n) / grade;
    let shown = dot(lamp, LUMA) * grade * exposure;
    return mix(lamp, vec3<f32>(dot(lamp, LUMA)), 0.7 * smoothstep(0.25, 1.2, shown));
}

// The lamp's share of a pixel whose other light is `base` (same units).
fn lamp_share(base: vec3<f32>, lamp: vec3<f32>) -> f32 {
    let l = dot(lamp, LUMA);
    return l / max(dot(base, LUMA) + l, 1e-6);
}
