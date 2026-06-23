//! The reminder scheduler: an interruptible loop that alternates phases and
//! fires notifications, plus the shared control state the tray menu pokes at.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::Duration;

use anyhow::Result;

use crate::config::{self, Config, Phase};
use crate::notify;

/// A snapshot pushed to the tray so it can render the live countdown and the
/// currently-configured durations.
pub struct TrayStatus {
    pub phase: Phase,
    pub remaining: Duration,
    pub sit_duration: Duration,
    pub stand_duration: Duration,
}

/// Called whenever the tray's display should refresh (phase change, the
/// once-a-second countdown tick, or a config reload).
pub type StatusCallback = Box<dyn Fn(TrayStatus) + Send>;

/// Command-line overrides re-applied on top of the file on every (re)load.
#[derive(Debug, Default, Clone)]
pub struct Overrides {
    pub sit: Option<Duration>,
    pub stand: Option<Duration>,
    pub no_sound: bool,
}

impl Overrides {
    pub fn apply(&self, config: &mut Config) {
        if let Some(sit) = self.sit {
            config.sit_duration = sit;
        }
        if let Some(stand) = self.stand {
            config.stand_duration = stand;
        }
        if self.no_sound {
            config.sound = false;
        }
    }
}

/// How often the loop wakes to check for pause / skip / quit requests.
const POLL_INTERVAL: Duration = Duration::from_millis(250);

/// Shared, thread-safe control flags driven by the tray menu, notification
/// actions, the screen-lock watcher, and Ctrl-C.
#[derive(Debug, Default)]
pub struct Controls {
    paused: AtomicBool,
    quit: AtomicBool,
    skip: AtomicBool,
    restart: AtomicBool,
    snooze: AtomicBool,
    reload: AtomicBool,
    locked: AtomicBool,
}

impl Controls {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    pub fn is_paused(&self) -> bool {
        self.paused.load(Ordering::Relaxed)
    }

    /// Flip the paused flag, returning the new state.
    pub fn toggle_paused(&self) -> bool {
        !self.paused.fetch_xor(true, Ordering::Relaxed)
    }

    pub fn request_skip(&self) {
        self.skip.store(true, Ordering::Relaxed);
    }

    /// Restart the current phase's countdown from the full duration.
    pub fn request_restart(&self) {
        self.restart.store(true, Ordering::Relaxed);
    }

    /// Postpone: go back to the previous phase for the snooze duration.
    pub fn request_snooze(&self) {
        self.snooze.store(true, Ordering::Relaxed);
    }

    /// Re-read the config file from disk.
    pub fn request_reload(&self) {
        self.reload.store(true, Ordering::Relaxed);
    }

    pub fn request_quit(&self) {
        self.quit.store(true, Ordering::Relaxed);
    }

    /// Set by the screen-lock watcher; freezes the countdown while `true`.
    pub fn set_locked(&self, locked: bool) {
        self.locked.store(locked, Ordering::Relaxed);
    }

    pub fn is_locked(&self) -> bool {
        self.locked.load(Ordering::Relaxed)
    }

    fn quit_requested(&self) -> bool {
        self.quit.load(Ordering::Relaxed)
    }

    fn take_skip(&self) -> bool {
        self.skip.swap(false, Ordering::Relaxed)
    }

    fn take_restart(&self) -> bool {
        self.restart.swap(false, Ordering::Relaxed)
    }

    fn take_snooze(&self) -> bool {
        self.snooze.swap(false, Ordering::Relaxed)
    }

    fn take_reload(&self) -> bool {
        self.reload.swap(false, Ordering::Relaxed)
    }
}

