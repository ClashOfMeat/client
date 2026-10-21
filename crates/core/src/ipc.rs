//! The local socket the daemon, CLI and desktop app talk over: a Unix domain socket, which
//! Windows 10 (1803) and later also provide natively.

#[cfg(unix)]
pub use std::os::unix::net::{UnixListener, UnixStream};
#[cfg(windows)]
pub use uds_windows::{UnixListener, UnixStream};
