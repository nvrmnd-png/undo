use crate::engine::{Engine, Target};
use crate::error::Result;
use crate::journal::model::{Operation, Status};
use crate::ops::Direction;
use crate::state::Conflict;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Filter {
    All,
    Applied,
    Undone,
}

impl Filter {
    pub fn label(self) -> &'static str {
        match self {
            Filter::All => "all",
            Filter::Applied => "undoable",
            Filter::Undone => "redoable",
        }
    }

    fn next(self) -> Filter {
        match self {
            Filter::All => Filter::Applied,
            Filter::Applied => Filter::Undone,
            Filter::Undone => Filter::All,
        }
    }

    fn matches(self, status: Status) -> bool {
        match self {
            Filter::All => true,
            Filter::Applied => status == Status::Applied,
            Filter::Undone => status == Status::Undone,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Modal {
    None,
    Confirm { direction: Direction },
    Help,
}

#[derive(Debug, Default, Clone)]
pub struct VerifyView {
    pub computed: bool,
    pub ok: bool,
    pub direction: Option<Direction>,
    pub conflicts: Vec<Conflict>,
    pub warnings: Vec<String>,
}

pub struct App {
    pub engine: Engine,
    pub entries: Vec<Operation>,
    pub selected: usize,
    pub filter: Filter,
    pub verify: VerifyView,
    pub modal: Modal,
    pub force: bool,
    pub status_line: String,
    pub should_quit: bool,
}

impl App {
    pub fn new(engine: Engine) -> Result<App> {
        let mut app = App {
            engine,
            entries: Vec::new(),
            selected: 0,
            filter: Filter::All,
            verify: VerifyView::default(),
            modal: Modal::None,
            force: false,
            status_line: "j/k move · v verify · u undo · r redo · ? help · q quit".into(),
            should_quit: false,
        };
        app.reload()?;
        Ok(app)
    }

    pub fn reload(&mut self) -> Result<()> {
        let all = self.engine.journal.history(None)?;
        self.entries = all
            .into_iter()
            .filter(|op| self.filter.matches(op.status))
            .collect();
        if self.selected >= self.entries.len() {
            self.selected = self.entries.len().saturating_sub(1);
        }
        self.verify = VerifyView::default();
        Ok(())
    }

    pub fn selected_entry(&self) -> Option<&Operation> {
        self.entries.get(self.selected)
    }

    pub fn select_next(&mut self) {
        if self.selected + 1 < self.entries.len() {
            self.selected += 1;
            self.verify = VerifyView::default();
        }
    }

    pub fn select_prev(&mut self) {
        if self.selected > 0 {
            self.selected -= 1;
            self.verify = VerifyView::default();
        }
    }

    pub fn select_first(&mut self) {
        self.selected = 0;
        self.verify = VerifyView::default();
    }

    pub fn select_last(&mut self) {
        self.selected = self.entries.len().saturating_sub(1);
        self.verify = VerifyView::default();
    }

    pub fn cycle_filter(&mut self) {
        self.filter = self.filter.next();
        self.selected = 0;
        let _ = self.reload();
    }

    pub fn toggle_force(&mut self) {
        self.force = !self.force;
    }

    fn actionable_direction(&self) -> Option<Direction> {
        match self.selected_entry()?.status {
            Status::Applied => Some(Direction::Undo),
            Status::Undone => Some(Direction::Redo),
            _ => None,
        }
    }

    pub fn refresh_verification(&mut self) {
        let Some(entry) = self.selected_entry() else {
            return;
        };
        let id = entry.id;
        let Some(direction) = self.actionable_direction() else {
            self.verify = VerifyView {
                computed: true,
                ok: false,
                direction: None,
                conflicts: Vec::new(),
                warnings: vec![format!("entry is '{}', not actionable", entry.status)],
            };
            return;
        };
        match self.engine.verify(id, direction) {
            Ok(report) => {
                self.verify = VerifyView {
                    computed: true,
                    ok: report.ok,
                    direction: Some(direction),
                    conflicts: report.conflicts,
                    warnings: report.warnings,
                };
            }
            Err(e) => {
                self.verify = VerifyView {
                    computed: true,
                    ok: false,
                    direction: Some(direction),
                    conflicts: Vec::new(),
                    warnings: vec![e.to_string()],
                };
            }
        }
    }

    pub fn open_confirm_undo(&mut self) {
        if self.selected_entry().map(|e| e.status) != Some(Status::Applied) {
            self.status_line = "only 'applied' entries can be undone".into();
            return;
        }
        self.refresh_verification();
        self.modal = Modal::Confirm {
            direction: Direction::Undo,
        };
    }

    pub fn open_confirm_redo(&mut self) {
        if self.selected_entry().map(|e| e.status) != Some(Status::Undone) {
            self.status_line = "only 'undone' entries can be redone".into();
            return;
        }
        self.refresh_verification();
        self.modal = Modal::Confirm {
            direction: Direction::Redo,
        };
    }

    pub fn perform(&mut self, direction: Direction) -> Result<()> {
        let Some(id) = self.selected_entry().map(|e| e.id) else {
            return Ok(());
        };
        let report = match direction {
            Direction::Undo => self.engine.undo(Target::Id(id), self.force, false),
            Direction::Redo => self.engine.redo(Target::Id(id), self.force, false),
        };
        match report {
            Ok(r) if r.ok => {
                let verb = if direction == Direction::Undo {
                    "Undid"
                } else {
                    "Redid"
                };
                self.status_line = format!("{verb} #{id}: {}", r.entry.summary());
            }
            Ok(r) => {
                let n = r.conflicts.len();
                self.status_line =
                    format!("#{id} refused: {n} conflict(s) — press f to force, then retry");
            }
            Err(e) => {
                self.status_line = format!("#{id} failed: {e}");
            }
        }
        self.force = false;
        self.reload()?;
        self.refresh_verification();
        Ok(())
    }
}
