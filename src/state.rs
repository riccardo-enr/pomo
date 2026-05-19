/*
 * Live runtime state for an active pomo session. The TUI writes this file
 * (atomically, ~1 Hz) so external observers like waybar can poll it via
 * `pomo status --json`.
 *
 * Path: `$XDG_RUNTIME_DIR/pomo/state.json`, falling back to
 * `/tmp/pomo-<euid>/state.json` if the runtime directory is not advertised.
 *
 * `StateFile` removes the file on drop so clean exits (and panic unwinds)
 * leave nothing behind. SIGKILL is not handled, but `read_for_status`
 * treats any file older than `STALE_AFTER` as idle.
 */

use crate::Kind;
use etcetera::{choose_app_strategy, AppStrategy, AppStrategyArgs};
use serde::{Deserialize, Serialize};
use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

const STALE_AFTER: Duration = Duration::from_secs(5);

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct State {
    pub label: String,
    pub kind: String,
    pub remaining_secs: u64,
    pub total_secs: u64,
    pub paused: bool,
    pub updated_at: String,
}

pub struct StateFile {
    path: Option<PathBuf>,
    last_remaining: Option<u64>,
    last_paused: Option<bool>,
}

impl StateFile {
    pub fn new() -> Self {
        Self {
            path: default_state_path(),
            last_remaining: None,
            last_paused: None,
        }
    }

    pub fn write(
        &mut self,
        label: &str,
        kind: Kind,
        remaining: Duration,
        total: Duration,
        paused: bool,
    ) {
        let Some(path) = self.path.as_ref() else { return };
        let secs = remaining.as_secs();
        if self.last_remaining == Some(secs) && self.last_paused == Some(paused) {
            return;
        }
        self.last_remaining = Some(secs);
        self.last_paused = Some(paused);

        let state = State {
            label: label.to_string(),
            kind: kind_str(kind).to_string(),
            remaining_secs: secs,
            total_secs: total.as_secs(),
            paused,
            updated_at: humantime::format_rfc3339_seconds(SystemTime::now()).to_string(),
        };
        if let Err(e) = atomic_write(path, &state) {
            let _ = e; // best-effort; no warning spam from the TUI
        }
    }
}

impl Drop for StateFile {
    fn drop(&mut self) {
        if let Some(p) = self.path.as_ref() {
            let _ = std::fs::remove_file(p);
        }
    }
}

fn atomic_write(path: &Path, state: &State) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        if !parent.exists() {
            std::fs::create_dir_all(parent)?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let _ = std::fs::set_permissions(parent, std::fs::Permissions::from_mode(0o700));
            }
        }
    }
    let tmp = path.with_extension("json.tmp");
    let json = serde_json::to_string(state)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    {
        let mut f: File = OpenOptions::new()
            .create(true)
            .truncate(true)
            .write(true)
            .open(&tmp)?;
        f.write_all(json.as_bytes())?;
    }
    std::fs::rename(&tmp, path)
}

fn kind_str(k: Kind) -> &'static str {
    match k {
        Kind::Work => "work",
        Kind::Break => "break",
        Kind::Timer => "timer",
    }
}

fn default_state_path() -> Option<PathBuf> {
    if let Ok(strategy) = choose_app_strategy(AppStrategyArgs {
        top_level_domain: String::new(),
        author: String::new(),
        app_name: "pomo".into(),
    }) {
        if let Some(dir) = strategy.runtime_dir() {
            return Some(dir.join("state.json"));
        }
    }
    Some(fallback_runtime_path())
}

fn fallback_runtime_path() -> PathBuf {
    let uid = unsafe { libc::geteuid() };
    PathBuf::from(format!("/tmp/pomo-{}", uid)).join("state.json")
}

/* -------------------------------------------------------------------------- */
/* status --json                                                              */
/* -------------------------------------------------------------------------- */

#[derive(Debug, Serialize, Deserialize)]
pub struct WaybarOutput {
    pub text: String,
    pub tooltip: String,
    pub class: String,
    pub percentage: u32,
}

pub fn read_for_status() -> WaybarOutput {
    let Some(path) = default_state_path() else {
        return idle_output();
    };
    match read_fresh(&path) {
        Some(state) => to_waybar(&state),
        None => idle_output(),
    }
}

fn read_fresh(path: &Path) -> Option<State> {
    let meta = std::fs::metadata(path).ok()?;
    let modified = meta.modified().ok()?;
    if SystemTime::now()
        .duration_since(modified)
        .map(|d| d > STALE_AFTER)
        .unwrap_or(false)
    {
        return None;
    }
    let text = std::fs::read_to_string(path).ok()?;
    serde_json::from_str(&text).ok()
}

