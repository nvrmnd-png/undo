use std::io::{IsTerminal, Write};
use std::process::ExitCode;

use clap::Parser;

use undo::cli::{Cli, Cmd, ConfigAction};
use undo::engine::{Engine, Target};
use undo::error::{Result, UndoError};
use undo::journal::{self, Journal};
use undo::output::{self, Format};
use undo::{config, exec, paths, shellinit, tui, update};

fn main() -> ExitCode {
    let cli = Cli::parse();
    match run(cli) {
        Ok(code) => code,
        Err(e) => {
            if !matches!(e, UndoError::Fallback(_) | UndoError::Excluded(_)) {
                eprintln!("undo: {e}");
            }
            e.exit_code()
        }
    }
}

fn ok_exit(ok: bool) -> ExitCode {
    if ok {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}

fn run(cli: Cli) -> Result<ExitCode> {
    let format = Format::from_flags(cli.json, cli.yaml);
    match cli.cmd {
        None => {
            let mut engine = Engine::open()?;
            engine.maybe_autoprune();
            let report = engine.undo(Target::Last, cli.force, cli.dry_run)?;
            output::print_report(format, &report)?;
            Ok(ok_exit(report.ok))
        }
        Some(Cmd::Redo) => {
            let mut engine = Engine::open()?;
            engine.maybe_autoprune();
            let report = engine.redo(Target::Last, cli.force, cli.dry_run)?;
            output::print_report(format, &report)?;
            Ok(ok_exit(report.ok))
        }
        Some(Cmd::History { limit, all }) => {
            let journal = Journal::open_default()?;
            let entries = journal.history(if all { None } else { Some(limit) })?;
            output::print_history(format, &entries)?;
            Ok(ExitCode::SUCCESS)
        }
        Some(Cmd::List) => {
            let journal = Journal::open_default()?;
            let (undo_stack, redo_stack) = journal.stacks(20)?;
            output::print_list(format, &undo_stack, &redo_stack)?;
            Ok(ExitCode::SUCCESS)
        }
        Some(Cmd::Search { needle }) => {
            let journal = Journal::open_default()?;
            let entries = journal.search(&needle)?;
            output::print_search(format, &needle, &entries)?;
            Ok(ExitCode::SUCCESS)
        }
        Some(Cmd::Log { limit }) => {
            let journal = Journal::open_default()?;
            let entries = journal.history(Some(limit))?;
            output::print_log(format, &entries)?;
            Ok(ExitCode::SUCCESS)
        }
        Some(Cmd::Show { id }) => {
            let journal = Journal::open_default()?;
            let op = journal.get(id)?.ok_or_else(|| {
                UndoError::msg(format!(
                    "no journal entry #{id} for your user (see 'undo history')"
                ))
            })?;
            output::print_show(format, &op)?;
            Ok(ExitCode::SUCCESS)
        }
        Some(Cmd::Repair { yes }) => {
            let data_dir = paths::ensure_data_dir()?;
            match journal::db_health(&data_dir)? {
                None => println!("Journal database is healthy — nothing to repair."),
                Some(reason) => {
                    eprintln!("undo: database integrity check failed: {reason}");
                    if !yes
                        && !confirm(
                            "undo: back up and rebuild the database? Journal history may be lost; your trash is untouched.",
                        )?
                    {
                        eprintln!("undo: repair aborted");
                        return Ok(ExitCode::FAILURE);
                    }
                    let summary = journal::repair(&data_dir)?;
                    println!("{summary}");
                }
            }
            Ok(ExitCode::SUCCESS)
        }
        Some(Cmd::Clear { yes }) => {
            let journal = Journal::open_default()?;
            let count = journal.history(None)?.len();
            if count == 0 {
                output::print_clear(format, 0)?;
                return Ok(ExitCode::SUCCESS);
            }
            if !yes && !confirm_clear(count)? {
                eprintln!("undo: clear aborted");
                return Ok(ExitCode::FAILURE);
            }
            let cleared = journal.clear()?;
            output::print_clear(format, cleared)?;
            Ok(ExitCode::SUCCESS)
        }
        Some(Cmd::Config { action }) => {
            match action {
                Some(ConfigAction::Reset { yes }) => {
                    if !yes && !confirm("undo: reset all settings to defaults?")? {
                        eprintln!("undo: reset aborted");
                        return Ok(ExitCode::FAILURE);
                    }
                    let path = config::Config::default().save()?;
                    println!("Reset configuration to defaults ({}).", path.display());
                }
                Some(ConfigAction::Show) => {
                    output::print_config(format, &config::Config::load()?)?;
                }
                None => {
                    if format.is_machine() {
                        output::print_config(format, &config::Config::load()?)?;
                    } else {
                        tui::config::run()?;
                    }
                }
            }
            Ok(ExitCode::SUCCESS)
        }
        Some(Cmd::Prune {
            older_than,
            empty_trash,
            yes,
        }) => {
            let cfg = config::Config::load()?;
            let days = older_than.unwrap_or(cfg.cleanup.max_age_days);
            let mut engine = Engine::open()?;
            if cli.dry_run {
                let report = engine.prune(days, empty_trash, true)?;
                output::print_prune(format, &report)?;
                return Ok(ExitCode::SUCCESS);
            }
            let preview = engine.prune(days, empty_trash, true)?;
            if preview.candidates == 0 {
                output::print_prune(format, &preview)?;
                return Ok(ExitCode::SUCCESS);
            }
            if !yes {
                let n = preview.candidates;
                let e = if n == 1 { "y" } else { "ies" };
                let msg = if empty_trash {
                    format!(
                        "undo: remove {n} old journal entr{e} AND permanently delete their trashed files?"
                    )
                } else {
                    format!("undo: remove {n} old journal entr{e}? (trashed files are kept)")
                };
                if !confirm(&msg)? {
                    eprintln!("undo: prune aborted");
                    return Ok(ExitCode::FAILURE);
                }
            }
            let report = engine.prune(days, empty_trash, false)?;
            output::print_prune(format, &report)?;
            Ok(ExitCode::SUCCESS)
        }
        Some(Cmd::Exec { args }) => {
            if format.is_machine() {
                return Err(UndoError::usage(
                    "--json/--yaml are not available for 'exec' (its stdout belongs to the wrapped command)",
                ));
            }
            let code = exec::run(args)?;
            Ok(ExitCode::from(code))
        }
        Some(Cmd::Init { shell }) => {
            print!("{}", shellinit::emit(shell));
            Ok(ExitCode::SUCCESS)
        }
        Some(Cmd::Update { check, yes }) => {
            let current = env!("CARGO_PKG_VERSION");
            let release = update::latest_release()?;
            if !update::is_newer(&release.version, current) {
                println!("undo {current} is already up to date.");
                return Ok(ExitCode::SUCCESS);
            }
            println!(
                "A newer version is available: {} (you have {current}).",
                release.version
            );
            if check {
                println!("Run 'undo update' to install it.");
                return Ok(ExitCode::SUCCESS);
            }
            if !yes
                && !confirm(&format!(
                    "undo: download and install {} now?",
                    release.version
                ))?
            {
                eprintln!("undo: update aborted");
                return Ok(ExitCode::FAILURE);
            }
            let path = update::install(&release.tag)?;
            println!(
                "Updated to {} at {}. Restart your shell to use it.",
                release.version,
                path.display()
            );
            Ok(ExitCode::SUCCESS)
        }
        Some(Cmd::Tui) => {
            if format.is_machine() {
                return Err(UndoError::usage(
                    "--json/--yaml are not available for 'tui'",
                ));
            }
            tui::run()?;
            Ok(ExitCode::SUCCESS)
        }
    }
}

fn confirm(prompt: &str) -> Result<bool> {
    if !std::io::stdin().is_terminal() {
        return Err(UndoError::usage(
            "refusing to proceed without a terminal; pass --yes to confirm",
        ));
    }
    eprint!("{prompt} [y/N] ");
    std::io::stderr().flush().ok();
    let mut line = String::new();
    std::io::stdin().read_line(&mut line).ok();
    Ok(matches!(line.trim(), "y" | "Y" | "yes" | "Yes"))
}

fn confirm_clear(count: usize) -> Result<bool> {
    if !std::io::stdin().is_terminal() {
        return Err(UndoError::usage(
            "refusing to clear without a terminal; pass --yes to confirm",
        ));
    }
    eprint!(
        "undo: forget {count} journal entr{} for your user? Trash contents are NOT touched. [y/N] ",
        if count == 1 { "y" } else { "ies" }
    );
    std::io::stderr().flush().ok();
    let mut line = String::new();
    std::io::stdin().read_line(&mut line).ok();
    Ok(matches!(line.trim(), "y" | "Y" | "yes" | "Yes"))
}
