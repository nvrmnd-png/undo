use serde::Serialize;

use crate::engine::Report;
use crate::error::Result;
use crate::journal::model::Operation;
use crate::ops::{self, Change};
use crate::state::Conflict;

pub const SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    Human,
    Json,
    Yaml,
}

impl Format {
    pub fn from_flags(json: bool, yaml: bool) -> Format {
        if json {
            Format::Json
        } else if yaml {
            Format::Yaml
        } else {
            Format::Human
        }
    }

    pub fn is_machine(self) -> bool {
        self != Format::Human
    }
}

fn zoned(ts_ms: i64) -> Option<jiff::Zoned> {
    jiff::Timestamp::from_millisecond(ts_ms)
        .ok()
        .map(|t| t.to_zoned(jiff::tz::TimeZone::system()))
}

pub fn fmt_ts_short(ts_ms: i64) -> String {
    zoned(ts_ms)
        .map(|z| z.strftime("%Y-%m-%d %H:%M").to_string())
        .unwrap_or_else(|| "?".into())
}

pub fn fmt_ts_rfc(ts_ms: i64) -> String {
    zoned(ts_ms)
        .map(|z| z.strftime("%Y-%m-%dT%H:%M:%S%:z").to_string())
        .unwrap_or_else(|| "?".into())
}

#[derive(Debug, Serialize)]
pub struct EntryView {
    pub id: i64,
    pub ts: String,
    pub uid: u32,
    pub user: String,
    pub cwd: String,
    pub command: String,
    pub argv: Vec<String>,
    pub status: String,
    pub summary: String,
}

impl EntryView {
    pub fn from_op(op: &Operation) -> EntryView {
        EntryView {
            id: op.id,
            ts: fmt_ts_rfc(op.ts_ms),
            uid: op.uid,
            user: op.username.clone(),
            cwd: op.cwd.display().to_string(),
            command: op.command.clone(),
            argv: op.argv.clone(),
            status: op.status.to_string(),
            summary: op.summary(),
        }
    }
}

#[derive(Serialize)]
struct ReportView<'a> {
    schema: u32,
    ok: bool,
    action: &'a str,
    dry_run: bool,
    entry: EntryView,
    changes: &'a [Change],
    warnings: &'a [String],
    conflicts: &'a [Conflict],
    #[serde(skip_serializing_if = "Option::is_none")]
    error: &'a Option<String>,
}

#[derive(Serialize)]
struct ListView {
    schema: u32,
    undo_stack: Vec<EntryView>,
    redo_stack: Vec<EntryView>,
}

#[derive(Serialize)]
struct HistoryView {
    schema: u32,
    entries: Vec<EntryView>,
}

#[derive(Serialize)]
struct ShowView {
    schema: u32,
    entry: EntryView,
    details: serde_json::Value,
}

#[derive(Serialize)]
struct ClearView {
    schema: u32,
    ok: bool,
    cleared: usize,
}

fn emit<T: Serialize>(format: Format, payload: &T) -> Result<()> {
    match format {
        Format::Json => println!("{}", serde_json::to_string_pretty(payload)?),
        Format::Yaml => print!(
            "{}",
            serde_norway::to_string(payload)
                .map_err(|e| crate::error::UndoError::msg(format!("yaml: {e}")))?
        ),
        Format::Human => unreachable!("emit() is machine-only"),
    }
    Ok(())
}

fn styled_status(status: &str) -> console::StyledObject<String> {
    let s = console::style(status.to_string());
    match status {
        "applied" => s.green(),
        "undone" => s.yellow(),
        "superseded" => s.dim(),
        "broken" => s.red().bold(),
        _ => s.magenta(),
    }
}

fn print_warnings(warnings: &[String]) {
    for w in warnings {
        eprintln!("undo: warning: {w}");
    }
}

pub fn print_report(format: Format, report: &Report) -> Result<()> {
    print_warnings(&report.warnings);
    if format.is_machine() {
        return emit(
            format,
            &ReportView {
                schema: SCHEMA_VERSION,
                ok: report.ok,
                action: report.action,
                dry_run: report.dry_run,
                entry: EntryView::from_op(&report.entry),
                changes: &report.changes,
                warnings: &report.warnings,
                conflicts: &report.conflicts,
                error: &report.error,
            },
        );
    }

    let entry = &report.entry;
    if !report.conflicts.is_empty() && !report.ok {
        eprintln!(
            "undo: refusing to {} #{} ({}):",
            report.action,
            entry.id,
            entry.summary()
        );
        for c in &report.conflicts {
            eprintln!("  {} {}", console::style("conflict:").red().bold(), c);
        }
        eprintln!("  (override with --force; displaced files are parked in the trash)");
        return Ok(());
    }
    if let Some(err) = &report.error {
        eprintln!("undo: {} #{} failed: {err}", report.action, entry.id);
        for change in &report.changes {
            eprintln!("  {} {change}", console::style("done:").yellow());
        }
        return Ok(());
    }
    if report.dry_run {
        println!(
            "{} #{} would {}: {}",
            console::style("dry-run").cyan().bold(),
            entry.id,
            report.action,
            entry.summary()
        );
        for c in &report.conflicts {
            println!("  {} {}", console::style("conflict:").red(), c);
        }
        return Ok(());
    }
    let verb = match report.action {
        "undo" => "Undid",
        "redo" => "Redid",
        _ => "Verified",
    };
    println!(
        "{} #{}: {}",
        console::style(verb).green().bold(),
        entry.id,
        entry.summary()
    );
    for change in &report.changes {
        println!("  {change}");
    }
    Ok(())
}

