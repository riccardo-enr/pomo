/*
 * Generates a tiny mono WAV containing a ~0.4s 880 Hz sine "bell" with a
 * short fade-in and exponential decay. Written to $OUT_DIR/bell.wav so the
 * main binary can include_bytes! it without committing any audio asset.
 *
 * Format: PCM 16-bit, 22050 Hz, mono. ~17 KB.
 */

use std::f32::consts::TAU;
use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::PathBuf;

const SAMPLE_RATE: u32 = 22050;
const DURATION_S: f32 = 0.4;
const FREQ_HZ: f32 = 880.0;
const FADE_IN_S: f32 = 0.005;
const DECAY: f32 = 4.0;

fn main() {
    let out = PathBuf::from(std::env::var_os("OUT_DIR").unwrap()).join("bell.wav");
    let n_samples = (SAMPLE_RATE as f32 * DURATION_S) as u32;
    let bytes_per_sample = 2u32;
    let data_size = n_samples * bytes_per_sample;

    let f = File::create(&out).expect("open bell.wav");
    let mut w = BufWriter::new(f);

    // RIFF header
    w.write_all(b"RIFF").unwrap();
    w.write_all(&(36 + data_size).to_le_bytes()).unwrap();
    w.write_all(b"WAVE").unwrap();

    // fmt chunk
    w.write_all(b"fmt ").unwrap();
    w.write_all(&16u32.to_le_bytes()).unwrap();
    w.write_all(&1u16.to_le_bytes()).unwrap(); // PCM
    w.write_all(&1u16.to_le_bytes()).unwrap(); // mono
    w.write_all(&SAMPLE_RATE.to_le_bytes()).unwrap();
    w.write_all(&(SAMPLE_RATE * bytes_per_sample).to_le_bytes()).unwrap();
    w.write_all(&(bytes_per_sample as u16).to_le_bytes()).unwrap();
    w.write_all(&16u16.to_le_bytes()).unwrap(); // bits per sample

    // data chunk
    w.write_all(b"data").unwrap();
    w.write_all(&data_size.to_le_bytes()).unwrap();

    for i in 0..n_samples {
        let t = i as f32 / SAMPLE_RATE as f32;
        let fade_in = (t / FADE_IN_S).min(1.0);
        let decay = (-DECAY * t).exp();
        let env = fade_in * decay;
        let sample = (TAU * FREQ_HZ * t).sin() * env * 0.6;
        let s16 = (sample.clamp(-1.0, 1.0) * i16::MAX as f32) as i16;
        w.write_all(&s16.to_le_bytes()).unwrap();
    }

    w.flush().unwrap();
    println!("cargo:rerun-if-changed=build.rs");
}
