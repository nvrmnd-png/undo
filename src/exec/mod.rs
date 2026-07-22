pub mod adapter;
pub mod modes;
pub mod sed_expr;

mod chmod;
mod chown;
mod cp;
mod ln;
mod mkdir;
mod mv;
mod rename;
mod rm;
mod rmdir;

use std::ffi::OsString;
use std::path::Path;

use crate::error::{Result, UndoError};

pub fn run(args: Vec<OsString>) -> Result<u8> {
    let argv: Vec<String> = args
        .into_iter()
        .map(|a| {
            a.into_string()
                .map_err(|_| UndoError::fallback("non-UTF-8 argument (unsupported in v1)"))
        })
        .collect::<Result<_>>()?;
    let Some(first) = argv.first() else {
        return Err(UndoError::usage(
            "exec: missing command, e.g. `undo exec -- mv a b`",
        ));
    };
    let cmd = Path::new(first)
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| first.clone());

    if let Some(result) = dispatch(&cmd, &argv) {
        return result;
    }

    // Not a built-in: consult the user's plugin aliases.
    let config = crate::config::Config::load().unwrap_or_default();
    if let Some(target) = config.plugins.get(&cmd) {
        if let Some(result) = dispatch(target, &argv) {
            return result;
        }
        return Err(UndoError::usage(format!(
            "exec: alias '{cmd}' maps to unknown command '{target}' (must be one of: mv cp rm mkdir rmdir chmod chown ln rename)"
        )));
    }

    Err(UndoError::usage(format!(
        "exec: unsupported command '{cmd}' (supported: mv cp rm mkdir rmdir chmod chown ln rename)"
    )))
}

fn dispatch(builtin: &str, argv: &[String]) -> Option<Result<u8>> {
    Some(match builtin {
        "mv" => mv::run(argv),
        "cp" => cp::run(argv),
        "rm" => rm::run(argv),
        "mkdir" => mkdir::run(argv),
        "rmdir" => rmdir::run(argv),
        "chmod" => chmod::run(argv),
        "chown" => chown::run(argv),
        "ln" => ln::run(argv),
        "rename" => rename::run(argv),
        _ => return None,
    })
}
