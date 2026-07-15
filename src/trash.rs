use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use percent_encoding::{AsciiSet, NON_ALPHANUMERIC, utf8_percent_encode};

use crate::error::{IoCtx, Result, UndoError};
use crate::journal::model::TrashRef;
use crate::ops;
use crate::paths::{self, Limits};
use crate::state;

const PATH_ENCODE: &AsciiSet = &NON_ALPHANUMERIC
    .remove(b'/')
    .remove(b'-')
    .remove(b'.')
    .remove(b'_')
    .remove(b'~');

const MAX_NAME_ATTEMPTS: u32 = 10_000;

#[derive(Debug, Clone)]
pub struct Trash {
    pub files_dir: PathBuf,
    pub info_dir: PathBuf,
}

impl Trash {
    pub fn home() -> Result<Trash> {
        let root = paths::trash_root()?;
        let files_dir = root.join("files");
        let info_dir = root.join("info");
        for dir in [&root, &files_dir, &info_dir] {
            fs::create_dir_all(dir).ctx(format!("creating trash dir {}", dir.display()))?;
        }
        fs::set_permissions(&root, fs::Permissions::from_mode(0o700))
            .ctx(format!("securing trash dir {}", root.display()))?;
        Ok(Trash {
            files_dir,
            info_dir,
        })
    }

    pub fn root(&self) -> &Path {
        self.files_dir.parent().unwrap_or(&self.files_dir)
    }

    fn reserve(&self, base: &str, origin: &Path) -> Result<(PathBuf, PathBuf)> {
        let base = if base.is_empty() { "trashed" } else { base };
        for attempt in 1..=MAX_NAME_ATTEMPTS {
            let name = if attempt == 1 {
                base.to_string()
            } else {
                format!("{base}.{attempt}")
            };
            let info = self.info_dir.join(format!("{name}.trashinfo"));
            let payload = self.files_dir.join(&name);
            match OpenOptions::new().write(true).create_new(true).open(&info) {
                Ok(mut f) => {
                    if fs::symlink_metadata(&payload).is_ok() {
                        drop(f);
                        let _ = fs::remove_file(&info);
                        continue;
                    }
                    f.write_all(trashinfo_body(origin).as_bytes())
                        .ctx(format!("writing {}", info.display()))?;
                    return Ok((payload, info));
                }
                Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
                Err(e) => return Err(UndoError::io(format!("reserving {}", info.display()), e)),
            }
        }
        Err(UndoError::msg(format!(
            "could not find a free trash name for '{base}' after {MAX_NAME_ATTEMPTS} attempts"
        )))
    }

    pub fn put(&self, origin: &Path, limits: &Limits) -> Result<TrashRef> {
        let origin = paths::absolutize(origin)?;
        let origin = origin.as_path();
        let (recorded, tree) = state::capture_with_tree(origin, limits)?;
        if recorded.is_absent() {
            return Err(UndoError::msg(format!(
                "cannot trash {}: no such file or directory",
                origin.display()
            )));
        }
        let base = origin
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let (payload, info) = self.reserve(&base, origin)?;

        match fs::rename(origin, &payload) {
            Ok(()) => {}
            Err(e) if e.kind() == io::ErrorKind::CrossesDevices => {
                if let Err(copy_err) = ops::verified_copy_node(origin, &payload) {
                    let _ = fs::remove_file(&info);
                    let _ = ops::remove_node(&payload);
                    return Err(copy_err);
                }
                ops::remove_node(origin)?;
            }
            Err(e) => {
                let _ = fs::remove_file(&info);
                return Err(UndoError::io(
                    format!("trashing {} -> {}", origin.display(), payload.display()),
                    e,
                ));
            }
        }

        Ok(TrashRef {
            origin: origin.to_path_buf(),
            file: payload,
            info,
            state: recorded,
            tree,
        })
    }

    pub fn restore(&self, tref: &TrashRef) -> Result<Vec<String>> {
        let mut warnings = Vec::new();
        if fs::symlink_metadata(&tref.file).is_err() {
            return Err(UndoError::msg(format!(
                "trash artifact {} is gone — cannot restore {}",
                tref.file.display(),
                tref.origin.display()
            )));
        }
        if fs::symlink_metadata(&tref.origin).is_ok() {
            return Err(UndoError::msg(format!(
                "cannot restore {}: path is occupied",
                tref.origin.display()
            )));
        }
        if let Some(parent) = tref.origin.parent()
            && fs::symlink_metadata(parent).is_err()
        {
            fs::create_dir_all(parent).ctx(format!("recreating {}", parent.display()))?;
            warnings.push(format!(
                "recreated missing parent directory {} (original metadata not preserved)",
                parent.display()
            ));
        }
        match fs::rename(&tref.file, &tref.origin) {
            Ok(()) => {}
            Err(e) if e.kind() == io::ErrorKind::CrossesDevices => {
                ops::verified_copy_node(&tref.file, &tref.origin)?;
                ops::remove_node(&tref.file)?;
            }
            Err(e) => {
                return Err(UndoError::io(
                    format!(
                        "restoring {} -> {}",
                        tref.file.display(),
                        tref.origin.display()
                    ),
                    e,
                ));
            }
        }
        if let Err(e) = fs::remove_file(&tref.info) {
            warnings.push(format!("could not remove {}: {e}", tref.info.display()));
        }
        Ok(warnings)
    }
}

fn trashinfo_body(origin: &Path) -> String {
    let origin_str = origin.to_string_lossy();
    let encoded = utf8_percent_encode(&origin_str, PATH_ENCODE);
    let date = jiff::Zoned::now().strftime("%Y-%m-%dT%H:%M:%S");
    format!("[Trash Info]\nPath={encoded}\nDeletionDate={date}\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trashinfo_encodes_special_characters() {
        let body = trashinfo_body(Path::new("/home/nyzor/my file 100%.txt"));
        assert!(body.starts_with("[Trash Info]\n"));
        assert!(body.contains("Path=/home/nyzor/my%20file%20100%25.txt\n"));
        let date_line = body
            .lines()
            .find(|l| l.starts_with("DeletionDate="))
            .unwrap();
        let date = date_line.trim_start_matches("DeletionDate=");
        assert_eq!(date.len(), 19, "unexpected DeletionDate format: {date}");
        assert_eq!(&date[4..5], "-");
        assert_eq!(&date[10..11], "T");
    }

    #[test]
    fn trashinfo_keeps_unreserved_characters() {
        let body = trashinfo_body(Path::new("/a-b/c.d_e~f/g"));
        assert!(body.contains("Path=/a-b/c.d_e~f/g\n"));
    }

    #[test]
    fn trashinfo_encodes_utf8_bytes() {
        let body = trashinfo_body(Path::new("/home/nyzor/übung"));
        assert!(body.contains("Path=/home/nyzor/%C3%BCbung\n"));
    }
}
