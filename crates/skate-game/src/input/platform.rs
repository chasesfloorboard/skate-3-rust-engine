//! Windows device transport. Raw signed axes/trigger bytes reach the TU3
//! converter without Bevy/gilrs deadzones or normalized-axis reconstruction.
use skate_core::input::xbox::XboxState;

pub(crate) struct DevicePacket {
    pub number: u32,
    pub state: XboxState,
    pub subtype: u8,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum DeviceError {
    Disconnected,
    State(u32),
    Capabilities(u32),
}

/// Device identity is metadata; raw input is still sampled every host frame.
/// Refresh periodically as well as after errors, so hot swaps cannot leave a
/// subtype cached indefinitely even if Windows never exposes a disconnect.
#[derive(Default)]
pub(crate) struct CapabilityCache {
    value: Option<(u8, std::time::Instant)>,
}
impl CapabilityCache {
    pub(crate) fn invalidate(&mut self) {
        self.value = None;
    }
    fn get(
        &mut self,
        now: std::time::Instant,
        read: impl FnOnce() -> Result<u8, DeviceError>,
    ) -> Result<u8, DeviceError> {
        if let Some((subtype, expires)) = self.value {
            if now < expires {
                return Ok(subtype);
            }
        }
        self.value = None;
        let subtype = read()?;
        self.value = Some((subtype, now + std::time::Duration::from_secs(1)));
        Ok(subtype)
    }
}

#[cfg(windows)]
mod windows {
    use super::*;
    use std::mem::MaybeUninit;

    // ABI from the installed Windows SDK Xinput.h. No OS-owned pointers are
    // retained and only successful calls permit reading output storage.
    #[repr(C)]
    struct Gamepad {
        buttons: u16,
        left_trigger: u8,
        right_trigger: u8,
        left_x: i16,
        left_y: i16,
        right_x: i16,
        right_y: i16,
    }
    #[repr(C)]
    struct State {
        number: u32,
        gamepad: Gamepad,
    }
    #[repr(C)]
    struct Vibration {
        left: u16,
        right: u16,
    }
    #[repr(C)]
    struct Capabilities {
        device_type: u8,
        subtype: u8,
        flags: u16,
        gamepad: Gamepad,
        vibration: Vibration,
    }
    const _: () = assert!(size_of::<Gamepad>() == 12);
    const _: () = assert!(size_of::<State>() == 16);
    const _: () = assert!(size_of::<Capabilities>() == 20);

    #[link(name = "xinput")]
    unsafe extern "system" {
        fn XInputGetState(index: u32, state: *mut State) -> u32;
        fn XInputGetCapabilities(index: u32, flags: u32, capabilities: *mut Capabilities) -> u32;
    }

    pub(super) fn poll(
        index: u32,
        cache: &mut CapabilityCache,
    ) -> Result<DevicePacket, DeviceError> {
        let mut state = MaybeUninit::<State>::uninit();
        // SAFETY: properly aligned writable storage with the SDK's exact C ABI.
        let result = unsafe { XInputGetState(index, state.as_mut_ptr()) };
        if result != 0 {
            cache.invalidate();
        }
        if result == 1167 {
            return Err(DeviceError::Disconnected);
        }
        if result != 0 {
            return Err(DeviceError::State(result));
        }
        let subtype = cache.get(std::time::Instant::now(), || {
            let mut capabilities = MaybeUninit::<Capabilities>::uninit();
            // SAFETY: writable storage with the SDK ABI; read only on success.
            let result = unsafe { XInputGetCapabilities(index, 1, capabilities.as_mut_ptr()) };
            if result != 0 {
                return Err(DeviceError::Capabilities(result));
            }
            Ok(unsafe { capabilities.assume_init() }.subtype)
        })?;
        // SAFETY: successful XInputGetState initialized the complete structure.
        let state = unsafe { state.assume_init() };
        Ok(DevicePacket {
            number: state.number,
            state: XboxState {
                buttons: state.gamepad.buttons,
                triggers: [state.gamepad.left_trigger, state.gamepad.right_trigger],
                left: [state.gamepad.left_x, state.gamepad.left_y],
                right: [state.gamepad.right_x, state.gamepad.right_y],
            },
            subtype,
        })
    }
}

/// Linux/macOS device transport via gilrs (evdev on Linux, IOKit on macOS).
/// Buttons are packed into the same XInput bit layout the rest of the engine
/// expects (see crates/skate-core/src/input/xbox.rs), so downstream code is
/// unchanged: bits 0-9 are dpad/start/back/thumbs/shoulders in XInput order,
/// bits 12-15 are A/B/X/Y in XInput order.
#[cfg(not(windows))]
mod gamepad {
    use super::*;
    use gilrs::{Axis, Button, Gilrs};
    use std::sync::{Mutex, OnceLock};

    /// `None` if the backend failed to start (e.g. no udev); every slot then
    /// reports disconnected instead of panicking the game.
    fn instance() -> Option<&'static Mutex<Gilrs>> {
        static INSTANCE: OnceLock<Option<Mutex<Gilrs>>> = OnceLock::new();
        INSTANCE
            .get_or_init(|| match Gilrs::new() {
                Ok(gilrs) => Some(Mutex::new(gilrs)),
                Err(error) => {
                    bevy::log::error!("gilrs gamepad backend unavailable: {error}");
                    None
                }
            })
            .as_ref()
    }

    fn axis_i16(value: f32) -> i16 {
        (value.clamp(-1.0, 1.0) * i16::MAX as f32) as i16
    }

