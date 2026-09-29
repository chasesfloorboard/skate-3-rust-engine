#import bevy_core_pipeline::fullscreen_vertex_shader::FullscreenVertexOutput
@group(0) @binding(0) var source: texture_2d<f32>;
@group(0) @binding(1) var source_sampler: sampler;
@group(0) @binding(2) var<storage, read> exposure: vec4<f32>;

@fragment
fn fragment(i: FullscreenVertexOutput) -> @location(0) vec4<f32> {
    let c = textureSample(source,source_sampler,i.uv);
    // Day/night grade (day_cycle.rs): blue night, warm light around sunset.
    // Applied before the curve, whose square root softens it, hence the depth.
    // Beyond 1 (ambient light below Auto) only the dimming deepens.
    let depth = exposure.w;
    let night = min(depth, 1.0);
    // Share of this pixel lit by street lamps (retail_world.wgsl, 1 - alpha):
    // that part skips the blue tint and the colour drain.
    let lamp = saturate(1.0 - c.a);
    let dusk = max(0.0, 1.0 - abs(night - 0.3) / 0.3);
    // Geometric fade: each step of `night` dims by the same ratio, so the
    // dusk-to-night transition reads evenly instead of snapping dark at the end.
    let grade = pow(vec3<f32>(0.010,0.016,0.045), vec3<f32>(depth))
        * mix(vec3<f32>(1.0), vec3<f32>(1.12,0.86,0.66), dusk*0.6);
    // Moonlight: colour drains away at night (eyes lose colour in the dark),
    // then the blue grade and a slight crush of the shadows.
    // Only the dark drains: anything lit well enough after the grade (a
    // street lamp's pool) keeps its colour, as eyes do.
    let luma = dot(c.rgb, vec3<f32>(0.2126,0.7152,0.0722));
    let lit = dot(c.rgb*exposure.x/2.5*grade, vec3<f32>(0.2126,0.7152,0.0722));
    let scotopic = 1.0 - smoothstep(0.03, 0.3, lit);
    let night_rgb = mix(c.rgb, vec3<f32>(luma), 0.7*night*scotopic*(1.0 - lamp));
    let neutral = mix(grade, vec3<f32>(dot(grade, vec3<f32>(0.2126,0.7152,0.0722))), lamp);
    let graded = max(night_rgb*exposure.x/2.5*neutral,vec3<f32>(0.0));
    let xe = graded*mix(vec3<f32>(1.0), graded/(graded+vec3<f32>(0.01)), night);
    let t = saturate(1.0-xe);
    let tm = max(xe*0.25+0.75,vec3<f32>(1.0))-t*t;
    let gamma = saturate(sqrt(max(tm*0.5,vec3<f32>(0.0)))*1.41);
    // Bevy's final output attachment performs sRGB encoding. The retail
    // curve already includes gamma; invert sRGB here to avoid encoding twice.
    let linear = select(gamma/12.92,pow((gamma+0.055)/1.055,vec3<f32>(2.4)),gamma>vec3<f32>(0.04045));
    // The reference's final tone pass is opaque. Material coverage has
    // already been resolved; carrying it into the presentation blit would
    // composite foliage a second time against the window's clear colour.
    return vec4<f32>(linear,1.0);
}
