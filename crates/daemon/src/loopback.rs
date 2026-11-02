//! The browser's way back to this computer while it links. The app's link page sends its
//! approval (a short-lived grant) to `127.0.0.1` only, so only the computer whose browser
//! approved can exchange it: a phished link lands on the victim's own computer, where nothing
//! waits for it. Typing the code stays the fallback for a browser on another device.

use std::io::{self, BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

const POLL: Duration = Duration::from_millis(100);

/// Exchanges a grant; `Ok` is where to send the browser next, `Err` what to tell the player.
pub type Deliver = dyn Fn(&str) -> Result<String, String> + Send + Sync;

/// Listens on a free local port until dropped, or until a grant linked the computer.
pub struct Loopback {
    pub port: u16,
    stop: Arc<AtomicBool>,
}

impl Loopback {
    pub fn start(deliver: Arc<Deliver>) -> io::Result<Self> {
        let listener = TcpListener::bind(("127.0.0.1", 0))?;
        listener.set_nonblocking(true)?;
        let port = listener.local_addr()?.port();
        let stop = Arc::new(AtomicBool::new(false));
        let stopped = stop.clone();
        thread::spawn(move || {
            while !stopped.load(Ordering::Relaxed) {
                match listener.accept() {
                    // One thread a connection: browsers open idle spare connections.
                    Ok((stream, _)) => {
                        let (deliver, stopped) = (deliver.clone(), stopped.clone());
                        thread::spawn(move || {
                            if answer(stream, deliver.as_ref()) {
                                stopped.store(true, Ordering::Relaxed);
                            }
                        });
                    }
                    Err(error) if error.kind() == io::ErrorKind::WouldBlock => thread::sleep(POLL),
                    Err(_) => break,
                }
            }
        });
        Ok(Self { port, stop })
    }
}

impl Drop for Loopback {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

/// Answers one request; true once its grant linked the computer.
fn answer(mut stream: TcpStream, deliver: &Deliver) -> bool {
    let _ = stream.set_nonblocking(false);
    let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
    let mut reader = BufReader::new((&stream).take(8192));
    let mut request = String::new();
    if reader.read_line(&mut request).is_err() {
        return false;
    }
    // Read the headers too: closing with unread input resets the connection before the
    // browser reads the answer.
    let mut header = String::new();
    while reader.read_line(&mut header).is_ok_and(|read| read > 2) {
        header.clear();
    }
    let (linked, response) = match grant_in(&request) {
        None => (false, page(404, "Not found", "Nothing here.")),
        Some(grant) => match deliver(grant) {
            Ok(next) => (
                true,
                format!("HTTP/1.1 302 Found\r\nLocation: {next}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"),
            ),
            Err(message) => (false, page(400, "Not linked", &message)),
        },
    };
    let _ = stream.write_all(response.as_bytes());
    linked
}

/// The grant in `GET /done?grant=… HTTP/1.1`, when it looks like one.
pub(crate) fn grant_in(request_line: &str) -> Option<&str> {
    let target = request_line.strip_prefix("GET ")?.split(' ').next()?;
    let grant = target.strip_prefix("/done?grant=")?;
    let well_formed = !grant.is_empty()
        && grant.len() <= 512
        && grant
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'));
    well_formed.then_some(grant)
}

fn page(status: u16, title: &str, message: &str) -> String {
    let escaped = message
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;");
    let body = format!(
        "<!doctype html><meta charset=utf-8><title>Clash of Meat</title>\
         <body style=\"font:16px system-ui;margin:3rem auto;max-width:32rem\">\
         <h1>{title}</h1><p>{escaped}</p><p>Go back to your terminal.</p>"
    );
    format!(
        "HTTP/1.1 {status} {title}\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_done_request_with_a_plain_grant_counts() {
        assert_eq!(grant_in("GET /done?grant=ab-c_D.e9 HTTP/1.1\r\n"), Some("ab-c_D.e9"));
        for line in [
            "POST /done?grant=abc HTTP/1.1",
            "GET /done?grant= HTTP/1.1",
            "GET /done?grant=a%20b HTTP/1.1",
            "GET /done?grant=a&next=x HTTP/1.1",
            "GET /favicon.ico HTTP/1.1",
        ] {
            assert_eq!(grant_in(line), None, "{line}");
        }
    }

    #[test]
    fn a_grant_redirects_the_browser_and_stops_listening() {
        let deliver: Arc<Deliver> = Arc::new(|grant| match grant {
            "good" => Ok("http://app.test/link?linked=1".into()),
            _ => Err("This link <expired>.".into()),
        });
        let loopback = Loopback::start(deliver).unwrap();
        let get = |path: &str| {
            let mut stream = TcpStream::connect(("127.0.0.1", loopback.port)).unwrap();
            write!(stream, "GET {path} HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n").unwrap();
            let mut response = String::new();
            stream.read_to_string(&mut response).unwrap();
            response
        };
        let refused = get("/done?grant=bad");
        assert!(refused.starts_with("HTTP/1.1 400"));
        assert!(refused.contains("This link &lt;expired&gt;."));
        assert!(get("/favicon.ico").starts_with("HTTP/1.1 404"));
        let linked = get("/done?grant=good");
        assert!(linked.starts_with("HTTP/1.1 302"));
        assert!(linked.contains("Location: http://app.test/link?linked=1\r\n"));
    }
}
