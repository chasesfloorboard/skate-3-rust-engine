#import bevy_pbr::{forward_io::VertexOutput, mesh_view_bindings as frame}
#import bevy_pbr::shadows::fetch_directional_shadow
#import bevy_pbr::clustered_forward as clustering
#import bevy_pbr::mesh_view_types::POINT_LIGHT_FLAGS_SPOT_LIGHT_Y_NEGATIVE

#import skate_retail::material_bindings as bindings
#ifdef BINDLESS
#import bevy_pbr::mesh_bindings::mesh
#endif

// Textures and base UVs both have V flipped by the exporter. Scale in the
// authored coordinate system, then return to the flipped texture rows.
fn scaled_uv(uv: vec2<f32>, scale: f32) -> vec2<f32> {
    return vec2<f32>(uv.x*scale, 1.0-(1.0-uv.y)*scale);
}

// Dynamic point/spot lights (street_lights.rs, vehicle headlights) on top of
// the baked lighting: diffuse only, inverse square with Bevy's range falloff.
// Scaled into the baked lightmap's units by DYNAMIC_LIGHT_SCALE.
const DYNAMIC_LIGHT_SCALE: f32 = 2.0e-4;
fn dynamic_lights(frag: vec4<f32>, world: vec3<f32>, n: vec3<f32>) -> vec3<f32> {
    let view_z = (frame::view.view_from_world * vec4<f32>(world, 1.0)).z;
    let cluster = clustering::fragment_cluster_index(frag.xy, view_z, false);
    let ranges = clustering::unpack_clusterable_object_index_ranges(cluster);
    var total = vec3<f32>(0.0);
    for (var i = ranges.first_point_light_index_offset; i < ranges.first_reflection_probe_index_offset; i += 1u) {
        let light = &frame::clusterable_objects.data[clustering::get_clusterable_object_id(i)];
        let to_light = (*light).position_radius.xyz - world;
        // At least a metre: no blown-out hot spot on whatever sits by the lamp.
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
        total += (*light).color_inverse_square_range.rgb * saturate(dot(n, l)) * range * range / d2 * cone;
    }
    return total * DYNAMIC_LIGHT_SCALE;
}

