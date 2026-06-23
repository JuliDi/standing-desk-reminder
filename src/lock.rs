//! Pause the countdown while the screen is locked.
//!
//! Uses systemd-logind's `LockedHint`, which both GNOME and KDE set on lock and
//! unlock (including idle auto-lock, unlike the `Lock`/`Unlock` signals).
//!
//! The graphical session is found via the logind *user* object's `Display`
//! session, rather than `GetSessionByPID`: the latter fails for processes that
//! aren't in a login-session cgroup, which includes `systemd --user` services.
//! If the system bus or logind is unavailable, this degrades to a no-op.

use std::os::unix::fs::MetadataExt;
use std::sync::Arc;
use std::thread;

use anyhow::{Context, Result};
use zbus::blocking::Connection;
use zbus::zvariant::OwnedObjectPath;

use crate::reminder::Controls;

#[zbus::proxy(
    interface = "org.freedesktop.login1.Manager",
    default_service = "org.freedesktop.login1",
    default_path = "/org/freedesktop/login1"
)]
trait Manager {
    fn get_user(&self, uid: u32) -> zbus::Result<OwnedObjectPath>;
}

#[zbus::proxy(
    interface = "org.freedesktop.login1.User",
    default_service = "org.freedesktop.login1"
)]
trait User {
    /// The user's primary graphical session: `(session_id, object_path)`.
    #[zbus(property)]
    fn display(&self) -> zbus::Result<(String, OwnedObjectPath)>;
}

#[zbus::proxy(
    interface = "org.freedesktop.login1.Session",
    default_service = "org.freedesktop.login1"
)]
trait Session {
    #[zbus(property)]
    fn locked_hint(&self) -> zbus::Result<bool>;
}

/// Spawn a background thread mirroring the screen-lock state into `controls`.
pub fn spawn(controls: Arc<Controls>) {
    thread::spawn(move || {
        if let Err(error) = watch(&controls) {
            log::warn!("screen-lock detection disabled: {error:#}");
        }
    });
}

fn watch(controls: &Arc<Controls>) -> Result<()> {
    let conn = Connection::system().context("connecting to the system D-Bus")?;

    // Our own uid: /proc/self is owned by it (no libc dependency needed).
    let uid = std::fs::metadata("/proc/self")
        .context("reading our uid")?
        .uid();

    let manager = ManagerProxyBlocking::new(&conn).context("creating the logind manager proxy")?;
    let user_path = manager.get_user(uid).context("looking up our logind user")?;
    let user = UserProxyBlocking::builder(&conn)
        .path(user_path)?
        .build()
        .context("creating the logind user proxy")?;
    let (_id, session_path) = user.display().context("finding the graphical session")?;

    let session = SessionProxyBlocking::builder(&conn)
        .path(session_path)?
        .build()
        .context("creating the logind session proxy")?;

    let mut last = session.locked_hint().unwrap_or(false);
    controls.set_locked(last);
    log::info!("screen-lock detection active (locked={last})");

    // Blocks on this thread, applying every LockedHint change as it arrives.
    // The stream replays the current value first, so ignore no-op repeats.
    for change in session.receive_locked_hint_changed() {
        let locked = change.get().unwrap_or(false);
        if locked == last {
            continue;
        }
        last = locked;
        controls.set_locked(locked);
        log::info!(
            "screen {}",
            if locked {
                "locked — pausing"
            } else {
                "unlocked — resuming"
            }
        );
    }
    Ok(())
}
