// Dynamic props (props.rs): no baked lightmap, so they are lit like the retail
// character: local probe irradiance (the skater's SH, published each frame in
// the shared frame state) plus the sun with its shadow, at the world's 2.5
// exposure baseline.
#import bevy_pbr::{forward_io::VertexOutput, mesh_view_bindings as view_bindings}
#import bevy_pbr::shadows::fetch_directional_shadow

struct FrameState { shadow: vec4<f32>, clock: vec4<f32>, pca: array<vec4<f32>, 7>, sun: vec4<f32>, sh: array<vec4<f32>, 9> }
@group(#{MATERIAL_BIND_GROUP}) @binding(0) var<uniform> params: vec4<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(1) var diffuse: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(2) var diffuse_sampler: sampler;
@group(#{MATERIAL_BIND_GROUP}) @binding(3) var<storage, read> frame: FrameState;
@group(#{MATERIAL_BIND_GROUP}) @binding(4) var<uniform> shine: vec4<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(5) var matcap: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(6) var matcap_sampler: sampler;

@fragment
fn fragment(i: VertexOutput) -> @location(0) vec4<f32> {
    let a = textureSample(diffuse, diffuse_sampler, i.uv);
    if a.a < params.w { discard; }
    let d = a.rgb * params.rgb;
    let n = normalize(i.world_normal);
    let sh = frame.sh;
    let irr = saturate(sh[0].rgb+n.x*sh[1].rgb+n.y*sh[2].rgb+n.z*sh[3].rgb
        +n.x*n.z*sh[4].rgb+n.z*n.y*sh[5].rgb+n.y*n.x*sh[6].rgb
        +(3.0*n.z*n.z-1.0)*sh[7].rgb+(n.x*n.x-n.y*n.y)*sh[8].rgb);
    let probe = dot(sh[0].rgb, vec3<f32>(1.0)) > 0.0;
    let ambient = select(vec3<f32>(0.30 + 0.15*(0.5+0.5*n.y)), irr, probe);
    let sun = select(normalize(vec3<f32>(0.3,0.8,0.4)), frame.sun.xyz, frame.sun.w > 0.5);
    let ndl = saturate(dot(n, sun));
    let view_z = (view_bindings::view.view_from_world * i.world_position).z;
    var shadow = 1.0;
    for (var id = 0u; id < view_bindings::lights.n_directional_lights; id += 1u) {
        // The all-caster light, as the character shader uses.
        if (view_bindings::lights.directional_lights[id].flags & 5u) == 1u {
            shadow = fetch_directional_shadow(id, i.world_position, n, view_z);
            break;
        }
    }
    let lit = ambient + vec3<f32>(0.95, 0.90, 0.80) * 0.75 * ndl * shadow;
    if shine.x > 0.5 {
        // Metal: the sphere map by view-space normal, tinted by the albedo
        // (dark albedos, Metal Mario's, reflect untinted), dimmed with the
        // scene light, plus a sharp sun highlight.
        let nv = normalize((view_bindings::view.view_from_world * vec4<f32>(n, 0.0)).xyz);
        let reflection = textureSample(matcap, matcap_sampler, vec2<f32>(0.5 + 0.48 * nv.x, 0.5 - 0.48 * nv.y)).rgb;
        let tint = max(d, vec3<f32>(1.0 - dot(d, vec3<f32>(0.333))));
        let light = clamp(dot(lit, vec3<f32>(0.333)) * 1.4, 0.08, 1.6);
        let to_eye = normalize(view_bindings::view.world_position - i.world_position.xyz);
        let highlight = pow(saturate(dot(reflect(-to_eye, n), sun)), 60.0) * shadow * 3.0;
        let metal = reflection * tint * light * 2.5 + vec3<f32>(highlight) * tint;
        // Dark albedos (Metal Mario's) keep a silver body under the reflection.
        let silver = vec3<f32>(0.42 * (1.0 - dot(d, vec3<f32>(0.333))));
        return vec4<f32>((d * 0.6 + silver * 2.5) * lit + metal, a.a);
    }
    return vec4<f32>(d * lit * 2.5, a.a);
}