@fragment
fn fragment(i: VertexOutput) -> @location(0) vec4<f32> {
#ifdef BINDLESS
    let slot = mesh[i.instance_index].material_and_lightmap_bind_group_slot & 0xffffu;
    let index = bindings::indices[slot];
    let p = bindings::params[index.params];
    let frame_state = bindings::frames[index.frame_state];
#else
    let slot = 0u;
    let p = bindings::p;
    let frame_state = bindings::frame_state;
#endif
    let fam = u32(p.mode.x);
    let flags = u32(p.mode.y);
    var diffuse_uv=i.uv;
    if fam==14u { diffuse_uv+=fract(frame_state.clock.x*p.water[1].xy*vec2<f32>(1.0,-1.0)); }
    let a = bindings::sample_diffuse(slot, diffuse_uv);
    let lm = bindings::sample_lightmap(slot, i.uv_b, 0.0).rgb;
    // Sample before alpha rejection: implicit derivatives must be uniform.
    var nm = vec3<f32>(0.5,0.5,1.0);
    var detail = vec2<f32>(0.5);
    var overlay_sample = vec3<f32>(0.5);
    var art = vec4<f32>(0.0);
    var masks = vec3<f32>(0.0);
    if (flags & 1u) != 0u && (fam <= 6u || fam == 13u) { nm = bindings::sample_normal_map(slot,i.uv).rgb; }
    if (flags & 2u) != 0u && fam != 2u {
        detail = bindings::sample_detail_map(slot,scaled_uv(i.uv,p.surface.z)).rg;
    }
    if fam == 5u || fam == 6u || fam == 13u {
        // The reference folds the reflective material's constant detail texel.
        if (flags & 128u) != 0u { detail = bindings::load_detail(slot,vec2<i32>(0),0).rg; }
    }
    if (flags & 4u) != 0u { overlay_sample = bindings::sample_macro_map(slot,scaled_uv(i.uv,p.surface.x)).rgb; }
    if (flags & 8u) != 0u && (fam == 3u || fam == 4u) { art = bindings::sample_decal_map(slot,i.color.xy); }
    if (flags & 16u) != 0u { masks = bindings::sample_specular_map(slot,i.uv).rgb; }
    var wn = normalize(i.world_normal);
    let dp1 = dpdx(i.world_position.xyz);
    let dp2 = dpdy(i.world_position.xyz);
    // Exporter flips V and texture rows; recover original UV derivatives.
    let du1 = dpdx(i.uv * vec2<f32>(1.0,-1.0));
    let du2 = dpdy(i.uv * vec2<f32>(1.0,-1.0));
    let dp2p = cross(dp2,wn);
    let dp1p = cross(wn,dp1);
    var kt = dp2p * du1.x + dp1p * du2.x;
    var kb = dp2p * du1.y + dp1p * du2.y;
    kt *= -inverseSqrt(max(dot(kt,kt),1e-12));
    kb *= inverseSqrt(max(dot(kb,kb),1e-12));
#ifdef VERTEX_TANGENTS
    if dot(i.world_tangent.xyz,i.world_tangent.xyz) > 0.01 {
        kt = normalize(i.world_tangent.xyz);
        kb = normalize(cross(wn,kt)) * i.world_tangent.w;
    }
#endif
    let rpos = i.world_position.xyz - frame::view.world_position;
    let vd = -normalize(rpos);
    // Authored render-location direction for the tangent-space sign terms.
    // This does not add directional light energy or a shadow source.
    let sun = p.sun_direction.xyz;
    var d = a.rgb*a.rgb;
    var alpha = 1.0;
    var lin = vec3<f32>(0.0);
    var baked = lm*lm;
    if (flags & 32u) == 0u && (fam<=8u || fam==13u) {
        // No baked lightmap (placed dynamic props). Like the retail character:
        // local probe irradiance (the skater's, published each frame) plus the
        // sun; sun shadows below still apply. Day/night moves the sun.
        let sun_now = select(sun, frame_state.sun.xyz, frame_state.sun.w > 0.5);
        let ndl = saturate(dot(wn, sun_now));
        let sh = frame_state.sh;
        let v = wn;
        let irr = saturate(sh[0].rgb+v.x*sh[1].rgb+v.y*sh[2].rgb+v.z*sh[3].rgb
            +v.x*v.z*sh[4].rgb+v.z*v.y*sh[5].rgb+v.y*v.x*sh[6].rgb
            +(3.0*v.z*v.z-1.0)*sh[7].rgb+(v.x*v.x-v.y*v.y)*sh[8].rgb);
        let probe = dot(sh[0].rgb, vec3<f32>(1.0)) > 0.0;
        let sky = 0.5 + 0.5*wn.y;
        baked = select(vec3<f32>(0.20 + 0.14*sky), irr, probe) + vec3<f32>(0.95,0.90,0.80)*0.75*ndl;
    }
    if frame_state.shadow.w>0.0 && (fam<=8u || fam==13u) {
        let view_z=(frame::view.view_from_world*i.world_position).z;
        for (var light_id=0u; light_id<frame::lights.n_directional_lights; light_id+=1u) {
            // The lightmapped receiver source contains only player/board casters.
            if (frame::lights.directional_lights[light_id].flags & 5u)==5u {
                let visibility=fetch_directional_shadow(light_id,i.world_position,wn,view_z);
                baked=min(baked,vec3<f32>(visibility)+frame_state.shadow.rgb);
                break;
            }
        }
    }
    if fam==14u {
        lin=d*p.water[0].y;
    } else if fam==32u {
        lin=d*p.water[0].x;
        alpha=saturate((i.world_position.y-p.water[0].z)/max(p.water[0].y-p.water[0].z,1e-4));
    } else if fam==31u {
        let raw_uv=vec2<f32>(i.uv.x,1.0-i.uv.y);
        let uv=raw_uv*p.water[1].w;
        let sample_uv=vec2<f32>(uv.x,1.0-uv.y);
        let c0=bindings::sample_normal_map(slot,sample_uv)*2.0-1.0;
        let c1=bindings::sample_detail_map(slot,sample_uv)*2.0-1.0;
        let pca=(vec3<f32>(dot(c0,frame_state.pca[1])+dot(c1,frame_state.pca[2]),
            dot(c0,frame_state.pca[3])+dot(c1,frame_state.pca[4]),
            dot(c0,frame_state.pca[5])+dot(c1,frame_state.pca[6]))+frame_state.pca[0].xyz)*2.0-1.0;
        var overlay=1.0;
        if (flags & 4u)!=0u {
            let uv_overlay=raw_uv*p.surface.x;
            overlay=bindings::sample_macro_map(slot,vec2<f32>(uv_overlay.x,1.0-uv_overlay.y)).r;
        }
        let tonedown=2.0*p.water[1].z*saturate(dot(c1,frame_state.pca[6])+overlay);
        let nt=normalize(max(abs(normalize(mix(vec3<f32>(0.0,0.0,1.0),pca,tonedown))),vec3<f32>(0.001)));
        let n=nt.xzy;
        let rv=vd-2.0*n*dot(vd,n);
        var cube=vec3<f32>(0.0);
        if (flags & 64u)!=0u { cube=bindings::sample_environment(slot,vec3<f32>(-rv.x,-rv.y,rv.z),log2(frame::view.viewport.w/640.0)).rgb; }
        let olm=bindings::sample_lightmap(slot,i.uv_b+0.01*nt.xy*vec2<f32>(1.0,-1.0),0.0).rgb;
        let fres=0.25+0.75*pow(1.0-abs(dot(n,vd)),5.0);
        let u=normalize(vec3<f32>(-n.z,0.0,n.x));
        let v=cross(n,u);
        let ax=p.water[2].z/p.water[2].x;
        let ay=p.water[2].z/p.water[2].y;
        let h=normalize(sun+vd);
        let eu=dot(h,u)/ax;
        let ev=dot(h,v)/ay;
        let den=sqrt(abs(dot(n,sun)*dot(n,vd)))*12.566371*ax*ay;
        let ward=exp(-2.0*(eu*eu+ev*ev)/(1.0+dot(h,n)))/max(den,1e-6);
        lin=(cube*olm*olm*fres+ward*p.water[0].rgb)*p.water[1].y;
        alpha=1.0;
    } else if fam==30u || fam==33u {
        let t=frame_state.clock.x;
        // Convert to original UVs for scale/scroll, then back to flipped rows.
        let raw_uv=vec2<f32>(i.uv.x,1.0-i.uv.y);
        let uv_scale=select(1.0,p.water[3].x,fam==33u);
        let uv1=raw_uv*p.water[2].xy*uv_scale+p.water[1].xy*t;
        let uv2=raw_uv*p.water[2].zw*uv_scale+p.water[1].zw*t;
        let n1=bindings::sample_normal_map(slot,vec2<f32>(uv1.x,1.0-uv1.y));
        let n2=bindings::sample_normal_map(slot,vec2<f32>(uv2.x,1.0-uv2.y));
        var vn=normalize((2.0*n1.rgb+2.0*n2.rgb-2.0)*p.water[0].xzw);
        var water_n=normalize(vn.x*kt+vn.y*kb+vn.z*wn);
        var sample_uv=i.uv;
        if fam==33u {
            let c1=bindings::sample_detail_map(slot,vec2<f32>(uv1.x,1.0-uv1.y))*2.0-1.0;
            let c2=bindings::sample_detail_map(slot,vec2<f32>(uv2.x,1.0-uv2.y))*2.0-1.0;
            let a1=n1*2.0-1.0;
            let a2=n2*2.0-1.0;
            // water_defaultPS instructions 22..54: the native mean is XYZ,
            // weights are R/G/B pairs. FrameState stores the ocean's R/B/G
            // arrangement; recover those original registers here.
            let mean=frame_state.pca[0].xzy;
            let pca1=vec3<f32>(dot(a1,frame_state.pca[1])+dot(c1,frame_state.pca[2])+mean.x,
                dot(a1,frame_state.pca[5])+dot(c1,frame_state.pca[6])+mean.y,
                dot(a1,frame_state.pca[3])+dot(c1,frame_state.pca[4])+mean.z);
            let pca2=vec3<f32>(dot(a2,frame_state.pca[1])+dot(c2,frame_state.pca[2])+mean.x,
                dot(a2,frame_state.pca[5])+dot(c2,frame_state.pca[6])+mean.y,
                dot(a2,frame_state.pca[3])+dot(c2,frame_state.pca[4])+mean.z);
            let first=vec3<f32>((pca2.x*2.0-1.0)*p.water[0].z,
                1.0+2.0*(pca2.y-1.0)*p.water[0].z,(pca2.z*2.0-1.0)*p.water[0].z);
            let second=vec3<f32>((pca1.x*2.0-1.0)*p.water[0].w,
                1.0+2.0*(pca1.y-1.0)*p.water[0].w,(pca1.z*2.0-1.0)*p.water[0].w);
            water_n=first*inverseSqrt(max(dot(first,first),1e-12));
            let refract=second*inverseSqrt(max(dot(second,second),1e-12));
            sample_uv+=0.02*refract.xz*vec2<f32>(1.0,-1.0);
            d=bindings::sample_diffuse(slot,sample_uv).rgb;
            d*=d;
            vn=water_n;
        }
        let water_lm_uv=i.uv_b+0.01*vn.xz*vec2<f32>(1.0,-1.0);
        var wlm=bindings::sample_lightmap(slot,water_lm_uv,0.0).rgb;
        if fam==33u {
            // Native tf3 fetches at all four half-texel corners, averages,
            // then squares. Squaring each tap would change baked lighting.
            let texel=0.5/vec2<f32>(bindings::lightmap_dimensions(slot));
            wlm=(bindings::sample_lightmap(slot,water_lm_uv+texel,0.0).rgb
                +bindings::sample_lightmap(slot,water_lm_uv-texel,0.0).rgb
                +bindings::sample_lightmap(slot,water_lm_uv+texel*vec2<f32>(-1.0,1.0),0.0).rgb
                +bindings::sample_lightmap(slot,water_lm_uv+texel*vec2<f32>(1.0,-1.0),0.0).rgb)*0.25;
        }
        var lml=wlm*wlm;
        if frame_state.shadow.w>0.0 {
            let vz=(frame::view.view_from_world*i.world_position).z;
            for(var id=0u;id<frame::lights.n_directional_lights;id+=1u) {
                if (frame::lights.directional_lights[id].flags & 5u)==5u {
                    let floor=select(frame_state.shadow.rgb,vec3<f32>(0.09,0.13,0.05),fam==33u);
                    lml=min(lml,vec3<f32>(fetch_directional_shadow(id,i.world_position,wn,vz))+floor); break;
                }
            }
        }
        let kd=dot(water_n,vec3<f32>(0.58*sign(sun.x),0.62*sign(sun.y),0.39))*2.39562;
        var wm=vec2<f32>(0.0);
        if (flags & 16u)!=0u { wm=saturate(bindings::sample_specular_map(slot,sample_uv).xz-p.water[3].y); }
        let reflected_light=2.0*water_n*dot(water_n,sun)-sun;
        let ks=pow(max(saturate(dot(vd,reflected_light)),1e-6),p.water[3].z);
        var spec=ks*wm.x*vec3<f32>(2.1,1.8,1.5)*saturate(lml.g-0.1);
        if (flags & 64u)!=0u {
            let rv=vd-2.0*water_n*dot(vd,water_n);
            let cube=bindings::sample_environment(slot,vec3<f32>(-rv.x,-rv.y,rv.z),log2(frame::view.viewport.w/640.0)).rgb;
            let lum=0.3*saturate(4.0*wm.y-2.6);
            spec+=cube*(lml.g+lum*(1.0-lml.g))*wm.y*1.5;
        }
        lin=(lml*kd*d+spec)*p.water[0].y;
        alpha=max(spec.g,p.water[3].w);
    } else if fam == 9u || fam == 10u {
        lin = d * max(lm*lm,vec3<f32>(p.family.y)) * p.family.x;
        if fam == 9u { lin *= p.family.z; }
        alpha = a.a;
    } else if fam == 11u || fam == 12u {
        lin = d;
        if fam == 11u { lin *= p.family.w; }
    } else {
        if (fam == 3u || fam == 4u) && (flags & 8u) != 0u && (flags & 512u) == 0u { d = mix(d,art.rgb*art.rgb,art.a*p.decal.x); }
        if (flags & 4u) != 0u && fam < 13u && (flags & 256u) == 0u { d *= saturate((overlay_sample-0.5)*p.surface.y+0.5); }
        var kd = 0.93429;
        if (fam <= 6u || fam == 13u) && (flags & 1u) != 0u {
            var dxy = vec2<f32>(0.5);
            if (flags & 2u) != 0u && fam != 2u { dxy = detail; }
            if (fam == 5u || fam == 6u || fam == 13u) && (flags & 128u) != 0u { dxy = detail; }
            let raw = vec3<f32>(nm.xy*2.0+dxy*2.0-2.0,nm.z*2.0-1.0);
            var vnd = raw;
            if fam != 2u && fam != 6u { vnd = raw * inverseSqrt(max(dot(raw,raw),1e-12)); }
            var tt = kt;
            var bb = kb;
            if fam == 5u || fam == 6u || fam == 13u {
                let up = vec3<f32>(0.0,1.0,0.0)-wn*wn.y;
                if length(up) > 0.05 {
                    let b2 = normalize(up);
                    let t2 = cross(b2,wn);
                    tt = t2*select(-1.0,1.0,dot(t2,kt)>=0.0);
                    bb = b2*select(-1.0,1.0,dot(b2,kb)>=0.0);
                }
            }
            wn = normalize(raw.x*tt+raw.y*bb+wn*max(raw.z,0.05));
            kd = (vnd.x*0.58*sign(dot(kt,sun))+vnd.y*0.62*sign(dot(kb,sun))+vnd.z*0.39)*2.39562;
        }
        var lml = baked;
        lin = lml*kd*d;
        if fam == 13u { lin = lml*d*a.a; }
        if (flags & 16u) != 0u {
            let lit = vec3<f32>(-0.14,0.5,0.9);
            let reflected = lit-2.0*wn*dot(wn,lit);
            let bp = saturate(dot(vd,-reflected));
            let ks = pow(max(bp,1e-6),10.0+290.0*masks.y);
            lin += ks*vec3<f32>(2.1,1.8,1.5)*lml.g*masks.x;
        }
        if (fam == 5u || fam == 6u || fam == 13u) && (flags & 64u) != 0u {
            let rv = vd-2.0*wn*dot(vd,wn);
            let dir = vec3<f32>(-rv.x,-rv.y,rv.z);
            let cube = bindings::sample_environment(slot,dir,log2(frame::view.viewport.w/640.0)).rgb;
            let rl = 0.3*saturate(4.0*masks.z-2.6);
            let lum = lml.g+rl*(1.0-lml.g);
            lin += cube*lum*masks.z*1.5;
        }
        if fam >= 7u { alpha = a.a; }
        if fam == 13u { alpha *= alpha; }
    }
    // Lamps and headlights on lit surfaces: not water or sky layers, and not
    // foliage (9, 10), so lamps light the street rather than tree crowns.
    // The tone pass grades night down to ~1% with a blue tint; lamp light is
    // pre-divided by the grade's brightness only, and its share of the pixel
    // goes out in alpha (opaque surfaces) so the tone pass leaves that part
    // untinted and in colour.
    var lamp_share = 0.0;
    if fam != 14u && fam != 30u && fam != 31u && fam != 32u && fam != 33u && fam != 9u && fam != 10u {
        let weights = vec3<f32>(0.2126,0.7152,0.0722);
        let grade = dot(pow(vec3<f32>(0.010,0.016,0.045), vec3<f32>(max(frame_state.clock.z, 0.0))), weights);
        var lamp = d*dynamic_lights(i.position, i.world_position.xyz, wn)/grade;
        // Hot cores go pale, as a bright lamp does; the pool stays warm.
        let shown = dot(lamp, weights)*grade*p.mode.w;
        lamp = mix(lamp, vec3<f32>(dot(lamp, weights)), 0.7*smoothstep(0.25, 1.2, shown));
        let base = dot(lin, weights);
        lamp_share = dot(lamp, weights)/max(base + dot(lamp, weights), 1e-6);
        lin += lamp;
    }
    var f = saturate(length(rpos)*p.fog_ramp.x+p.fog_ramp.y);
    if p.fog_ramp.z != 1.0 { f = pow(max(f,1e-6),p.fog_ramp.z); }
    var fog_a = 1.0+p.fog_color.a*f;
    if fam <= 8u || fam == 13u { fog_a *= p.surface.w; }
    var xe = max((lin*fog_a+p.fog_color.rgb*f)*p.mode.w,vec3<f32>(0.0));
    // Reduced curve is the full curve with the linear input capped at one.
    if fam == 8u { xe = min(xe,vec3<f32>(1.0)); }
    if p.foliage_debug.w != 0.0 { return vec4<f32>(p.foliage_debug.rgb, 1.0); }
    if a.a < p.mode.z { discard; }
    // Opaque surfaces carry 1 - lamp share in alpha for the tone pass.
    if alpha >= 1.0 && fam != 14u && fam != 32u { return vec4<f32>(xe, 1.0 - lamp_share); }
    return vec4<f32>(xe,alpha);
}
