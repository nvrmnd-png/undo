use std::ffi::OsString;

use clap::{Parser, Subcommand, ValueEnum};

#[derive(Debug, Parser)]
#[command(
    name = "undo",
    version,
    about = "Undo shell filesystem operations (mv, cp, rm, … journaled with a trash safety net)",
    long_about = None
)]
pub struct Cli {
    #[command(subcommand)]
    pub cmd: Option<Cmd>,

    #[arg(long, global = true, conflicts_with = "yaml")]
    pub json: bool,

    #[arg(long, global = true)]
    pub yaml: bool,

    #[arg(short = 'f', long, global = true)]
    pub force: bool,

    #[arg(long, global = true)]
    pub dry_run: bool,
}

#[derive(Debug, Subcommand)]
pub enum Cmd {
    History {
        #[arg(short = 'n', long, default_value_t = 50)]
        limit: usize,
        #[arg(long)]
        all: bool,
    },
    Redo,
    List,
    Show {
        id: i64,
    },
    Clear {
        #[arg(long)]
        yes: bool,
    },
    Exec {
        #[arg(trailing_var_arg = true, allow_hyphen_values = true, required = true)]
        args: Vec<OsString>,
    },
    Init {
        #[arg(value_enum)]
        shell: ShellKind,
    },
    Tui,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum ShellKind {
    Zsh,
    Bash,
    Fish,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cli_definition_is_consistent() {
        use clap::CommandFactory;
        Cli::command().debug_assert();
    }

    #[test]
    fn exec_captures_hyphen_args() {
        let cli = Cli::parse_from(["undo", "exec", "--", "mv", "-v", "a", "b"]);
        match cli.cmd {
            Some(Cmd::Exec { args }) => {
                assert_eq!(args, vec!["mv", "-v", "a", "b"]);
            }
            other => panic!("expected exec, got {other:?}"),
        }
    }

    #[test]
    fn json_and_yaml_conflict() {
        assert!(Cli::try_parse_from(["undo", "--json", "--yaml", "list"]).is_err());
    }
}
