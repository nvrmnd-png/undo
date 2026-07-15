use std::fs;
use std::io;
use std::os::unix::fs::PermissionsExt;
use std::path::{Component, Path, PathBuf};

use clap::Parser;

use super::adapter::{Session, fallback_parse};
use super::modes;
use crate::error::{Result, UndoError};
use crate::journal::model::Action;
use crate::ops;
use crate::state;

#[derive(Debug, Parser)]
#[command(name = "mkdir", disable_help_flag = true, disable_version_flag = true)]
struct MkdirArgs {
    #[arg(short = 'p', long = "parents")]
    parents: bool,
    #[arg(
        short = 'm',
        long = "mode",
        value_name = "MODE",
        allow_hyphen_values = true
    )]
    mode: Option<String>,
    #[arg(short = 'v', long = "verbose")]
    verbose: bool,
    #[arg(value_name = "DIR", num_args = 1..)]
    dirs: Vec<PathBuf>,
}

pub fn run(argv: &[String]) -> Result<u8> {
    let args = MkdirArgs::try_parse_from(argv).map_err(|e| fallback_parse("mkdir", e))?;
    let umask = ops::process_umask();
    let default_mode = 0o777 & !umask;
    let final_mode = match &args.mode {
        Some(spec) => {
            let parsed = modes::parse(spec)
                .map_err(|e| UndoError::fallback(format!("mkdir: unsupported mode: {e}")))?;
            modes::apply(&parsed, default_mode, true, umask)
        }
        None => default_mode,
    };
    let intermediate_mode = default_mode | 0o300;

    let mut session = Session::new("mkdir", argv)?;
    session.verbose = args.verbose;
    for dir in &args.dirs {
        session.protect(dir)?;
    }

    for dir in &args.dirs {
        if args.parents {
            make_parents(
                &mut session,
                dir,
                intermediate_mode,
                final_mode,
                args.verbose,
            )?;
        } else {
            session.begin()?;
            match fs::create_dir(dir) {
                Ok(()) => {
                    record_created(&mut session, dir.clone(), final_mode, args.verbose)?;
                }
                Err(e) => {
                    let reason = if e.kind() == io::ErrorKind::AlreadyExists {
                        "File exists".to_string()
                    } else {
                        e.to_string()
                    };
                    session.err(format!(
                        "cannot create directory '{}': {reason}",
                        dir.display()
                    ));
                }
            }
        }
    }

    session.finish()
}

fn record_created(session: &mut Session, path: PathBuf, mode: u32, verbose: bool) -> Result<()> {
    fs::set_permissions(&path, fs::Permissions::from_mode(mode))
        .map_err(|e| UndoError::io(format!("setting mode on {}", path.display()), e))?;
    let post = state::capture(&path, &session.limits)?;
    if verbose {
        println!("created directory '{}'", path.display());
    }
    session.record(Action::CreateDir { path, mode, post })
}

fn make_parents(
    session: &mut Session,
    dir: &Path,
    intermediate_mode: u32,
    final_mode: u32,
    verbose: bool,
) -> Result<()> {
    let mut cur = PathBuf::new();
    let components: Vec<Component> = dir.components().collect();
    let last = components.len().saturating_sub(1);
    for (i, comp) in components.into_iter().enumerate() {
        cur.push(comp);
        if matches!(
            comp,
            Component::RootDir | Component::CurDir | Component::ParentDir
        ) {
            continue;
        }
        match fs::symlink_metadata(&cur) {
            Ok(m) if m.is_dir() => continue,
            Ok(m)
                if m.file_type().is_symlink()
                    && fs::metadata(&cur).map(|t| t.is_dir()).unwrap_or(false) =>
            {
                continue;
            }
            Ok(_) => {
                session.err(format!(
                    "cannot create directory '{}': Not a directory",
                    cur.display()
                ));
                return Ok(());
            }
            Err(_) => {}
        }
        session.begin()?;
        match fs::create_dir(&cur) {
            Ok(()) => {
                let mode = if i == last {
                    final_mode
                } else {
                    intermediate_mode
                };
                record_created(session, cur.clone(), mode, verbose)?;
            }
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(e) => {
                session.err(format!("cannot create directory '{}': {e}", cur.display()));
                return Ok(());
            }
        }
    }
    Ok(())
}
