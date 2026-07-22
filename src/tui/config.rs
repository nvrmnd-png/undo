use std::io::IsTerminal;

use ratatui::Frame;
use ratatui::crossterm::event::{self, Event, KeyCode, KeyEventKind};
use ratatui::layout::{Constraint, Direction, Layout};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};

use crate::config::Config;
use crate::error::{Result, UndoError};

#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    Bool,
    Number,
    Text,
}

struct Field {
    label: &'static str,
    kind: Kind,
    help: &'static str,
}

const FIELDS: [Field; 6] = [
    Field {
        label: "auto-clear enabled",
        kind: Kind::Bool,
        help: "run cleanup automatically (journal only, never the trash)",
    },
    Field {
        label: "max age (days)",
        kind: Kind::Number,
        help: "journal entries older than this are pruned",
    },
    Field {
        label: "max database size (MB)",
        kind: Kind::Number,
        help: "advisory size target for the journal database",
    },
    Field {
        label: "storage path",
        kind: Kind::Text,
        help: "where the journal lives (empty = default). Trash stays in XDG.",
    },
    Field {
        label: "exclude paths",
        kind: Kind::Text,
        help: "comma-separated paths that are not journaled",
    },
    Field {
        label: "logging enabled",
        kind: Kind::Bool,
        help: "append each journaled operation to undo.log",
    },
];

struct ConfigApp {
    config: Config,
    selected: usize,
    editing: Option<String>,
    status: String,
    dirty: bool,
    should_quit: bool,
}

impl ConfigApp {
    fn value(&self, i: usize) -> String {
        match i {
            0 => self.config.cleanup.enabled.to_string(),
            1 => self.config.cleanup.max_age_days.to_string(),
            2 => self.config.cleanup.max_database_size.to_string(),
            3 => self.config.storage.path.clone().unwrap_or_default(),
            4 => self.config.exclude.paths.join(", "),
            5 => self.config.logging.enabled.to_string(),
            _ => String::new(),
        }
    }

    fn toggle_bool(&mut self) {
        match self.selected {
            0 => self.config.cleanup.enabled = !self.config.cleanup.enabled,
            5 => self.config.logging.enabled = !self.config.logging.enabled,
            _ => return,
        }
        self.dirty = true;
    }

    fn commit(&mut self, buf: &str) {
        let buf = buf.trim();
        match self.selected {
            1 => match buf.parse::<u64>() {
                Ok(n) => {
                    self.config.cleanup.max_age_days = n;
                    self.dirty = true;
                }
                Err(_) => self.status = "not a number".into(),
            },
            2 => match buf.parse::<u64>() {
                Ok(n) => {
                    self.config.cleanup.max_database_size = n;
                    self.dirty = true;
                }
                Err(_) => self.status = "not a number".into(),
            },
            3 => {
                self.config.storage.path = if buf.is_empty() {
                    None
                } else {
                    Some(buf.to_string())
                };
                self.dirty = true;
            }
            4 => {
                self.config.exclude.paths = buf
                    .split(',')
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())
                    .collect();
                self.dirty = true;
            }
            _ => {}
        }
    }
}

pub fn run() -> Result<()> {
    if !std::io::stdout().is_terminal() {
        return Err(UndoError::usage(
            "config editor requires a terminal (use 'undo config show' otherwise)",
        ));
    }
    let mut app = ConfigApp {
        config: Config::load()?,
        selected: 0,
        editing: None,
        status: "j/k move · space toggle · e edit · s save · q quit".into(),
        dirty: false,
        should_quit: false,
    };

    let mut terminal = ratatui::init();
    let result = event_loop(&mut terminal, &mut app);
    ratatui::restore();
    result
}

fn event_loop(terminal: &mut ratatui::DefaultTerminal, app: &mut ConfigApp) -> Result<()> {
    while !app.should_quit {
        terminal
            .draw(|frame| draw(frame, app))
            .map_err(|e| UndoError::io("drawing the config editor", e))?;

        if event::poll(std::time::Duration::from_millis(250))
            .map_err(|e| UndoError::io("polling for input", e))?
            && let Event::Key(key) = event::read().map_err(|e| UndoError::io("reading input", e))?
            && key.kind == KeyEventKind::Press
        {
            handle_key(app, key.code)?;
        }
    }
    Ok(())
}

fn handle_key(app: &mut ConfigApp, code: KeyCode) -> Result<()> {
    if let Some(buf) = app.editing.as_mut() {
        match code {
            KeyCode::Char(c) => buf.push(c),
            KeyCode::Backspace => {
                buf.pop();
            }
            KeyCode::Enter => {
                let buf = app.editing.take().unwrap();
                app.commit(&buf);
            }
            KeyCode::Esc => app.editing = None,
            _ => {}
        }
        return Ok(());
    }

    match code {
        KeyCode::Char('q') | KeyCode::Esc => app.should_quit = true,
        KeyCode::Char('j') | KeyCode::Down => {
            app.selected = (app.selected + 1) % FIELDS.len();
        }
        KeyCode::Char('k') | KeyCode::Up => {
            app.selected = (app.selected + FIELDS.len() - 1) % FIELDS.len();
        }
        KeyCode::Char(' ') => app.toggle_bool(),
        KeyCode::Char('e') | KeyCode::Enter => match FIELDS[app.selected].kind {
            Kind::Bool => app.toggle_bool(),
            _ => app.editing = Some(app.value(app.selected)),
        },
        KeyCode::Char('s') => match app.config.save() {
            Ok(path) => {
                app.dirty = false;
                app.status = format!("saved to {}", path.display());
            }
            Err(e) => app.status = format!("save failed: {e}"),
        },
        KeyCode::Char('r') => match Config::load() {
            Ok(c) => {
                app.config = c;
                app.dirty = false;
                app.status = "reloaded from disk".into();
            }
            Err(e) => app.status = format!("reload failed: {e}"),
        },
        _ => {}
    }
    Ok(())
}

fn draw(frame: &mut Frame, app: &ConfigApp) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(8),
            Constraint::Length(2),
            Constraint::Length(1),
        ])
        .split(frame.area());

    let mut lines: Vec<Line> = Vec::new();
    for (i, field) in FIELDS.iter().enumerate() {
        let selected = i == app.selected;
        let marker = if selected { "▶ " } else { "  " };
        let value = if selected && app.editing.is_some() {
            format!("{}_", app.editing.as_deref().unwrap_or(""))
        } else {
            app.value(i)
        };
        let style = if selected {
            Style::default().add_modifier(Modifier::REVERSED)
        } else {
            Style::default()
        };
        lines.push(Line::from(vec![
            Span::styled(format!("{marker}{:<24}", field.label), style),
            Span::raw("  "),
            Span::styled(value, Style::default().fg(Color::Cyan)),
        ]));
    }
    let title = if app.dirty {
        " undo config — unsaved changes (press s) "
    } else {
        " undo config "
    };
    let list = Paragraph::new(lines).block(Block::default().borders(Borders::ALL).title(title));
    frame.render_widget(list, chunks[0]);

    let help = Paragraph::new(Line::from(Span::styled(
        FIELDS[app.selected].help,
        Style::default().fg(Color::DarkGray),
    )))
    .block(Block::default().borders(Borders::TOP));
    frame.render_widget(help, chunks[1]);

    let footer = if app.editing.is_some() {
        "type a value · enter: commit · esc: cancel".to_string()
    } else {
        app.status.clone()
    };
    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(
            footer,
            Style::default().fg(Color::Yellow),
        ))),
        chunks[2],
    );
}
