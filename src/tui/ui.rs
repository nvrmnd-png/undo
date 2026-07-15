use ratatui::Frame;
use ratatui::layout::{Alignment, Constraint, Direction as LayoutDir, Flex, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph, Wrap};

use super::app::{App, Modal, VerifyView};
use crate::journal::model::{Operation, Status};
use crate::ops;
use crate::output;

pub fn draw(frame: &mut Frame, app: &App) {
    let chunks = Layout::default()
        .direction(LayoutDir::Vertical)
        .constraints([
            Constraint::Min(6),
            Constraint::Length(10),
            Constraint::Length(1),
        ])
        .split(frame.area());

    draw_list(frame, app, chunks[0]);
    draw_detail(frame, app, chunks[1]);
    draw_status(frame, app, chunks[2]);

    match app.modal {
        Modal::Help => draw_help(frame),
        Modal::Confirm { direction } => draw_confirm(frame, app, direction),
        Modal::None => {}
    }
}

fn status_style(status: Status) -> Style {
    match status {
        Status::Applied => Style::default().fg(Color::Green),
        Status::Undone => Style::default().fg(Color::Yellow),
        Status::Superseded => Style::default().fg(Color::DarkGray),
        Status::Broken => Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
        _ => Style::default().fg(Color::Magenta),
    }
}

fn draw_list(frame: &mut Frame, app: &App, area: Rect) {
    let items: Vec<ListItem> = app
        .entries
        .iter()
        .map(|op| {
            let line = Line::from(vec![
                Span::styled(
                    format!("#{:<4}", op.id),
                    Style::default().add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    format!("{}  ", output::fmt_ts_short(op.ts_ms)),
                    Style::default().fg(Color::DarkGray),
                ),
                Span::styled(
                    format!("{:<11}", op.status.as_str()),
                    status_style(op.status),
                ),
                Span::raw(" "),
                Span::raw(op.summary()),
            ]);
            ListItem::new(line)
        })
        .collect();

    let title = format!(
        " journal · filter: {} ({} entries) ",
        app.filter.label(),
        app.entries.len()
    );
    let list = List::new(items)
        .block(Block::default().borders(Borders::ALL).title(title))
        .highlight_style(Style::default().add_modifier(Modifier::REVERSED))
        .highlight_symbol("▶ ");

    let mut state = ListState::default();
    if !app.entries.is_empty() {
        state.select(Some(app.selected));
    }
    frame.render_stateful_widget(list, area, &mut state);
}

fn badge(verify: &VerifyView) -> Span<'static> {
    if !verify.computed {
        return Span::styled(
            " press v to verify ",
            Style::default().fg(Color::Black).bg(Color::Gray),
        );
    }
    if verify.direction.is_none() {
        return Span::styled(
            " not actionable ",
            Style::default().fg(Color::Black).bg(Color::DarkGray),
        );
    }
    if verify.ok {
        Span::styled(
            " ✓ ready ",
            Style::default()
                .fg(Color::Black)
                .bg(Color::Green)
                .add_modifier(Modifier::BOLD),
        )
    } else {
        Span::styled(
            " ✗ blocked ",
            Style::default()
                .fg(Color::White)
                .bg(Color::Red)
                .add_modifier(Modifier::BOLD),
        )
    }
}

fn draw_detail(frame: &mut Frame, app: &App, area: Rect) {
    let block = Block::default().borders(Borders::ALL).title(" details ");
    let Some(op) = app.selected_entry() else {
        let empty = Paragraph::new("Journal is empty. Run some wrapped commands, then come back.")
            .block(block)
            .wrap(Wrap { trim: true });
        frame.render_widget(empty, area);
        return;
    };

    let mut lines: Vec<Line> = Vec::new();
    lines.push(Line::from(vec![
        Span::styled(
            format!("#{} ", op.id),
            Style::default().add_modifier(Modifier::BOLD),
        ),
        Span::styled(op.summary(), Style::default().add_modifier(Modifier::BOLD)),
        Span::raw("   "),
        badge(&app.verify),
    ]));
    lines.push(Line::from(vec![
        Span::styled("status ", Style::default().fg(Color::DarkGray)),
        Span::styled(op.status.as_str().to_string(), status_style(op.status)),
        Span::styled("   when ", Style::default().fg(Color::DarkGray)),
        Span::raw(output::fmt_ts_rfc(op.ts_ms)),
        Span::styled("   cwd ", Style::default().fg(Color::DarkGray)),
        Span::raw(op.cwd.display().to_string()),
    ]));

    for (i, action) in op.details.actions.iter().enumerate().take(3) {
        lines.push(Line::from(format!(
            "  {}. {}",
            i + 1,
            ops::describe(action)
        )));
    }
    if op.details.actions.len() > 3 {
        lines.push(Line::from(Span::styled(
            format!("  … {} more action(s)", op.details.actions.len() - 3),
            Style::default().fg(Color::DarkGray),
        )));
    }

    if app.verify.computed {
        for conflict in app.verify.conflicts.iter().take(3) {
            lines.push(Line::from(Span::styled(
                format!("  conflict: {conflict}"),
                Style::default().fg(Color::Red),
            )));
        }
        for warning in app.verify.warnings.iter().take(2) {
            lines.push(Line::from(Span::styled(
                format!("  warning: {warning}"),
                Style::default().fg(Color::Yellow),
            )));
        }
    }

    let para = Paragraph::new(lines).block(block).wrap(Wrap { trim: true });
    frame.render_widget(para, area);
}

