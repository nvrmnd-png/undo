use std::io::{IsTerminal, Write};
use std::process::ExitCode;

use clap::Parser;

use undo::cli::{Cli, Cmd};
use undo::engine::{Engine, Target};
use undo::error::{Result, UndoError};
use undo::journal::Journal;
use undo::output::{self, Format};
use undo::{exec, shellinit, tui};

fn main() -> ExitCode {
    let cli = Cli::parse();
    match run(cli) {
        Ok(code) => code,
        Err(e) => {
            if !matches!(e, UndoError::Fallback(_)) {
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
            let report = engine.undo(Target::Last, cli.force, cli.dry_run)?;
            output::print_report(format, &report)?;
            Ok(ok_exit(report.ok))
        }
        Some(Cmd::Redo) => {
            let mut engine = Engine::open()?;
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
            print!("{}", shellinit::snippet(shell));
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
