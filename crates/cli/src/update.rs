//! `clashofmeat update`: on Linux, where no desktop app updates the CLI, it asks the app for the
//! stable release (the desktop app's update endpoint, `target=linux`), downloads this machine's
//! archive, checks its signature against the release key, swaps the binary install.sh put in place
//! and restarts the service. The daily `clashofmeat-update.timer` runs it with `--quiet`.

use std::fs;
use std::io::{Read, Write};
use std::path::Path;
use std::process::Command;
use std::time::Duration;

use anyhow::{anyhow, bail, Context, Result};
use base64::Engine as _;
use clashofmeat_core::Env;
use clashofmeat_daemon::config;
use serde_json::Value;

use crate::linux;

/// The updater's minisign public key (apps/desktop/src-tauri/updater.pub, install/install.sh).
const UPDATER_KEY: &str = "RWSC/TfveVpY2OTPXC0Xtm3TKi6XKYNZIc4H2Mu2JZ0GO3V3n+tTFSXp";
const MAX_ARCHIVE: u64 = 64 << 20;
const MAX_BINARY: u64 = 128 << 20;

/// The release key; debug builds take a test key from the environment, release builds never do.
fn public_key() -> String {
    if cfg!(debug_assertions) {
        if let Ok(key) = std::env::var("CLASHOFMEAT_UPDATE_PUBKEY") {
            return key;
        }
    }
    UPDATER_KEY.to_owned()
}

pub fn update(env: &Env, quiet: bool) -> Result<()> {
    if !cfg!(target_os = "linux") {
        bail!("The Clash of Meat desktop app keeps itself and the CLI up to date on this system.");
    }
    let exe = std::env::current_exe()?.canonicalize()?;
    let installed = linux::installed_exe(env);
    if installed.canonicalize().ok().as_deref() != Some(exe.as_path()) {
        bail!(
            "{} was not installed by install.sh ({}); update it the way you installed it.",
            exe.display(),
            installed.display()
        );
    }
    let app = config::read_config(env)?.app_url.ok_or_else(|| {
        anyhow!("No app address yet; run the install command from the setup page.")
    })?;
    let current = env!("CARGO_PKG_VERSION");
    let arch = std::env::consts::ARCH;
    let agent = ureq::AgentBuilder::new()
        .timeout_connect(Duration::from_secs(15))
        .timeout(Duration::from_secs(300))
        .build();
    let response = agent
        .get(&format!("{app}/api/clients/desktop/update"))
        .query("target", "linux")
        .query("arch", arch)
        .query("current", current)
        .set("x-clashofmeat-client", &format!("desktop/{current}"))
        .call()
        .map_err(|error| anyhow!("checking for updates: {error}"))?;
    if response.status() == 204 {
        if !quiet {
            println!("Clash of Meat {current} is up to date.");
        }
        return Ok(());
    }
    let offer: Value = response.into_json().context("reading the update offer")?;
    let version = offer["version"]
        .as_str()
        .ok_or_else(|| anyhow!("the update offer has no version"))?;
    if semver(version)? <= semver(current)? {
        if !quiet {
            println!("Clash of Meat {current} is up to date.");
        }
        return Ok(());
    }
    let platform = &offer["platforms"][format!("linux-{arch}")];
    let (Some(url), Some(signature)) = (platform["url"].as_str(), platform["signature"].as_str())
    else {
        bail!("release {version} has no build for linux-{arch}");
    };
    let local = cfg!(debug_assertions) && url.starts_with("http://127.0.0.1:");
    if !url.starts_with("https://") && !local {
        bail!("refusing a non-https download: {url}");
    }
    let file = url.rsplit('/').next().unwrap_or_default();

    let mut archive = Vec::new();
    agent
        .get(url)
        .call()
        .map_err(|error| anyhow!("downloading {file}: {error}"))?
        .into_reader()
        .take(MAX_ARCHIVE + 1)
        .read_to_end(&mut archive)?;
    if archive.len() as u64 > MAX_ARCHIVE {
        bail!("{file} is larger than {} MB", MAX_ARCHIVE >> 20);
    }
    verify(&archive, signature, file, version, &public_key())?;
    let binary = extract(&archive)?;
    replace(&exe, &binary, version)?;
    println!("Updated Clash of Meat {current} to {version}.");
    if linux::restart_service(env)? {
        println!("Restarted the Clash of Meat service.");
    }
    Ok(())
}

