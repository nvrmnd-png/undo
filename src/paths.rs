use std::env;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use crate::error::{IoCtx, Result, UndoError};

#[derive(Debug, Clone, Copy)]
pub struct Limits {
    pub hash_max: u64,
    pub tree_cap: u64,
}

pub const DEFAULT_HASH_MAX: u64 = 64 * 1024 * 1024;
pub const DEFAULT_TREE_CAP: u64 = 10_000;

impl Limits {
    pub fn from_env() -> Self {
        let parse = |var: &str, default: u64| {
            env::var(var)
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(default)
        };
        Limits {
            hash_max: parse("UNDO_HASH_MAX_BYTES", DEFAULT_HASH_MAX),
            tree_cap: parse("UNDO_TREE_CAP", DEFAULT_TREE_CAP),
        }
    }
}

pub fn home_dir() -> Result<PathBuf> {
    match env::var_os("HOME") {
        Some(h) if !h.is_empty() => Ok(PathBuf::from(h)),
        _ => Err(UndoError::msg("HOME is not set")),
    }
}

fn xdg_data_home() -> Result<PathBuf> {
    match env::var_os("XDG_DATA_HOME") {
        Some(d) if !d.is_empty() => Ok(PathBuf::from(d)),
        _ => Ok(home_dir()?.join(".local/share")),
    }
}

pub fn data_dir() -> Result<PathBuf> {
    if let Some(d) = env::var_os("UNDO_DATA_DIR")
        && !d.is_empty()
    {
        return Ok(PathBuf::from(d));
    }
    if let Ok(config) = crate::config::Config::load()
        && let Some(dir) = config.data_dir_override()?
    {
        return Ok(dir);
    }
    Ok(xdg_data_home()?.join("undo"))
}

pub fn ensure_data_dir() -> Result<PathBuf> {
    let dir = data_dir()?;
    fs::create_dir_all(&dir).ctx(format!("creating data dir {}", dir.display()))?;
    fs::set_permissions(&dir, fs::Permissions::from_mode(0o700))
        .ctx(format!("securing data dir {}", dir.display()))?;
    Ok(dir)
}

pub fn trash_root() -> Result<PathBuf> {
    Ok(xdg_data_home()?.join("Trash"))
}

pub fn journal_path(data_dir: &Path) -> PathBuf {
    data_dir.join("journal.db")
}

pub fn lock_path(data_dir: &Path) -> PathBuf {
    data_dir.join("lock")
}

pub fn absolutize(path: &Path) -> Result<PathBuf> {
    std::path::absolute(path).ctx(format!("resolving path {}", path.display()))
}

pub fn euid() -> u32 {
    rustix::process::geteuid().as_raw()
}

pub fn username() -> String {
    env::var("USER").unwrap_or_default()
}