fn print_table(entries: &[Operation]) {
    let id_width = entries
        .iter()
        .map(|e| e.id.to_string().len())
        .max()
        .unwrap_or(1);
    for e in entries {
        println!(
            "#{:<id_width$}  {}  {:<11}  {}",
            e.id,
            console::style(fmt_ts_short(e.ts_ms)).dim(),
            styled_status(e.status.as_str()),
            e.summary(),
        );
    }
}

pub fn print_list(
    format: Format,
    undo_stack: &[Operation],
    redo_stack: &[Operation],
) -> Result<()> {
    if format.is_machine() {
        return emit(
            format,
            &ListView {
                schema: SCHEMA_VERSION,
                undo_stack: undo_stack.iter().map(EntryView::from_op).collect(),
                redo_stack: redo_stack.iter().map(EntryView::from_op).collect(),
            },
        );
    }
    if undo_stack.is_empty() && redo_stack.is_empty() {
        println!("Journal is empty — nothing to undo or redo.");
        return Ok(());
    }
    if !undo_stack.is_empty() {
        println!("{}", console::style("Undo stack (newest first):").bold());
        print_table(undo_stack);
    }
    if !redo_stack.is_empty() {
        if !undo_stack.is_empty() {
            println!();
        }
        println!("{}", console::style("Redo stack (newest first):").bold());
        print_table(redo_stack);
    }
    Ok(())
}

pub fn print_history(format: Format, entries: &[Operation]) -> Result<()> {
    if format.is_machine() {
        return emit(
            format,
            &HistoryView {
                schema: SCHEMA_VERSION,
                entries: entries.iter().map(EntryView::from_op).collect(),
            },
        );
    }
    if entries.is_empty() {
        println!("Journal is empty.");
        return Ok(());
    }
    print_table(entries);
    Ok(())
}

pub fn print_show(format: Format, op: &Operation) -> Result<()> {
    if format.is_machine() {
        return emit(
            format,
            &ShowView {
                schema: SCHEMA_VERSION,
                entry: EntryView::from_op(op),
                details: serde_json::to_value(&op.details)?,
            },
        );
    }
    println!(
        "{} {}",
        console::style(format!("#{}", op.id)).bold(),
        console::style(op.summary()).bold()
    );
    println!("  when:    {}", fmt_ts_rfc(op.ts_ms));
    println!("  user:    {} (uid {})", op.username, op.uid);
    println!("  cwd:     {}", op.cwd.display());
    println!("  status:  {}", styled_status(op.status.as_str()));
    if let Some(ts) = op.undo_ts_ms {
        println!("  undone:  {}", fmt_ts_rfc(ts));
    }
    if let Some(ts) = op.redo_ts_ms {
        println!("  redone:  {}", fmt_ts_rfc(ts));
    }
    if let Some(at) = op.details.broken_at {
        println!(
            "  {}  failed at step {}",
            console::style("broken:").red().bold(),
            at + 1
        );
    }
    if !op.details.actions.is_empty() {
        println!("  actions:");
        for (i, action) in op.details.actions.iter().enumerate() {
            println!("    {}. {}", i + 1, ops::describe(action));
        }
    }
    if !op.details.undo_artifacts.is_empty() {
        println!("  parked in trash by undo:");
        for t in &op.details.undo_artifacts {
            println!("    {} -> {}", t.origin.display(), t.file.display());
        }
    }
    if !op.details.force_evictions.is_empty() {
        println!("  evicted by --force:");
        for t in &op.details.force_evictions {
            println!("    {} -> {}", t.origin.display(), t.file.display());
        }
    }
    Ok(())
}

pub fn print_clear(format: Format, cleared: usize) -> Result<()> {
    if format.is_machine() {
        return emit(
            format,
            &ClearView {
                schema: SCHEMA_VERSION,
                ok: true,
                cleared,
            },
        );
    }
    println!("Cleared {cleared} journal entries. Trash contents were not touched.");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_from_flags() {
        assert_eq!(Format::from_flags(false, false), Format::Human);
        assert_eq!(Format::from_flags(true, false), Format::Json);
        assert_eq!(Format::from_flags(false, true), Format::Yaml);
    }

    #[test]
    fn timestamps_render() {
        let s = fmt_ts_short(1_752_000_000_000);
        assert!(s.starts_with("20"), "unexpected: {s}");
        let r = fmt_ts_rfc(1_752_000_000_000);
        assert!(r.contains('T'), "unexpected: {r}");
    }
}
