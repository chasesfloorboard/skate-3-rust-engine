#import bevy_pbr::{forward_io::Vertex, mesh_view_bindings::view}
struct SkyParams { settings: vec4<f32>, sun_direction: vec4<f32>, tint: vec4<f32>, cycle: vec4<f32> }
@group(#{MATERIAL_BIND_GROUP}) @binding(0) var<uniform> p: SkyParams;
@group(#{MATERIAL_BIND_GROUP}) @binding(1) var panorama: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(2) var panorama_sampler: sampler;
@group(#{MATERIAL_BIND_GROUP}) @binding(3) var sun_gradient: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(4) var sun_sampler: sampler;
struct SkyOut {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) dome_position: vec3<f32>,
}
@vertex
fn vertex(v: Vertex) -> SkyOut {
    var out: SkyOut;
    let world = v.position + vec3<f32>(view.world_position.x,p.settings.x,view.world_position.z);
    out.position = view.clip_from_world*vec4<f32>(world,1.0);
    // Reverse-Z far plane: the authored dome is larger than the camera's far distance.
    out.position.z = 0.0000001*out.position.w;
    out.uv = v.uv;
    out.dome_position = v.position;
    return out;
}
fn hash3(q: vec3<f32>) -> f32 {
    return fract(sin(dot(q, vec3<f32>(127.1, 311.7, 74.7))) * 43758.5453);
}
// Stars on a grid over the view direction: one candidate per cell, jittered,
// sparse, varied in brightness and colour, slowly twinkling.
fn stars(dir: vec3<f32>, time: f32) -> vec3<f32> {
    var total = vec3<f32>(0.0);
    for (var layer = 0; layer < 2; layer++) {
        let scale = select(90.0, 170.0, layer == 1);
        let g = dir * scale;
        let cell = floor(g);
        let h = hash3(cell + f32(layer) * 17.0);
        let density = select(0.10, 0.05, layer == 1);
        if h < density {
            let jitter = vec3<f32>(hash3(cell + 1.3), hash3(cell + 7.1), hash3(cell + 3.7)) * 0.6 + 0.2;
            let d = length(g - cell - jitter);
            let size = select(0.13, 0.10, layer == 1);
            let brightness = pow(h / density, 3.0) * 0.9 + 0.1;
            let twinkle = 0.7 + 0.3 * sin(time * (1.5 + 3.0 * hash3(cell + 9.0)) + h * 60.0);
            let tint = mix(vec3<f32>(0.75, 0.85, 1.0), vec3<f32>(1.0, 0.88, 0.7), hash3(cell + 5.0));
            total += tint * brightness * twinkle * smoothstep(size, 0.0, d);
        }
    }
    return total;
}
@fragment
fn fragment(i: SkyOut) -> @location(0) vec4<f32> {
    let d = textureSample(panorama,panorama_sampler,i.uv).rgb;
    var linear = d*d;
    if p.settings.w > 0.0 {
        // Native sky_defaultPS: sine-angle lookup in the material's specular
        // gradient, alpha controls the HDR core. World position minus the sky
        // anchor is exactly the local dome position used here.
        let dot_pl = saturate(dot(normalize(i.dome_position),p.sun_direction.xyz));
        let sin_angle = sqrt(saturate(1.0-dot_pl*dot_pl));
        let sun = textureSample(sun_gradient,sun_sampler,vec2<f32>(sin_angle/p.settings.w,0.5/16.0));
        linear += sun.rgb*sun.rgb/saturate(sun.a+0.01);
    }
    // Player sky colour preset: desaturate, then tint.
    let luminance = dot(linear, vec3<f32>(0.2126, 0.7152, 0.0722));
    linear = mix(linear, vec3<f32>(luminance), p.tint.a) * p.tint.rgb;
    var out = linear*p.settings.y*p.settings.z;
    let dir = normalize(i.dome_position);
    let up = dir.y;
    // Sunset: warm the sky, strongest near the horizon and toward the sun.
    let dusk = p.cycle.x;
    let toward_sun = saturate(dot(normalize(vec3<f32>(dir.x, 0.0, dir.z)), normalize(vec3<f32>(p.sun_direction.x, 0.0, p.sun_direction.z) + vec3<f32>(1e-4, 0.0, 0.0))));
    let horizon = 1.0 - saturate(up * 2.5);
    let glow = vec3<f32>(2.6, 1.05, 0.45) * horizon * (0.45 + 0.55 * toward_sun * toward_sun);
    let luminance_out = dot(out, vec3<f32>(0.2126, 0.7152, 0.0722));
    let sunset = mix(out * vec3<f32>(1.35, 0.72, 0.62), vec3<f32>(luminance_out) * vec3<f32>(1.2, 0.6, 0.75), 0.35) + glow * p.settings.y;
    out = mix(out, sunset, dusk);
    // Night: the tone pass grades everything down to moonlight
    // (retail_tone.wgsl), so the night sky is authored against that grade:
    // what should reach the screen, divided by it.
    let night = p.cycle.y;
    if night > 0.0 {
        let grade = pow(vec3<f32>(0.010, 0.016, 0.045), vec3<f32>(p.cycle.w));
        let zenith = vec3<f32>(0.004, 0.007, 0.022);
        let low = vec3<f32>(0.020, 0.030, 0.070);
        var sky = mix(low, zenith, saturate(up * 1.6));
        if up > -0.02 {
            sky += stars(dir, p.cycle.z) * 1.6 * saturate(up * 8.0 + 0.2);
            // Moon opposite the sun.
            let moon = -normalize(p.sun_direction.xyz + vec3<f32>(0.0, 1e-4, 0.0));
            let m = dot(dir, moon);
            sky += vec3<f32>(0.9, 0.93, 1.0) * (smoothstep(0.99955, 0.99975, m) * 1.2 + pow(saturate(m), 400.0) * 0.08);
        }
        out = mix(out, sky / grade * 2.5, night);
    }
    return vec4<f32>(out,1.0);
}
