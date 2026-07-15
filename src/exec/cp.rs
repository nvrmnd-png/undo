use std::fs;
use std::path::{Path, PathBuf};

use clap::Parser;

use super::adapter::{Session, dest_for, fallback_parse, is_dir, same_file};
use crate::error::{Result, UndoError};
use crate::journal::model::Action;
use crate::ops;
use crate::state;

#[derive(Debug, Parser)]
#[command(name = "cp", disable_help_flag = true, disable_version_flag = true)]
struct CpArgs {
    #[arg(short = 'r', long = "recursive")]
    recursive: bool,
    #[arg(short = 'R', hide = true)]
    recursive_alias: bool,
    #[arg(short = 'a', long = "archive")]
    archive: bool,
    #[arg(short = 'd')]
    no_deref_d: bool,
    #[arg(short = 'P', long = "no-dereference")]
    no_deref: bool,
    #[arg(short = 'p')]
    preserve_p: bool,
    #[arg(long = "preserve", value_delimiter = ',', num_args = 0..=1,
          default_missing_value = "mode,ownership,timestamps")]
    preserve: Option<Vec<String>>,
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

enum Step {
    File { src: PathBuf, dst: PathBuf },
    Symlink { src: PathBuf, dst: PathBuf },
    MergeDir { src: PathBuf, dst: PathBuf },
    FreshTree { src: PathBuf, dst: PathBuf },
}

pub fn run(argv: &[String]) -> Result<u8> {
    let args = CpArgs::try_parse_from(argv).map_err(|e| fallback_parse("cp", e))?;
    let recursive = args.recursive || args.recursive_alias || args.archive;
    let no_deref = args.no_deref_d || args.no_deref || args.archive;
    let mut preserve = args.preserve_p || args.archive;
    if let Some(list) = &args.preserve {
        for item in list {
            match item.as_str() {
                "mode" | "ownership" | "timestamps" | "all" => preserve = true,
                other => {
                    return Err(UndoError::fallback(format!(
                        "cp: --preserve={other} is not supported"
                    )));
                }
            }
        }
    }

    let (sources, dest, forced_direct) = match &args.target_directory {
        Some(t) => {
            if args.paths.is_empty() {
                return Err(UndoError::fallback("cp: missing file operand"));
            }
            (args.paths.clone(), t.clone(), false)
        }
        None => {
            if args.paths.len() < 2 {
                return Err(UndoError::fallback("cp: missing destination operand"));
            }
            let mut sources = args.paths.clone();
            let dest = sources.pop().expect("len checked");
            if args.no_target_directory && sources.len() > 1 {
                return Err(UndoError::fallback("cp: extra operand with -T"));
            }
            (sources, dest, args.no_target_directory)
        }
    };
    if args.target_directory.is_some() && !is_dir(&dest) {
        return Err(UndoError::fallback(format!(
            "cp: target '{}' is not a directory",
            dest.display()
        )));
    }
    let into_dir = if args.target_directory.is_some() {
        true
    } else {
        !forced_direct && is_dir(&dest)
    };
    if sources.len() > 1 && !into_dir {
        return Err(UndoError::fallback(format!(
            "cp: target '{}' is not a directory",
            dest.display()
        )));
    }

    let mut session = Session::new("cp", argv)?;
    session.verbose = args.verbose;

    let mut steps: Vec<Step> = Vec::new();
    let mut planned_errors: Vec<String> = Vec::new();

    for src in &sources {
        let dst = dest_for(src, &dest, into_dir);
        session.protect(src)?;
        session.protect(&dst)?;

        let src_meta = match fs::symlink_metadata(src) {
            Ok(m) => m,
            Err(_) => {
                planned_errors.push(format!(
                    "cannot stat '{}': No such file or directory",
                    src.display()
                ));
                continue;
            }
        };

        if src_meta.file_type().is_symlink() {
            if no_deref {
                steps.push(Step::Symlink {
                    src: src.clone(),
                    dst,
                });
                continue;
            }
            match fs::metadata(src) {
                Ok(m) if m.is_dir() => {
                    plan_dir(
                        &mut session,
                        src,
                        &dst,
                        recursive,
                        &mut steps,
                        &mut planned_errors,
                    )?;
                }
                Ok(_) => steps.push(Step::File {
                    src: src.clone(),
                    dst,
                }),
                Err(_) => planned_errors
                    .push(format!("cannot stat '{}': dangling symlink", src.display())),
            }
            continue;
        }

        if src_meta.is_dir() {
            plan_dir(
                &mut session,
                src,
                &dst,
                recursive,
                &mut steps,
                &mut planned_errors,
            )?;
        } else if src_meta.is_file() {
            if same_file(src, &dst) {
                planned_errors.push(format!(
                    "'{}' and '{}' are the same file",
                    src.display(),
                    dst.display()
                ));
                continue;
            }
            steps.push(Step::File {
                src: src.clone(),
                dst,
            });
        } else {
            return Err(UndoError::fallback(format!(
                "cp: special file '{}' is not supported",
                src.display()
            )));
        }
    }

    for msg in planned_errors {
        session.err(msg);
    }

    for step in steps {
        match step {
            Step::File { src, dst } => {
                copy_one_file(&mut session, &src, &dst, &args, preserve)?;
            }
            Step::Symlink { src, dst } => {
                copy_one_symlink(&mut session, &src, &dst, &args)?;
            }
            Step::MergeDir { src, dst } => match fs::symlink_metadata(&dst) {
                Ok(m) if m.is_dir() => {}
                Ok(_) => {
                    session.err(format!(
                        "cannot overwrite non-directory '{}' with directory '{}'",
                        dst.display(),
                        src.display()
                    ));
                }
                Err(_) => {
                    use std::os::unix::fs::PermissionsExt;
                    let mode = dir_mode_for(&src, preserve);
                    session.begin()?;
                    if let Err(e) = fs::create_dir(&dst) {
                        session.err(format!("cannot create directory '{}': {e}", dst.display()));
                        continue;
                    }
                    let _ = fs::set_permissions(&dst, fs::Permissions::from_mode(mode));
                    let post = state::capture(&dst, &session.limits)?;
                    session.record(Action::CreateDir {
                        path: dst.clone(),
                        mode,
                        post,
                    })?;
                    if args.verbose {
                        println!("'{}' -> '{}'", src.display(), dst.display());
                    }
                }
            },
            Step::FreshTree { src, dst } => {
                session.begin()?;
                match ops::copy_tree_for_cp(&src, &dst, preserve) {
                    Ok(()) => {
                        let src_root = state::capture(&src, &session.limits)?;
                        let dst_root = state::capture(&dst, &session.limits)?;
                        let summary = state::summarize_tree(&dst, &session.limits)?;
                        session.record(Action::CopyTree {
                            src: src.clone(),
                            dst: dst.clone(),
                            src_root,
                            dst_root,
                            summary,
                            preserve,
                        })?;
                        if args.verbose {
                            println!("'{}' -> '{}'", src.display(), dst.display());
                        }
                    }
                    Err(e) => {
                        if fs::symlink_metadata(&dst).is_ok() {
                            let _ = ops::remove_node(&dst);
                        }
                        session.err(format!(
                            "cannot copy '{}' to '{}': {e}",
                            src.display(),
                            dst.display()
                        ));
                    }
                }
            }
        }
    }

    session.finish()
}

