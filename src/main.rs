mod audio;
mod tui;

use anyhow::Result;
use clap::{Parser, Subcommand};
use std::time::Duration;

#[derive(Parser)]
#[command(name = "pomo", about = "Pomodoro timer and generic countdowns", version)]
struct Cli {
    #[command(subcommand)]
    cmd: Option<Cmd>,

    /// Free-form duration, e.g. 25m, 1h30m, 45s. Used when no subcommand is given.
    duration: Option<String>,

    /// Suppress the audible bell on interval completion.
    #[arg(long, global = true)]
    no_sound: bool,
}

#[derive(Subcommand)]
enum Cmd {
    /// Run a single work interval (default 25m)
    Work {
        #[arg(default_value = "25m")]
        duration: String,
    },
    /// Short break (default 5m)
    Break {
        #[arg(default_value = "5m")]
        duration: String,
    },
    /// Long break (default 15m)
    Long {
        #[arg(default_value = "15m")]
        duration: String,
    },
    /// Full pomodoro cycle: N work intervals separated by short breaks, ending with a long break
    Cycle {
        /// Number of work intervals before the long break
        #[arg(short = 'n', long, default_value_t = 4)]
        rounds: u32,
        #[arg(long, default_value = "25m")]
        work: String,
        #[arg(long, default_value = "5m")]
        short: String,
        #[arg(long, default_value = "15m")]
        long: String,
    },
    /// Generic timer
    Timer { duration: String },
}

fn parse_dur(s: &str) -> Result<Duration> {
    Ok(humantime::parse_duration(s)?)
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let audio = audio::AudioCtx::new(!cli.no_sound);
    let result = match cli.cmd {
        Some(Cmd::Work { duration }) => run_one(&audio, "Work", parse_dur(&duration)?, Kind::Work),
        Some(Cmd::Break { duration }) => run_one(&audio, "Short break", parse_dur(&duration)?, Kind::Break),
        Some(Cmd::Long { duration }) => run_one(&audio, "Long break", parse_dur(&duration)?, Kind::Break),
        Some(Cmd::Timer { duration }) => run_one(&audio, "Timer", parse_dur(&duration)?, Kind::Timer),
        Some(Cmd::Cycle { rounds, work, short, long }) => {
            let w = parse_dur(&work)?;
            let s = parse_dur(&short)?;
            let l = parse_dur(&long)?;
            let mut out = Ok(());
            for r in 1..=rounds {
                out = run_one(&audio, &format!("Work {}/{}", r, rounds), w, Kind::Work);
                if out.is_err() {
                    break;
                }
                if r < rounds {
                    out = run_one(&audio, "Short break", s, Kind::Break);
                    if out.is_err() {
                        break;
                    }
                }
            }
            out.and_then(|_| run_one(&audio, "Long break", l, Kind::Break))
        }
        None => {
            let dur = cli
                .duration
                .ok_or_else(|| anyhow::anyhow!("provide a duration, e.g. `pomo 25m`, or a subcommand"))?;
            run_one(&audio, "Timer", parse_dur(&dur)?, Kind::Timer)
        }
    };

    // Give the detached bell time to drain before the audio device is torn down.
    std::thread::sleep(Duration::from_millis(500));
    result
}

#[derive(Clone, Copy)]
pub enum Kind {
    Work,
    Break,
    Timer,
}

fn run_one(audio: &audio::AudioCtx, label: &str, dur: Duration, kind: Kind) -> Result<()> {
    let outcome = tui::run(label, dur, kind)?;
    if outcome == tui::Outcome::Finished {
        audio.play_bell();
        notify(label, kind);
    }
    Ok(())
}

fn notify(label: &str, kind: Kind) {
    let body = match kind {
        Kind::Work => "Time's up - take a break.",
        Kind::Break => "Break over - back to work.",
        Kind::Timer => "Timer finished.",
    };
    let _ = notify_rust::Notification::new()
        .summary(&format!("pomo: {}", label))
        .body(body)
        .timeout(notify_rust::Timeout::Milliseconds(5000))
        .show();
}
