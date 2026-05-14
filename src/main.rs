// Apollo Fleet — Windows-only tray supervisor for N isolated Apollo (Sunshine) instances.
#![cfg(windows)]
#![windows_subsystem = "windows"]

mod config;
mod fleet;
mod launcher;
mod paths;
mod propagate;
mod seat;
mod shared_state;
mod singleton;
mod tray;
mod win;

use std::path::PathBuf;
use std::process::ExitCode;

use anyhow::{Context, Result};
use clap::Parser;

const EMBEDDED_SEATS_EXAMPLE: &str = include_str!("../resources/seats.toml.example");

#[derive(Parser, Debug)]
#[command(name = "apollo-fleet", version, about = "Apollo Fleet — tray supervisor")]
struct Cli {
    /// Path to TOML config. Defaults to %APPDATA%/apollo-fleet/seats.toml.
    #[arg(short, long)]
    config: Option<PathBuf>,

    /// Don't validate that audio_sink values match active Windows endpoints.
    #[arg(long)]
    skip_sink_check: bool,

    /// Run the supervisor in the foreground (no tray icon). Used for testing.
    #[arg(long)]
    supervisor: bool,

    /// Generate per-seat configs without spawning Apollo.
    #[arg(long)]
    dry_run: bool,

    /// Print active Windows audio render endpoints and exit.
    #[arg(long)]
    list_sinks: bool,

    /// Launcher mode used by Apollo's apps.json — moves a spawned window to the seat's
    /// virtual display. Format: --launch <seat_name> -- <program> [args...]
    #[arg(long, value_name = "SEAT")]
    launch: Option<String>,

    /// Positional args after `--launch <seat> --`: the program (and args) to spawn.
    #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
    launch_args: Vec<String>,
}

fn main() -> ExitCode {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info"))
        .target(env_logger::Target::Stdout)
        .init();

    let cli = Cli::parse();

    let result = run(cli);
    match result {
        Ok(code) => ExitCode::from(code),
        Err(e) => {
            log::error!("{e:#}");
            ExitCode::from(1)
        }
    }
}

fn run(cli: Cli) -> Result<u8> {
    if cli.list_sinks {
        match win::audio::list_audio_endpoints() {
            Ok(endpoints) if !endpoints.is_empty() => {
                for name in endpoints {
                    println!("{name}");
                }
                return Ok(0);
            }
            Ok(_) => {
                eprintln!("(no endpoints found)");
                return Ok(1);
            }
            Err(e) => {
                eprintln!("could not enumerate audio endpoints: {e:#}");
                return Ok(1);
            }
        }
    }

    if let Some(seat_name) = cli.launch {
        let argv: Vec<String> = cli
            .launch_args
            .into_iter()
            .skip_while(|a| a == "--")
            .collect();
        return Ok(launcher::run(&seat_name, &argv) as u8);
    }

    let config_path = paths::resolve_config_path(cli.config.as_deref())?;
    paths::ensure_user_config(&config_path, EMBEDDED_SEATS_EXAMPLE)
        .context("could not seed user config")?;

    if cli.dry_run {
        let fleet = fleet::build(&config_path, true)?;
        for seat in fleet.seats() {
            println!("[dry-run] {} -> {}", seat.name, seat.config_file.display());
        }
        return Ok(0);
    }

    if cli.supervisor {
        let mut fleet = fleet::build(&config_path, cli.skip_sink_check)?;
        fleet.run_blocking()?;
        return Ok(0);
    }

    let _lock = match singleton::acquire() {
        Some(lock) => lock,
        None => {
            // Another instance already owns the tray.
            return Ok(0);
        }
    };

    tray::run(config_path, cli.skip_sink_check)?;
    Ok(0)
}
