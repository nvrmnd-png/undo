use std::collections::BTreeMap;
use std::env;
use std::fs;
use std::io;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::error::{IoCtx, Result, UndoError};
use crate::paths;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Config {
    #[serde(default)]
    pub cleanup: Cleanup,
    #[serde(default)]
    pub storage: Storage,
    #[serde(default)]
    pub exclude: Exclude,
    #[serde(default)]
    pub logging: Logging,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub plugins: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Cleanup {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default = "default_max_age_days")]
    pub max_age_days: u64,
    #[serde(default = "default_max_database_size")]
    pub max_database_size: u64,
}

fn default_max_age_days() -> u64 {
    90
}

fn default_max_database_size() -> u64 {
    500
}

impl Default for Cleanup {
    fn default() -> Self {
        Cleanup {
            enabled: false,
            max_age_days: default_max_age_days(),
            max_database_size: default_max_database_size(),
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Storage {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Exclude {
    #[serde(default)]
    pub paths: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Logging {
    #[serde(default)]
    pub enabled: bool,
}

pub fn config_path() -> Result<PathBuf> {
    let base = match env::var_os("XDG_CONFIG_HOME") {
        Some(d) if !d.is_empty() => PathBuf::from(d),
        _ => paths::home_dir()?.join(".config"),
    };
    Ok(base.join("undo").join("config.toml"))
}

fn expand_tilde(p: &str) -> Result<PathBuf> {
    if let Some(rest) = p.strip_prefix("~/") {
        Ok(paths::home_dir()?.join(rest))
    } else if p == "~" {
        paths::home_dir()
    } else {
        Ok(PathBuf::from(p))
    }
}

impl Config {
    pub fn load() -> Result<Config> {
        let path = config_path()?;
        match fs::read_to_string(&path) {
            Ok(s) => toml::from_str(&s)
                .map_err(|e| UndoError::msg(format!("config {}: {e}", path.display()))),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(Config::default()),
            Err(e) => Err(UndoError::io(format!("reading {}", path.display()), e)),
        }
    }

    pub fn save(&self) -> Result<PathBuf> {
        let path = config_path()?;
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).ctx(format!("creating {}", parent.display()))?;
        }
        let text =
            toml::to_string_pretty(self).map_err(|e| UndoError::msg(format!("config: {e}")))?;
        fs::write(&path, text).ctx(format!("writing {}", path.display()))?;
        Ok(path)
    }

    pub fn data_dir_override(&self) -> Result<Option<PathBuf>> {
        match &self.storage.path {
            Some(p) if !p.is_empty() => Ok(Some(expand_tilde(p)?)),
            _ => Ok(None),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_sensible() {
        let c = Config::default();
        assert!(!c.cleanup.enabled);
        assert_eq!(c.cleanup.max_age_days, 90);
        assert_eq!(c.cleanup.max_database_size, 500);
        assert!(c.storage.path.is_none());
        assert!(c.exclude.paths.is_empty());
        assert!(!c.logging.enabled);
    }

    #[test]
    fn partial_config_keeps_field_defaults() {
        let text = "[cleanup]\nenabled = true\n";
        let c: Config = toml::from_str(text).unwrap();
        assert!(c.cleanup.enabled);
        assert_eq!(c.cleanup.max_age_days, 90);
        assert_eq!(c.cleanup.max_database_size, 500);
    }

    #[test]
    fn roundtrip() {
        let mut c = Config::default();
        c.cleanup.enabled = true;
        c.cleanup.max_age_days = 30;
        c.exclude.paths = vec![".cache".into(), "node_modules".into()];
        c.logging.enabled = true;
        c.storage.path = Some("~/.undo".into());
        let text = toml::to_string_pretty(&c).unwrap();
        let back: Config = toml::from_str(&text).unwrap();
        assert_eq!(back.cleanup.max_age_days, 30);
        assert!(back.cleanup.enabled);
        assert_eq!(back.exclude.paths.len(), 2);
        assert!(back.logging.enabled);
        assert_eq!(back.storage.path.as_deref(), Some("~/.undo"));
    }
}
