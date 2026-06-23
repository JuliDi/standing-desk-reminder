//! standing-desk-reminder: a small cross-desktop nudge to alternate between
//! standing and sitting. Shows desktop notifications on a configurable cadence,
//! with an optional system tray icon and an optional systemd --user service.

mod config;
mod notify;
mod reminder;
mod service;

#[cfg(target_os = "linux")]
mod icon;
#[cfg(target_os = "linux")]
mod tray;

use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use clap::{Args, Parser, Subcommand};

use crate::config::Config;
use crate::reminder::{Controls, StatusCallback};

#[derive(Parser)]
#[command(name = "standing-desk-reminder", version, about, long_about = None)]
struct Cli {
    /// Path to the config file (default: ~/.config/standing-desk-reminder/config.toml)
    #[arg(long, global = true, value_name = "FILE")]
    config: Option<PathBuf>,

    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
    /// Run the reminder loop (this is the default when no command is given)
    Run(RunArgs),
    /// Print the path to the configuration file
    ConfigPath,
    /// Install and enable a `systemd --user` service for background use
    InstallService {
        /// Write the unit file but do not enable or start it
        #[arg(long)]
        no_enable: bool,
    },
    /// Stop and remove the `systemd --user` service
    UninstallService,
}

#[derive(Args, Default)]
struct RunArgs {
    /// Override how long to sit before the stand reminder (e.g. 45m, 1h)
    #[arg(long, value_name = "DURATION", value_parser = parse_duration)]
    sit: Option<Duration>,

    /// Override how long to stand before the sit reminder (e.g. 15m)
    #[arg(long, value_name = "DURATION", value_parser = parse_duration)]
    stand: Option<Duration>,

    /// Do not show a system tray icon
    #[arg(long)]
    no_tray: bool,

    /// Do not request a notification sound
    #[arg(long)]
    no_sound: bool,
}

fn parse_duration(input: &str) -> Result<Duration, String> {
    humantime::parse_duration(input).map_err(|error| error.to_string())
}

fn main() -> ExitCode {
    env_logger::Builder::from_env(
        env_logger::Env::default().default_filter_or("info"),
    )
    .format_target(false)
    .init();

    let cli = Cli::parse();
    let command = cli
        .command
        .unwrap_or_else(|| Command::Run(RunArgs::default()));

    let result = match command {
        Command::Run(args) => resolve_config_path(cli.config).and_then(|path| run(path, args)),
        Command::ConfigPath => {
            resolve_config_path(cli.config).map(|path| println!("{}", path.display()))
        }
        Command::InstallService { no_enable } => service::install(!no_enable),
        Command::UninstallService => service::uninstall(),
    };

    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            log::error!("{error:#}");
            ExitCode::FAILURE
        }
    }
}

fn resolve_config_path(from_cli: Option<PathBuf>) -> Result<PathBuf> {
    match from_cli {
        Some(path) => Ok(path),
        None => config::default_path(),
    }
}

fn run(config_path: PathBuf, args: RunArgs) -> Result<()> {
    let mut config = config::load(&config_path)
        .with_context(|| format!("loading config from {}", config_path.display()))?;

    if let Some(sit) = args.sit {
        config.sit_duration = sit;
    }
    if let Some(stand) = args.stand {
        config.stand_duration = stand;
    }
    if args.no_sound {
        config.sound = false;
    }
    config.validate()?;

    log::info!(
        "config loaded from {} (sit {}, stand {})",
        config_path.display(),
        humantime::format_duration(config.sit_duration),
        humantime::format_duration(config.stand_duration),
    );

    let controls = Controls::new();

    {
        let controls = Arc::clone(&controls);
        ctrlc::set_handler(move || {
            log::info!("interrupt received; shutting down");
            controls.request_quit();
        })
        .context("installing Ctrl-C handler")?;
    }

    let on_status = start_tray(&config, config_path, Arc::clone(&controls), args.no_tray);

    notify::send_startup(&config);
    reminder::run(&config, controls, on_status);
    Ok(())
}

/// Start the tray icon and return a callback that refreshes its phase/countdown.
/// Keeping the returned callback alive keeps the tray registered. On non-Linux
/// platforms the tray is unsupported, so this always returns `None`.
fn start_tray(
    config: &Config,
    config_path: PathBuf,
    controls: Arc<Controls>,
    no_tray: bool,
) -> Option<StatusCallback> {
    if no_tray {
        log::info!("tray icon disabled (--no-tray)");
        return None;
    }
    #[cfg(target_os = "linux")]
    {
        let handle = tray::spawn(
            controls,
            config.start_phase,
            config_path,
            config.sit_duration,
            config.stand_duration,
        )?;
        Some(Box::new(move |phase, remaining| {
            handle.set_status(phase, remaining)
        }))
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = (config, config_path, controls);
        None
    }
}
