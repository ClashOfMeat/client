use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use anyhow::{bail, Context, Result};
use clashofmeat_core::env::Env;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

#[derive(Serialize, Deserialize, Default, Clone, Debug)]
pub struct Config {
    #[serde(rename = "appUrl", skip_serializing_if = "Option::is_none")]
    pub app_url: Option<String>,
    #[serde(flatten)]
    pub other: Map<String, Value>,
}

pub fn home(env: &Env) -> PathBuf {
    env.vars
        .get("CLASHOFMEAT_HOME")
        .filter(|value| !value.is_empty())
        .map_or_else(|| env.home.join(".clashofmeat"), PathBuf::from)
}

pub fn store_path(env: &Env) -> PathBuf {
    home(env).join("desktop/activity.sqlite")
}

pub fn read_config(env: &Env) -> Result<Config> {
    let path = home(env).join("config.json");
    match fs::read(&path) {
        Ok(bytes) => {
            serde_json::from_slice(&bytes).with_context(|| format!("reading {}", path.display()))
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Config::default()),
        Err(error) => Err(error).with_context(|| format!("reading {}", path.display())),
    }
}

pub fn update_config(env: &Env, update: impl FnOnce(&mut Config)) -> Result<Config> {
    static NEXT_TEMP: AtomicU64 = AtomicU64::new(0);
    let mut config = read_config(env)?;
    update(&mut config);
    let dir = home(env);
    fs::create_dir_all(&dir)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&dir, fs::Permissions::from_mode(0o700))?;
    }
    let path = dir.join("config.json");
    let temporary = dir.join(format!(
        ".config.{}.{}.tmp",
        std::process::id(),
        NEXT_TEMP.fetch_add(1, Ordering::Relaxed)
    ));
    let result = (|| -> Result<()> {
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(&temporary)?;
        serde_json::to_writer_pretty(&mut file, &config)?;
        file.write_all(b"\n")?;
        file.sync_all()?;
        fs::rename(&temporary, &path)?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result?;
    Ok(config)
}

pub fn app_origin(url: &str) -> Result<String> {
    let (scheme, rest) = url.split_once("://").context("invalid app URL")?;
    if !matches!(scheme, "http" | "https") {
        bail!("Use an HTTPS app URL, or http://localhost for local development.");
    }
    let authority = rest.strip_suffix('/').unwrap_or(rest);
    if authority.is_empty()
        || authority.contains(['/', '?', '#', '@', '\\'])
        || authority.chars().any(char::is_whitespace)
    {
        bail!("Use an HTTPS app URL, or http://localhost for local development.");
    }
    let (host, port, has_port) = if authority.starts_with('[') {
        let end = authority.find(']').context("invalid app URL")?;
        let suffix = &authority[end + 1..];
        if !suffix.is_empty() && !suffix.starts_with(':') {
            bail!("invalid app URL");
        }
        (
            &authority[..=end],
            suffix.strip_prefix(':').unwrap_or(""),
            !suffix.is_empty(),
        )
    } else if let Some((host, port)) = authority.split_once(':') {
        (host, port, true)
    } else {
        (authority, "", false)
    };
    if host.is_empty() || (has_port && port.parse::<u16>().ok().filter(|n| *n > 0).is_none()) {
        bail!("invalid app URL");
    }
    if scheme == "http" && !matches!(host, "localhost" | "127.0.0.1" | "[::1]") {
        bail!("Use an HTTPS app URL, or http://localhost for local development.");
    }
    Ok(format!("{scheme}://{authority}"))
}

/// Extra local account homes; paths never enter uploads.
pub fn provider_variable(provider: &str) -> Option<&'static str> {
    match provider {
        "claude-code" => Some("CLAUDE_CONFIG_DIR"),
        "codex" => Some("CODEX_HOME"),
        "gemini" => Some("GEMINI_CLI_HOME"),
        _ => None,
    }
}

pub fn source_roots(
    env: &Env,
    config: &Config,
    provider: &str,
    expression: &str,
) -> Vec<(PathBuf, Option<String>)> {
    let default = env.resolve(expression);
    let default = default.canonicalize().unwrap_or(default);
    let mut roots = vec![(default, None)];
    let Some(variable) = provider_variable(provider) else {
        return roots;
    };
    if let Some(homes) = config
        .other
        .get("providerHomes")
        .and_then(|map| map.get(provider))
        .and_then(Value::as_array)
    {
        for home in homes.iter().filter_map(Value::as_str).take(8) {
            let mut account = env.clone();
            account.vars.insert(variable.into(), home.into());
            let root = account.resolve(expression);
            let root = root.canonicalize().unwrap_or(root);
            if roots.iter().any(|(path, _)| *path == root) {
                continue;
            }
            let profile = clashofmeat_core::ids::stable_id(&format!("{provider}:{home}"));
            roots.push((root, Some(profile)));
        }
    }
    roots
}
