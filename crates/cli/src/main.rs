mod client;
mod hook;
mod install;
mod migrate;

use std::process::{Command, ExitCode};
use std::time::{Duration, Instant};

use anyhow::{anyhow, Result};
use clap::{Parser, Subcommand};
use clashofmeat_core::Env;
use clashofmeat_daemon::{config, run};
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
        } => connect(env, cli.json, app, no_open, force),
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
                connect(env, cli.json, None, false, false)?;
            }
            Ok(())
        }
        Action::Uninstall { keep_data } => install::uninstall(env, keep_data),
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

fn connect(
    env: &Env,
    json_output: bool,
    app: Option<String>,
    no_open: bool,
    force: bool,
) -> Result<()> {
    let origin = app.as_deref().map(config::app_origin).transpose()?;
    let mut client = client::Client::connect(env)?;
    let status = client.request("status", json!({}))?;
    if status["paired"] == true && !force {
        if json_output {
            println!("{status}");
        } else {
            println!(
                "Clash of Meat is already connected to {}.",
                status["app_url"].as_str().unwrap_or("the app")
            );
        }
        return Ok(());
    }
    if let Some(origin) = origin {
        config::update_config(env, |config| config.app_url = Some(origin))?;
    }
    let mut events = client::Client::connect(env)?;
    events.subscribe()?;
    let pairing = client.request("pair_start", json!({}))?;
    let link = pairing["link"]
        .as_str()
        .ok_or_else(|| anyhow!("Missing pairing link"))?;
    let code = pairing["code"]
        .as_str()
        .ok_or_else(|| anyhow!("Missing pairing code"))?;
    if json_output {
        println!("{pairing}");
    } else {
        let grouped = if code.len() == 10 {
            format!("{}-{}", &code[..5], &code[5..])
        } else {
            code.to_owned()
        };
        println!("Open this link to connect Clash of Meat:\n  {link}\nYour code is {grouped}. Check that your browser shows the same code.");
    }
    if !no_open && std::env::var("CLASHOFMEAT_NO_OPEN").as_deref() != Ok("1") {
        let _ = Command::new("/usr/bin/open").arg(link).spawn();
    }
    let deadline = Instant::now() + Duration::from_secs(600);
    while Instant::now() < deadline {
        if let Some(event) = events.next_event(Duration::from_secs(1))? {
            match event["event"].as_str() {
                Some("paired") => {
                    if json_output {
                        println!("{}", event["data"]);
                    } else {
                        println!("Connected. Activity from now on is collected.");
                    }
                    return Ok(());
                }
                Some("pair_failed") => {
                    let reason = event["data"]["reason"].as_str().unwrap_or("unknown");
                    let message = match reason {
                        "declined" => "Clash of Meat: Connection refused in the browser.",
                        "expired" => "Clash of Meat: This code expired. Run the command again.",
                        _ => event["data"]["message"]
                            .as_str()
                            .unwrap_or("Connection failed."),
                    };
                    return Err(anyhow!("{message}"));
                }
                _ => {}
            }
        }
    }
    Err(anyhow!(
        "Clash of Meat: This code expired. Run the command again."
    ))
}