fn draw_status(frame: &mut Frame, app: &App, area: Rect) {
    let force = if app.force {
        Span::styled(
            " FORCE ",
            Style::default()
                .fg(Color::Black)
                .bg(Color::Red)
                .add_modifier(Modifier::BOLD),
        )
    } else {
        Span::raw("")
    };
    let line = Line::from(vec![
        Span::styled(
            " undo ",
            Style::default()
                .fg(Color::Black)
                .bg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(" "),
        Span::raw(app.status_line.clone()),
        Span::raw("  "),
        force,
    ]);
    frame.render_widget(Paragraph::new(line), area);
}

fn centered(area: Rect, width: u16, height: u16) -> Rect {
    let [row] = Layout::horizontal([Constraint::Length(width)])
        .flex(Flex::Center)
        .areas(area);
    let [cell] = Layout::vertical([Constraint::Length(height)])
        .flex(Flex::Center)
        .areas(row);
    cell
}

fn draw_confirm(frame: &mut Frame, app: &App, direction: crate::ops::Direction) {
    let verb = if direction == crate::ops::Direction::Undo {
        "Undo"
    } else {
        "Redo"
    };
    let summary = app
        .selected_entry()
        .map(Operation::summary)
        .unwrap_or_default();

    let mut lines = vec![
        Line::from(Span::styled(
            format!(
                "{verb} #{}?",
                app.selected_entry().map(|e| e.id).unwrap_or(0)
            ),
            Style::default().add_modifier(Modifier::BOLD),
        )),
        Line::from(summary),
        Line::from(""),
        Line::from(vec![Span::raw("verification: "), badge(&app.verify)]),
    ];
    if !app.verify.ok && app.verify.computed {
        for c in app.verify.conflicts.iter().take(3) {
            lines.push(Line::from(Span::styled(
                format!("  {c}"),
                Style::default().fg(Color::Red),
            )));
        }
    }
    lines.push(Line::from(""));
    let force_hint = if app.force {
        Span::styled(
            "FORCE is ON — occupied targets go to trash",
            Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
        )
    } else {
        Span::styled("f: toggle --force", Style::default().fg(Color::DarkGray))
    };
    lines.push(Line::from(force_hint));
    lines.push(Line::from(Span::styled(
        "y: confirm    n/Esc: cancel",
        Style::default().fg(Color::Cyan),
    )));

    let area = centered(frame.area(), 60, (lines.len() as u16) + 2);
    frame.render_widget(Clear, area);
    let block = Block::default()
        .borders(Borders::ALL)
        .title(format!(" confirm {} ", verb.to_lowercase()))
        .title_alignment(Alignment::Center)
        .style(Style::default().bg(Color::Black));
    frame.render_widget(Paragraph::new(lines).block(block), area);
}

fn draw_help(frame: &mut Frame) {
    let lines = vec![
        Line::from(Span::styled(
            "undo — interactive journal",
            Style::default().add_modifier(Modifier::BOLD),
        )),
        Line::from(""),
        Line::from("j / ↓      move down"),
        Line::from("k / ↑      move up"),
        Line::from("g / G      first / last"),
        Line::from("Tab        cycle filter (all · undoable · redoable)"),
        Line::from("v / Enter  verify selected entry"),
        Line::from("u          undo selected (applied entries)"),
        Line::from("r          redo selected (undone entries)"),
        Line::from("f          toggle --force"),
        Line::from("? then any key   close this help"),
        Line::from("q / Esc    quit"),
        Line::from(""),
        Line::from(Span::styled(
            "press any key to close",
            Style::default().fg(Color::Cyan),
        )),
    ];
    let area = centered(frame.area(), 56, (lines.len() as u16) + 2);
    frame.render_widget(Clear, area);
    let block = Block::default()
        .borders(Borders::ALL)
        .title(" help ")
        .title_alignment(Alignment::Center)
        .style(Style::default().bg(Color::Black));
    frame.render_widget(Paragraph::new(lines).block(block), area);
}
