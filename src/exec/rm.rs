use std::path::PathBuf;

use clap::Parser;

use super::adapter::{Session, dir_is_empty, fallback_parse};
use crate::error::Result;
use crate::journal::model::{Action, FileState};
use crate::state;

#[derive(Debug, Parser)]
#[command(name = "rm", disable_help_flag = true, disable_version_flag = true)]
struct RmArgs {
    #[arg(short = 'r', long = "recursive")]
    recursive: bool,
    #[arg(short = 'R', hide = true)]
    recursive_alias: bool,
    #[arg(short = 'f', long = "force", overrides_with = "interactive")]
    force: bool,
    #[arg(short = 'i', overrides_with = "force")]
    interactive: bool,
    #[arg(short = 'I')]
    interactive_once: bool,
    #[arg(short = 'd', long = "dir")]
    dir: bool,
    #[arg(short = 'v', long = "verbose")]
    verbose: bool,
    #[arg(value_name = "PATH")]
    paths: Vec<PathBuf>,
}

pub fn run(argv: &[String]) -> Result<u8> {
    let args = RmArgs::try_parse_from(argv).map_err(|e| fallback_parse("rm", e))?;
    let recursive = args.recursive || args.recursive_alias;

    if args.paths.is_empty() {
        return Err(crate::error::UndoError::fallback("rm: missing operand"));
    }

    let mut session = Session::new("rm", argv)?;
    for path in &args.paths {
        session.protect(path)?;
    }

    if args.interactive_once && (args.paths.len() > 3 || recursive) {
        let q = format!(
            "remove {} argument{}{}?",
            args.paths.len(),
            if args.paths.len() == 1 { "" } else { "s" },
            if recursive { " recursively" } else { "" }
        );
        if !session.prompt(&q) {
            return session.finish();
        }
    }

    for path in &args.paths {
        let recorded = match state::capture(path, &session.limits) {
            Ok(s) => s,
            Err(e) => {
                session.err(e);
                continue;
            }
        };
        match &recorded {
            FileState::Absent => {
                if !args.force {
                    session.err(format!(
                        "cannot remove '{}': No such file or directory",
                        path.display()
                    ));
                }
                continue;
            }
            FileState::Dir { .. } if !recursive => {
                if !args.dir {
                    session.err(format!(
                        "cannot remove '{}': Is a directory",
                        path.display()
                    ));
                    continue;
                }
                match dir_is_empty(path) {
                    Ok(true) => {}
                    Ok(false) => {
                        session.err(format!(
                            "cannot remove '{}': Directory not empty",
                            path.display()
                        ));
                        continue;
                    }
                    Err(e) => {
                        session.err(format!("cannot remove '{}': {e}", path.display()));
                        continue;
                    }
                }
            }
            _ => {}
        }

        if args.interactive {
            let kind = match &recorded {
                FileState::Dir { .. } => "directory",
                FileState::Symlink { .. } => "symbolic link",
                FileState::File { size: 0, .. } => "regular empty file",
                FileState::File { .. } => "regular file",
                _ => "file",
            };
            if !session.prompt(&format!("remove {kind} '{}'?", path.display())) {
                continue;
            }
        }

        session.begin()?;
        match session.park_in_trash(path) {
            Ok(tref) => {
                session.record(Action::TrashPut {
                    origin: path.clone(),
                    trash: tref,
                })?;
                if args.verbose {
                    println!("removed '{}'", path.display());
                }
            }
            Err(e) => {
                session.err(format!("cannot remove '{}': {e}", path.display()));
            }
        }
    }

    session.finish()
}
