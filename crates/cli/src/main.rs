mod client;
mod connect;
mod hook;
mod install;
mod linux;
mod migrate;
mod term;
mod update;

use std::process::ExitCode;

use anyhow::{anyhow, Result};
use clap::{Parser, Subcommand};
use clashofmeat_core::Env;
use clashofmeat_daemon::run;
use serde_json::{json, Value};

#[derive(Parser)]
#[command(version, about = "Clash of Meat desktop client")]
struct Cli {
    #[arg(long, global = true)]
    json: bool,
    #[command(subcommand)]
    command: Action,
}

#[derive(Subcommand)]
enum Action {
    Daemon,
    Status,
    Sync,
    Connect {
        #[arg(long)]
        app: Option<String>,
        #[arg(long)]
        no_open: bool,
        #[arg(long)]
        force: bool,
    },
    Install {
        /// The web app; defaults to the one this build was made for.
        #[arg(long)]
        app: Option<String>,
        #[arg(long)]
        no_connect: bool,
        #[arg(long)]
        no_launch_agent: bool,
    },
    Uninstall {
        #[arg(long)]
        keep_data: bool,
    },
    /// Linux: install the latest release (a daily timer runs this with --quiet).
    Update {
        #[arg(long)]
        quiet: bool,
    },
    #[command(hide = true)]
    CursorHook,
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    if matches!(cli.command, Action::CursorHook) {
        hook::run();
        return ExitCode::SUCCESS;
    }
    let env = Env::from_process();
    match execute(cli, &env) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("{error}");
            ExitCode::from(if error.downcast_ref::<client::NotRunning>().is_some() {
                3
            } else {
                1
            })
        }
    }
}

fn execute(cli: Cli, env: &Env) -> Result<()> {
    match cli.command {
        Action::Daemon => run::run_forever(run::RunOptions {
            env: env.clone(),
            version: env!("CARGO_PKG_VERSION").to_owned(),
        }),
        Action::Status => {
            let mut client = client::Client::connect(env)?;
            let status = client.request("status", json!({}))?;
            if cli.json {
                println!("{status}");
            } else {
                print_status(&status);
            }
            Ok(())
        }
        Action::Sync => {
            let mut client = client::Client::connect(env)?;
            let result = client.request("sync_now", json!({}))?;
            if cli.json {
                println!("{result}");
            } else {
                println!("Sync queued.");
            }
            Ok(())
        }
        Action::Connect {
            app,
            no_open,
            force,
        } => connect::connect(env, cli.json, app, no_open, force),
        Action::Install {
            app,
            no_connect,
            no_launch_agent,
        } => {
            let app = app
                .or_else(|| option_env!("CLASHOFMEAT_DEFAULT_APP_URL").map(str::to_owned))
                .ok_or_else(|| anyhow!("pass --app <url>: this build has no default app"))?;
            install::install(env, &app, no_launch_agent)?;
            if !no_connect {
                connect::after_install(env, cli.json)?;
            }
            Ok(())
        }
        Action::Uninstall { keep_data } => install::uninstall(env, keep_data),
        Action::Update { quiet } => update::update(env, quiet),
        Action::CursorHook => Ok(()),
    }
}

fn print_status(status: &Value) {
    if status["paired"] == true {
        println!(
            "Connected to {}.",
            status["app_url"].as_str().unwrap_or("unknown app")
        );
    } else {
        println!("Not connected. Run `clashofmeat connect`.");
    }
    if let Some(blocked) = status["blocked"].as_str() {
        println!("Uploads blocked: {blocked}");
    }
    if status["reauth"] == true {
        println!("Connection expired. Run `clashofmeat connect` again.");
    }
    if let Some(last) = status["last_sync_at"].as_str() {
        println!("Last sync {last}");
    }
    if let Some(providers) = status["providers"].as_array() {
        for provider in providers {
            let health = &provider["health"];
            println!(
                "  {}: detected={}, enabled={}, records={}, malformed={}, oversized={}, errors={}",
                provider["name"].as_str().unwrap_or("unknown"),
                provider["detected"],
                provider["enabled"],
                health["records"],
                health["malformed"],
                health["oversized"],
                health["errors"]
            );
        }
    }
}
