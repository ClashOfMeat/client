//! Linux: the systemd user service, the session-only fallback and `clashofmeat update`.
#![cfg(target_os = "linux")]

use std::fs;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::os::unix::fs::{symlink, PermissionsExt};
use std::os::unix::net::UnixListener;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::thread;

use base64::Engine as _;
use tempfile::TempDir;

struct Fixture {
    root: TempDir,
    exe: PathBuf,
    systemctl: PathBuf,
    log: PathBuf,
    _socket: UnixListener,
}

impl Fixture {
    /// A home with the CLI where install.sh puts it and a fake `systemctl` that logs its calls
    /// (`show-environment` fails when `systemd` is false).
    fn new(systemd: bool) -> Self {
        let root = tempfile::tempdir().unwrap();
        let home = root.path();
        let exe = home.join(".local/share/clashofmeat/bin/clashofmeat");
        fs::create_dir_all(exe.parent().unwrap()).unwrap();
        fs::copy(env!("CARGO_BIN_EXE_clashofmeat"), &exe).unwrap();
        fs::create_dir_all(home.join(".local/bin")).unwrap();
        symlink(&exe, home.join(".local/bin/clashofmeat")).unwrap();
        let systemctl = home.join("systemctl");
        let log = home.join("systemctl.log");
        let fail = if systemd {
            ""
        } else {
            "[ \"$2\" = show-environment ] && exit 1\n"
        };
        fs::write(
            &systemctl,
            format!(
                "#!/bin/sh\nprintf '%s\\n' \"$*\" >> \"{}\"\n{fail}exit 0\n",
                log.display()
            ),
        )
        .unwrap();
        fs::set_permissions(&systemctl, fs::Permissions::from_mode(0o755)).unwrap();
        fs::create_dir_all(home.join(".clashofmeat")).unwrap();
        let socket = UnixListener::bind(home.join(".clashofmeat/daemon.sock")).unwrap();
        Self {
            root,
            exe,
            systemctl,
            log,
            _socket: socket,
        }
    }

    fn home(&self) -> &Path {
        self.root.path()
    }

    fn units(&self) -> PathBuf {
        self.home().join(".config/systemd/user")
    }

    fn run(&self, args: &[&str], vars: &[(&str, &str)]) -> Output {
        Command::new(&self.exe)
            .args(args)
            .env_clear()
            .env("HOME", self.home())
            .env("PATH", "/usr/bin:/bin")
            .env("CLASHOFMEAT_SYSTEMCTL", &self.systemctl)
            .env("CLASHOFMEAT_NO_OPEN", "1")
            .envs(vars.iter().copied())
            .output()
            .unwrap()
    }

    fn calls(&self) -> Vec<String> {
        fs::read_to_string(&self.log)
            .unwrap_or_default()
            .lines()
            .map(str::to_owned)
            .collect()
    }
}

