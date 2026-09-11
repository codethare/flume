// SPDX-FileCopyrightText: © 2026 Julian Andrews
// SPDX-License-Identifier: AGPL-3.0-or-later

use std::io;
use std::path::{Path, PathBuf};

use crate::keybinding::{default_keybindings, default_pointer_bindings};
use crate::types::Config;

/// Candidate config paths, in order: `$XDG_CONFIG_HOME/flume/config.toml`,
/// `$HOME/.config/flume/config.toml`.
fn config_paths() -> Vec<PathBuf> {
    let mut paths = Vec::new();
    if let Some(xdg) = std::env::var_os("XDG_CONFIG_HOME")
        && !xdg.is_empty()
    {
        paths.push(PathBuf::from(xdg).join("flume").join("config.toml"));
    }
    if let Some(home) = std::env::var_os("HOME")
        && !home.is_empty()
    {
        paths.push(
            PathBuf::from(home)
                .join(".config")
                .join("flume")
                .join("config.toml"),
        );
    }
    paths
}

fn parse(content: &str) -> Result<Config, toml::de::Error> {
    let mut config: Config = toml::from_str(content)?;
    // Empty binding lists fall back to the built-in defaults (rill-ed
    // behavior); window_rules and spawn_at_startup default to empty.
    if config.keybindings.is_empty() {
        config.keybindings = default_keybindings();
    }
    if config.pointer_bindings.is_empty() {
        config.pointer_bindings = default_pointer_bindings();
    }
    Ok(config)
}

fn read(path: &Path) -> Result<Config, io::Error> {
    let content = std::fs::read_to_string(path)?;
    parse(&content).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
}

/// Load config from the first candidate path that exists. Falls back to
/// defaults when no file is found. Parse errors propagate so the caller can
/// exit loudly.
pub fn load(explicit: Option<&Path>) -> Result<Config, io::Error> {
    if let Some(path) = explicit {
        return read(path);
    }
    for path in config_paths() {
        match read(&path) {
            Ok(config) => return Ok(config),
            Err(e) if e.kind() == io::ErrorKind::NotFound => continue,
            Err(e) => return Err(e),
        }
    }
    Ok(default_config())
}

/// Reload config. Returns `None` (caller keeps the old config) when no file
/// is found or parsing fails.
pub fn reload(explicit: Option<&Path>) -> Option<Config> {
    if let Some(path) = explicit {
        return read(path).ok();
    }
    for path in config_paths() {
        match read(&path) {
            Ok(config) => return Some(config),
            Err(e) if e.kind() == io::ErrorKind::NotFound => continue,
            Err(_) => return None,
        }
    }
    None
}

pub fn default_config() -> Config {
    Config {
        keybindings: default_keybindings(),
        pointer_bindings: default_pointer_bindings(),
        ..Config::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::actions::KeybindingAction;

    #[test]
    fn default_config_has_bindings() {
        let config = default_config();
        assert!(config.keybindings.len() > 50);
        assert!(
            config
                .keybindings
                .iter()
                .any(|kb| kb.action == KeybindingAction::CloseWindow)
        );
        assert_eq!(config.pointer_bindings.len(), 2);
    }

    #[test]
    fn parse_empty_falls_back_to_default_bindings() {
        let config = parse("").unwrap();
        assert_eq!(config.keybindings.len(), default_keybindings().len());
        assert_eq!(config.pointer_bindings, default_pointer_bindings());
        assert!(config.window_rules.is_empty());
    }

    #[test]
    fn parse_user_bindings_override_defaults() {
        let config =
            parse("[[keybindings]]\nkey = \"x\"\nmodifiers = []\naction = \"exit\"\n").unwrap();
        assert_eq!(config.keybindings.len(), 1);
        assert_eq!(config.keybindings[0].action, KeybindingAction::Exit);
        // Pointer bindings still default when omitted.
        assert_eq!(config.pointer_bindings.len(), 2);
    }

    #[test]
    fn parse_error_is_reported() {
        assert!(parse("vertical_gap = \"nine\"").is_err());
    }

    /// Serde ignores unknown fields by default, so a typo'd toggle
    /// (`focus_folows_pointer`) would be silently dropped and the user would
    /// never know the setting didn't apply. Unknown keys must be loud.
    #[test]
    fn unknown_keys_are_rejected() {
        assert!(parse("focus_folows_pointer = true").is_err());
        assert!(parse("border = { widht = 3 }").is_err());
        assert!(parse("[[keybindings]]\nkey = \"x\"\nmodifers = []\naction = \"exit\"\n").is_err());
        assert!(parse("[[pointer_bindings]]\nbuton = \"middle\"\nmodifiers = []\naction = \"move_window\"\n").is_err());
        assert!(parse("[[window_rules]]\napp_id = \"x\"\nfloatng = true\n").is_err());
    }

    /// The shipped example must parse against the schema and must not change
    /// behaviour when copied verbatim: its binding sections are commented out,
    /// so it falls back to the built-in defaults. A hand-maintained copy of
    /// the defaults lived here before and had already drifted (46 of 64
    /// bindings), silently dropping keybindings for anyone who copied it.
    #[test]
    fn example_config_parses() {
        let content = include_str!("../config.example.toml");
        let config = parse(content).expect("config.example.toml must parse");
        assert_eq!(config.keybindings, default_keybindings());
        assert_eq!(config.pointer_bindings, default_pointer_bindings());
    }

    /// `-c/--config <path>`: the explicit-path branch of load/reload had no
    /// coverage (only the argument parser and `parse` were tested).
    #[test]
    fn explicit_path_loads_and_reloads() {
        let dir = std::env::temp_dir().join(format!("flume-config-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("config.toml");

        std::fs::write(&path, "vertical_gap = 42\n").unwrap();
        assert_eq!(load(Some(&path)).unwrap().vertical_gap, 42);

        // A broken file is an error for load, and keeps the old config (None)
        // for reload.
        std::fs::write(&path, "vertical_gap = \"nine\"\n").unwrap();
        assert!(load(Some(&path)).is_err());
        assert!(reload(Some(&path)).is_none());

        // An explicitly named file that does not exist is reported, not
        // silently replaced by the built-in defaults.
        let missing = dir.join("nope.toml");
        assert_eq!(
            load(Some(&missing)).unwrap_err().kind(),
            std::io::ErrorKind::NotFound
        );
        assert!(reload(Some(&missing)).is_none());

        std::fs::remove_dir_all(&dir).unwrap();
    }
}
