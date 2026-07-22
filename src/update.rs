use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::error::{IoCtx, Result, UndoError};

pub struct Release {
    pub tag: String,
    pub version: String,
}

fn repo_slug() -> (String, String) {
    let repo = env!("CARGO_PKG_REPOSITORY");
    let tail = repo
        .trim_end_matches('/')
        .trim_end_matches(".git")
        .rsplit("github.com/")
        .next()
        .unwrap_or("nvrmnd-png/undo");
    match tail.split_once('/') {
        Some((owner, name)) => (owner.to_string(), name.to_string()),
        None => ("nvrmnd-png".into(), "undo".into()),
    }
}

pub fn target_triple() -> Result<String> {
    let arch = match std::env::consts::ARCH {
        "x86_64" => "x86_64",
        "aarch64" => "aarch64",
        other => {
            return Err(UndoError::msg(format!(
                "self-update has no prebuilt for architecture '{other}'; build from source or use manager.sh"
            )));
        }
    };
    let os = match std::env::consts::OS {
        "linux" => "unknown-linux-gnu",
        "macos" => "apple-darwin",
        other => {
            return Err(UndoError::msg(format!(
                "self-update has no prebuilt for OS '{other}'; build from source or use manager.sh"
            )));
        }
    };
    Ok(format!("{arch}-{os}"))
}

fn have(tool: &str) -> bool {
    Command::new("sh")
        .arg("-c")
        .arg(format!("command -v {tool} >/dev/null 2>&1"))
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

fn http_to_file(url: &str, out: &Path) -> Result<()> {
    let status = if have("curl") {
        Command::new("curl")
            .args(["-fsSL", "--proto", "=https", "--tlsv1.2", url, "-o"])
            .arg(out)
            .status()
    } else if have("wget") {
        Command::new("wget").arg("-qO").arg(out).arg(url).status()
    } else {
        return Err(UndoError::msg(
            "self-update needs curl or wget; install one or use manager.sh install --prebuilt",
        ));
    };
    match status {
        Ok(s) if s.success() => Ok(()),
        _ => Err(UndoError::msg(format!("download failed: {url}"))),
    }
}

fn http_to_string(url: &str) -> Result<String> {
    let output = if have("curl") {
        Command::new("curl")
            .args([
                "-fsSL",
                "--proto",
                "=https",
                "--tlsv1.2",
                "-A",
                "undo-self-update",
                url,
            ])
            .output()
    } else if have("wget") {
        Command::new("wget").args(["-qO-", url]).output()
    } else {
        return Err(UndoError::msg(
            "self-update needs curl or wget; install one or use manager.sh install --prebuilt",
        ));
    };
    match output {
        Ok(o) if o.status.success() => {
            String::from_utf8(o.stdout).map_err(|_| UndoError::msg("non-UTF-8 response"))
        }
        _ => Err(UndoError::msg(format!("request failed: {url}"))),
    }
}

pub fn latest_release() -> Result<Release> {
    let (owner, repo) = repo_slug();
    let url = format!("https://api.github.com/repos/{owner}/{repo}/releases/latest");
    let body = http_to_string(&url)?;
    let json: serde_json::Value = serde_json::from_str(&body)
        .map_err(|e| UndoError::msg(format!("bad API response: {e}")))?;
    let tag = json
        .get("tag_name")
        .and_then(|v| v.as_str())
        .ok_or_else(|| UndoError::msg("no release found (has one been published yet?)"))?
        .to_string();
    let version = tag.trim_start_matches('v').to_string();
    Ok(Release { tag, version })
}

fn parse_version(v: &str) -> (u64, u64, u64) {
    let mut parts = v.split('.').map(|p| p.trim().parse::<u64>().unwrap_or(0));
    (
        parts.next().unwrap_or(0),
        parts.next().unwrap_or(0),
        parts.next().unwrap_or(0),
    )
}

pub fn is_newer(candidate: &str, current: &str) -> bool {
    parse_version(candidate) > parse_version(current)
}

fn sha256_of(path: &Path) -> Result<String> {
    if !have("sha256sum") {
        return Err(UndoError::msg(
            "self-update needs sha256sum to verify the download",
        ));
    }
    let out = Command::new("sha256sum")
        .arg(path)
        .output()
        .map_err(|e| UndoError::io("running sha256sum", e))?;
    if !out.status.success() {
        return Err(UndoError::msg("sha256sum failed"));
    }
    let text = String::from_utf8_lossy(&out.stdout);
    Ok(text.split_whitespace().next().unwrap_or("").to_string())
}

fn find_binary(dir: &Path) -> Option<PathBuf> {
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        let entries = fs::read_dir(&d).ok()?;
        for entry in entries.flatten() {
            let path = entry.path();
            let ft = entry.file_type().ok()?;
            if ft.is_dir() {
                stack.push(path);
            } else if entry.file_name() == "undo" {
                return Some(path);
            }
        }
    }
    None
}

