mod app;
pub mod config;
mod ui;

use std::io::IsTerminal;

use ratatui::crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};

use crate::engine::Engine;
use crate::error::{Result, UndoError};

use app::{App, Modal};

pub fn run() -> Result<()> {
    if !std::io::stdout().is_terminal() {
        return Err(UndoError::usage(
            "tui requires a terminal (stdout is not a tty)",
        ));
    }
    let engine = Engine::open()?;
    let mut app = App::new(engine)?;

    let mut terminal = ratatui::init();
    let result = event_loop(&mut terminal, &mut app);
    ratatui::restore();
    result
}

fn event_loop(terminal: &mut ratatui::DefaultTerminal, app: &mut App) -> Result<()> {
    while !app.should_quit {
        terminal
            .draw(|frame| ui::draw(frame, app))
            .map_err(|e| UndoError::io("drawing the TUI", e))?;

        if event::poll(std::time::Duration::from_millis(250))
            .map_err(|e| UndoError::io("polling for input", e))?
            && let Event::Key(key) = event::read().map_err(|e| UndoError::io("reading input", e))?
            && key.kind == KeyEventKind::Press
        {
            handle_key(app, key.code, key.modifiers)?;
        }
    }
    Ok(())
}

fn handle_key(app: &mut App, code: KeyCode, mods: KeyModifiers) -> Result<()> {
    match app.modal {
        Modal::Help => {
            app.modal = Modal::None;
            return Ok(());
        }
        Modal::Confirm { direction } => {
            match code {
                KeyCode::Char('y') | KeyCode::Char('Y') | KeyCode::Enter => {
                    app.modal = Modal::None;
                    app.perform(direction)?;
                }
                KeyCode::Char('f') | KeyCode::Char('F') => app.toggle_force(),
                KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => app.modal = Modal::None,
                _ => {}
            }
            return Ok(());
        }
        Modal::None => {}
    }

    match code {
        KeyCode::Char('q') | KeyCode::Esc => app.should_quit = true,
        KeyCode::Char('c') if mods.contains(KeyModifiers::CONTROL) => app.should_quit = true,
        KeyCode::Char('j') | KeyCode::Down => app.select_next(),
        KeyCode::Char('k') | KeyCode::Up => app.select_prev(),
        KeyCode::Char('g') | KeyCode::Home => app.select_first(),
        KeyCode::Char('G') | KeyCode::End => app.select_last(),
        KeyCode::Tab => app.cycle_filter(),
        KeyCode::Char('v') | KeyCode::Enter => app.refresh_verification(),
        KeyCode::Char('u') => app.open_confirm_undo(),
        KeyCode::Char('r') => app.open_confirm_redo(),
        KeyCode::Char('f') => app.toggle_force(),
        KeyCode::Char('?') => app.modal = Modal::Help,
        _ => {}
    }
    Ok(())
}