/// Run the reminder loop until a quit is requested. Blocks the calling thread.
///
/// Owns `config` so it can be swapped out when a reload is requested (by the
/// tray menu or the config-file watcher); `overrides` are re-applied each time.
pub fn run(
    mut config: Config,
    config_path: PathBuf,
    overrides: Overrides,
    controls: Arc<Controls>,
    on_status: Option<StatusCallback>,
) {
    let mut phase = config.start_phase;
    let mut remaining = config.duration(phase);

    notify_status(&on_status, &config, phase, remaining);
    let mut last_secs = remaining.as_secs();

    log::info!(
        "started in the {phase} phase; first switch in {}",
        humantime::format_duration(remaining)
    );

    while !controls.quit_requested() {
        // Re-read the config file. Shortening the current phase takes effect
        // immediately (clamp); lengthening applies from the next switch.
        if controls.take_reload() {
            match reload(&config_path, &overrides) {
                Ok(new_config) => {
                    config = new_config;
                    remaining = remaining.min(config.duration(phase));
                    last_secs = remaining.as_secs();
                    notify_status(&on_status, &config, phase, remaining);
                    log::info!(
                        "config reloaded (sit {}, stand {})",
                        humantime::format_duration(config.sit_duration),
                        humantime::format_duration(config.stand_duration),
                    );
                }
                Err(error) => {
                    log::warn!("config reload failed, keeping current settings: {error:#}");
                    notify::send_simple(
                        "Config reload failed",
                        &format!("{error:#}\nKeeping the previous settings."),
                    );
                }
            }
            continue;
        }

        // An explicit "switch now" wins, even while paused. No reminder
        // notification here: the user just triggered the switch, so there is
        // nothing to confirm.
        if controls.take_skip() {
            phase = phase.other();
            remaining = config.duration(phase);
            notify_status(&on_status, &config, phase, remaining);
            last_secs = remaining.as_secs();
            log::info!(
                "manually switched to {phase}; next switch in {}",
                humantime::format_duration(remaining)
            );
            continue;
        }

        // "I did it" — restart the current phase's countdown from now.
        if controls.take_restart() {
            remaining = config.duration(phase);
            notify_status(&on_status, &config, phase, remaining);
            last_secs = remaining.as_secs();
            log::info!(
                "restarted {phase} phase; next switch in {}",
                humantime::format_duration(remaining)
            );
            continue;
        }

        // "Snooze" — revert to the previous phase for the snooze duration.
        if controls.take_snooze() {
            phase = phase.other();
            remaining = config.snooze_duration;
            notify_status(&on_status, &config, phase, remaining);
            last_secs = remaining.as_secs();
            log::info!(
                "snoozed; back to {phase} for {}",
                humantime::format_duration(remaining)
            );
            continue;
        }

        // Freeze while manually paused or while the screen is locked.
        if controls.is_paused() || controls.is_locked() {
            thread::sleep(POLL_INTERVAL);
            continue;
        }

        let tick = POLL_INTERVAL.min(remaining);
        thread::sleep(tick);
        remaining = remaining.saturating_sub(tick);

        if remaining.is_zero() {
            phase = phase.other();
            remaining = config.duration(phase);
            announce_switch(&config, phase, &controls);
            notify_status(&on_status, &config, phase, remaining);
            last_secs = remaining.as_secs();
        } else {
            // Refresh the tray's countdown at most once per second.
            let secs = remaining.as_secs();
            if secs != last_secs {
                last_secs = secs;
                notify_status(&on_status, &config, phase, remaining);
            }
        }
    }

    log::info!("reminder loop stopped");
}

fn announce_switch(config: &Config, phase: Phase, controls: &Arc<Controls>) {
    notify::send_reminder(config, phase, controls);
    log::info!(
        "switched to {phase}; next switch in {}",
        humantime::format_duration(config.duration(phase))
    );
}

fn notify_status(
    on_status: &Option<StatusCallback>,
    config: &Config,
    phase: Phase,
    remaining: Duration,
) {
    if let Some(callback) = on_status {
        callback(TrayStatus {
            phase,
            remaining,
            sit_duration: config.sit_duration,
            stand_duration: config.stand_duration,
        });
    }
}

/// Re-read the config from disk and re-apply the command-line overrides.
fn reload(path: &Path, overrides: &Overrides) -> Result<Config> {
    let mut config = config::load(path)?;
    overrides.apply(&mut config);
    config.validate()?;
    Ok(config)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn toggle_paused_flips_and_reports_new_state() {
        let controls = Controls::new();
        assert!(!controls.is_paused());
        assert!(controls.toggle_paused());
        assert!(controls.is_paused());
        assert!(!controls.toggle_paused());
        assert!(!controls.is_paused());
    }

    #[test]
    fn skip_is_taken_once() {
        let controls = Controls::new();
        controls.request_skip();
        assert!(controls.take_skip());
        assert!(!controls.take_skip());
    }

    #[test]
    fn restart_and_snooze_are_taken_once() {
        let controls = Controls::new();
        controls.request_restart();
        controls.request_snooze();
        assert!(controls.take_restart());
        assert!(!controls.take_restart());
        assert!(controls.take_snooze());
        assert!(!controls.take_snooze());
    }

    #[test]
    fn lock_state_round_trips() {
        let controls = Controls::new();
        assert!(!controls.is_locked());
        controls.set_locked(true);
        assert!(controls.is_locked());
        controls.set_locked(false);
        assert!(!controls.is_locked());
    }
}
