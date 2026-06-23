//! Desktop notifications via the freedesktop `org.freedesktop.Notifications`
//! D-Bus service (works on GNOME, KDE, and other XDG desktops).

use notify_rust::{Notification, Timeout, Urgency};

use crate::config::{Config, Phase};

const APP_NAME: &str = "Standing Desk Reminder";

/// Show the reminder for the phase the user is switching *into*.
pub fn send_reminder(config: &Config, phase: Phase) {
    let (summary, icon) = match phase {
        Phase::Standing => ("Time to stand up", "go-up"),
        Phase::Sitting => ("Time to sit down", "go-down"),
    };

    let mut notification = Notification::new();
    notification
        .appname(APP_NAME)
        .summary(summary)
        .body(config.message(phase))
        .icon(icon)
        .urgency(Urgency::Normal)
        .timeout(Timeout::Milliseconds(12_000));

    if config.sound {
        // Freedesktop sound-naming-spec event; honored by the notification
        // daemon (reliably on KDE, best-effort on GNOME).
        notification.sound_name("message");
    }

    if let Err(error) = notification.show() {
        log::warn!("could not display notification: {error}");
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
