//! Linking this computer to a player's account: the daemon starts a pairing, the player
//! types the code shown here on the app's link page.

use std::io::{self, IsTerminal, Write};
use std::process::Command;
use std::sync::mpsc::TryRecvError;
use std::time::{Duration, Instant};

use anyhow::{anyhow, Result};
use clashofmeat_core::Env;
use clashofmeat_daemon::config;
use serde_json::{json, Value};

use crate::{bigtext, client::Client, term};

const PAIRING_TIMEOUT: Duration = Duration::from_secs(600);
const TICK: Duration = Duration::from_millis(100);
const EXPIRED: &str = "Clash of Meat: This code expired. Run the command again.";
const INDENT: &str = "  ";
/// Width to assume for a terminal that doesn't report one.
const DEFAULT_WIDTH: usize = 80;

/// `clashofmeat connect`.
pub fn connect(
    env: &Env,
    json_output: bool,
    app: Option<String>,
    no_open: bool,
    force: bool,
) -> Result<()> {
    let origin = app.as_deref().map(config::app_origin).transpose()?;
    let mut client = Client::connect(env)?;
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
    if status["paired"] == true {
        // Re-pairing: revoke the old token on the app that issued it, before any new address.
        clashofmeat_daemon::sync::revoke_installation(env, env!("CARGO_PKG_VERSION"))?;
    }
    if let Some(origin) = origin {
        config::update_config(env, |config| config.app_url = Some(origin))?;
    }
    pair(env, client, json_output, no_open)
}

/// The end of `clashofmeat install`: link the computer unless it already is.
pub fn after_install(env: &Env, json_output: bool) -> Result<()> {
    let Ok(mut client) = Client::connect(env) else {
        eprintln!(
            "The Clash of Meat background service isn't running yet, so this computer isn't linked. \
             Run `clashofmeat connect` to link it."
        );
        return Ok(());
    };
    let status = client.request("status", json!({}))?;
    if status["paired"] == true {
        if !json_output {
            println!(
                "This computer is already linked to {}.",
                status["app_url"].as_str().unwrap_or("the app")
            );
        }
        return Ok(());
    }
    pair(env, client, json_output, false)
}

fn pair(env: &Env, mut client: Client, json_output: bool, no_open: bool) -> Result<()> {
    let mut events = Client::connect(env)?;
    events.subscribe()?;
    let pairing = client.request("pair_start", json!({}))?;
    let link = pairing["link"]
        .as_str()
        .ok_or_else(|| anyhow!("Missing pairing link"))?;
    let code = pairing["code"]
        .as_str()
        .ok_or_else(|| anyhow!("Missing pairing code"))?;
    let may_open = !no_open && env.vars.get("CLASHOFMEAT_NO_OPEN").map(String::as_str) != Some("1");

    if json_output {
        println!("{pairing}");
        if may_open {
            open(link);
        }
        let paired = wait(&mut events, || {})?;
        println!("{paired}");
        return Ok(());
    }

    let interactive = io::stdout().is_terminal();
    let width = interactive.then(|| term::width(env).unwrap_or(DEFAULT_WIDTH));
    print!("{}", code_screen(code, width));
    let mut enter = if may_open && interactive {
        term::enter_presses()
    } else {
        None
    };
    if enter.is_some() {
        println!("{INDENT}Press Enter to open the link page (or open {link})");
    } else if may_open {
        open(link);
        println!("{INDENT}Opening the link page in your browser (or open {link})");
    } else {
        println!("{INDENT}Open {link} and type this code.");
    }
    println!();

    let mut waiting = Waiting::new(interactive);
    if enter.is_none() {
        waiting.start();
    }
    let result = wait(&mut events, || {
        if let Some(pressed) = enter.as_ref().map(|presses| presses.try_recv()) {
            match pressed {
                Ok(()) => {
                    open(link);
                    enter = None;
                    waiting.start();
                }
                Err(TryRecvError::Disconnected) => {
                    enter = None;
                    waiting.start();
                }
                Err(TryRecvError::Empty) => {}
            }
        }
        waiting.tick();
    });
    waiting.clear();
    result?;
    println!("✔ Linked! Your hero is on its way.");
    Ok(())
}

