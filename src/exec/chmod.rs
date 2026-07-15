use std::fs;
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::PathBuf;

use super::adapter::Session;
use super::modes::{self, ModeSpec};
use crate::error::{Result, UndoError};
use crate::journal::model::Action;
use crate::ops;

struct ChmodArgs {
    recursive: bool,
    verbose: bool,
    changes_only: bool,
    spec: ModeSpec,
    files: Vec<PathBuf>,
}

fn parse_args(argv: &[String]) -> Result<ChmodArgs> {
    let mut recursive = false;
    let mut verbose = false;
    let mut changes_only = false;
    let mut mode_str: Option<String> = None;
    let mut files = Vec::new();
    let mut after_ddash = false;

    for arg in &argv[1..] {
        if !after_ddash {
            if arg == "--" {
                after_ddash = true;
                continue;
            }
            match arg.as_str() {
                "-R" | "--recursive" => {
                    recursive = true;
                    continue;
                }
                "-v" | "--verbose" => {
                    verbose = true;
                    continue;
                }
                "-c" | "--changes" => {
                    changes_only = true;
                    continue;
                }
                _ => {}
            }
            if mode_str.is_none() {
                if modes::parse(arg).is_ok() {
                    mode_str = Some(arg.clone());
                    continue;
                }
                if arg.starts_with('-') {
                    return Err(UndoError::fallback(format!(
                        "chmod: unsupported option '{arg}'"
                    )));
                }
                return Err(UndoError::fallback(format!("chmod: invalid mode: '{arg}'")));
            }
            if arg.starts_with('-') {
                return Err(UndoError::fallback(format!(
                    "chmod: unsupported option '{arg}' after mode"
                )));
            }
        }
        files.push(PathBuf::from(arg));
    }

    let Some(mode_str) = mode_str else {
        return Err(UndoError::fallback("chmod: missing operand"));
    };
    if files.is_empty() {
        return Err(UndoError::fallback(format!(
            "chmod: missing operand after '{mode_str}'"
        )));
    }
    let spec = modes::parse(&mode_str).map_err(|e| UndoError::fallback(format!("chmod: {e}")))?;
    Ok(ChmodArgs {
        recursive,
        verbose,
        changes_only,
        spec,
        files,
    })
}

pub fn run(argv: &[String]) -> Result<u8> {
    let args = parse_args(argv)?;
    let umask = ops::process_umask();

    let mut session = Session::new("chmod", argv)?;
    for f in &args.files {
        session.protect(f)?;
    }

    let mut targets: Vec<PathBuf> = Vec::new();
    for operand in &args.files {
        let meta = match fs::symlink_metadata(operand) {
            Ok(m) => m,
            Err(_) => {
                session.err(format!(
                    "cannot access '{}': No such file or directory",
                    operand.display()
                ));
                continue;
            }
        };
        let path = if meta.file_type().is_symlink() {
            match fs::canonicalize(operand) {
                Ok(resolved) => resolved,
                Err(_) => {
                    session.err(format!(
                        "cannot operate on dangling symlink '{}'",
                        operand.display()
                    ));
                    continue;
                }
            }
        } else {
            operand.clone()
        };
        targets.push(path.clone());
        if args.recursive
            && fs::symlink_metadata(&path)
                .map(|m| m.is_dir())
                .unwrap_or(false)
        {
            collect_recursive(&path, &mut targets, session.limits.tree_cap)?;
        }
    }

    for path in targets.into_iter().rev() {
        let meta = match fs::symlink_metadata(&path) {
            Ok(m) => m,
            Err(e) => {
                session.err(format!("cannot access '{}': {e}", path.display()));
                continue;
            }
        };
        let old = meta.mode() & 0o7777;
        let new = modes::apply(&args.spec, old, meta.is_dir(), umask);
        if new == old {
            if args.verbose {
                println!("mode of '{}' retained as {:04o}", path.display(), old);
            }
            continue;
        }
        session.begin()?;
        match fs::set_permissions(&path, fs::Permissions::from_mode(new)) {
            Ok(()) => {
                session.record(Action::SetMode {
                    path: path.clone(),
                    old,
                    new,
                })?;
                if args.verbose || args.changes_only {
                    println!(
                        "mode of '{}' changed from {:04o} to {:04o}",
                        path.display(),
                        old,
                        new
                    );
                }
            }
            Err(e) => {
                session.err(format!("changing permissions of '{}': {e}", path.display()));
            }
        }
    }

    session.finish()
}

fn collect_recursive(root: &PathBuf, targets: &mut Vec<PathBuf>, cap: u64) -> Result<()> {
    let entries =
        fs::read_dir(root).map_err(|e| UndoError::io(format!("reading {}", root.display()), e))?;
    for entry in entries {
        let entry = entry.map_err(|e| UndoError::io(format!("reading {}", root.display()), e))?;
        let path = entry.path();
        let meta = entry
            .metadata()
            .map_err(|e| UndoError::io(format!("inspecting {}", path.display()), e))?;
        if meta.file_type().is_symlink() {
            continue;
        }
        targets.push(path.clone());
        if targets.len() as u64 > cap {
            return Err(UndoError::fallback(format!(
                "chmod: recursion over {cap} nodes is not journaled"
            )));
        }
        if meta.is_dir() {
            collect_recursive(&path, targets, cap)?;
        }
    }
    Ok(())
}
