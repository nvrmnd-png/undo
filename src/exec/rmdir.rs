use std::fs;
use std::path::{Path, PathBuf};

use clap::Parser;

use super::adapter::{Session, dir_is_empty, fallback_parse};
use crate::error::Result;
use crate::journal::model::Action;

#[derive(Debug, Parser)]
#[command(name = "rmdir", disable_help_flag = true, disable_version_flag = true)]
struct RmdirArgs {
    #[arg(short = 'p', long = "parents")]
    parents: bool,
    #[arg(long = "ignore-fail-on-non-empty")]
    ignore_non_empty: bool,
    #[arg(short = 'v', long = "verbose")]
    verbose: bool,
    #[arg(value_name = "DIR", num_args = 1..)]
    dirs: Vec<PathBuf>,
}

pub fn run(argv: &[String]) -> Result<u8> {
    let args = RmdirArgs::try_parse_from(argv).map_err(|e| fallback_parse("rmdir", e))?;

    let mut session = Session::new("rmdir", argv)?;
    session.verbose = args.verbose;

    let mut chains: Vec<Vec<PathBuf>> = Vec::new();
    for dir in &args.dirs {
        let mut chain = vec![dir.clone()];
        if args.parents {
            let mut cur = dir.as_path();
            while let Some(parent) = cur.parent() {
                if parent.as_os_str().is_empty() || parent == Path::new("/") {
                    break;
                }
                chain.push(parent.to_path_buf());
                cur = parent;
            }
        }
        for p in &chain {
            session.protect(p)?;
        }
        chains.push(chain);
    }

    for chain in chains {
        for dir in chain {
            if !remove_one(&mut session, &dir, args.ignore_non_empty, args.verbose)? {
                break;
            }
        }
    }

    session.finish()
}

fn remove_one(
    session: &mut Session,
    dir: &Path,
    ignore_non_empty: bool,
    verbose: bool,
) -> Result<bool> {
    let meta = match fs::symlink_metadata(dir) {
        Ok(m) => m,
        Err(_) => {
            session.err(format!(
                "failed to remove '{}': No such file or directory",
                dir.display()
            ));
            return Ok(false);
        }
    };
    if !meta.is_dir() {
        session.err(format!(
            "failed to remove '{}': Not a directory",
            dir.display()
        ));
        return Ok(false);
    }
    match dir_is_empty(dir) {
        Ok(true) => {}
        Ok(false) => {
            if !ignore_non_empty {
                session.err(format!(
                    "failed to remove '{}': Directory not empty",
                    dir.display()
                ));
            }
            return Ok(false);
        }
        Err(e) => {
            session.err(format!("failed to remove '{}': {e}", dir.display()));
            return Ok(false);
        }
    }

    if verbose {
        println!("rmdir: removing directory, '{}'", dir.display());
    }
    session.begin()?;
    match session.park_in_trash(dir) {
        Ok(tref) => {
            if !dir_is_empty(&tref.file).unwrap_or(false) {
                let restore = session.trash().restore(&tref);
                match restore {
                    Ok(_) => {
                        session.err(format!(
                            "failed to remove '{}': Directory not empty",
                            dir.display()
                        ));
                    }
                    Err(e) => {
                        session.record(Action::TrashPut {
                            origin: dir.to_path_buf(),
                            trash: tref,
                        })?;
                        session.err(format!(
                            "failed to remove '{}' cleanly: {e} (content parked in trash)",
                            dir.display()
                        ));
                    }
                }
                return Ok(false);
            }
            session.record(Action::TrashPut {
                origin: dir.to_path_buf(),
                trash: tref,
            })?;
            Ok(true)
        }
        Err(e) => {
            session.err(format!("failed to remove '{}': {e}", dir.display()));
            Ok(false)
        }
    }
}
