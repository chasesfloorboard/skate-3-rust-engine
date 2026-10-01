//! Day/night on retail districts. Their lighting is baked, so the cycle grades
//! the final image in the tone pass (blue night, warm dusk; see
//! retail_tone.wgsl) and moves the sun used by prop lighting and the sky's
//! sun disc. Custom maps' real sun light is still driven by map_render::advance_day.
use bevy::prelude::*;
use std::collections::HashMap;

/// Full night; the tone pass keeps a moonlit floor so it never goes black.
const MAX_NIGHT: f32 = 1.0;

/// Sun height in -1..1 for an hour of the day (noon 1, midnight -1).
fn elevation(hour: f32) -> f32 {
    ((hour - 6.0) / 12.0 * std::f32::consts::PI).sin()
}

/// Real seconds each phase (day -> sunset, sunset -> night and back) takes.
const PHASE_SECONDS: f32 = 30.0;

/// Game hours one phase spans at a cycle speed. Slow speeds keep a natural
/// sunset length; very fast ones are capped so full day and night still show.
fn phase_hours(day_speed: u32) -> f32 {
    (PHASE_SECONDS * day_speed as f32 / 3600.0).clamp(0.75, 4.0)
}

fn smooth(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// (night amount, dusk warmth) for a sun height; `width` is the phase length
/// in game hours.
pub(crate) fn tint(elevation: f32, width: f32) -> (f32, f32) {
    // Signed game hours from the nearest sunrise/sunset (positive = sun up).
    let hours = elevation.clamp(-1.0, 1.0).asin() * 12.0 / std::f32::consts::PI;
    // Warmth builds over one phase before the horizon and fades over the next
    // while night falls, so sunset still reads as sunset.
    let dusk = smooth(1.0 - hours.abs() / width);
    let night = smooth(-hours / width);
    (night * MAX_NIGHT, dusk)
}

pub(crate) fn advance(
    time: Res<Time<Virtual>>,
    mut menu: ResMut<crate::graphics_menu::Menu>,
    custom: Query<(), With<crate::map_render::DayEnvironment>>,
    mut state: ResMut<crate::retail_render::ShadowState>,
    mut skies: ResMut<Assets<crate::retail_render::RetailSkyMaterial>>,
    mut authored: Local<HashMap<AssetId<crate::retail_render::RetailSkyMaterial>, Vec3>>,
) {
    // map_render::advance_day already advanced the clock when a map has a
    // DayEnvironment light (retail districts spawn one too); only read it then.
    // The tint only affects retail world materials, so applying it always is safe.
    let hour = menu.advance_day(if custom.is_empty() { time.delta_secs() } else { 0.0 });
    // Test hook: pin the hour without touching the player's saved settings.
    let hour = std::env::var("SKATE_DEBUG_HOUR").ok().and_then(|h| h.parse().ok()).unwrap_or(hour);
    let e = elevation(hour);
    let (night, dusk) = tint(e, phase_hours(menu.day_speed()));
    state.1.z = night * menu.night_depth();
    state.1.w = dusk;
    if std::env::var("SKATE_DEBUG_DAY").as_deref() == Ok("1") && (time.elapsed_secs() % 2.0) < time.delta_secs() {
        info!("DAY_DEBUG hour={hour:.2} night={night:.2} dusk={dusk:.2} custom={}", !custom.is_empty());
    }
    let ids: Vec<_> = skies.ids().collect();
    authored.retain(|id, _| skies.contains(*id));
    let mut sun = None;
    for id in ids {
        let Some(sky) = skies.get(id) else { continue };
        let original = *authored.entry(id).or_insert(sky.params.sun_direction.truncate());
        // Keep the district's authored compass direction; only the height moves.
        let horizontal = Vec3::new(original.x, 0.0, original.z).normalize_or(Vec3::X);
        let angle = (hour - 6.0) / 12.0 * std::f32::consts::PI;
        let direction = (horizontal * angle.cos() + Vec3::Y * angle.sin()).normalize();
        sun = Some(direction);
        if let Some(sky) = skies.get_mut(id) {
            sky.params.sun_direction = direction.extend(0.0);
            let (_, rgb, desaturate) = crate::retail_render::SKY_PRESETS[menu.sky_preset()];
            sky.params.tint = Vec3::from(rgb).extend(desaturate);
            // Day -> sunset -> stars -> sunset -> day follows the same curves
            // as the tone grade.
            // Stars only come out in the second half of the fade to night.
            let stars = ((night / MAX_NIGHT - 0.45) / 0.55).clamp(0.0, 1.0);
            let stars = stars * stars * (3.0 - 2.0 * stars);
            sky.params.cycle = Vec4::new(dusk * (1.0 - stars), stars, time.elapsed_secs_wrapped(), night);
        }
    }
    state.3 = sun.map_or(Vec4::ZERO, |d| d.extend(1.0));
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn noon_is_day_midnight_is_night_and_dusk_warms() {
        let w = phase_hours(120);
        assert!(tint(elevation(12.0), w).0 < 0.01);
        assert!((tint(elevation(0.0), w).0 - MAX_NIGHT).abs() < 0.01);
        assert!(tint(elevation(18.0), w).1 > 0.99);
        assert!(tint(elevation(12.0), w).1 < 0.01);
    }
    #[test]
    fn phases_last_thirty_seconds_at_120x() {
        // 120x: one game hour is 30 real seconds.
        let w = phase_hours(120);
        assert!((w - 1.0).abs() < 1e-4);
        assert!(tint(elevation(17.0), w).1 < 0.01);
        assert!(tint(elevation(17.5), w).1 > 0.4);
        assert!(tint(elevation(18.5), w).0 > 0.4 && tint(elevation(18.5), w).0 < 0.6);
        assert!(tint(elevation(19.0), w).0 > 0.99);
    }
}
