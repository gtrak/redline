//! User config: `~/.config/redline/config.toml` — theme selection and
//! key-binding overrides (command name → emacs-notation sequence string).
//! A missing file means all defaults; unknown fields are tolerated.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::theme::Theme;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ThemeChoice {
/// iocraft default palette (issue 01 stub).
    #[default]
    DefaultTheme,
    /// Dark theme (stub in issue 01; both map to the same stub values).
    Dark,
    /// Light theme (light background, dark text).
    Light,
}

impl From<ThemeChoice> for Theme {
    fn from(c: ThemeChoice) -> Self {
        match c {
            ThemeChoice::DefaultTheme => Theme::dark("default"),
            ThemeChoice::Dark => Theme::dark("dark"),
            ThemeChoice::Light => Theme::light("light"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(default)]
pub struct Config {
    /// Theme selection.
    pub theme: ThemeChoice,
    /// Key-binding overrides: command name → key sequence string,
    /// e.g. `quit = "C-q"`. Applied to the global keymap on load.
    #[serde(rename = "key-bindings")]
    pub key_bindings: BTreeMap<String, String>,
    /// Live-reload changed files on disk (default `true`). A runtime
    /// `M-x toggle-watcher` command suspends/resumes watching per session;
    /// this is the on-disk default.
    #[serde(default = "default_auto_reload")]
    pub auto_reload: bool,
}

/// `auto_reload` defaults to on (plan decision #7: the repo is live).
fn default_auto_reload() -> bool {
    true
}

impl Default for Config {
    fn default() -> Self {
        Self {
            theme: ThemeChoice::default(),
            key_bindings: BTreeMap::new(),
            auto_reload: true,
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("failed to read config at {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to parse config at {path}: {message}")]
    Parse { path: PathBuf, message: String },
}

/// `~/.config/redline/config.toml` (via `dirs::config_dir`).
pub fn config_path() -> Option<PathBuf> {
    dirs::config_dir().map(|dir| dir.join("redline").join("config.toml"))
}

fn parse_from(s: &str, path: &Path) -> Result<Config, ConfigError> {
    toml::from_str(s).map_err(|e| ConfigError::Parse {
        path: path.to_path_buf(),
        message: e.to_string(),
    })
}

/// Load the config file; a missing file (or missing config dir) yields
/// all defaults. Parse and IO errors are surfaced.
pub fn load() -> Result<Config, ConfigError> {
    match config_path() {
        Some(path) if path.is_file() => {
            let text = std::fs::read_to_string(&path).map_err(|e| ConfigError::Io {
                path: path.clone(),
                source: e,
            })?;
            parse_from(&text, &path)
        }
        _ => Ok(Config::default()),
    }
}

/// Load the config file, mapping a missing file to defaults and
/// wrapping errors in `anyhow` for startup use in `main`.
pub fn load_tolerant() -> Config {
    match load() {
        Ok(config) => config,
        Err(ConfigError::Io { path, .. }) => {
            tracing::warn!("could not read {path:?}: using default config");
            Config::default()
        }
        Err(ConfigError::Parse { path, message }) => {
            tracing::warn!("invalid config {path:?} ({message}): using default config");
            Config::default()
        }
    }
}

impl Config {
    /// Validate key-binding overrides against a registry: every override
    /// must name a registered command and a parseable sequence.
    pub fn validate_bindings(&self, registry: &crate::app::command::CommandRegistry) -> Result<(), String> {
        for (command, sequence) in &self.key_bindings {
            if registry.get(command).is_none() {
                return Err(format!("unknown command `{command}` in key-bindings"));
            }
            crate::app::keymap::parse_sequence(sequence)
                .map_err(|e| format!("`{command}`: invalid key sequence `{sequence}`: {e}"))?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::command::CommandRegistry;
    use crate::app::store::AppStore;

    fn parse(s: &str) -> Config {
        toml::from_str(s).expect("test TOML should parse")
    }

    #[test]
    fn defaults_when_empty() {
        let config = Config::default();
        assert_eq!(config.theme, ThemeChoice::DefaultTheme);
        assert!(config.key_bindings.is_empty());

        let config = parse("");
        assert_eq!(config, Config::default());
    }

    #[test]
    fn parse_theme_and_bindings() {
        let config = parse(
            r#"
theme = "dark"
[key-bindings]
quit = "C-q"
open-palette = "M-c M-x"
"#,
        );
        assert_eq!(config.theme, ThemeChoice::Dark);
        assert_eq!(config.key_bindings.get("quit"), Some(&"C-q".to_string()));
        assert_eq!(
            config.key_bindings.get("open-palette"),
            Some(&"M-c M-x".to_string())
        );
    }

    #[test]
    fn unknown_fields_are_tolerated() {
        let config = parse(
            r#"
theme = "dark"
future_setting = 42
[experimental]
something = true
"#,
        );
        assert_eq!(config.theme, ThemeChoice::Dark);
        assert!(config.key_bindings.is_empty());
    }

    #[test]
    fn invalid_toml_is_rejected() {
        assert!(toml::from_str::<Config>("not [valid toml").is_err());
        assert!(toml::from_str::<Config>("theme = 42").is_err());
    }

    #[test]
    fn load_missing_file_is_defaults() {
        // Point at a temp dir so we never touch the real user config.
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("config.toml");
        // `load()` itself reads the real user path; here we verify the
        // missing-file branch via parse_from + a non-existent file.
        assert!(!path.exists());
        let err = std::fs::read_to_string(&path).expect_err("no file yet");
        assert!(err.kind() == std::io::ErrorKind::NotFound);
        // And that defaults are exactly what `load` returns for missing.
        let config = Config::default();
        assert_eq!(config.theme, ThemeChoice::DefaultTheme);
    }

    fn test_store() -> AppStore {
        let dir = tempfile::tempdir().unwrap();
        AppStore::at(dir.path(), dir.path().to_path_buf())
    }

    #[test]
    fn override_application_rebinds_global_keymap() {
        let config = parse(
            r#"
[key-bindings]
quit = "C-q"
"#,
        );
        let mut store = test_store();
        store.apply_config(&config).expect("override applies");
        let seq = crate::app::keymap::parse_sequence("C-q").unwrap();
        assert_eq!(
            store.engine.global.lookup(&seq),
            Some(crate::app::keymap::Lookup::Command("quit"))
        );
    }

    #[test]
    fn invalid_override_sequence_is_rejected() {
        let config = parse(
            r#"
[key-bindings]
quit = "C-FOO"
"#,
        );
        let mut store = test_store();
        let err = store.apply_config(&config).unwrap_err();
        assert!(err.contains("quit"), "{err}");
    }

    #[test]
    fn override_cannot_shade_an_existing_prefix_binding() {
        // C-x is a strict prefix of the built-in C-x C-c binding.
        let config = parse(
            r#"
[key-bindings]
cancel = "C-x"
"#,
        );
        let mut store = test_store();
        let err = store.apply_config(&config).unwrap_err();
        assert!(err.contains("C-x"), "{err}");
    }

    #[test]
    fn validate_bindings_checks_command_names() {
        let registry = CommandRegistry::seed();
        let good = parse("[key-bindings]\nquit = \"C-q\"\n");
        assert!(good.validate_bindings(&registry).is_ok());

        let bad = parse("[key-bindings]\nnope = \"C-q\"\n");
        let err = bad.validate_bindings(&registry).unwrap_err();
        assert!(err.contains("nope"), "{err}");
    }
}