/// The heading, the code in block letters when the terminal is wide enough, and the plain
/// code on its own line to copy. `width` is `None` when stdout isn't a terminal.
fn code_screen(code: &str, width: Option<usize>) -> String {
    let grouped = bigtext::group(code);
    let mut screen = format!("\n{INDENT}Your link code\n\n");
    let big = width.and_then(|columns| {
        bigtext::render(&grouped).filter(|rows| INDENT.len() + bigtext::width(rows) <= columns)
    });
    match big {
        Some(rows) => {
            for row in &rows {
                screen.push_str(&format!("{INDENT}{row}\n"));
            }
            let center = bigtext::width(&rows).saturating_sub(grouped.len()) / 2;
            screen.push_str(&format!("\n{INDENT}{}{grouped}\n\n", " ".repeat(center)));
        }
        None => screen.push_str(&format!("{INDENT}{INDENT}{grouped}\n\n")),
    }
    screen
}

/// The "waiting for the browser" line: a spinner on a terminal, a single line otherwise.
struct Waiting {
    interactive: bool,
    started: bool,
    frame: usize,
}

impl Waiting {
    const TEXT: &'static str = "Waiting for you to type the code on the link page…";
    const FRAMES: [char; 10] = ['⠋', '⠙', '⠹', '⠸', '⠼', '⠴', '⠦', '⠧', '⠇', '⠏'];

    fn new(interactive: bool) -> Self {
        Self {
            interactive,
            started: false,
            frame: 0,
        }
    }

    fn start(&mut self) {
        if self.started {
            return;
        }
        self.started = true;
        if !self.interactive {
            println!("{}", Self::TEXT);
        }
    }

    fn tick(&mut self) {
        if !(self.started && self.interactive) {
            return;
        }
        let frame = Self::FRAMES[self.frame % Self::FRAMES.len()];
        self.frame += 1;
        print!("\r\x1b[2K{frame} {}", Self::TEXT);
        let _ = io::stdout().flush();
    }

    fn clear(&self) {
        if self.started && self.interactive {
            print!("\r\x1b[2K");
            let _ = io::stdout().flush();
        }
    }
}

/// Waits for the daemon's `paired` event, calling `tick` between reads.
fn wait(events: &mut Client, mut tick: impl FnMut()) -> Result<Value> {
    let deadline = Instant::now() + PAIRING_TIMEOUT;
    while Instant::now() < deadline {
        tick();
        let Some(event) = events.next_event(TICK)? else {
            continue;
        };
        match event["event"].as_str() {
            Some("paired") => return Ok(event["data"].clone()),
            Some("pair_failed") => {
                let message = match event["data"]["reason"].as_str().unwrap_or("unknown") {
                    "declined" => "Clash of Meat: Connection refused in the browser.",
                    "expired" => EXPIRED,
                    _ => event["data"]["message"]
                        .as_str()
                        .unwrap_or("Connection failed."),
                };
                return Err(anyhow!("{message}"));
            }
            _ => {}
        }
    }
    Err(anyhow!(EXPIRED))
}

fn open(link: &str) {
    let _ = Command::new("/usr/bin/open").arg(link).spawn();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wide_terminals_get_block_letters_and_the_plain_code() {
        let screen = code_screen("ABCDE12345", Some(80));
        let lines: Vec<_> = screen.lines().collect();
        assert_eq!(lines[1], "  Your link code");
        assert!(lines[3].contains('█'));
        assert_eq!(lines[3..8].len(), 5);
        assert_eq!(lines[9].trim(), "ABCDE-12345");
        assert!(lines.iter().all(|line| line.chars().count() <= 80));
    }

    #[test]
    fn narrow_terminals_and_pipes_get_the_plain_code() {
        for width in [Some(40), None] {
            let screen = code_screen("ABCDE12345", width);
            assert!(!screen.contains('█'));
            assert_eq!(screen, "\n  Your link code\n\n    ABCDE-12345\n\n");
        }
    }

    #[test]
    fn codes_without_glyphs_fall_back_to_plain() {
        assert!(!code_screen("ghijk", Some(200)).contains('█'));
    }
}
