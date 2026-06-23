//! System tray icon via the StatusNotifierItem spec (`ksni`).
//!
//! Native on KDE Plasma; on GNOME it requires the "AppIndicator and
//! KStatusNotifierItem Support" extension. If no tray host is available,
//! [`spawn`] returns `None` and the app keeps running headless.

use std::path::PathBuf;
use std::process::Command;
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use ksni::blocking::{Handle, TrayMethods};
use ksni::menu::{CheckmarkItem, StandardItem};
use ksni::{Category, Icon, MenuItem, Status, ToolTip, Tray};

use crate::config::Phase;
use crate::icon;
use crate::reminder::Controls;

struct ReminderTray {
    controls: Arc<Controls>,
    phase: Phase,
    /// Time left in the current phase; refreshed once per second by the loop.
    remaining: Duration,
    /// Configured durations, shown so the user can confirm a config edit took
    /// effect (these reflect the values loaded at startup).
    sit_duration: Duration,
    stand_duration: Duration,
    config_path: PathBuf,
}

impl ReminderTray {
    /// Open the config file in the desktop's default handler for the file type
    /// (typically a text editor). Runs in a detached thread so it neither
    /// blocks the tray nor leaves a zombie process behind.
    fn open_config(&self) {
        let path = self.config_path.clone();
        log::info!("opening config {}", path.display());
        thread::spawn(move || {
            if let Err(error) = Command::new("xdg-open").arg(&path).status() {
                log::warn!("could not open config with xdg-open: {error}");
            }
        });
    }
}

impl Tray for ReminderTray {
    fn id(&self) -> String {
        "standing-desk-reminder".into()
    }

    fn title(&self) -> String {
        "Standing Desk Reminder".into()
    }

    fn category(&self) -> Category {
        Category::ApplicationStatus
    }

    fn status(&self) -> Status {
        Status::Active
    }

    /// Primary click (left-click, or double-click depending on the host):
    /// pause or resume the reminders. ksni re-renders afterwards, so the icon
    /// switches to/from its grey "paused" colour.
    fn activate(&mut self, _x: i32, _y: i32) {
        let paused = self.controls.toggle_paused();
        log::info!("tray activated: {}", if paused { "paused" } else { "resumed" });
    }

    /// Secondary click (middle-click): switch to the other phase right now.
    fn secondary_activate(&mut self, _x: i32, _y: i32) {
        self.controls.request_skip();
        log::info!("tray secondary-activated: switch now");
    }

    fn icon_pixmap(&self) -> Vec<Icon> {
        icon::render(self.phase, self.controls.is_paused())
    }

    fn tool_tip(&self) -> ToolTip {
        let suffix = if self.controls.is_paused() {
            " — paused"
        } else {
            ""
        };
        ToolTip {
            title: "Standing Desk Reminder".into(),
            description: format!("Currently {}{}", self.phase, suffix),
            icon_name: String::new(),
            icon_pixmap: Vec::new(),
        }
    }

    fn menu(&self) -> Vec<MenuItem<Self>> {
        let paused_suffix = if self.controls.is_paused() {
            " (paused)"
        } else {
            ""
        };
        vec![
            StandardItem {
                label: format!(
                    "Currently {} — {} left{}",
                    self.phase,
                    format_remaining(self.remaining),
                    paused_suffix,
                ),
                enabled: false,
                ..Default::default()
            }
            .into(),
            StandardItem {
                label: format!(
                    "Configured: sit {}, stand {}",
                    humantime::format_duration(self.sit_duration),
                    humantime::format_duration(self.stand_duration),
                ),
                enabled: false,
                ..Default::default()
            }
            .into(),
            MenuItem::Separator,
            CheckmarkItem {
                label: "Pause reminders".into(),
                checked: self.controls.is_paused(),
                activate: Box::new(|tray: &mut Self| {
                    tray.controls.toggle_paused();
                }),
                ..Default::default()
            }
            .into(),
            StandardItem {
                label: "Switch now".into(),
                activate: Box::new(|tray: &mut Self| {
                    tray.controls.request_skip();
                }),
                ..Default::default()
            }
            .into(),
            MenuItem::Separator,
            StandardItem {
                label: "Edit configuration…".into(),
                icon_name: "document-edit".into(),
                activate: Box::new(|tray: &mut Self| tray.open_config()),
                ..Default::default()
            }
            .into(),
            MenuItem::Separator,
            StandardItem {
                label: "Quit".into(),
                icon_name: "application-exit".into(),
                activate: Box::new(|tray: &mut Self| {
                    tray.controls.request_quit();
                }),
                ..Default::default()
            }
            .into(),
        ]
    }
}

/// Format a remaining duration compactly, e.g. "1h 05m", "12m 30s", "45s".
fn format_remaining(remaining: Duration) -> String {
    let secs = remaining.as_secs();
    let (h, m, s) = (secs / 3600, (secs % 3600) / 60, secs % 60);
    if h > 0 {
        format!("{h}h {m:02}m")
    } else if m > 0 {
        format!("{m}m {s:02}s")
    } else {
        format!("{s}s")
    }
}

/// A live handle to the running tray, used to push status updates.
pub struct TrayHandle {
    handle: Handle<ReminderTray>,
}

impl TrayHandle {
    /// Refresh the icon/menu/tooltip to reflect the current phase and countdown.
    pub fn set_status(&self, phase: Phase, remaining: Duration) {
        self.handle.update(|tray| {
            tray.phase = phase;
            tray.remaining = remaining;
        });
    }
}

/// Try to register a tray icon. Returns `None` (and logs) if no StatusNotifier
/// host is available, so the caller can continue without a tray.
pub fn spawn(
    controls: Arc<Controls>,
    start_phase: Phase,
    config_path: PathBuf,
    sit_duration: Duration,
    stand_duration: Duration,
) -> Option<TrayHandle> {
    let tray = ReminderTray {
        controls,
        phase: start_phase,
        remaining: match start_phase {
            Phase::Sitting => sit_duration,
            Phase::Standing => stand_duration,
        },
        sit_duration,
        stand_duration,
        config_path,
    };
    match tray.spawn() {
        Ok(handle) => {
            log::info!("system tray icon registered");
            Some(TrayHandle { handle })
        }
        Err(error) => {
            log::warn!("system tray unavailable ({error}); continuing without a tray icon");
            None
        }
    }
}