    fn trigger_u8(value: f32) -> u8 {
        (value.clamp(0.0, 1.0) * u8::MAX as f32) as u8
    }

    pub(super) fn poll(index: u32, _cache: &mut CapabilityCache) -> Result<DevicePacket, DeviceError> {
        let Some(instance) = instance() else {
            return Err(DeviceError::Disconnected);
        };
        let mut gilrs = instance.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        // Drain the event queue so gilrs' cached gamepad state is current;
        // some backends (evdev) only update state as events are pumped.
        while gilrs.next_event().is_some() {}

        // gilrs also enumerates non-pad evdev nodes that expose a few buttons
        // or hat axes (e.g. a keyboard's "System Control" interface, which it
        // reports with a Driver mapping); require a left stick and a face
        // button so those cannot claim slot 0 ahead of a real pad.
        let Some((id, _)) = gilrs
            .gamepads()
            .filter(|(_, pad)| {
                pad.axis_code(Axis::LeftStickX).is_some() && pad.button_code(Button::South).is_some()
            })
            .nth(index as usize)
        else {
            return Err(DeviceError::Disconnected);
        };
        let pad = gilrs.gamepad(id);
        if !pad.is_connected() {
            return Err(DeviceError::Disconnected);
        }

        let mut buttons: u16 = 0;
        let mut set = |bit: u16, pressed: bool| {
            if pressed {
                buttons |= bit;
            }
        };
        // PlayStation pads (DualShock 4 / DualSense via hid-playstation) and
        // many others report the D-pad as a hat. gilrs normally turns that into
        // DPad buttons, but not for partially mapped pads, so read the hat
        // axes too (gilrs normalises them: +Y up, +X right).
        let (hat_x, hat_y) = (pad.value(Axis::DPadX), pad.value(Axis::DPadY));
        set(0x0001, pad.is_pressed(Button::DPadUp) || hat_y > 0.5);
        set(0x0002, pad.is_pressed(Button::DPadDown) || hat_y < -0.5);
        set(0x0004, pad.is_pressed(Button::DPadLeft) || hat_x < -0.5);
        set(0x0008, pad.is_pressed(Button::DPadRight) || hat_x > 0.5);
        set(0x0010, pad.is_pressed(Button::Start));
        set(0x0020, pad.is_pressed(Button::Select));
        set(0x0040, pad.is_pressed(Button::LeftThumb));
        set(0x0080, pad.is_pressed(Button::RightThumb));
        set(0x0100, pad.is_pressed(Button::LeftTrigger));
        set(0x0200, pad.is_pressed(Button::RightTrigger));
        set(0x1000, pad.is_pressed(Button::South));
        set(0x2000, pad.is_pressed(Button::East));
        set(0x4000, pad.is_pressed(Button::West));
        set(0x8000, pad.is_pressed(Button::North));

        // With an SDL mapping (standard for Xbox-style pads) analog triggers
        // are reported as LeftTrigger2/RightTrigger2 button values; unmapped
        // devices expose them as the raw Z axes instead.
        let trigger = |button: Button, axis: Axis| {
            let pressed = pad.button_data(button).map_or(0.0, |data| data.value());
            trigger_u8(pressed.max(pad.value(axis)))
        };
        let left_trigger = trigger(Button::LeftTrigger2, Axis::LeftZ);
        let right_trigger = trigger(Button::RightTrigger2, Axis::RightZ);
        let left = [
            axis_i16(pad.value(Axis::LeftStickX)),
            axis_i16(pad.value(Axis::LeftStickY)),
        ];
        let right = [
            axis_i16(pad.value(Axis::RightStickX)),
            axis_i16(pad.value(Axis::RightStickY)),
        ];

        Ok(DevicePacket {
            number: index,
            state: XboxState {
                buttons,
                triggers: [left_trigger, right_trigger],
                left,
                right,
            },
            // gilrs doesn't expose the XInput subtype byte; 1 = gamepad, the
            // only value the rest of the engine currently branches on.
            subtype: 1,
        })
    }
}

pub(crate) fn poll_cached(
    index: usize,
    cache: &mut CapabilityCache,
) -> Result<DevicePacket, DeviceError> {
    assert!(index < 4);
    #[cfg(windows)]
    return windows::poll(index as u32, cache);
    #[cfg(not(windows))]
    return gamepad::poll(index as u32, cache);
}

#[cfg(test)]
mod cache_tests {
    use super::*;
    #[test]
    fn capability_cache_refreshes_and_never_caches_errors() {
        let start = std::time::Instant::now();
        let mut cache = CapabilityCache::default();
        assert_eq!(cache.get(start, || Ok(1)), Ok(1));
        assert_eq!(
            cache.get(start + std::time::Duration::from_millis(999), || panic!(
                "redundant capability query"
            )),
            Ok(1)
        );
        assert_eq!(
            cache.get(start + std::time::Duration::from_secs(1), || Ok(2)),
            Ok(2)
        );
        cache.invalidate();
        assert_eq!(
            cache.get(start, || Err(DeviceError::Capabilities(5))),
            Err(DeviceError::Capabilities(5))
        );
        assert_eq!(cache.get(start, || Ok(3)), Ok(3));
        cache.invalidate();
        assert_eq!(cache.get(start, || Ok(4)), Ok(4));
    }
}

// Preserve the uncached API for menu-only polling.
pub(crate) fn poll(index: usize) -> Result<DevicePacket, DeviceError> {
    poll_cached(index, &mut CapabilityCache::default())
}
