use std::path::PathBuf;

use crate::error::{Result, UndoError};
use crate::journal::Journal;
use crate::journal::model::{Operation, Status};
use crate::lockfile;
use crate::ops::{self, Change, Direction, OpCtx};
use crate::paths::{self, Limits};
use crate::state::Conflict;
use crate::trash::Trash;

#[derive(Debug, Clone, Copy)]
pub enum Target {
    Last,
    Id(i64),
}

#[derive(Debug)]
pub struct Report {
    pub action: &'static str,
    pub entry: Operation,
    pub changes: Vec<Change>,
    pub warnings: Vec<String>,
    pub conflicts: Vec<Conflict>,
    pub error: Option<String>,
    pub ok: bool,
    pub dry_run: bool,
}

#[derive(Debug)]
pub struct PruneReport {
    pub candidates: usize,
    pub removed: usize,
    pub trash_purged: usize,
    pub size_before: u64,
    pub size_after: u64,
    pub dry_run: bool,
    pub empty_trash: bool,
}

pub struct Engine {
    pub journal: Journal,
    trash: Trash,
    limits: Limits,
    data_dir: PathBuf,
}

impl Engine {
    pub fn open() -> Result<Engine> {
        let data_dir = paths::ensure_data_dir()?;
        Ok(Engine {
            journal: Journal::open(&data_dir)?,
            trash: Trash::home()?,
            limits: Limits::from_env(),
            data_dir,
        })
    }

    pub fn prune(
        &mut self,
        older_than_days: u64,
        empty_trash: bool,
        dry_run: bool,
    ) -> Result<PruneReport> {
        let _lock = lockfile::acquire(&self.data_dir)?;
        let cutoff_ms =
            jiff::Timestamp::now().as_millisecond() - (older_than_days as i64) * 86_400_000;
        let victims = self.journal.select_older_than(cutoff_ms)?;
        let size_before = self.journal.db_size_bytes().unwrap_or(0);
        if dry_run {
            return Ok(PruneReport {
                candidates: victims.len(),
                removed: 0,
                trash_purged: 0,
                size_before,
                size_after: size_before,
                dry_run: true,
                empty_trash,
            });
        }
        let ids: Vec<i64> = victims.iter().map(|o| o.id).collect();
        let removed = self.journal.delete_ids(&ids)?;
        let mut trash_purged = 0;
        if empty_trash {
            for op in &victims {
                for tref in op.details.trash_refs() {
                    if self.trash.purge(tref).is_ok() {
                        trash_purged += 1;
                    }
                }
            }
        }
        self.journal.vacuum()?;
        let size_after = self.journal.db_size_bytes().unwrap_or(size_before);
        Ok(PruneReport {
            candidates: victims.len(),
            removed,
            trash_purged,
            size_before,
            size_after,
            dry_run: false,
            empty_trash,
        })
    }

    pub fn maybe_autoprune(&mut self) {
        let config = match crate::config::Config::load() {
            Ok(c) => c,
            Err(_) => return,
        };
        if !config.cleanup.enabled {
            return;
        }
        let day_ms: i64 = 24 * 60 * 60 * 1000;
        let now = jiff::Timestamp::now().as_millisecond();
        let marker = self.data_dir.join(".last_prune");
        if let Ok(s) = std::fs::read_to_string(&marker)
            && let Ok(last) = s.trim().parse::<i64>()
            && now - last < day_ms
        {
            return;
        }
        let _ = self.prune(config.cleanup.max_age_days, false, false);
        let _ = std::fs::write(&marker, now.to_string());
    }

    pub fn undo(&mut self, target: Target, force: bool, dry_run: bool) -> Result<Report> {
        self.run(Direction::Undo, target, force, dry_run)
    }

    pub fn redo(&mut self, target: Target, force: bool, dry_run: bool) -> Result<Report> {
        self.run(Direction::Redo, target, force, dry_run)
    }

