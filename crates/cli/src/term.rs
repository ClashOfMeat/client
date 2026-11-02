//! The little terminal handling the link screen needs, without a terminal library.

use std::sync::mpsc::Receiver;

use clashofmeat_core::Env;

/// Columns of the terminal behind stdout: `COLUMNS` when set, else the tty's own size.
pub fn width(env: &Env) -> Option<usize> {
    if let Some(columns) = env
        .vars
        .get("COLUMNS")
        .and_then(|value| value.trim().parse::<usize>().ok())
        .filter(|columns| *columns > 0)
    {
        return Some(columns);
    }
    tty_width()
}

#[cfg(unix)]
fn tty_width() -> Option<usize> {
    // SAFETY: TIOCGWINSZ only writes a `winsize` into the struct we pass.
    let mut size: libc::winsize = unsafe { std::mem::zeroed() };
    let ok = unsafe { libc::ioctl(libc::STDOUT_FILENO, libc::TIOCGWINSZ, &mut size) } == 0;
    (ok && size.ws_col > 0).then_some(usize::from(size.ws_col))
}

#[cfg(not(unix))]
fn tty_width() -> Option<usize> {
    None
}

/// Sends once when Enter is pressed on the controlling terminal. Under `curl … | sh`, stdin
/// is the download, so this reads `/dev/tty`. `None` when there is no terminal to read; the
/// sender is dropped without sending on end of file.
#[cfg(unix)]
pub fn enter_presses() -> Option<Receiver<()>> {
    use std::io::{BufRead, BufReader};

    let tty = std::fs::File::open("/dev/tty").ok()?;
    let (sender, receiver) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut line = String::new();
        if BufReader::new(tty)
            .read_line(&mut line)
            .is_ok_and(|read| read > 0)
        {
            let _ = sender.send(());
        }
    });
    Some(receiver)
}

#[cfg(not(unix))]
pub fn enter_presses() -> Option<Receiver<()>> {
    None
}
