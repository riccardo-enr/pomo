/*
 * Audio output for interval-end notifications. Holds a `rodio` device sink
 * for the lifetime of the process so individual `play_bell` calls can be
 * fire-and-forget (the player is detached and the sink keeps the audio
 * thread alive).
 *
 * Construction may fail when there is no audio device available; in that
 * case `AudioCtx::silent` returns a no-op context so callers can stay
 * branch-free.
 */

use rodio::{DeviceSinkBuilder, MixerDeviceSink};
use std::io::Cursor;

static BELL_WAV: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/bell.wav"));

pub struct AudioCtx {
    sink: Option<MixerDeviceSink>,
}

impl AudioCtx {
    pub fn new(enabled: bool) -> Self {
        if !enabled {
            return Self::silent();
        }
        match DeviceSinkBuilder::open_default_sink() {
            Ok(mut sink) => {
                sink.log_on_drop(false);
                Self { sink: Some(sink) }
            }
            Err(e) => {
                eprintln!("pomo: audio disabled ({}); continuing without sound", e);
                Self::silent()
            }
        }
    }

    pub fn silent() -> Self {
        Self { sink: None }
    }

    pub fn play_bell(&self) {
        let Some(sink) = self.sink.as_ref() else { return };
        match rodio::play(sink.mixer(), Cursor::new(BELL_WAV)) {
            Ok(player) => player.detach(),
            Err(e) => eprintln!("pomo: bell playback failed: {}", e),
        }
    }
}