fn idle_output() -> WaybarOutput {
    WaybarOutput {
        text: String::new(),
        tooltip: String::new(),
        class: "idle".into(),
        percentage: 0,
    }
}

fn to_waybar(s: &State) -> WaybarOutput {
    let clock = format_clock(s.remaining_secs);
    let total_clock = format_clock(s.total_secs);
    let class = if s.paused { "paused" } else { s.kind.as_str() }.to_string();
    let percentage = if s.total_secs == 0 {
        0
    } else {
        let done = s.total_secs.saturating_sub(s.remaining_secs);
        ((done as f64 / s.total_secs as f64) * 100.0).round() as u32
    };
    let tooltip = if s.paused {
        format!("{} (paused, {} / {})", s.label, clock, total_clock)
    } else {
        format!("{} ({} / {})", s.label, clock, total_clock)
    };
    WaybarOutput {
        text: clock,
        tooltip,
        class,
        percentage,
    }
}

fn format_clock(total: u64) -> String {
    let h = total / 3600;
    let m = (total % 3600) / 60;
    let s = total % 60;
    if h > 0 {
        format!("{:02}:{:02}:{:02}", h, m, s)
    } else {
        format!("{:02}:{:02}", m, s)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp_path(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "pomo-state-test-{}-{}",
            std::process::id(),
            name
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir.join("state.json")
    }

    #[test]
    fn write_then_read_roundtrip() {
        let path = tmp_path("roundtrip");
        let state = State {
            label: "Work 1/4".into(),
            kind: "work".into(),
            remaining_secs: 1234,
            total_secs: 1500,
            paused: false,
            updated_at: "2026-05-19T22:00:00Z".into(),
        };
        atomic_write(&path, &state).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        let parsed: State = serde_json::from_str(&text).unwrap();
        assert_eq!(parsed.label, "Work 1/4");
        assert_eq!(parsed.remaining_secs, 1234);
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn atomic_write_replaces_previous_content() {
        let path = tmp_path("atomic");
        let mut state = State {
            label: "Work".into(),
            kind: "work".into(),
            remaining_secs: 100,
            total_secs: 100,
            paused: false,
            updated_at: "x".into(),
        };
        atomic_write(&path, &state).unwrap();
        state.remaining_secs = 50;
        atomic_write(&path, &state).unwrap();
        let parsed: State = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(parsed.remaining_secs, 50);
        assert!(!path.with_extension("json.tmp").exists());
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn waybar_idle_when_no_state() {
        let w = idle_output();
        assert_eq!(w.class, "idle");
        assert_eq!(w.text, "");
        assert_eq!(w.percentage, 0);
    }

    #[test]
    fn waybar_running_renders_clock_and_class() {
        let s = State {
            label: "Work 1/4".into(),
            kind: "work".into(),
            remaining_secs: 1234,
            total_secs: 1500,
            paused: false,
            updated_at: "x".into(),
        };
        let w = to_waybar(&s);
        assert_eq!(w.class, "work");
        assert_eq!(w.text, "20:34");
        assert!(w.tooltip.contains("Work 1/4"));
        assert_eq!(w.percentage, 18);
    }

    #[test]
    fn waybar_paused_uses_paused_class_but_keeps_clock() {
        let s = State {
            label: "Work".into(),
            kind: "work".into(),
            remaining_secs: 60,
            total_secs: 120,
            paused: true,
            updated_at: "x".into(),
        };
        let w = to_waybar(&s);
        assert_eq!(w.class, "paused");
        assert_eq!(w.text, "01:00");
        assert!(w.tooltip.contains("paused"));
    }

    #[test]
    fn drop_removes_file() {
        let path = tmp_path("drop");
        {
            let mut sf = StateFile {
                path: Some(path.clone()),
                last_remaining: None,
                last_paused: None,
            };
            sf.write("Work", Kind::Work, Duration::from_secs(10), Duration::from_secs(60), false);
            assert!(path.exists());
        }
        assert!(!path.exists(), "Drop should remove the state file");
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn throttle_skips_writes_within_same_second() {
        let path = tmp_path("throttle");
        let mut sf = StateFile {
            path: Some(path.clone()),
            last_remaining: None,
            last_paused: None,
        };
        sf.write("Work", Kind::Work, Duration::from_millis(10_500), Duration::from_secs(60), false);
        let m1 = std::fs::metadata(&path).unwrap().modified().unwrap();
        std::thread::sleep(Duration::from_millis(30));
        // same displayed second (10s), same paused -> should be a no-op
        sf.write("Work", Kind::Work, Duration::from_millis(10_100), Duration::from_secs(60), false);
        let m2 = std::fs::metadata(&path).unwrap().modified().unwrap();
        assert_eq!(m1, m2, "no rewrite expected within the same displayed second");
        drop(sf);
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }
}
