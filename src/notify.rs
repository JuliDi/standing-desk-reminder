//! Desktop notifications via the freedesktop `org.freedesktop.Notifications`
//! D-Bus service (works on GNOME, KDE, and other XDG desktops).

use std::sync::Arc;
use std::thread;

use notify_rust::{Notification, Timeout, Urgency};

use crate::config::{Config, Phase};
use crate::reminder::Controls;

const APP_NAME: &str = "Standing Desk Reminder";

/// Action identifiers attached to reminder notifications.
const ACTION_RESTART: &str = "restart";
const ACTION_SNOOZE: &str = "snooze";

/// Show the reminder for the phase the user is switching *into*.
///
/// The notification is persistent (it stays until acted on, so it isn't missed
/// while away) and carries two actions wired back to the reminder loop via
/// [`Controls`]: confirm ("I'm standing/sitting") restarts the countdown, and
/// "Snooze" postpones the switch.
pub fn send_reminder(config: &Config, phase: Phase, controls: &Arc<Controls>) {
    let (summary, icon, confirm_label) = match phase {
        Phase::Standing => ("Time to stand up", "go-up", "I'm standing"),
        Phase::Sitting => ("Time to sit down", "go-down", "I'm sitting"),
    };
    let snooze_label = format!(
        "Snooze {}",
        humantime::format_duration(config.snooze_duration)
    );

    let mut notification = Notification::new();
    notification
        .appname(APP_NAME)
        .summary(summary)
        .body(config.message(phase))
        .icon(icon)
        // Critical + Never keeps it on screen until the user reacts, on both
        // KDE and GNOME (which otherwise auto-hides non-critical banners).
        .urgency(Urgency::Critical)
        .timeout(Timeout::Never)
        .action(ACTION_RESTART, confirm_label)
        .action(ACTION_SNOOZE, &snooze_label);

    if config.sound {
        // Freedesktop sound-naming-spec event; honored by the notification
        // daemon (reliably on KDE, best-effort on GNOME).
        notification.sound_name("message");
    }

    match notification.show() {
        Ok(handle) => {
            let controls = Arc::clone(controls);
            // wait_for_action blocks until an action is clicked or the
            // notification is dismissed, so run it off the reminder thread.
            thread::spawn(move || {
                handle.wait_for_action(|action| match action {
                    ACTION_RESTART => controls.request_restart(),
                    ACTION_SNOOZE => controls.request_snooze(),
                    _ => {} // "default" / "__closed": nothing to do
                });
            });
        }
        Err(error) => log::warn!("could not display notification: {error}"),
    }
}

/// A brief, low-urgency confirmation that the reminder is running.
pub fn send_startup(config: &Config) {
    let body = format!(
        "Running. Sit for {}, stand for {}. First reminder in {}.",
        humantime::format_duration(config.sit_duration),
        humantime::format_duration(config.stand_duration),
        humantime::format_duration(config.duration(config.start_phase)),
    );

    let mut notification = Notification::new();
    notification
        .appname(APP_NAME)
        .summary(APP_NAME)
        .body(&body)
        .icon("appointment-soon")
        .urgency(Urgency::Low)
        .timeout(Timeout::Milliseconds(6_000));

    if let Err(error) = notification.show() {
        log::warn!("could not display startup notification: {error}");
    }
}