fn assert_success(output: &Output) {
    assert!(
        output.status.success(),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn install_writes_the_user_service_and_uninstall_removes_everything() {
    let fixture = Fixture::new(true);
    let units = fixture.units();
    // The Node collector's timer, left from an earlier install.
    fs::create_dir_all(&units).unwrap();
    fs::write(units.join("clashofmeat-collector.timer"), "[Timer]\n").unwrap();
    fs::write(units.join("clashofmeat-collector.service"), "[Service]\n").unwrap();

    let args = ["install", "--app", "https://example.com", "--no-connect"];
    let output = fixture.run(&args, &[("CODEX_HOME", "/home/me/codex")]);
    assert_success(&output);
    let service = fs::read_to_string(units.join("clashofmeat.service")).unwrap();
    let exe = fixture.exe.canonicalize().unwrap();
    assert!(
        service.contains(&format!("ExecStart=\"{}\" daemon\n", exe.display())),
        "{service}"
    );
    assert!(service.contains("Environment=\"CODEX_HOME=/home/me/codex\"\n"));
    assert!(service.contains("Environment=\"PATH=/usr/bin:/bin\"\n"));
    let update = fs::read_to_string(units.join("clashofmeat-update.service")).unwrap();
    assert!(update.contains(&format!("ExecStart=\"{}\" update --quiet\n", exe.display())));
    assert!(fs::read_to_string(units.join("clashofmeat-update.timer"))
        .unwrap()
        .contains("OnCalendar=daily"));
    assert!(!units.join("clashofmeat-collector.timer").exists());
    assert!(!units.join("clashofmeat-collector.service").exists());

    let calls = fixture.calls();
    assert!(calls.contains(
        &"--user disable --now clashofmeat-collector.timer clashofmeat-collector.service".into()
    ));
    let tail: Vec<_> = calls
        .iter()
        .rev()
        .take(5)
        .rev()
        .map(String::as_str)
        .collect();
    assert_eq!(
        tail,
        [
            "--user show-environment",
            "--user daemon-reload",
            "--user enable clashofmeat.service clashofmeat-update.timer",
            "--user restart clashofmeat.service",
            "--user start clashofmeat-update.timer",
        ]
    );

    // Idempotent: the same units, the service restarted again.
    assert_success(&fixture.run(&args, &[("CODEX_HOME", "/home/me/codex")]));
    assert_eq!(
        fs::read_to_string(units.join("clashofmeat.service")).unwrap(),
        service
    );

    assert_success(&fixture.run(&["uninstall"], &[]));
    assert!(fixture
        .calls()
        .contains(&"--user disable --now clashofmeat-update.timer clashofmeat.service".into()));
    for unit in [
        "clashofmeat.service",
        "clashofmeat-update.service",
        "clashofmeat-update.timer",
    ] {
        assert!(!units.join(unit).exists(), "{unit} left behind");
    }
    assert!(fs::symlink_metadata(fixture.home().join(".local/bin/clashofmeat")).is_err());
    assert!(!fixture.home().join(".local/share/clashofmeat").exists());
}

#[test]
fn without_systemd_install_starts_the_daemon_for_this_session() {
    let fixture = Fixture::new(false);
    let output = fixture.run(
        &["install", "--app", "https://example.com", "--no-connect"],
        &[],
    );
    assert_success(&output);
    assert!(String::from_utf8_lossy(&output.stdout).contains("No systemd user session"));
    assert!(!fixture.units().join("clashofmeat.service").exists());
    assert_eq!(fixture.calls(), ["--user show-environment"]);
}

/// A signed release archive holding a stand-in `clashofmeat` that reports `version`.
fn release(version: &str, file: &str, key: &minisign::KeyPair) -> (Vec<u8>, String) {
    let mut archive = Vec::new();
    {
        let gz = flate2::write::GzEncoder::new(&mut archive, flate2::Compression::default());
        let mut tar = tar::Builder::new(gz);
        let script = format!("#!/bin/sh\necho clashofmeat-cli {version}\n");
        let mut header = tar::Header::new_gnu();
        header.set_size(script.len() as u64);
        header.set_mode(0o755);
        header.set_cksum();
        tar.append_data(&mut header, "clashofmeat", script.as_bytes())
            .unwrap();
        tar.into_inner().unwrap().finish().unwrap();
    }
    let signature = minisign::sign(
        Some(&key.pk),
        &key.sk,
        std::io::Cursor::new(&archive),
        Some(&format!("timestamp:1\tfile:{file}\tversion:{version}")),
        Some("signature from tauri secret key"),
    )
    .unwrap();
    let encoded = base64::engine::general_purpose::STANDARD.encode(signature.to_string());
    (archive, encoded)
}

fn update_fixture(origin: &str) -> Fixture {
    let fixture = Fixture::new(true);
    fs::write(
        fixture.home().join(".clashofmeat/config.json"),
        format!("{{\"appUrl\":\"{origin}\"}}\n"),
    )
    .unwrap();
    fixture
}

fn offer(origin: &str, version: &str, file: &str, signature: &str) -> String {
    let arch = std::env::consts::ARCH;
    format!(
        "{{\"version\":\"{version}\",\"platforms\":{{\"linux-{arch}\":{{\"url\":\"{origin}/download/{file}\",\"signature\":\"{signature}\"}}}}}}"
    )
}

/// `clashofmeat update` checks a new binary by its `--version` line; the stand-in above prints the
/// same shape as the real CLI.
#[test]
fn version_line_is_what_update_expects() {
    let output = Command::new(env!("CARGO_BIN_EXE_clashofmeat"))
        .arg("--version")
        .output()
        .unwrap();
    assert_eq!(
        String::from_utf8_lossy(&output.stdout).trim(),
        format!("clashofmeat-cli {}", env!("CARGO_PKG_VERSION"))
    );
}

#[test]
fn update_installs_a_signed_release() {
    let key = minisign::KeyPair::generate_unencrypted_keypair().unwrap();
    let file = format!("clashofmeat_99.0.0_linux-{}.tar.gz", std::env::consts::ARCH);
    let (archive, signature) = release("99.0.0", &file, &key);
    let origin = serve(
        |origin| Some(offer(origin, "99.0.0", &file, &signature)),
        archive,
    );
    let fixture = update_fixture(&origin);
    let pubkey = key.pk.to_base64();
    let output = fixture.run(&["update"], &[("CLASHOFMEAT_UPDATE_PUBKEY", &pubkey)]);
    assert_success(&output);
    assert!(String::from_utf8_lossy(&output.stdout).contains("to 99.0.0"));
    let replaced = Command::new(&fixture.exe)
        .arg("--version")
        .output()
        .unwrap();
    assert_eq!(
        String::from_utf8_lossy(&replaced.stdout).trim(),
        "clashofmeat-cli 99.0.0"
    );
    // The service unit is absent here, so nothing was restarted.
    assert!(fixture.calls().iter().all(|call| !call.contains("restart")));
}

#[test]
fn update_refuses_another_key_or_version() {
    let key = minisign::KeyPair::generate_unencrypted_keypair().unwrap();
    let other = minisign::KeyPair::generate_unencrypted_keypair().unwrap();
    let file = format!("clashofmeat_99.0.0_linux-{}.tar.gz", std::env::consts::ARCH);
    let (archive, signature) = release("99.0.0", &file, &key);
    let origin = serve(
        |origin| Some(offer(origin, "99.0.0", &file, &signature)),
        archive.clone(),
    );
    let fixture = update_fixture(&origin);
    let before = fs::read(&fixture.exe).unwrap();
    let output = fixture.run(
        &["update"],
        &[("CLASHOFMEAT_UPDATE_PUBKEY", &other.pk.to_base64())],
    );
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("not signed"));
    assert_eq!(fs::read(&fixture.exe).unwrap(), before);

    // A genuine signature for 99.0.0 offered as 99.0.1.
    let origin = serve(
        |origin| Some(offer(origin, "99.0.1", &file, &signature)),
        archive,
    );
    let fixture = update_fixture(&origin);
    let output = fixture.run(
        &["update"],
        &[("CLASHOFMEAT_UPDATE_PUBKEY", &key.pk.to_base64())],
    );
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("another file or version"));
}

