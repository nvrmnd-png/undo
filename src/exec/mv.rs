use std::path::PathBuf;

use clap::Parser;

use super::adapter::{Session, dest_for, fallback_parse, is_dir, same_file};
use crate::error::{Result, UndoError};
use crate::journal::model::Action;
use crate::ops::{self, MoveKind};
use crate::state;

#[derive(Debug, Parser)]
#[command(name = "mv", disable_help_flag = true, disable_version_flag = true)]
struct MvArgs {
    #[arg(short = 'f', long = "force", overrides_with_all = ["interactive", "no_clobber"])]
    force: bool,
    #[arg(short = 'i', long = "interactive", overrides_with_all = ["force", "no_clobber"])]
    interactive: bool,
    #[arg(short = 'n', long = "no-clobber", overrides_with_all = ["force", "interactive"])]
    no_clobber: bool,
    #[arg(short = 'v', long = "verbose")]
    verbose: bool,
    #[arg(short = 't', long = "target-directory", value_name = "DIR")]
    target_directory: Option<PathBuf>,
    #[arg(
        short = 'T',
        long = "no-target-directory",
        conflicts_with = "target_directory"
    )]
    no_target_directory: bool,
    #[arg(value_name = "PATH", num_args = 1..)]
    paths: Vec<PathBuf>,
}

pub fn run(argv: &[String]) -> Result<u8> {
    let args = MvArgs::try_parse_from(argv).map_err(|e| fallback_parse("mv", e))?;

    let (sources, dest, forced_direct) = match &args.target_directory {
        Some(t) => {
            if args.paths.is_empty() {
                return Err(UndoError::fallback("mv: missing file operand"));
            }
            (args.paths.clone(), t.clone(), false)
        }
        None => {
            if args.paths.len() < 2 {
                return Err(UndoError::fallback("mv: missing destination operand"));
            }
            let mut sources = args.paths.clone();
            let dest = sources.pop().expect("len checked");
            if args.no_target_directory && sources.len() > 1 {
                return Err(UndoError::fallback("mv: extra operand with -T"));
            }
            (sources, dest, args.no_target_directory)
        }
    };
    let into_dir = if args.target_directory.is_some() {
        true
    } else {
        !forced_direct && is_dir(&dest)
    };
    if args.target_directory.is_some() && !is_dir(&dest) {
        return Err(UndoError::fallback(format!(
            "mv: target '{}' is not a directory",
            dest.display()
        )));
    }
    if sources.len() > 1 && !into_dir {
        return Err(UndoError::fallback(format!(
            "mv: target '{}' is not a directory",
            dest.display()
        )));
    }

    let mut session = Session::new("mv", argv)?;
    session.verbose = args.verbose;
    for src in &sources {
        session.protect(src)?;
        session.protect(&dest_for(src, &dest, into_dir))?;
    }

    for src in &sources {
        let dst = dest_for(src, &dest, into_dir);

        let pre = match state::capture(src, &session.limits) {
            Ok(s) if s.is_absent() => {
                session.err(format!(
                    "cannot stat '{}': No such file or directory",
                    src.display()
                ));
                continue;
            }
            Ok(s) => s,
            Err(e) => {
                session.err(e);
                continue;
            }
        };
        if same_file(src, &dst) {
            session.err(format!(
                "'{}' and '{}' are the same file",
                src.display(),
                dst.display()
            ));
            continue;
        }

        let dst_exists = std::fs::symlink_metadata(&dst).is_ok();
        let mut backup = None;
        if dst_exists {
            if args.no_clobber {
                continue;
            }
            if args.interactive && !session.prompt(&format!("overwrite '{}'?", dst.display())) {
                continue;
            }
            session.begin()?;
            match session.park_in_trash(&dst) {
                Ok(t) => backup = Some(t),
                Err(e) => {
                    session.err(format!("cannot overwrite '{}': {e}", dst.display()));
                    continue;
                }
            }
        }

        session.begin()?;
        match ops::safe_move(src, &dst) {
            Ok(kind) => {
                let post = match kind {
                    MoveKind::Rename => pre.clone(),
                    MoveKind::Xdev => state::capture(&dst, &session.limits)?,
                };
                let action = match kind {
                    MoveKind::Rename => Action::Move {
                        src: src.clone(),
                        dst: dst.clone(),
                        pre,
                        post,
                        backup,
                    },
                    MoveKind::Xdev => Action::MoveXdev {
                        src: src.clone(),
                        dst: dst.clone(),
                        pre,
                        post,
                        backup,
                    },
                };
                session.record(action)?;
                if args.verbose {
                    println!("renamed '{}' -> '{}'", src.display(), dst.display());
                }
            }
            Err(e) => {
                session.err(format!(
                    "cannot move '{}' to '{}': {e}",
                    src.display(),
                    dst.display()
                ));
                if let Some(b) = backup.take() {
                    session.recover_backup(b)?;
                }
            }
        }
    }

    session.finish()
}