    pub fn verify(&self, id: i64, dir: Direction) -> Result<Report> {
        let entry = self.select(Target::Id(id), dir, true)?;
        let (warnings, conflicts) = self.verify_entry(&entry, dir);
        let ok = conflicts.is_empty();
        Ok(Report {
            action: "verify",
            entry,
            changes: Vec::new(),
            warnings,
            conflicts,
            error: None,
            ok,
            dry_run: true,
        })
    }

    fn action_name(dir: Direction) -> &'static str {
        match dir {
            Direction::Undo => "undo",
            Direction::Redo => "redo",
        }
    }

    fn ordered_indices(n: usize, dir: Direction) -> Vec<usize> {
        match dir {
            Direction::Undo => (0..n).rev().collect(),
            Direction::Redo => (0..n).collect(),
        }
    }

    fn verify_entry(&self, entry: &Operation, dir: Direction) -> (Vec<String>, Vec<Conflict>) {
        let mut warnings = Vec::new();
        let mut conflicts = Vec::new();
        let removed = Self::removed_dirs(entry, dir);
        for idx in Self::ordered_indices(entry.details.actions.len(), dir) {
            let v = ops::verify_action(&entry.details.actions[idx], dir, &self.limits, &removed);
            warnings.extend(v.warnings);
            conflicts.extend(v.conflicts);
        }
        (warnings, conflicts)
    }

    fn removed_dirs(entry: &Operation, dir: Direction) -> std::collections::HashSet<PathBuf> {
        use crate::journal::model::Action;
        let mut set = std::collections::HashSet::new();
        if dir == Direction::Undo {
            for action in &entry.details.actions {
                if let Action::CreateDir { path, .. } = action {
                    set.insert(path.clone());
                }
            }
        }
        set
    }

    fn select(&self, target: Target, dir: Direction, verify_only: bool) -> Result<Operation> {
        match target {
            Target::Last => {
                let candidate = match dir {
                    Direction::Undo => self.journal.latest(Status::Applied)?,
                    Direction::Redo => self.journal.earliest(Status::Undone)?,
                };
                if let Some(op) = candidate {
                    return Ok(op);
                }
                match dir {
                    Direction::Undo => Err(UndoError::msg("nothing to undo")),
                    Direction::Redo => {
                        if self.journal.latest(Status::Superseded)?.is_some() {
                            Err(UndoError::msg(
                                "nothing to redo — a newer operation superseded the redo stack",
                            ))
                        } else {
                            Err(UndoError::msg("nothing to redo"))
                        }
                    }
                }
            }
            Target::Id(id) => {
                let op = self
                    .journal
                    .get_any(id)?
                    .ok_or_else(|| UndoError::msg(format!("no journal entry #{id}")))?;
                if op.uid != self.journal.uid {
                    return Err(UndoError::msg(format!(
                        "entry #{id} belongs to uid {}, not you (uid {}) — refusing",
                        op.uid, self.journal.uid
                    )));
                }
                if !verify_only {
                    self.check_status(&op, dir, true)?;
                }
                Ok(op)
            }
        }
    }

    fn check_status(&self, op: &Operation, dir: Direction, allow_broken: bool) -> Result<()> {
        let ok = match (dir, op.status) {
            (Direction::Undo, Status::Applied) | (Direction::Redo, Status::Undone) => true,
            (_, Status::Broken) => allow_broken,
            _ => false,
        };
        if ok {
            return Ok(());
        }
        let verb = Self::action_name(dir);
        match op.status {
            Status::Superseded => Err(UndoError::msg(format!(
                "entry #{} was superseded by a newer operation — cannot {verb}",
                op.id
            ))),
            _ => Err(UndoError::msg(format!(
                "entry #{} is '{}' — cannot {verb}",
                op.id, op.status
            ))),
        }
    }

    fn run(
        &mut self,
        dir: Direction,
        target: Target,
        force: bool,
        dry_run: bool,
    ) -> Result<Report> {
        let _lock = lockfile::acquire(&self.data_dir)?;

        for id in self.journal.sweep_pending()? {
            eprintln!(
                "undo: warning: operation #{id} was interrupted mid-flight — marked broken (inspect with 'undo show {id}')"
            );
        }
        let mut warnings = Vec::new();

        let mut entry = self.select(target, dir, false)?;
        if entry.status == Status::Broken && !force {
            return Err(UndoError::msg(format!(
                "entry #{} is broken (crashed mid-flight); retry with --force after inspecting 'undo show {}'",
                entry.id, entry.id
            )));
        }
        self.check_status(&entry, dir, force)?;

        let (verify_warnings, conflicts) = self.verify_entry(&entry, dir);
        warnings.extend(verify_warnings);

        if !conflicts.is_empty() && !force {
            return Ok(Report {
                action: Self::action_name(dir),
                entry,
                changes: Vec::new(),
                warnings,
                conflicts,
                error: None,
                ok: false,
                dry_run,
            });
        }
        if dry_run {
            let ok = conflicts.is_empty() || force;
            return Ok(Report {
                action: Self::action_name(dir),
                entry,
                changes: Vec::new(),
                warnings,
                conflicts,
                error: None,
                ok,
                dry_run: true,
            });
        }

        let pending = match dir {
            Direction::Undo => Status::PendingUndo,
            Direction::Redo => Status::PendingRedo,
        };
        let revert_to = entry.status;
        self.journal.set_status(entry.id, pending)?;

        let mut ctx = OpCtx::new(&self.trash, &self.limits, force);
        let order = Self::ordered_indices(entry.details.actions.len(), dir);
        let mut processed: Vec<usize> = Vec::new();
        let mut failure: Option<(usize, String)> = None;

        for idx in order {
            match ops::apply_action(&mut entry.details.actions[idx], dir, &mut ctx) {
                Ok(()) => processed.push(idx),
                Err(e) => {
                    failure = Some((idx, e.to_string()));
                    break;
                }
            }
        }

        let persist =
            |journal: &Journal, entry: &mut Operation, ctx: &mut OpCtx<'_>| -> Result<()> {
                entry.details.force_evictions.append(&mut ctx.evictions);
                entry.details.undo_artifacts.append(&mut ctx.artifacts);
                journal.update_details(entry.id, &entry.details)
            };

        if let Some((failed_idx, err)) = failure {
            warnings.append(&mut ctx.warnings);
            let mut compensation_failed = false;
            for &idx in processed.iter().rev() {
                if let Err(e) =
                    ops::apply_action(&mut entry.details.actions[idx], dir.opposite(), &mut ctx)
                {
                    compensation_failed = true;
                    warnings.push(format!("compensation failed for step {}: {e}", idx + 1));
                    break;
                }
            }
            if compensation_failed {
                entry.details.broken_at = Some(failed_idx);
                persist(&self.journal, &mut entry, &mut ctx)?;
                self.journal.set_status(entry.id, Status::Broken)?;
                entry.status = Status::Broken;
                warnings.push(format!(
                    "entry #{} is now broken — nothing was permanently lost; every displaced node is in the trash ('undo show {}')",
                    entry.id, entry.id
                ));
            } else {
                persist(&self.journal, &mut entry, &mut ctx)?;
                self.journal.set_status(entry.id, revert_to)?;
                entry.status = revert_to;
            }
            warnings.append(&mut ctx.warnings);
            return Ok(Report {
                action: Self::action_name(dir),
                entry,
                changes: ctx.changes,
                warnings,
                conflicts,
                error: Some(err),
                ok: false,
                dry_run: false,
            });
        }

        warnings.append(&mut ctx.warnings);
        let changes = std::mem::take(&mut ctx.changes);
        entry.details.broken_at = None;
        persist(&self.journal, &mut entry, &mut ctx)?;
        let final_status = match dir {
            Direction::Undo => Status::Undone,
            Direction::Redo => Status::Applied,
        };
        self.journal.set_status(entry.id, final_status)?;
        entry.status = final_status;

        Ok(Report {
            action: Self::action_name(dir),
            entry,
            changes,
            warnings,
            conflicts,
            error: None,
            ok: true,
            dry_run: false,
        })
    }
}
