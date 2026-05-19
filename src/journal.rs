/*
 * Append-only JSONL log of pomodoro intervals.
 *
 * One line per interval, regardless of outcome. The `completed` field
 * distinguishes a natural finish from a user-aborted run so downstream
 * analysis can compute both completion rate and elapsed time.
 *
 * Writes are best-effort: any I/O failure produces a one-time stderr
 * warning and never blocks or panics the foreground TUI.
 */

use crate::Kind;
use etcetera::{choose_app_strategy, AppStrategy, AppStrategyArgs};
use serde::Serialize;
use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::{Duration, SystemTime};

#[derive(Debug, Serialize)]
pub struct Entry<'a> {
    pub started_at: String,
    pub ended_at: String,
    pub kind: &'static str,
    pub label: &'a str,
    pub planned_secs: u64,
    pub completed: bool,
}

pub struct Journal {
    path: Option<PathBuf>,
    warned: OnceLock<()>,
}

impl Journal {
    pub fn new() -> Self {
        Self {
            path: default_log_path(),
            warned: OnceLock::new(),
        }
    }

    #[cfg(test)]
    pub fn with_path(path: PathBuf) -> Self {
        Self {
            path: Some(path),
            warned: OnceLock::new(),
        }
    }

    pub fn record(
        &self,
        kind: Kind,
        label: &str,
        started: SystemTime,
        ended: SystemTime,
        planned: Duration,
        completed: bool,
    ) -> bool {
        let Some(path) = self.path.as_ref() else { return false };
        let entry = Entry {
            started_at: format_rfc3339(started),
            ended_at: format_rfc3339(ended),
            kind: kind_str(kind),
            label,
            planned_secs: planned.as_secs(),
            completed,
        };
        match append_entry(path, &entry) {
            Ok(()) => true,
            Err(e) => {
                if self.warned.set(()).is_ok() {
                    eprintln!(
                        "pomo: could not write session log {} ({}); further failures will be silent",
                        path.display(),
                        e
                    );
                }
                false
            }
        }
    }
}

fn append_entry(path: &Path, entry: &Entry<'_>) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        ensure_dir(parent)?;
    }
    let mut line = serde_json::to_string(entry)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    line.push('\n');
    let mut f: File = OpenOptions::new().create(true).append(true).open(path)?;
    f.write_all(line.as_bytes())
}

fn ensure_dir(dir: &Path) -> std::io::Result<()> {
    if dir.exists() {
        return Ok(());
    }
    std::fs::create_dir_all(dir)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700));
    }
    Ok(())
}

fn kind_str(k: Kind) -> &'static str {
    match k {
        Kind::Work => "work",
        Kind::Break => "break",
        Kind::Timer => "timer",
    }
}

fn format_rfc3339(t: SystemTime) -> String {
    humantime::format_rfc3339_seconds(t).to_string()
}

fn default_log_path() -> Option<PathBuf> {
    let strategy = choose_app_strategy(AppStrategyArgs {
        top_level_domain: String::new(),
        author: String::new(),
        app_name: "pomo".into(),
    })
    .ok()?;
    Some(strategy.data_dir().join("log.jsonl"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn tmp_log(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "pomo-test-{}-{}-{:?}",
            std::process::id(),
            name,
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        dir.join("nested").join("log.jsonl")
    }

    fn read_lines(p: &Path) -> Vec<String> {
        std::fs::read_to_string(p)
            .unwrap()
            .lines()
            .map(|s| s.to_string())
            .collect()
    }

    #[test]
    fn creates_parent_dir_and_writes_valid_json() {
        let path = tmp_log("creates_parent_dir_and_writes_valid_json");
        let j = Journal::with_path(path.clone());
        let t0 = SystemTime::UNIX_EPOCH + Duration::from_secs(1_700_000_000);
        let t1 = t0 + Duration::from_secs(60);
        j.record(Kind::Work, "Work 1/4", t0, t1, Duration::from_secs(60), true);

        assert!(path.exists(), "log file should be created");
        let lines = read_lines(&path);
        assert_eq!(lines.len(), 1);
        let v: serde_json::Value = serde_json::from_str(&lines[0]).unwrap();
        assert_eq!(v["kind"], "work");
        assert_eq!(v["label"], "Work 1/4");
        assert_eq!(v["planned_secs"], 60);
        assert_eq!(v["completed"], true);
        assert!(v["started_at"].as_str().unwrap().ends_with('Z'));

        let _ = std::fs::remove_dir_all(path.parent().unwrap().parent().unwrap());
    }

    #[test]
    fn appends_multiple_records() {
        let path = tmp_log("appends_multiple_records");
        let j = Journal::with_path(path.clone());
        let t0 = SystemTime::UNIX_EPOCH + Duration::from_secs(1_700_000_000);
        j.record(Kind::Work, "Work", t0, t0 + Duration::from_secs(60), Duration::from_secs(60), true);
        j.record(Kind::Break, "Break", t0, t0 + Duration::from_secs(10), Duration::from_secs(60), false);

        let lines = read_lines(&path);
        assert_eq!(lines.len(), 2);
        let v0: serde_json::Value = serde_json::from_str(&lines[0]).unwrap();
        let v1: serde_json::Value = serde_json::from_str(&lines[1]).unwrap();
        assert_eq!(v0["completed"], true);
        assert_eq!(v1["completed"], false);
        assert_eq!(v1["kind"], "break");

        let _ = std::fs::remove_dir_all(path.parent().unwrap().parent().unwrap());
    }

    #[test]
    fn unwritable_path_does_not_panic() {
        // /proc is read-only on Linux; on other systems this may still succeed silently.
        let path = PathBuf::from("/proc/pomo-cannot-write/log.jsonl");
        let j = Journal::with_path(path);
        let t0 = SystemTime::UNIX_EPOCH;
        j.record(Kind::Timer, "x", t0, t0, Duration::from_secs(1), true);
    }

    #[cfg(unix)]
    #[test]
    fn parent_dir_has_0700_perms() {
        use std::os::unix::fs::PermissionsExt;
        let path = tmp_log("parent_dir_has_0700_perms");
        let j = Journal::with_path(path.clone());
        let t0 = SystemTime::UNIX_EPOCH;
        j.record(Kind::Work, "x", t0, t0, Duration::from_secs(1), true);
        let mode = std::fs::metadata(path.parent().unwrap()).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o700);
        let _ = std::fs::remove_dir_all(path.parent().unwrap().parent().unwrap());
    }
}
