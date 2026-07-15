use std::fs;
use std::path::{Path, PathBuf};

use clap::Parser;

use super::adapter::{Session, fallback_parse};
use crate::error::{Result, UndoError};
use crate::journal::model::Action;
use crate::state;

#[derive(Debug, Parser)]
#[command(name = "ln", disable_help_flag = true, disable_version_flag = true)]
struct LnArgs {
    #[arg(short = 's', long = "symbolic")]
    symbolic: bool,
    #[arg(short = 'f', long = "force")]
    force: bool,
    #[arg(short = 'n', long = "no-dereference")]
    no_deref: bool,
    #[arg(
        short = 'T',
        long = "no-target-directory",
        conflicts_with = "target_directory"
    )]
    no_target_directory: bool,
    #[arg(short = 't', long = "target-directory", value_name = "DIR")]
    target_directory: Option<PathBuf>,
    #[arg(short = 'v', long = "verbose")]
    verbose: bool,
    #[arg(value_name = "PATH", num_args = 1..)]
    paths: Vec<PathBuf>,
}

fn link_location(target: &Path, dest: &Path, into_dir: bool) -> PathBuf {
    if into_dir {
        match target.file_name() {
            Some(name) => dest.join(name),
            None => dest.join(target.as_os_str()),
        }
    } else {
        dest.to_path_buf()
    }
}

fn treat_as_dir(path: &Path, no_deref: bool) -> bool {
    match fs::symlink_metadata(path) {
        Ok(m) if m.is_dir() => true,
        Ok(m) if m.file_type().is_symlink() && !no_deref => {
            fs::metadata(path).map(|t| t.is_dir()).unwrap_or(false)
        }
        _ => false,
    }
}

pub fn run(argv: &[String]) -> Result<u8> {
    let args = LnArgs::try_parse_from(argv).map_err(|e| fallback_parse("ln", e))?;

    let pairs: Vec<(PathBuf, PathBuf)> = match &args.target_directory {
        Some(dir) => {
            if args.paths.is_empty() {
                return Err(UndoError::fallback("ln: missing file operand"));
            }
            args.paths
                .iter()
                .map(|t| (t.clone(), link_location(t, dir, true)))
                .collect()
        }
        None => match args.paths.len() {
            0 => return Err(UndoError::fallback("ln: missing file operand")),
            1 => {
                let target = args.paths[0].clone();
                let link = link_location(&target, Path::new("."), true);
                vec![(target, link)]
            }
            2 => {
                let target = args.paths[0].clone();
                let dest = args.paths[1].clone();
                let into = !args.no_target_directory && treat_as_dir(&dest, args.no_deref);
                vec![(target.clone(), link_location(&target, &dest, into))]
            }
            _ => {
                if args.no_target_directory {
                    return Err(UndoError::fallback("ln: extra operand with -T"));
                }
                let (targets, dest) = args.paths.split_at(args.paths.len() - 1);
                let dest = &dest[0];
                if !treat_as_dir(dest, args.no_deref) {
                    return Err(UndoError::fallback(format!(
                        "ln: target '{}' is not a directory",
                        dest.display()
                    )));
                }
                targets
                    .iter()
                    .map(|t| (t.clone(), link_location(t, dest, true)))
                    .collect()
            }
        },
    };

    let mut session = Session::new("ln", argv)?;
    for (_, link) in &pairs {
        session.protect(link)?;
    }

    for (target, link) in pairs {
        if !args.symbolic && fs::symlink_metadata(&target).is_err() {
            session.err(format!(
                "failed to access '{}': No such file or directory",
                target.display()
            ));
            continue;
        }

        let mut backup = None;
        if fs::symlink_metadata(&link).is_ok() {
            if !args.force {
                session.err(format!(
                    "failed to create link '{}': File exists",
                    link.display()
                ));
                continue;
            }
            session.begin()?;
            match session.park_in_trash(&link) {
                Ok(t) => backup = Some(t),
                Err(e) => {
                    session.err(format!("cannot replace '{}': {e}", link.display()));
                    continue;
                }
            }
        }

        session.begin()?;
        let result = if args.symbolic {
            std::os::unix::fs::symlink(&target, &link)
        } else {
            fs::hard_link(&target, &link)
        };
        match result {
            Ok(()) => {
                let post = state::capture(&link, &session.limits)?;
                let action = if args.symbolic {
                    Action::Symlink {
                        target: target.clone(),
                        link: link.clone(),
                        post,
                        backup,
                    }
                } else {
                    Action::Hardlink {
                        src: target.clone(),
                        link: link.clone(),
                        post,
                        backup,
                    }
                };
                session.record(action)?;
                if args.verbose {
                    if args.symbolic {
                        println!("'{}' -> '{}'", link.display(), target.display());
                    } else {
                        println!("'{}' => '{}'", link.display(), target.display());
                    }
                }
            }
            Err(e) => {
                session.err(format!(
                    "failed to create link '{}' -> '{}': {e}",
                    link.display(),
                    target.display()
                ));
                if let Some(b) = backup {
                    session.recover_backup(b)?;
                }
            }
        }
    }

    session.finish()
}
