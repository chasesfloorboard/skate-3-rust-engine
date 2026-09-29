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

pub(crate) fn poll_controllers(mut input: ResMut<ControllerInput>,config:Res<crate::config::Config>,net:Option<Res<crate::multiplayer::Multiplayer>>,windows:Query<&Window>,mut capabilities:Local<[platform::CapabilityCache;4]>) {
    let previous = input.status;
    let focused=windows.iter().any(|w|w.focused);
    let active=net.is_some_and(|n|n.active());
    input.collect(std::array::from_fn(|slot| {
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
