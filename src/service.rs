//! Install/remove a `systemd --user` service so the reminder runs in the
//! background and starts automatically on login.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, bail};
use directories::BaseDirs;

const UNIT_NAME: &str = "standing-desk-reminder.service";

fn unit_dir() -> Result<PathBuf> {
    let base = BaseDirs::new().context("could not determine the home directory")?;
    Ok(base.config_dir().join("systemd").join("user"))
}

/// Write the unit file (pointing at the current executable) and, unless
/// `enable` is false, reload systemd and enable + start the service.
pub fn install(enable: bool) -> Result<()> {
    let exe = std::env::current_exe().context("locating the current executable")?;
    let dir = unit_dir()?;
    fs::create_dir_all(&dir)
        .with_context(|| format!("creating unit directory {}", dir.display()))?;

    let path = dir.join(UNIT_NAME);
    fs::write(&path, unit_contents(&exe))
        .with_context(|| format!("writing service unit {}", path.display()))?;
    log::info!("wrote service unit to {}", path.display());

    run_systemctl(&["daemon-reload"])?;

    if enable {
        run_systemctl(&["enable", "--now", UNIT_NAME])?;
        println!("Service installed, enabled and started.");
        println!("  Status:  systemctl --user status {UNIT_NAME}");
        println!("  Logs:    journalctl --user -u {UNIT_NAME} -f");
        println!("  Remove:  standing-desk-reminder uninstall-service");
    } else {
        println!("Service unit written to {}", path.display());
        println!("Enable it with: systemctl --user enable --now {UNIT_NAME}");
    }
    Ok(())
}

/// Stop, disable and delete the service unit. Best-effort: a service that was
/// never enabled is not treated as an error.
pub fn uninstall() -> Result<()> {
    let _ = run_systemctl(&["disable", "--now", UNIT_NAME]);

    let path = unit_dir()?.join(UNIT_NAME);
    if path.exists() {
        fs::remove_file(&path).with_context(|| format!("removing {}", path.display()))?;
        log::info!("removed {}", path.display());
    }

    let _ = run_systemctl(&["daemon-reload"]);
    println!("Service removed.");
    Ok(())
}

fn run_systemctl(args: &[&str]) -> Result<()> {
    let status = Command::new("systemctl")
        .arg("--user")
        .args(args)
        .status()
        .context("failed to run `systemctl` (is systemd available?)")?;
    if !status.success() {
        bail!("`systemctl --user {}` failed ({status})", args.join(" "));
    }
    Ok(())
}

/// The default unit file template, read from dist/.
const UNIT_TEMPLATE: &str = include_str!("../dist/standing-desk-reminder.service");

/// Generate the unit file contents by replacing the ExecStart placeholder
/// with the actual executable path.
fn unit_contents(exe: &Path) -> String {
    UNIT_TEMPLATE.replace("%h/.cargo/bin/standing-desk-reminder", &exe.display().to_string())
}
