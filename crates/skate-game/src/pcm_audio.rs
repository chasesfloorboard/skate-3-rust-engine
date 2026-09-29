//! Clips decoded once to samples in memory, so they can start anywhere
//! instantly. Bevy's AudioSource makes a fresh decoder per play, and starting
//! partway into it (PlaybackSettings::start_position) decodes and throws away
//! everything before that point on the main thread: the wheel spin-down,
//! started up to ten seconds in on every landing, stalled a frame ~16 ms.
use bevy::audio::{AddAudioSource, Decodable, Source};
use bevy::prelude::*;
use std::sync::{Arc, Mutex};
use std::time::Duration;

pub(crate) struct PcmAudioPlugin;
impl Plugin for PcmAudioPlugin {
    fn build(&self, app: &mut App) {
        app.add_audio_source::<PcmClip>();
    }
}

/// Interleaved 16-bit samples of a whole clip (the discs' own precision;
/// half the memory of floats).
pub(crate) struct Pcm {
    samples: Arc<[i16]>,
    channels: u16,
    rate: u32,
}

/// Plays a decoded clip from `start` samples in.
#[derive(Asset, TypePath, Clone)]
pub(crate) struct PcmClip {
    samples: Arc<[i16]>,
    channels: u16,
    rate: u32,
    start: usize,
}
impl Pcm {
    /// A clip starting `offset` into the audio (clamped to its end).
    pub(crate) fn from(&self, offset: Duration) -> PcmClip {
        let frame = (offset.as_secs_f64() * f64::from(self.rate)) as usize;
        let start = (frame * self.channels as usize).min(self.samples.len());
        PcmClip { samples: self.samples.clone(), channels: self.channels, rate: self.rate, start }
    }
}

pub(crate) struct PcmSource {
    samples: Arc<[i16]>,
    channels: u16,
    rate: u32,
    at: usize,
}
impl Iterator for PcmSource {
    type Item = i16;
    fn next(&mut self) -> Option<i16> {
        let sample = self.samples.get(self.at).copied();
        self.at += 1;
        sample
    }
}
impl Source for PcmSource {
    fn current_frame_len(&self) -> Option<usize> {
        None
    }
    fn channels(&self) -> u16 {
        self.channels
    }
    fn sample_rate(&self) -> u32 {
        self.rate
    }
    fn total_duration(&self) -> Option<Duration> {
        None
    }
}
impl Decodable for PcmClip {
    type DecoderItem = i16;
    type Decoder = PcmSource;
    fn decoder(&self) -> PcmSource {
        PcmSource { samples: self.samples.clone(), channels: self.channels, rate: self.rate, at: self.start }
    }
}

/// A clip being decoded: filled once done, left empty if the file cannot be
/// decoded (callers then play the compressed clip).
pub(crate) type Slot = Arc<Mutex<Option<Arc<Pcm>>>>;

/// Decodes `paths` one after another on a background thread.
pub(crate) fn decode_in_background(paths: Vec<std::path::PathBuf>) -> Vec<Slot> {
    let slots: Vec<Slot> = paths.iter().map(|_| Arc::new(Mutex::new(None))).collect();
    let filled = slots.clone();
    std::thread::spawn(move || {
        for (path, slot) in paths.into_iter().zip(filled) {
            let Ok(bytes) = std::fs::read(&path) else { continue };
            let Ok(decoder) = rodio::Decoder::new(std::io::Cursor::new(bytes)) else { continue };
            let (channels, rate) = (decoder.channels(), decoder.sample_rate());
            let samples: Arc<[i16]> = decoder.collect();
            *slot.lock().unwrap() = Some(Arc::new(Pcm { samples, channels, rate }));
        }
    });
    slots
}

/// A slot's clip once decoded.
pub(crate) fn ready(slot: &Slot) -> Option<Arc<Pcm>> {
    slot.lock().ok()?.clone()
}