fn plan_dir(
    session: &mut Session,
    src: &Path,
    dst: &Path,
    recursive: bool,
    steps: &mut Vec<Step>,
    planned_errors: &mut Vec<String>,
) -> Result<()> {
    if !recursive {
        planned_errors.push(format!(
            "-r not specified; omitting directory '{}'",
            src.display()
        ));
        return Ok(());
    }
    match fs::symlink_metadata(dst) {
        Err(_) => {
            scan_tree_for_fallback(src, &mut 0, u64::MAX)?;
            steps.push(Step::FreshTree {
                src: src.to_path_buf(),
                dst: dst.to_path_buf(),
            });
        }
        Ok(m) if m.is_dir() => {
            let mut count = 0u64;
            scan_tree_for_fallback(src, &mut count, session.limits.tree_cap)?;
            steps.push(Step::MergeDir {
                src: src.to_path_buf(),
                dst: dst.to_path_buf(),
            });
            expand_merge(src, dst, steps)?;
        }
        Ok(_) => {
            planned_errors.push(format!(
                "cannot overwrite non-directory '{}' with directory '{}'",
                dst.display(),
                src.display()
            ));
        }
    }
    Ok(())
}

fn scan_tree_for_fallback(root: &Path, count: &mut u64, cap: u64) -> Result<()> {
    let entries =
        fs::read_dir(root).map_err(|e| UndoError::io(format!("reading {}", root.display()), e))?;
    for entry in entries {
        let entry = entry.map_err(|e| UndoError::io(format!("reading {}", root.display()), e))?;
        *count += 1;
        if *count > cap {
            return Err(UndoError::fallback(format!(
                "cp: merge of '{}' exceeds {} nodes",
                root.display(),
                cap
            )));
        }
        let ft = entry
            .metadata()
            .map_err(|e| UndoError::io(format!("reading {}", entry.path().display()), e))?
            .file_type();
        if ft.is_dir() {
            scan_tree_for_fallback(&entry.path(), count, cap)?;
        } else if !ft.is_file() && !ft.is_symlink() {
            return Err(UndoError::fallback(format!(
                "cp: special file '{}' is not supported",
                entry.path().display()
            )));
        }
    }
    Ok(())
}

