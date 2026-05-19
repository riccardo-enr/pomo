/*
 * User config loaded from `$XDG_CONFIG_HOME/pomo/config.toml`.
 *
 * Precedence in the rest of the program: CLI flag > config file value >
 * built-in default. The defaults live in `RawConfig::default` and are the
 * single source of truth for what shipping `pomo` does with no arguments.
 *
 * Unknown keys are rejected so typos produce a clear error rather than
 * silently doing nothing.
 */

use anyhow::{Context, Result};
use etcetera::{choose_app_strategy, AppStrategy, AppStrategyArgs};
use serde::Deserialize;
use std::path::{Path, PathBuf};
use std::time::Duration;

#[derive(Debug, Clone)]
pub struct Config {
    pub work: Duration,
    pub short_break: Duration,
    pub long_break: Duration,
    pub rounds: u32,
    pub sound: bool,
    pub sound_path: Option<PathBuf>,
    pub desktop_notification: bool,
    pub bell_volume: f32,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields, default)]
struct RawConfig {
    work: String,
    short_break: String,
    long_break: String,
    rounds: u32,
    sound: bool,
    sound_path: String,
    desktop_notification: bool,
    bell_volume: f32,
}

impl Default for RawConfig {
    fn default() -> Self {
        Self {
            work: "25m".into(),
            short_break: "5m".into(),
            long_break: "15m".into(),
            rounds: 4,
            sound: true,
            sound_path: String::new(),
            desktop_notification: true,
            bell_volume: 1.0,
        }
    }
}

impl RawConfig {
    fn into_config(self) -> Result<Config> {
        let work = humantime::parse_duration(&self.work)
            .with_context(|| format!("invalid `work` duration {:?}", self.work))?;
        let short_break = humantime::parse_duration(&self.short_break)
            .with_context(|| format!("invalid `short_break` duration {:?}", self.short_break))?;
        let long_break = humantime::parse_duration(&self.long_break)
            .with_context(|| format!("invalid `long_break` duration {:?}", self.long_break))?;
        let sound_path = if self.sound_path.trim().is_empty() {
            None
        } else {
            Some(PathBuf::from(self.sound_path))
        };
        Ok(Config {
            work,
            short_break,
            long_break,
            rounds: self.rounds,
            sound: self.sound,
            sound_path,
            desktop_notification: self.desktop_notification,
            bell_volume: self.bell_volume,
        })
    }
}

impl Default for Config {
    fn default() -> Self {
        RawConfig::default().into_config().expect("built-in defaults are valid")
    }
}

impl Config {
    pub fn load() -> Result<Self> {
        let Some(path) = config_path() else {
            return Ok(Self::default());
        };
        Self::load_from(&path)
    }

    pub fn load_from(path: &Path) -> Result<Self> {
        let text = match std::fs::read_to_string(path) {
            Ok(t) => t,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return Ok(Self::default());
            }
            Err(e) => {
                eprintln!(
                    "pomo: could not read {} ({}); using built-in defaults",
                    path.display(),
                    e
                );
                return Ok(Self::default());
            }
        };
        let raw: RawConfig = toml::from_str(&text)
            .with_context(|| format!("parsing config {}", path.display()))?;
        raw.into_config()
    }
}

fn config_path() -> Option<PathBuf> {
    let strategy = choose_app_strategy(AppStrategyArgs {
        top_level_domain: String::new(),
        author: String::new(),
        app_name: "pomo".into(),
    })
    .ok()?;
    Some(strategy.config_dir().join("config.toml"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(toml_str: &str) -> Result<Config> {
        let raw: RawConfig = toml::from_str(toml_str)?;
        raw.into_config()
    }

    #[test]
    fn default_matches_shipping_behavior() {
        let c = Config::default();
        assert_eq!(c.work, Duration::from_secs(25 * 60));
        assert_eq!(c.short_break, Duration::from_secs(5 * 60));
        assert_eq!(c.long_break, Duration::from_secs(15 * 60));
        assert_eq!(c.rounds, 4);
        assert!(c.sound);
        assert!(c.sound_path.is_none());
        assert!(c.desktop_notification);
        assert_eq!(c.bell_volume, 1.0);
    }

    #[test]
    fn partial_config_keeps_defaults_for_missing_keys() {
        let c = parse(r#"work = "10m""#).unwrap();
        assert_eq!(c.work, Duration::from_secs(10 * 60));
        assert_eq!(c.short_break, Duration::from_secs(5 * 60));
        assert_eq!(c.rounds, 4);
    }

    #[test]
    fn unknown_key_rejected() {
        let err = parse(r#"work = "1m"
foo_bar = 3"#).unwrap_err();
        let msg = format!("{}", err);
        assert!(msg.contains("foo_bar"), "error should name the unknown field: {}", msg);
    }

    #[test]
    fn bad_duration_rejected() {
        let err = parse(r#"work = "twenty minutes""#).unwrap_err();
        let msg = format!("{:#}", err);
        assert!(msg.contains("work"), "error should name the bad field: {}", msg);
    }

    #[test]
    fn empty_sound_path_means_none() {
        let c = parse(r#"sound_path = """#).unwrap();
        assert!(c.sound_path.is_none());
    }

    #[test]
    fn missing_file_returns_default() {
        let path = std::env::temp_dir().join("pomo-nonexistent-12345.toml");
        let _ = std::fs::remove_file(&path);
        let c = Config::load_from(&path).unwrap();
        assert_eq!(c.work, Duration::from_secs(25 * 60));
    }
}