pub fn install(tag: &str) -> Result<PathBuf> {
    let (owner, repo) = repo_slug();
    let target = target_triple()?;
    let base = format!("https://github.com/{owner}/{repo}/releases/download/{tag}");
    let tarball_name = format!("undo-{target}.tar.gz");
    let tarball_url = format!("{base}/{tarball_name}");
    let sha_url = format!("{tarball_url}.sha256");

    let tmp = std::env::temp_dir().join(format!("undo-update-{}", std::process::id()));
    fs::create_dir_all(&tmp).ctx(format!("creating {}", tmp.display()))?;
    let guard = TmpDir(tmp.clone());

    let tarball = tmp.join(&tarball_name);
    let sha_file = tmp.join(format!("{tarball_name}.sha256"));
    http_to_file(&tarball_url, &tarball)?;
    http_to_file(&sha_url, &sha_file)?;

    let expected = fs::read_to_string(&sha_file)
        .ctx("reading checksum")?
        .split_whitespace()
        .next()
        .unwrap_or("")
        .to_string();
    let actual = sha256_of(&tarball)?;
    if expected.is_empty() || expected != actual {
        return Err(UndoError::msg(format!(
            "checksum mismatch (expected {expected}, got {actual}); refusing to install"
        )));
    }

    let extract_dir = tmp.join("extracted");
    fs::create_dir_all(&extract_dir).ctx("preparing extract dir")?;
    let status = Command::new("tar")
        .arg("-xzf")
        .arg(&tarball)
        .arg("-C")
        .arg(&extract_dir)
        .status()
        .map_err(|e| UndoError::io("running tar", e))?;
    if !status.success() {
        return Err(UndoError::msg("could not extract the release archive"));
    }
    let new_bin = find_binary(&extract_dir)
        .ok_or_else(|| UndoError::msg("no 'undo' binary in the archive"))?;

    let current =
        std::env::current_exe().map_err(|e| UndoError::io("finding current binary", e))?;
    let dir = current
        .parent()
        .ok_or_else(|| UndoError::msg("cannot locate the install directory"))?;
    let staged = dir.join(".undo-update.tmp");
    fs::copy(&new_bin, &staged).map_err(|e| {
        UndoError::io(
            format!(
                "installing to {} (need write permission there)",
                dir.display()
            ),
            e,
        )
    })?;
    fs::set_permissions(&staged, fs::Permissions::from_mode(0o755)).ctx("setting permissions")?;
    fs::rename(&staged, &current)
        .map_err(|e| UndoError::io(format!("replacing {}", current.display()), e))?;

    drop(guard);
    Ok(current)
}

struct TmpDir(PathBuf);

impl Drop for TmpDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_ordering() {
        assert!(is_newer("0.1.4", "0.1.3"));
        assert!(is_newer("0.2.0", "0.1.9"));
        assert!(is_newer("1.0.0", "0.9.9"));
        assert!(!is_newer("0.1.3", "0.1.3"));
        assert!(!is_newer("0.1.2", "0.1.3"));
        assert!(!is_newer("0.1.0", "0.1.3"));
    }

    #[test]
    fn repo_slug_from_cargo_metadata() {
        let (owner, repo) = repo_slug();
        assert_eq!(owner, "nvrmnd-png");
        assert_eq!(repo, "undo");
    }

    #[test]
    fn target_triple_is_known() {
        // On the CI/dev host this must resolve (x86_64/aarch64 linux/mac).
        let t = target_triple().unwrap();
        assert!(t.contains("linux") || t.contains("darwin"));
    }
}
