use rusqlite::Connection;

use crate::error::Result;

const SCHEMA_V1: &str = "
CREATE TABLE operations (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    ts_ms       INTEGER NOT NULL,
    uid         INTEGER NOT NULL,
    username    TEXT    NOT NULL DEFAULT '',
    cwd         TEXT    NOT NULL,
    command     TEXT    NOT NULL CHECK (command IN
                  ('mv','cp','rm','mkdir','rmdir','chmod','chown','ln','rename')),
    argv        TEXT    NOT NULL,
    status      TEXT    NOT NULL CHECK (status IN
                  ('pending_exec','applied','pending_undo','undone',
                   'pending_redo','superseded','broken')),
    details     TEXT    NOT NULL,
    undo_ts_ms  INTEGER,
    redo_ts_ms  INTEGER
);
CREATE INDEX idx_ops_stack ON operations (uid, status, id DESC);
";

pub fn migrate(conn: &Connection) -> Result<()> {
    let version: i64 = conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
    if version < 1 {
        conn.execute_batch(SCHEMA_V1)?;
        conn.pragma_update(None, "user_version", 1)?;
    }
    Ok(())
}
