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
            run_one(&audio, &notifier, &journal, &mut state_file, "Work", d, Kind::Work)
        }
        Some(Cmd::Break { duration }) => {
            let d = resolve_dur(duration, cfg.short_break)?;
            run_one(&audio, &notifier, &journal, &mut state_file, "Short break", d, Kind::Break)
        }
        Some(Cmd::Long { duration }) => {
            let d = resolve_dur(duration, cfg.long_break)?;
            run_one(&audio, &notifier, &journal, &mut state_file, "Long break", d, Kind::Break)
        }
        Some(Cmd::Timer { duration }) => {
            run_one(&audio, &notifier, &journal, &mut state_file, "Timer", parse_dur(&duration)?, Kind::Timer)
        }
        Some(Cmd::Cycle { rounds, work, short, long }) => {
            let n = rounds.unwrap_or(cfg.rounds);
            let w = resolve_dur(work, cfg.work)?;
            let s = resolve_dur(short, cfg.short_break)?;
            let l = resolve_dur(long, cfg.long_break)?;
            let mut out: Result<()> = Ok(());
            for r in 1..=n {
                out = run_one(&audio, &notifier, &journal, &mut state_file, &format!("Work {}/{}", r, n), w, Kind::Work);
                if out.is_err() {
                    break;
                }
                if r < n {
                    out = run_one(&audio, &notifier, &journal, &mut state_file, "Short break", s, Kind::Break);
                    if out.is_err() {
                        break;
                    }
                }
            }
            if out.is_ok() {
                run_one(&audio, &notifier, &journal, &mut state_file, "Long break", l, Kind::Break)
            } else {
                out
            }
        }
        None => {
            let dur = cli
                .duration
                .ok_or_else(|| anyhow::anyhow!("provide a duration, e.g. `pomo 25m`, or a subcommand"))?;
            run_one(&audio, &notifier, &journal, &mut state_file, "Timer", parse_dur(&dur)?, Kind::Timer)
        }
        Some(Cmd::Status { .. }) => unreachable!("handled above"),
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
) -> Result<()> {
    let started = SystemTime::now();
    let outcome = tui::run(label, dur, kind, state_file)?;
    let ended = SystemTime::now();
    let completed = outcome == tui::Outcome::Finished;
    journal.record(kind, label, started, ended, dur, completed);
    if completed {
        audio.play_bell();
        notifier.notify(label, kind);
    }
    Ok(())
}