#[test]
fn update_reports_up_to_date() {
    let origin = serve(|_| None, Vec::new());
    let fixture = update_fixture(&origin);
    let output = fixture.run(&["update"], &[]);
    assert_success(&output);
    assert!(String::from_utf8_lossy(&output.stdout).contains("is up to date"));
    let quiet = fixture.run(&["update", "--quiet"], &[]);
    assert_success(&quiet);
    assert!(quiet.stdout.is_empty());
}

#[test]
fn update_refuses_a_copy_install_sh_did_not_put_in_place() {
    let fixture = Fixture::new(true);
    let output = Command::new(env!("CARGO_BIN_EXE_clashofmeat"))
        .arg("update")
        .env_clear()
        .env("HOME", fixture.home())
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("was not installed by install.sh"));
}

/// Serves the update endpoint (the offer `offer` builds from the server's origin, or 204 when it
/// gives None) and the archive, one request at a time. Returns the origin.
fn serve(offer: impl FnOnce(&str) -> Option<String>, archive: Vec<u8>) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    let offer = offer(&origin);
    thread::spawn(move || {
        for stream in listener.incoming() {
            let mut stream = stream.unwrap();
            let mut line = String::new();
            BufReader::new(stream.try_clone().unwrap())
                .read_line(&mut line)
                .unwrap();
            let path = line.split(' ').nth(1).unwrap_or("/").to_owned();
            let (status, body) = if path.starts_with("/api/clients/desktop/update?target=linux&") {
                match &offer {
                    Some(offer) => ("200 OK", offer.clone().into_bytes()),
                    None => ("204 No Content", Vec::new()),
                }
            } else if path.starts_with("/download/") {
                ("200 OK", archive.clone())
            } else {
                ("404 Not Found", Vec::new())
            };
            let _ = write!(
                stream,
                "HTTP/1.1 {status}\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
                body.len()
            );
            let _ = stream.write_all(&body);
            let _ = stream.flush();
            let mut rest = [0u8; 1024];
            let _ = stream.read(&mut rest);
        }
    });
    origin
}
