//! Configuration: a small TOML file in the user's XDG config directory.

use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context, Result, bail};
use directories::ProjectDirs;
use serde::{Deserialize, Serialize};

/// Which posture the user is currently in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Phase {
    Sitting,
    Standing,
}

impl Phase {
    /// The opposite posture.
    pub fn other(self) -> Phase {
        match self {
            Phase::Sitting => Phase::Standing,
            Phase::Standing => Phase::Sitting,
        }
    }
}

impl fmt::Display for Phase {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = match self {
            Phase::Sitting => "sitting",
            Phase::Standing => "standing",
        };
        f.write_str(s)
    }
}

/// User configuration, loaded from `config.toml`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Config {
    /// How long to stay seated before being reminded to stand.
    #[serde(with = "humantime_serde")]
    pub sit_duration: Duration,
    /// How long to stand before being reminded to sit down.
    #[serde(with = "humantime_serde")]
    pub stand_duration: Duration,
    /// Notification body shown when it is time to stand.
    pub stand_message: String,
    /// Notification body shown when it is time to sit.
    pub sit_message: String,
    /// Ask the desktop to play its notification sound with each reminder.
    pub sound: bool,
    /// Which posture to assume the user is in at startup.
    pub start_phase: Phase,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            sit_duration: Duration::from_secs(45 * 60),
            stand_duration: Duration::from_secs(15 * 60),
            stand_message: "Time to stand up and stretch your legs.".to_string(),
            sit_message: "Time to sit down for a bit.".to_string(),
            sound: true,
            start_phase: Phase::Sitting,
        }
    }
}

impl Config {
    /// How long the given phase should last before switching.
    pub fn duration(&self, phase: Phase) -> Duration {
        match phase {
            Phase::Sitting => self.sit_duration,
            Phase::Standing => self.stand_duration,
        }
    }

    /// The notification body to show when entering the given phase.
    pub fn message(&self, phase: Phase) -> &str {
        match phase {
            Phase::Sitting => &self.sit_message,
            Phase::Standing => &self.stand_message,
        }
    }

    /// Reject nonsensical values that would otherwise busy-loop or never fire.
    pub fn validate(&self) -> Result<()> {
        if self.sit_duration.is_zero() {
            bail!("`sit_duration` must be greater than zero");
        }
        if self.stand_duration.is_zero() {
            bail!("`stand_duration` must be greater than zero");
        }
        Ok(())
    }
}

/// The default config file location, e.g. `~/.config/standing-desk-reminder/config.toml`.
pub fn default_path() -> Result<PathBuf> {
    let dirs = ProjectDirs::from("", "", "standing-desk-reminder")
        .context("could not determine a configuration directory for the current user")?;
    Ok(dirs.config_dir().join("config.toml"))
}

/// Load the config, creating a commented default file on first run.
pub fn load(path: &Path) -> Result<Config> {
    if !path.exists() {
        write_default(path)?;
        log::info!("created a default config at {}", path.display());
    }
    let text =
        fs::read_to_string(path).with_context(|| format!("reading config {}", path.display()))?;
    let config: Config =
        toml::from_str(&text).with_context(|| format!("parsing config {}", path.display()))?;
    config.validate()?;
    Ok(config)
}

fn write_default(path: &Path) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("creating config directory {}", parent.display()))?;
    }
    fs::write(path, DEFAULT_CONFIG_TEMPLATE)
        .with_context(|| format!("writing default config {}", path.display()))?;
    Ok(())
}

/// The commented file written on first run. Kept in sync with [`Config::default`]
/// by the `default_template_matches_default` test below.
const DEFAULT_CONFIG_TEMPLATE: &str = r#"# Configuration for standing-desk-reminder.
# Durations accept friendly units, e.g. "30s", "15m", "1h 30m".

# How long to stay seated before you are reminded to stand up.
sit_duration = "45m"

# How long to stand before you are reminded to sit down again.
stand_duration = "15m"

# Notification text shown when it is time to stand up.
stand_message = "Time to stand up and stretch your legs."

# Notification text shown when it is time to sit down.
sit_message = "Time to sit down for a bit."

# Ask the desktop to play its notification sound with each reminder.
sound = true

# Which posture to start in when the program launches: "sitting" or "standing".
start_phase = "sitting"
"#;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_template_matches_default() {
        let parsed: Config = toml::from_str(DEFAULT_CONFIG_TEMPLATE).unwrap();
        assert_eq!(parsed, Config::default());
    }

    #[test]
    fn default_config_is_valid() {
        Config::default().validate().unwrap();
    }

    #[test]
    fn zero_duration_is_rejected() {
        let config = Config {
            stand_duration: Duration::ZERO,
            ..Config::default()
        };
        assert!(config.validate().is_err());
    }

    #[test]
    fn phase_round_trips() {
        assert_eq!(Phase::Sitting.other(), Phase::Standing);
        assert_eq!(Phase::Standing.other(), Phase::Sitting);
    }
}
