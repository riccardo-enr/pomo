mod audio;
mod config;
mod journal;
mod state;
mod tui;

use anyhow::Result;
use clap::{Parser, Subcommand};
use std::time::{Duration, SystemTime};

#[derive(Parser)]
#[command(name = "pomo", about = "Pomodoro timer and generic countdowns", version)]
struct Cli {
    #[command(subcommand)]
    cmd: Option<Cmd>,

    /// Free-form duration, e.g. 25m, 1h30m, 45s. Used when no subcommand is given.
    duration: Option<String>,

    /// Suppress the audible bell on interval completion (overrides config).
    #[arg(long, global = true)]
    no_sound: bool,
}

#[derive(Subcommand)]
enum Cmd {
    /// Run a single work interval (config: `work`)
    Work { duration: Option<String> },
    /// Short break (config: `short_break`)
    Break { duration: Option<String> },
    /// Long break (config: `long_break`)
    Long { duration: Option<String> },
    /// Full pomodoro cycle: N work intervals separated by short breaks, ending with a long break
    Cycle {
        /// Number of work intervals before the long break (config: `rounds`)
        #[arg(short = 'n', long)]
        rounds: Option<u32>,
        #[arg(long)]
        work: Option<String>,
        #[arg(long)]
        short: Option<String>,
        #[arg(long)]
        long: Option<String>,
    },
    /// Generic timer
    Timer { duration: String },
    /// Emit the current pomo session state (use --json for waybar integration)
    Status {
        /// Emit waybar-compatible JSON
        #[arg(long)]
        json: bool,
    },
}

fn parse_dur(s: &str) -> Result<Duration> {
    Ok(humantime::parse_duration(s)?)
}

fn resolve_dur(cli_value: Option<String>, config_value: Duration) -> Result<Duration> {
    match cli_value {
        Some(s) => parse_dur(&s),
        None => Ok(config_value),
    }
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    // status runs without loading config, audio, or any other heavy setup.
    if let Some(Cmd::Status { json }) = &cli.cmd {
        let out = state::read_for_status();
        if *json {
            println!("{}", serde_json::to_string(&out).unwrap());
        } else if out.class == "idle" {
            println!("idle");
        } else {
            println!("{} ({})", out.text, out.tooltip);
        }
        return Ok(());
    }

    let cfg = config::Config::load()?;

    let sound_enabled = cfg.sound && !cli.no_sound;
    let audio = audio::AudioCtx::new(sound_enabled, cfg.sound_path.as_deref(), cfg.bell_volume);
    let notifier = Notifier { enabled: cfg.desktop_notification };
    let journal = journal::Journal::new();
    let mut state_file = state::StateFile::new();

    let result = match cli.cmd {
        Some(Cmd::Work { duration }) => {
            let d = resolve_dur(duration, cfg.work)?;
            run_one(&audio, &notifier, &journal, &mut state_file, "Work", d, Kind::Work).map(|_| ())
        }
        Some(Cmd::Break { duration }) => {
            let d = resolve_dur(duration, cfg.short_break)?;
            run_one(&audio, &notifier, &journal, &mut state_file, "Short break", d, Kind::Break).map(|_| ())
        }
        Some(Cmd::Long { duration }) => {
            let d = resolve_dur(duration, cfg.long_break)?;
            run_one(&audio, &notifier, &journal, &mut state_file, "Long break", d, Kind::Break).map(|_| ())
        }
        Some(Cmd::Timer { duration }) => {
            run_one(&audio, &notifier, &journal, &mut state_file, "Timer", parse_dur(&duration)?, Kind::Timer).map(|_| ())
        }
        Some(Cmd::Cycle { rounds, work, short, long }) => {
            let n = rounds.unwrap_or(cfg.rounds);
            let w = resolve_dur(work, cfg.work)?;
            let s = resolve_dur(short, cfg.short_break)?;
            let l = resolve_dur(long, cfg.long_break)?;
            run_cycle(n, w, s, l, |label, dur, kind| {
                run_one(&audio, &notifier, &journal, &mut state_file, label, dur, kind)
            })
            .map(|_| ())
        }
        None => {
            let dur = cli
                .duration
                .ok_or_else(|| anyhow::anyhow!("provide a duration, e.g. `pomo 25m`, or a subcommand"))?;
            run_one(&audio, &notifier, &journal, &mut state_file, "Timer", parse_dur(&dur)?, Kind::Timer).map(|_| ())
        }
        Some(Cmd::Status { .. }) => unreachable!("handled above"),
    };

    // Give the detached bell time to drain before the audio device is torn down.
    std::thread::sleep(Duration::from_millis(500));
    result
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Work,
    Break,
    Timer,
}

