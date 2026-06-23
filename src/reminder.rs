//! The reminder scheduler: an interruptible loop that alternates phases and
//! fires notifications, plus the shared control state the tray menu pokes at.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::Duration;

use crate::config::{Config, Phase};
use crate::notify;

/// Called with the current `(phase, remaining)` so a tray icon can refresh its
/// countdown. Invoked on every phase change and at most once per second.
pub type StatusCallback = Box<dyn Fn(Phase, Duration) + Send>;

/// How often the loop wakes to check for pause / skip / quit requests.
const POLL_INTERVAL: Duration = Duration::from_millis(250);

/// Shared, thread-safe control flags driven by the tray menu and Ctrl-C.
#[derive(Debug, Default)]
pub struct Controls {
    paused: AtomicBool,
    quit: AtomicBool,
    skip: AtomicBool,
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

    pub fn request_quit(&self) {
        self.quit.store(true, Ordering::Relaxed);
    }

    fn quit_requested(&self) -> bool {
        self.quit.load(Ordering::Relaxed)
    }

    fn take_skip(&self) -> bool {
        self.skip.swap(false, Ordering::Relaxed)
    }
}

/// Run the reminder loop until a quit is requested. Blocks the calling thread.
pub fn run(config: &Config, controls: Arc<Controls>, on_status: Option<StatusCallback>) {
    let mut phase = config.start_phase;
    let mut remaining = config.duration(phase);

    notify_status(&on_status, phase, remaining);
    let mut last_secs = remaining.as_secs();

    log::info!(
        "started in the {phase} phase; first switch in {}",
        humantime::format_duration(remaining)
    );

    while !controls.quit_requested() {
        // An explicit "switch now" wins, even while paused.
        if controls.take_skip() {
            phase = phase.other();
            remaining = config.duration(phase);
            announce_switch(config, phase);
            notify_status(&on_status, phase, remaining);
            last_secs = remaining.as_secs();
            continue;
        }

        if controls.is_paused() {
            thread::sleep(POLL_INTERVAL);
            continue;
        }

        let tick = POLL_INTERVAL.min(remaining);
        thread::sleep(tick);
        remaining = remaining.saturating_sub(tick);

        if remaining.is_zero() {
            phase = phase.other();
            remaining = config.duration(phase);
            announce_switch(config, phase);
            notify_status(&on_status, phase, remaining);
            last_secs = remaining.as_secs();
        } else {
            // Refresh the tray's countdown at most once per second.
            let secs = remaining.as_secs();
            if secs != last_secs {
                last_secs = secs;
                notify_status(&on_status, phase, remaining);
            }
        }
    }

    log::info!("reminder loop stopped");
}

fn announce_switch(config: &Config, phase: Phase) {
    notify::send_reminder(config, phase);
    log::info!(
        "switched to {phase}; next switch in {}",
        humantime::format_duration(config.duration(phase))
    );
}

fn notify_status(on_status: &Option<StatusCallback>, phase: Phase, remaining: Duration) {
    if let Some(callback) = on_status {
        callback(phase, remaining);
    }
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
}
