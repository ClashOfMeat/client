//! Linking this computer to a player's account: the daemon starts a pairing and listens on
//! 127.0.0.1, the browser it opens approves it there. From another device, the player types
//! the code shown here on the app's link page instead; that is the only way for a computer
//! without a browser, such as a Linux server.

use std::io::{self, IsTerminal, Write};
use std::process::{Command, Stdio};
use std::sync::mpsc::TryRecvError;
use std::time::{Duration, Instant};

use anyhow::{anyhow, Result};
use clashofmeat_core::Env;
use clashofmeat_daemon::config;
use serde_json::{json, Value};

use crate::{client::Client, term};

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
    // Daemons before the loopback listener send only the code link.
    let link_page = pairing["link_page"].as_str().map_or_else(
        || link.split('?').next().unwrap_or(link).to_string(),
        str::to_string,
    );
    let browser = has_browser(env);
    let may_open =
        browser && !no_open && env.vars.get("CLASHOFMEAT_NO_OPEN").map(String::as_str) != Some("1");

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
    let mut enter = if may_open && interactive {
        term::enter_presses()
    } else {
        None
    };
    println!();
    let (waiting_text, below) = if !browser {
        print!("{}", headless(&link_page, code));
        (Waiting::LINKING, HEADLESS_NOTE.to_string())
    } else {
        if enter.is_some() {
            println!("{INDENT}Press Enter to link this computer in your browser…");
        } else if may_open {
            open(link);
            println!("{INDENT}Linking this computer in your browser…");
        } else {
            println!("{INDENT}Open {link} on this computer to link it.");
        }
        (Waiting::BROWSER, fallback(&link_page, code))
    };
    println!();
    let width = interactive.then(|| term::width(env).unwrap_or(DEFAULT_WIDTH));
    let mut waiting = Waiting::start(waiting_text, &below, width);
    let result = wait(&mut events, || {
        if let Some(pressed) = enter.as_ref().map(|presses| presses.try_recv()) {
            match pressed {
                Ok(()) => {
                    open(link);
                    enter = None;
                }
                Err(TryRecvError::Disconnected) => enter = None,
                Err(TryRecvError::Empty) => {}
            }
        }
        waiting.tick();
    });
    waiting.clear();
    result?;
    println!("{INDENT}✔ Linked! Your hero is on its way.");
    Ok(())
}

/// Whether this computer has a browser for the link page. A Linux computer without a graphical
/// session (a server, an SSH login) has none: the player links it from another device.
fn has_browser(env: &Env) -> bool {
    !cfg!(target_os = "linux")
        || ["DISPLAY", "WAYLAND_DISPLAY"]
            .iter()
            .any(|name| env.vars.get(*name).is_some_and(|value| !value.is_empty()))
}

/// The link page without its scheme, and the code grouped as the link page shows it.
fn page_and_code(link_page: &str, code: &str) -> (String, String) {
    let page = link_page
        .strip_prefix("https://")
        .or_else(|| link_page.strip_prefix("http://"))
        .unwrap_or(link_page);
    let code = if code.len() == 10 && code.is_ascii() {
        format!("{}-{}", &code[..5], &code[5..])
    } else {
        code.to_owned()
    };
    (page.to_owned(), code)
}

/// The way to link from a browser on another device, which can't reach this computer's
/// listener: the link page and the code to type there.
fn fallback(link_page: &str, code: &str) -> String {
    let (page, code) = page_and_code(link_page, code);
    format!("{INDENT}Linking from another device? Open {page}\n{INDENT}and type  {code}\n")
}

/// What a computer without a browser shows instead of "Press Enter": the code to type on another device.
fn headless(link_page: &str, code: &str) -> String {
    let (page, code) = page_and_code(link_page, code);
    format!("{INDENT}To link this computer, open {page} on your phone or laptop\n{INDENT}and type  {code}\n")
}

/// The daemon keeps polling for the approval until the code expires, whatever happens to this command.
const HEADLESS_NOTE: &str =
    "  The code expires in 10 minutes. Closing this terminal doesn't cancel it:\n  \
                             the background service finishes linking.\n";

/// The waiting line above the text below it: a spinner redrawn in place on a terminal,
/// printed once otherwise. `width` is `None` when stdout isn't a terminal.
struct Waiting {
    text: &'static str,
    /// Rows from the cursor up to the spinner line.
    rows: Option<usize>,
    frame: usize,
}

impl Waiting {
    const BROWSER: &'static str = "Waiting for your browser…";
    const LINKING: &'static str = "Waiting for you to link this computer…";
    const FRAMES: [char; 10] = ['⠋', '⠙', '⠹', '⠸', '⠼', '⠴', '⠦', '⠧', '⠇', '⠏'];

    fn start(text: &'static str, below: &str, width: Option<usize>) -> Self {
        let Some(width) = width else {
            println!("{INDENT}{text}\n");
            print!("{below}");
            return Self {
                text,
                rows: None,
                frame: 0,
            };
        };
        println!("\n");
        print!("{below}");
        let _ = io::stdout().flush();
        // The spinner line, the blank line, then the text below as the terminal wraps it.
        let wrapped: usize = below
            .lines()
            .map(|line| line.chars().count().max(1).div_ceil(width.max(1)))
            .sum();
        Self {
            text,
            rows: Some(2 + wrapped),
            frame: 0,
        }
    }

    fn tick(&mut self) {
        let Some(rows) = self.rows else {
            return;
        };
        let frame = Self::FRAMES[self.frame % Self::FRAMES.len()];
        self.frame += 1;
        print!(
            "\x1b[{rows}A\r\x1b[2K{INDENT}{frame} {}\x1b[{rows}B\r",
            self.text
        );
        let _ = io::stdout().flush();
    }

    /// Erases the spinner and the fallback, for the result to take their place.
    fn clear(&self) {
        if let Some(rows) = self.rows {
            print!("\x1b[{rows}A\r\x1b[J");
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

/// Opens the link page in this computer's browser: `xdg-open` in a Linux desktop session.
fn open(link: &str) {
    let mut command = if cfg!(target_os = "linux") {
        Command::new("xdg-open")
    } else {
        Command::new("/usr/bin/open")
    };
    let _ = command
        .arg(link)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_fallback_names_the_link_page_and_the_grouped_code() {
        assert_eq!(
            fallback("https://app.clashofmeat.com/link", "ABCDE12345"),
            "  Linking from another device? Open app.clashofmeat.com/link\n  and type  ABCDE-12345\n"
        );
        assert!(fallback("http://localhost:3110/link", "odd").ends_with("and type  odd\n"));
    }

    #[test]
    fn a_computer_without_a_browser_is_linked_from_another_device() {
        assert_eq!(
            headless("https://app.clashofmeat.com/link", "ABCDE12345"),
            "  To link this computer, open app.clashofmeat.com/link on your phone or laptop\n  and type  ABCDE-12345\n"
        );
    }

    #[test]
    fn only_a_linux_computer_without_a_display_has_no_browser() {
        let mut env = Env::for_home("/home/me");
        assert_eq!(has_browser(&env), !cfg!(target_os = "linux"));
        env.vars.insert("DISPLAY".into(), String::new());
        assert_eq!(has_browser(&env), !cfg!(target_os = "linux"));
        env.vars
            .insert("WAYLAND_DISPLAY".into(), "wayland-0".into());
        assert!(has_browser(&env));
    }
}