fn expand_merge(src: &Path, dst: &Path, steps: &mut Vec<Step>) -> Result<()> {
    let entries =
        fs::read_dir(src).map_err(|e| UndoError::io(format!("reading {}", src.display()), e))?;
    for entry in entries {
        let entry = entry.map_err(|e| UndoError::io(format!("reading {}", src.display()), e))?;
        let from = entry.path();
        let to = dst.join(entry.file_name());
        let ft = entry
            .metadata()
            .map_err(|e| UndoError::io(format!("reading {}", from.display()), e))?
            .file_type();
        if ft.is_dir() {
            steps.push(Step::MergeDir {
                src: from.clone(),
                dst: to.clone(),
            });
            expand_merge(&from, &to, steps)?;
        } else if ft.is_symlink() {
            steps.push(Step::Symlink { src: from, dst: to });
        } else {
            steps.push(Step::File { src: from, dst: to });
        }
    }
    Ok(())
}

fn clear_destination(
    session: &mut Session,
    src: &Path,
    dst: &Path,
    args: &CpArgs,
) -> std::result::Result<Option<crate::journal::model::TrashRef>, ()> {
    match fs::symlink_metadata(dst) {
        Err(_) => Ok(None),
        Ok(m) if m.is_dir() => {
            session.err(format!(
                "cannot overwrite directory '{}' with non-directory '{}'",
                dst.display(),
                src.display()
            ));
            Err(())
        }
        Ok(_) => {
            if args.no_clobber {
                return Err(());
            }
            if args.interactive && !session.prompt(&format!("overwrite '{}'?", dst.display())) {
                return Err(());
            }
            if session.begin().is_err() {
                return Err(());
            }
            match session.park_in_trash(dst) {
                Ok(t) => Ok(Some(t)),
                Err(e) => {
                    session.err(format!("cannot overwrite '{}': {e}", dst.display()));
                    Err(())
                }
            }
        }
    }
}

fn copy_one_file(
    session: &mut Session,
    src: &Path,
    dst: &Path,
    args: &CpArgs,
    preserve: bool,
) -> Result<()> {
    let backup = match clear_destination(session, src, dst, args) {
        Ok(b) => b,
        Err(()) => return Ok(()),
    };
    session.begin()?;
    let src_state = state::capture(src, &session.limits)?;
    match ops::copy_file_for_cp(src, dst, preserve) {
        Ok(()) => {
            let post = state::capture(dst, &session.limits)?;
            session.record(Action::Copy {
                src: src.to_path_buf(),
                dst: dst.to_path_buf(),
                src_state,
                post,
                preserve,
                backup,
            })?;
            if args.verbose {
                println!("'{}' -> '{}'", src.display(), dst.display());
            }
        }
        Err(e) => {
            session.err(format!(
                "cannot copy '{}' to '{}': {e}",
                src.display(),
                dst.display()
            ));
            if let Some(b) = backup {
                session.recover_backup(b)?;
            }
        }
    }
    Ok(())
}

fn copy_one_symlink(session: &mut Session, src: &Path, dst: &Path, args: &CpArgs) -> Result<()> {
    let backup = match clear_destination(session, src, dst, args) {
        Ok(b) => b,
        Err(()) => return Ok(()),
    };
    session.begin()?;
    let target = match fs::read_link(src) {
        Ok(t) => t,
        Err(e) => {
            session.err(format!("cannot read link '{}': {e}", src.display()));
            if let Some(b) = backup {
                session.recover_backup(b)?;
            }
            return Ok(());
        }
    };
    match std::os::unix::fs::symlink(&target, dst) {
        Ok(()) => {
            let post = state::capture(dst, &session.limits)?;
            session.record(Action::Symlink {
                target,
                link: dst.to_path_buf(),
                post,
                backup,
            })?;
            if args.verbose {
                println!("'{}' -> '{}'", src.display(), dst.display());
            }
        }
        Err(e) => {
            session.err(format!("cannot create symlink '{}': {e}", dst.display()));
            if let Some(b) = backup {
                session.recover_backup(b)?;
            }
        }
    }
    Ok(())
}

fn dir_mode_for(src: &Path, preserve: bool) -> u32 {
    use std::os::unix::fs::MetadataExt;
    let src_mode = fs::metadata(src)
        .map(|m| m.mode() & 0o7777)
        .unwrap_or(0o755);
    if preserve {
        src_mode
    } else {
        src_mode & !ops::process_umask()
    }
}