fn semver(version: &str) -> Result<(u64, u64, u64)> {
    let parts: Vec<u64> = version
        .split('.')
        .map(str::parse)
        .collect::<Result<_, _>>()
        .map_err(|_| anyhow!("invalid version {version:?}"))?;
    match parts[..] {
        [major, minor, patch] => Ok((major, minor, patch)),
        _ => bail!("invalid version {version:?}"),
    }
}

/// Checks the release signature (Tauri's `.sig`: a base64 minisign signature) and that its trusted
/// comment names this file and version, so an older signed archive cannot pass for a newer one.
pub(crate) fn verify(
    data: &[u8],
    signature: &str,
    file: &str,
    version: &str,
    key: &str,
) -> Result<()> {
    let text = base64::engine::general_purpose::STANDARD
        .decode(signature.trim())
        .context("the release signature is not base64")?;
    let signature = minisign_verify::Signature::decode(
        std::str::from_utf8(&text).context("the release signature is not text")?,
    )
    .map_err(|error| anyhow!("invalid release signature: {error}"))?;
    let key = minisign_verify::PublicKey::from_base64(key)
        .map_err(|error| anyhow!("invalid release key: {error}"))?;
    key.verify(data, &signature, true).map_err(|_| {
        anyhow!("{file} is not signed with the Clash of Meat release key; nothing was installed")
    })?;
    let trusted = signature.trusted_comment();
    let field = |name: &str| {
        trusted
            .split(['\t', ' '])
            .find_map(|part| part.strip_prefix(name)?.strip_prefix(':'))
    };
    if field("file") != Some(file) || field("version") != Some(version) {
        bail!("the release signature is for another file or version ({trusted}); nothing was installed");
    }
    Ok(())
}

/// The `clashofmeat` file at the root of a release archive.
pub(crate) fn extract(archive: &[u8]) -> Result<Vec<u8>> {
    let mut tar = tar::Archive::new(flate2::read::GzDecoder::new(archive));
    for entry in tar.entries().context("reading the release archive")? {
        let entry = entry?;
        if entry.header().entry_type().is_file()
            && entry.path()?.as_ref() == Path::new("clashofmeat")
        {
            let mut binary = Vec::new();
            entry.take(MAX_BINARY + 1).read_to_end(&mut binary)?;
            if binary.len() as u64 > MAX_BINARY {
                bail!("the release binary is too large");
            }
            return Ok(binary);
        }
    }
    bail!("the release archive has no clashofmeat binary")
}

/// Writes the new binary beside the old one, checks it runs and reports the expected version,
/// then renames it over the old one (atomic on one filesystem; the running daemon keeps its copy).
fn replace(exe: &Path, binary: &[u8], version: &str) -> Result<()> {
    let dir = exe
        .parent()
        .ok_or_else(|| anyhow!("{} has no folder", exe.display()))?;
    let staged = dir.join(format!(".clashofmeat.new.{}", std::process::id()));
    let result = (|| -> Result<()> {
        let mut options = fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o755);
        }
        let mut out = options.open(&staged)?;
        out.write_all(binary)?;
        out.sync_all()?;
        drop(out);
        let output = Command::new(&staged)
            .arg("--version")
            .output()
            .context("running the new binary")?;
        // clap prints the package name and version: `clashofmeat-cli 0.1.3`.
        let reported = String::from_utf8_lossy(&output.stdout);
        if !output.status.success()
            || reported.trim() != format!("{} {version}", env!("CARGO_PKG_NAME"))
        {
            bail!(
                "the new binary reports {:?}, not version {version}",
                reported.trim()
            );
        }
        fs::rename(&staged, exe)?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&staged);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn release_key_matches_updater_pub() {
        let encoded = include_str!("../../../apps/desktop/src-tauri/updater.pub");
        let decoded = base64::engine::general_purpose::STANDARD
            .decode(encoded.trim())
            .unwrap();
        let text = String::from_utf8(decoded).unwrap();
        assert_eq!(text.lines().nth(1), Some(UPDATER_KEY));
    }

    #[test]
    fn versions_compare_numerically() {
        assert!(semver("0.1.10").unwrap() > semver("0.1.9").unwrap());
        assert!(semver("1.0").is_err());
        assert!(semver("1.0.x").is_err());
    }
}
