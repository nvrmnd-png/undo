use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use crate::error::{Result, UndoError};
use crate::journal::Journal;
use crate::journal::model::{Action, Details, TrashRef};
use crate::lockfile::{self, LockGuard};
use crate::paths::{self, Limits};
use crate::trash::Trash;

pub fn fallback_parse(cmd: &str, err: clap::Error) -> UndoError {
    UndoError::fallback(format!("{cmd}: unsupported invocation ({})", err.kind()))
}

pub struct Session {
    pub cmd: &'static str,
    pub argv: Vec<String>,
    pub cwd: PathBuf,
    pub limits: Limits,
    pub verbose: bool,
    trash: Trash,
    journal: Journal,
    data_dir: PathBuf,
    home: PathBuf,
    lock: Option<LockGuard>,
    pending_id: Option<i64>,
    details: Details,
    failed: bool,
    excludes: Vec<PathBuf>,
    logging: bool,
}

impl Session {
    pub fn new(cmd: &'static str, argv: &[String]) -> Result<Session> {
        let cwd =
            std::env::current_dir().map_err(|e| UndoError::io("resolving working directory", e))?;
        let data_dir = paths::ensure_data_dir()?;
        let config = crate::config::Config::load().unwrap_or_default();
        let excludes = config
            .exclude
            .paths
            .iter()
            .filter_map(|p| paths::absolutize(Path::new(p)).ok())
            .collect();
        Ok(Session {
            cmd,
            argv: argv.to_vec(),
            cwd,
            limits: Limits::from_env(),
            verbose: false,
            trash: Trash::home()?,
            journal: Journal::open(&data_dir)?,
            data_dir,
            home: paths::home_dir()?,
            lock: None,
            pending_id: None,
            details: Details::new(),
            failed: false,
            excludes,
            logging: config.logging.enabled,
        })
    }

    pub fn trash(&self) -> &Trash {
        &self.trash
    }

    pub fn err(&mut self, msg: impl std::fmt::Display) {
        eprintln!("{}: {}", self.cmd, msg);
        self.failed = true;
    }

    pub fn has_failed(&self) -> bool {
        self.failed
    }

    pub fn protect(&self, path: &Path) -> Result<()> {
        let abs = paths::absolutize(path)?;
        if abs == Path::new("/") {
            return Err(UndoError::usage(format!(
                "{}: refusing to operate on '/'",
                self.cmd
            )));
        }
        if abs == self.home {
            return Err(UndoError::usage(format!(
                "{}: refusing to operate on your home directory",
                self.cmd
            )));
        }
        if abs.starts_with(self.trash.root()) {
            return Err(UndoError::usage(format!(
                "{}: refusing to operate inside the trash ({})",
                self.cmd,
                abs.display()
            )));
        }
        if abs.starts_with(&self.data_dir) {
            return Err(UndoError::usage(format!(
                "{}: refusing to operate on undo's own data ({})",
                self.cmd,
                abs.display()
            )));
        }
        for ex in &self.excludes {
            if abs == *ex || abs.starts_with(ex) {
                return Err(UndoError::excluded(format!(
                    "{}: {} is on the exclude list; not journaling",
                    self.cmd,
                    abs.display()
                )));
            }
        }
        Ok(())
    }

    pub fn prompt(&self, question: &str) -> bool {
        eprint!("{}: {} ", self.cmd, question);
        io::stderr().flush().ok();
        let mut line = String::new();
        if io::stdin().read_line(&mut line).is_err() {
            return false;
        }
        matches!(line.trim_start().chars().next(), Some('y') | Some('Y'))
    }

    pub fn begin(&mut self) -> Result<()> {
        if self.pending_id.is_some() {
            return Ok(());
        }
        let lock = lockfile::acquire(&self.data_dir)?;
        for id in self.journal.sweep_pending()? {
            eprintln!("undo: warning: operation #{id} was interrupted mid-flight — marked broken");
        }
        let id = self
            .journal
            .insert_pending(self.cmd, &self.argv, &self.cwd)?;
        self.lock = Some(lock);
        self.pending_id = Some(id);
        Ok(())
    }

    pub fn began(&self) -> bool {
        self.pending_id.is_some()
    }

    pub fn record(&mut self, mut action: Action) -> Result<()> {
        let id = self
            .pending_id
            .expect("record() called before begin() — adapter bug");
        crate::ops::absolutize_action(&mut action)?;
        self.details.actions.push(action);
        self.journal.update_details(id, &self.details)
    }

    pub fn park_in_trash(&self, path: &Path) -> Result<TrashRef> {
        self.trash.put(path, &self.limits)
    }

    pub fn recover_backup(&mut self, backup: TrashRef) -> Result<()> {
        match self.trash.restore(&backup) {
            Ok(warnings) => {
                for w in warnings {
                    eprintln!("undo: warning: {w}");
                }
                Ok(())
            }
            Err(e) => {
                eprintln!(
                    "undo: warning: could not restore {} after a failed step: {e} — it stays recoverable in the trash",
                    backup.origin.display()
                );
                self.record(Action::TrashPut {
                    origin: backup.origin.clone(),
                    trash: backup,
                })
            }
        }
    }

    pub fn finish(mut self) -> Result<u8> {
        if let Some(id) = self.pending_id {
            if self.details.actions.is_empty() {
                self.journal.delete_row(id)?;
            } else {
                self.journal.finalize_exec(id, &self.details)?;
                self.log_operation();
            }
        }
        Ok(if self.failed { 1 } else { 0 })
    }

    fn log_operation(&self) {
        if !self.logging {
            return;
        }
        let ts = jiff::Zoned::now().strftime("%Y-%m-%dT%H:%M:%S%:z");
        let line = format!(
            "{ts}\tuid={}\t{}\t{}\tcwd={}\n",
            paths::euid(),
            self.cmd,
            self.argv.join(" "),
            self.cwd.display()
        );
        let logfile = self.data_dir.join("undo.log");
        if let Ok(mut f) = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&logfile)
        {
            let _ = f.write_all(line.as_bytes());
        }
    }
}

pub fn dest_for(src: &Path, dest: &Path, into_dir: bool) -> PathBuf {
    if into_dir {
        match src.file_name() {
            Some(name) => dest.join(name),
            None => dest.join(src.as_os_str()),
        }
    } else {
        dest.to_path_buf()
    }
}

pub fn same_file(a: &Path, b: &Path) -> bool {
    match (fs::canonicalize(a), fs::canonicalize(b)) {
        (Ok(x), Ok(y)) => x == y,
        _ => false,
    }
}

pub fn is_dir(path: &Path) -> bool {
    fs::metadata(path).map(|m| m.is_dir()).unwrap_or(false)
}

pub fn dir_is_empty(path: &Path) -> io::Result<bool> {
    Ok(fs::read_dir(path)?.next().is_none())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dest_for_joins_or_replaces() {
        assert_eq!(
            dest_for(Path::new("a/b.txt"), Path::new("dir"), true),
            PathBuf::from("dir/b.txt")
        );
        assert_eq!(
            dest_for(Path::new("a/b.txt"), Path::new("c.txt"), false),
            PathBuf::from("c.txt")
        );
    }

    #[test]
    fn same_file_handles_missing() {
        assert!(!same_file(
            Path::new("/nonexistent-a"),
            Path::new("/nonexistent-b")
        ));
    }
}
