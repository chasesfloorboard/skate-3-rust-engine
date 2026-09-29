//! Platform input adapter; no animation or physics state mutation here.
use crate::app::SimulationSet;
use bevy::prelude::*;

mod controllers;
pub(crate) mod gesture_catalog;
mod gesture_mapping_data;
pub(crate) mod gesture_mapping;
pub(crate) mod gesture_input;
pub(crate) mod platform;
pub(crate) use controllers::{ControllerInput, ControllerStatus};
use skate_core::input::tick::TickInput;

#[derive(Resource, Clone, Copy, Debug)]
pub(crate) struct PublishedTickInput(pub TickInput);

impl Default for PublishedTickInput {
    fn default() -> Self {
        Self(TickInput::new(
            0,
            skate_core::input::gameplay_map::GameplayActions::from_values([0.0; 18]),
            false,
        ))
    }
}

pub(crate) struct InputPlugin;
impl Plugin for InputPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<ControllerInput>()
            .init_resource::<PublishedTickInput>()
            .add_systems(PreUpdate, poll_controllers.run_if(crate::graphics_menu::gameplay_active))
            .add_systems(FixedUpdate, publish_actions.in_set(SimulationSet::Input))
            .add_systems(Update, cursor_follows_device);
    }
}

/// Hides the mouse cursor once the controller is in use and brings it back
/// as soon as the mouse moves. Only changes in pad state count, so a stick
/// resting off-centre cannot keep hiding it.
fn cursor_follows_device(
    input: Res<ControllerInput>,
    mouse: Res<bevy::input::mouse::AccumulatedMouseMotion>,
    mut cursors: Query<&mut bevy::window::CursorOptions, With<bevy::window::PrimaryWindow>>,
    mut last: Local<controllers::RawInput>,
) {
    let pad = input.raw_input();
    let moved = |a: [f32; 2], b: [f32; 2]| (a[0] - b[0]).abs() + (a[1] - b[1]).abs() > 0.2;
    let pad_used = pad.buttons != last.buttons || moved(pad.left, last.left) || moved(pad.right, last.right)
        || moved(pad.triggers, last.triggers);
    if pad_used { *last = pad; }
    let visible = if mouse.delta.length_squared() > 1.0 { true } else if pad_used { false } else { return };
    for mut cursor in &mut cursors {
        if cursor.visible != visible { cursor.visible = visible; }
    }
}

fn copy(s: &skate_core::input::xbox::XboxState) -> skate_core::input::xbox::XboxState {
    skate_core::input::xbox::XboxState { buttons: s.buttons, triggers: s.triggers, left: s.left, right: s.right }
}

/// Test hook: SKATE_DEBUG_INPUT=script plays pad 0 from a file of lines
/// `seconds lx ly rx ry lt rt buttons` (sticks -1..1, triggers 0..1, buttons
/// XInput bits in hex), each held until the next line's time.
fn scripted_pad(seconds: f32, packet: &mut u32, last: &mut Option<usize>) -> Option<platform::DevicePacket> {
    static SCRIPT: std::sync::OnceLock<Option<Vec<(f32, skate_core::input::xbox::XboxState)>>> = std::sync::OnceLock::new();
    let script = SCRIPT.get_or_init(|| {
        let text = std::fs::read_to_string(std::env::var_os("SKATE_DEBUG_INPUT")?).ok()?;
        let stick = |v: f32| (v.clamp(-1.0, 1.0) * 32767.0) as i16;
        let trigger = |v: f32| (v.clamp(0.0, 1.0) * 255.0) as u8;
        Some(text.lines().filter_map(|line| {
            let f: Vec<&str> = line.split_whitespace().collect();
            if f.len() < 8 || f[0].starts_with('#') { return None; }
            let n = |i: usize| f[i].parse::<f32>().ok();
            Some((n(0)?, skate_core::input::xbox::XboxState {
                left: [stick(n(1)?), stick(n(2)?)], right: [stick(n(3)?), stick(n(4)?)],
                triggers: [trigger(n(5)?), trigger(n(6)?)],
                buttons: u16::from_str_radix(f[7].trim_start_matches("0x"), 16).ok()?,
            }))
        }).collect())
    }).as_ref()?;
    let index = script.iter().rposition(|(at, _)| *at <= seconds)?;
    if *last != Some(index) { *last = Some(index); *packet += 1; }
    Some(platform::DevicePacket { number: *packet, state: copy(&script[index].1), subtype: 1 })
}

pub(crate) fn poll_controllers(mut input: ResMut<ControllerInput>,config:Res<crate::config::Config>,net:Option<Res<crate::multiplayer::Multiplayer>>,windows:Query<&Window>,mut capabilities:Local<[platform::CapabilityCache;4]>,
    time: Res<Time<Real>>, mut script: Local<(u32, Option<usize>)>) {
    let previous = input.status;
    let focused=windows.iter().any(|w|w.focused);
    let active=net.is_some_and(|n|n.active());
    let (packet, last) = &mut *script;
    let scripted = scripted_pad(time.elapsed_secs(), packet, last);
    input.collect(std::array::from_fn(|slot| {
        if slot == 0 && let Some(pad) = &scripted { return Ok(platform::DevicePacket { number: pad.number, state: copy(&pad.state), subtype: pad.subtype }); }
        if active && ((!focused && config.multiplayer.controller.is_none()) || config.multiplayer.controller.is_some_and(|selected|selected as usize!=slot)) {
            capabilities[slot].invalidate();
            Err(platform::DeviceError::Disconnected)
        } else {platform::poll_cached(slot, &mut capabilities[slot])}
    }));
    for (index, (&before, &after)) in previous.iter().zip(&input.status).enumerate() {
        if before != after {
            match after {
                ControllerStatus::Ready => info!("Controller {index}: raw XInput ready"),
                ControllerStatus::Unavailable(platform::DeviceError::Disconnected) => {
                    info!("Controller {index}: disconnected");
                }
                _ => warn!("Controller {index}: {after:?}"),
            }
        }
    }
}

fn publish_actions(
    mut input: ResMut<ControllerInput>,
    mut published: ResMut<PublishedTickInput>,
    menu:Option<Res<crate::graphics_menu::Menu>>,
) {
    if !crate::graphics_menu::gameplay_active(menu) {input.discard_gameplay();}
    input.publish_actions();
    published.0 = input.tick_input();
}

#[cfg(test)]
pub(crate) mod manual_replay;
