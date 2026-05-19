/*
 * Audio output for interval-end notifications. Holds a `rodio` device sink
 * for the lifetime of the process so individual `play_bell` calls can be
 * fire-and-forget (the player is detached and the sink keeps the audio
 * thread alive).
 *
 * Construction may fail when there is no audio device available; in that
 * case `AudioCtx::silent` returns a no-op context so callers can stay
 * branch-free.
 *
 * The bell sound defaults to a build-time-synthesized sine in `bell.wav`,
 * but a user-provided WAV (config `sound_path`) takes precedence when
 * loadable.
 */

use rodio::{DeviceSinkBuilder, MixerDeviceSink};
use std::io::Cursor;
use std::path::Path;

static BUNDLED_BELL: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/bell.wav"));

pub struct AudioCtx {
    sink: Option<MixerDeviceSink>,
    bell_bytes: &'static [u8],
    user_bell: Option<Vec<u8>>,
    volume: f32,
}

impl AudioCtx {
    pub fn new(enabled: bool, sound_path: Option<&Path>, volume: f32) -> Self {
        if !enabled {
            return Self::silent();
        }

        let user_bell = sound_path.and_then(|p| match std::fs::read(p) {
            Ok(bytes) => Some(bytes),
            Err(e) => {
                eprintln!(
                    "pomo: could not read sound_path {} ({}); using bundled bell",
                    p.display(),
                    e
                );
                None
            }
        });

        match DeviceSinkBuilder::open_default_sink() {
            Ok(mut sink) => {
                sink.log_on_drop(false);
                Self {
                    sink: Some(sink),
                    bell_bytes: BUNDLED_BELL,
                    user_bell,
                    volume: volume.clamp(0.0, 4.0),
                }
            }
            Err(e) => {
                eprintln!("pomo: audio disabled ({}); continuing without sound", e);
                Self::silent()
            }
        }
    }

    pub fn silent() -> Self {
        Self {
            sink: None,
            bell_bytes: BUNDLED_BELL,
            user_bell: None,
            volume: 1.0,
        }
    }

    pub fn play_bell(&self) {
        let Some(sink) = self.sink.as_ref() else { return };
        let bytes: &[u8] = self
            .user_bell
            .as_deref()
            .unwrap_or(self.bell_bytes);
        match rodio::play(sink.mixer(), Cursor::new(bytes.to_vec())) {
            Ok(player) => {
                player.set_volume(self.volume);
                player.detach();
            }
            Err(e) => eprintln!("pomo: bell playback failed: {}", e),
        }
    }
}
