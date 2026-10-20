//! Provider installation detection.

use std::path::Path;
use std::process::Command;

use clashofmeat_core::{Env, Manifest};

/// Reports whether any manifest detection hint is present.
pub fn detect(env: &Env, manifest: &Manifest) -> bool {
    let path = env.vars.get("PATH").map_or("", String::as_str);
    let command_found = manifest
        .detect
        .commands
        .iter()
        .any(|command| std::env::split_paths(path).any(|dir| dir.join(command).is_file()));
    let app_found = manifest.detect.apps.iter().any(|app| {
        let name = if app.ends_with(".app") {
            app.to_string()
        } else {
            format!("{app}.app")
        };
        Path::new("/Applications").join(&name).exists()
            || env.home.join("Applications").join(name).exists()
    });
    command_found
        || app_found
        || manifest
            .detect
            .paths
            .iter()
            .any(|path| env.resolve(path).exists())
}

/// Returns the user's computer name, with a hostname fallback.
pub fn device_name() -> String {
    #[cfg(target_os = "macos")]
    if let Ok(output) = Command::new("scutil")
        .args(["--get", "ComputerName"])
        .output()
    {
        if output.status.success() {
            let name = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if !name.is_empty() {
                return name;
            }
        }
    }
    Command::new("hostname")
        .output()
        .ok()
        .filter(|output| output.status.success())
        .map(|output| String::from_utf8_lossy(&output.stdout).trim().to_string())
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| "Mac".into())
}

/// Returns identifiers of detected built-in providers.
pub fn tools(env: &Env) -> Vec<String> {
    clashofmeat_providers::all()
        .iter()
        .filter(|provider| detect(env, provider.manifest()))
        .map(|provider| provider.manifest().id.to_string())
        .collect()
}
