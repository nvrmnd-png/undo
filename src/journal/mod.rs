pub mod model;
pub mod schema;

use std::fs;
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::{Path, PathBuf};

use rusqlite::{Connection, OptionalExtension, params};

use crate::error::{IoCtx, Result, UndoError};
use crate::paths;

pub use model::{Action, Details, FileState, Operation, Status, TrashRef, TreeSummary};

pub struct Journal {
    conn: Connection,
    pub uid: u32,
}

fn now_ms() -> i64 {
    jiff::Timestamp::now().as_millisecond()
}

impl Journal {
    pub fn open(data_dir: &Path) -> Result<Journal> {
        let uid = paths::euid();
        let db = paths::journal_path(data_dir);
        let existed = db.exists();
        if existed {
            let meta = fs::metadata(&db).ctx(format!("checking {}", db.display()))?;
            if meta.uid() != uid {
                return Err(UndoError::msg(format!(
                    "{} is owned by uid {}, not you (uid {}) — refusing to use it",
                    db.display(),
                    meta.uid(),
                    uid
                )));
            }
        }
        let conn = Connection::open(&db)?;
        if !existed {
            fs::set_permissions(&db, fs::Permissions::from_mode(0o600))
                .ctx(format!("securing {}", db.display()))?;
        }
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "synchronous", "NORMAL")?;
        conn.pragma_update(None, "busy_timeout", 5000)?;
        schema::migrate(&conn)?;
        Ok(Journal { conn, uid })
    }

    pub fn open_default() -> Result<Journal> {
        Journal::open(&paths::ensure_data_dir()?)
    }

    pub fn insert_pending(&self, command: &str, argv: &[String], cwd: &Path) -> Result<i64> {
        self.conn.execute(
            "INSERT INTO operations (ts_ms, uid, username, cwd, command, argv, status, details)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                now_ms(),
                self.uid,
                paths::username(),
                cwd.to_string_lossy(),
                command,
                serde_json::to_string(argv)?,
                Status::PendingExec.as_str(),
                serde_json::to_string(&Details::new())?,
            ],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    pub fn update_details(&self, id: i64, details: &Details) -> Result<()> {
        self.conn.execute(
            "UPDATE operations SET details = ?1 WHERE id = ?2 AND uid = ?3",
            params![serde_json::to_string(details)?, id, self.uid],
        )?;
        Ok(())
    }

    pub fn finalize_exec(&mut self, id: i64, details: &Details) -> Result<()> {
        let tx = self.conn.transaction()?;
        tx.execute(
            "UPDATE operations SET status = ?1, details = ?2 WHERE id = ?3 AND uid = ?4",
            params![
                Status::Applied.as_str(),
                serde_json::to_string(details)?,
                id,
                self.uid
            ],
        )?;
        tx.execute(
            "UPDATE operations SET status = ?1 WHERE uid = ?2 AND status = ?3 AND id != ?4",
            params![
                Status::Superseded.as_str(),
                self.uid,
                Status::Undone.as_str(),
                id
            ],
        )?;
        tx.commit()?;
        Ok(())
    }

    pub fn set_status(&self, id: i64, status: Status) -> Result<()> {
        let ts_col = match status {
            Status::Undone => Some("undo_ts_ms"),
            Status::Applied => Some("redo_ts_ms"),
            _ => None,
        };
        match ts_col {
            Some(col) => self.conn.execute(
                &format!(
                    "UPDATE operations SET status = ?1, {col} = ?2 WHERE id = ?3 AND uid = ?4"
                ),
                params![status.as_str(), now_ms(), id, self.uid],
            )?,
            None => self.conn.execute(
                "UPDATE operations SET status = ?1 WHERE id = ?2 AND uid = ?3",
                params![status.as_str(), id, self.uid],
            )?,
        };
        Ok(())
    }

    pub fn latest(&self, status: Status) -> Result<Option<Operation>> {
        self.conn
            .query_row(
                &format!("SELECT {COLS} FROM operations WHERE uid = ?1 AND status = ?2 ORDER BY id DESC LIMIT 1"),
                params![self.uid, status.as_str()],
                row_to_op,
            )
            .optional()
            .map_err(Into::into)
    }

    pub fn earliest(&self, status: Status) -> Result<Option<Operation>> {
        self.conn
            .query_row(
                &format!("SELECT {COLS} FROM operations WHERE uid = ?1 AND status = ?2 ORDER BY id ASC LIMIT 1"),
                params![self.uid, status.as_str()],
                row_to_op,
            )
            .optional()
            .map_err(Into::into)
    }

    pub fn get_any(&self, id: i64) -> Result<Option<Operation>> {
        self.conn
            .query_row(
                &format!("SELECT {COLS} FROM operations WHERE id = ?1"),
                params![id],
                row_to_op,
            )
            .optional()
            .map_err(Into::into)
    }

    pub fn get(&self, id: i64) -> Result<Option<Operation>> {
        Ok(self.get_any(id)?.filter(|op| op.uid == self.uid))
    }

    pub fn history(&self, limit: Option<usize>) -> Result<Vec<Operation>> {
        let sql = match limit {
            Some(n) => {
                format!("SELECT {COLS} FROM operations WHERE uid = ?1 ORDER BY id DESC LIMIT {n}")
            }
            None => format!("SELECT {COLS} FROM operations WHERE uid = ?1 ORDER BY id DESC"),
        };
        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt.query_map(params![self.uid], row_to_op)?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(Into::into)
    }

    fn by_status(&self, status: Status, limit: usize) -> Result<Vec<Operation>> {
        let mut stmt = self.conn.prepare(&format!(
            "SELECT {COLS} FROM operations WHERE uid = ?1 AND status = ?2 ORDER BY id DESC LIMIT {limit}"
        ))?;
        let rows = stmt.query_map(params![self.uid, status.as_str()], row_to_op)?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(Into::into)
    }

    pub fn stacks(&self, limit: usize) -> Result<(Vec<Operation>, Vec<Operation>)> {
        Ok((
            self.by_status(Status::Applied, limit)?,
            self.by_status(Status::Undone, limit)?,
        ))
    }

    pub fn clear(&self) -> Result<usize> {
        let n = self
            .conn
            .execute("DELETE FROM operations WHERE uid = ?1", params![self.uid])?;
        Ok(n)
    }

    pub fn delete_row(&self, id: i64) -> Result<()> {
        self.conn.execute(
            "DELETE FROM operations WHERE id = ?1 AND uid = ?2",
            params![id, self.uid],
        )?;
        Ok(())
    }

    pub fn sweep_pending(&self) -> Result<Vec<i64>> {
        let mut stmt = self.conn.prepare(
            "UPDATE operations SET status = 'broken'
             WHERE uid = ?1 AND status IN ('pending_exec','pending_undo','pending_redo')
             RETURNING id",
        )?;
        let ids = stmt.query_map(params![self.uid], |r| r.get::<_, i64>(0))?;
        ids.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(Into::into)
    }

    #[doc(hidden)]
    pub fn insert_raw(
        &self,
        uid: u32,
        command: &str,
        argv: &[String],
        cwd: &Path,
        status: Status,
        details: &Details,
    ) -> Result<i64> {
        self.conn.execute(
            "INSERT INTO operations (ts_ms, uid, username, cwd, command, argv, status, details)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                now_ms(),
                uid,
                "test",
                cwd.to_string_lossy(),
                command,
                serde_json::to_string(argv)?,
                status.as_str(),
                serde_json::to_string(details)?,
            ],
        )?;
        Ok(self.conn.last_insert_rowid())
    }
}

const COLS: &str =
    "id, ts_ms, uid, username, cwd, command, argv, status, details, undo_ts_ms, redo_ts_ms";

fn row_to_op(row: &rusqlite::Row<'_>) -> rusqlite::Result<Operation> {
    let argv_json: String = row.get(6)?;
    let status_str: String = row.get(7)?;
    let details_json: String = row.get(8)?;
    let cwd: String = row.get(4)?;
    Ok(Operation {
        id: row.get(0)?,
        ts_ms: row.get(1)?,
        uid: row.get(2)?,
        username: row.get(3)?,
        cwd: PathBuf::from(cwd),
        command: row.get(5)?,
        argv: serde_json::from_str(&argv_json).unwrap_or_default(),
        status: Status::parse(&status_str).unwrap_or(Status::Broken),
        details: serde_json::from_str(&details_json).unwrap_or_default(),
        undo_ts_ms: row.get(9)?,
        redo_ts_ms: row.get(10)?,
    })
}