struct Notifier {
    enabled: bool,
}

impl Notifier {
    fn notify(&self, label: &str, kind: Kind) {
        if !self.enabled {
            return;
        }
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
}

fn run_one(
    audio: &audio::AudioCtx,
    notifier: &Notifier,
    journal: &journal::Journal,
    state_file: &mut state::StateFile,
    label: &str,
    dur: Duration,
    kind: Kind,
) -> Result<tui::Outcome> {
    let started = SystemTime::now();
    let outcome = tui::run(label, dur, kind, state_file)?;
    let ended = SystemTime::now();
    let completed = outcome == tui::Outcome::Finished;
    journal.record(kind, label, started, ended, dur, completed);
    if completed {
        audio.play_bell();
        notifier.notify(label, kind);
    }
    Ok(outcome)
}

/*
 * Pure pomodoro-cycle scheduler. Calls `runner` once per interval in the
 * usual order (Work 1/N, Short break, Work 2/N, ..., Long break) and stops
 * on the first `Outcome::Aborted`, including during the trailing long break.
 *
 * Kept free of audio/journal/TUI concerns so the abort-cancels-everything
 * behavior can be unit-tested without a terminal.
 */
fn run_cycle<F>(
    rounds: u32,
    work: Duration,
    short_break: Duration,
    long_break: Duration,
    mut runner: F,
) -> Result<tui::Outcome>
where
    F: FnMut(&str, Duration, Kind) -> Result<tui::Outcome>,
{
    for r in 1..=rounds {
        let out = runner(&format!("Work {}/{}", r, rounds), work, Kind::Work)?;
        if out == tui::Outcome::Aborted {
            return Ok(tui::Outcome::Aborted);
        }
        if r < rounds {
            let out = runner("Short break", short_break, Kind::Break)?;
            if out == tui::Outcome::Aborted {
                return Ok(tui::Outcome::Aborted);
            }
        }
    }
    runner("Long break", long_break, Kind::Break)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tui::Outcome;

    fn d(s: u64) -> Duration {
        Duration::from_secs(s)
    }

    #[test]
    fn full_cycle_runs_all_intervals_in_order() {
        let mut calls: Vec<(String, Kind)> = Vec::new();
        let result = run_cycle(2, d(25), d(5), d(15), |label, _, kind| {
            calls.push((label.to_string(), kind));
            Ok(Outcome::Finished)
        })
        .unwrap();
        assert_eq!(result, Outcome::Finished);
        assert_eq!(
            calls,
            vec![
                ("Work 1/2".to_string(), Kind::Work),
                ("Short break".to_string(), Kind::Break),
                ("Work 2/2".to_string(), Kind::Work),
                ("Long break".to_string(), Kind::Break),
            ]
        );
    }

    #[test]
    fn abort_during_first_work_stops_cycle() {
        let mut calls: Vec<String> = Vec::new();
        let result = run_cycle(3, d(25), d(5), d(15), |label, _, _| {
            calls.push(label.to_string());
            Ok(Outcome::Aborted)
        })
        .unwrap();
        assert_eq!(result, Outcome::Aborted);
        assert_eq!(calls, vec!["Work 1/3"]);
    }

    #[test]
    fn abort_during_short_break_stops_cycle() {
        let mut calls: Vec<String> = Vec::new();
        let result = run_cycle(3, d(25), d(5), d(15), |label, _, _| {
            calls.push(label.to_string());
            if label == "Short break" {
                Ok(Outcome::Aborted)
            } else {
                Ok(Outcome::Finished)
            }
        })
        .unwrap();
        assert_eq!(result, Outcome::Aborted);
        assert_eq!(calls, vec!["Work 1/3", "Short break"]);
    }

    #[test]
    fn abort_during_long_break_returns_aborted() {
        let mut calls: Vec<String> = Vec::new();
        let result = run_cycle(1, d(25), d(5), d(15), |label, _, _| {
            calls.push(label.to_string());
            if label == "Long break" {
                Ok(Outcome::Aborted)
            } else {
                Ok(Outcome::Finished)
            }
        })
        .unwrap();
        assert_eq!(result, Outcome::Aborted);
        assert_eq!(calls, vec!["Work 1/1", "Long break"]);
    }

    #[test]
    fn single_round_skips_short_break() {
        let mut calls: Vec<String> = Vec::new();
        let _ = run_cycle(1, d(25), d(5), d(15), |label, _, _| {
            calls.push(label.to_string());
            Ok(Outcome::Finished)
        })
        .unwrap();
        assert_eq!(calls, vec!["Work 1/1", "Long break"]);
    }
}
