use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;

use clap::Parser;

use super::adapter::{Session, fallback_parse};
use super::sed_expr;
use crate::error::{Result, UndoError};
use crate::journal::model::Action;
use crate::ops::{self, MoveKind};
use crate::state;

#[derive(Debug, Parser)]
#[command(name = "rename", disable_help_flag = true, disable_version_flag = true)]
struct RenameArgs {
    #[arg(short = 'v', long = "verbose")]
    verbose: bool,
    #[arg(short = 'n', long = "nono")]
    dry_run: bool,
    #[arg(short = 'f', long = "force")]
    force: bool,
    #[arg(value_name = "EXPR")]
    expr: String,
    #[arg(value_name = "FILE", num_args = 1..)]
    files: Vec<PathBuf>,
}

pub fn run(argv: &[String]) -> Result<u8> {
    let args = RenameArgs::try_parse_from(argv).map_err(|e| fallback_parse("rename", e))?;
    let subst =
        sed_expr::parse(&args.expr).map_err(|e| UndoError::fallback(format!("rename: {e}")))?;

    let mut pairs: Vec<(PathBuf, PathBuf)> = Vec::new();
    for file in &args.files {
        let name = file.to_string_lossy();
        if let Some(new) = subst.apply(&name) {
            pairs.push((file.clone(), PathBuf::from(new)));
        }
    }

    if args.dry_run {
        for (old, new) in &pairs {
            println!("rename({}, {})", old.display(), new.display());
        }
        return Ok(0);
    }
    if pairs.is_empty() {
        return Ok(0);
    }

    let mut session = Session::new("rename", argv)?;
    for (old, new) in &pairs {
        session.protect(old)?;
        session.protect(new)?;
    }

    let mut occupancy: HashMap<PathBuf, bool> = HashMap::new();
    let occupied = |occ: &mut HashMap<PathBuf, bool>, p: &PathBuf| -> bool {
        *occ.entry(p.clone())
            .or_insert_with(|| fs::symlink_metadata(p).is_ok())
    };
    let mut problems: Vec<String> = Vec::new();
    for (old, new) in &pairs {
        if !occupied(&mut occupancy, old) {
            problems.push(format!(
                "cannot rename '{}': No such file or directory",
                old.display()
            ));
            continue;
        }
        occupancy.insert(old.clone(), false);
        if occupied(&mut occupancy, new) && !args.force {
            problems.push(format!(
                "'{}' not renamed: '{}' already exists (use -f to overwrite)",
                old.display(),
                new.display()
            ));
            occupancy.insert(old.clone(), true);
            continue;
        }
        occupancy.insert(new.clone(), true);
    }
    if !problems.is_empty() {
        for p in problems {
            session.err(p);
        }
        eprintln!("rename: nothing renamed (the batch is executed all-or-nothing)");
        return session.finish();
    }

    for (old, new) in pairs {
        let pre = match state::capture(&old, &session.limits) {
            Ok(s) if !s.is_absent() => s,
            _ => {
                session.err(format!(
                    "cannot rename '{}': No such file or directory (raced?)",
                    old.display()
                ));
                continue;
            }
        };

        let mut backup = None;
        if fs::symlink_metadata(&new).is_ok() {
            session.begin()?;
            match session.park_in_trash(&new) {
                Ok(t) => backup = Some(t),
                Err(e) => {
                    session.err(format!("cannot overwrite '{}': {e}", new.display()));
                    continue;
                }
            }
        }

        session.begin()?;
        match ops::safe_move(&old, &new) {
            Ok(kind) => {
                let post = match kind {
                    MoveKind::Rename => pre.clone(),
                    MoveKind::Xdev => state::capture(&new, &session.limits)?,
                };
                let action = match kind {
                    MoveKind::Rename => Action::Move {
                        src: old.clone(),
                        dst: new.clone(),
                        pre,
                        post,
                        backup,
                    },
                    MoveKind::Xdev => Action::MoveXdev {
                        src: old.clone(),
                        dst: new.clone(),
                        pre,
                        post,
                        backup,
                    },
                };
                session.record(action)?;
                if args.verbose {
                    println!("{} renamed as {}", old.display(), new.display());
                }
            }
            Err(e) => {
                session.err(format!(
                    "cannot rename '{}' to '{}': {e}",
                    old.display(),
                    new.display()
                ));
                if let Some(b) = backup {
                    session.recover_backup(b)?;
                }
            }
        }
    }

    session.finish()
}
