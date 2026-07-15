use std::io;
use std::process::ExitCode;

use thiserror::Error;

pub const FALLBACK_CODE: u8 = 125;

#[derive(Debug, Error)]
pub enum UndoError {
    #[error("{0}")]
    Fallback(String),

    #[error("{0}")]
    Usage(String),

    #[error("{0}")]
    Msg(String),

    #[error("{ctx}: {source}")]
    Io {
        ctx: String,
        #[source]
        source: io::Error,
    },

    #[error("journal: {0}")]
    Sql(#[from] rusqlite::Error),

    #[error("journal data: {0}")]
    Json(#[from] serde_json::Error),
}

pub type Result<T> = std::result::Result<T, UndoError>;

impl UndoError {
    pub fn msg(m: impl Into<String>) -> Self {
        UndoError::Msg(m.into())
    }

    pub fn usage(m: impl Into<String>) -> Self {
        UndoError::Usage(m.into())
    }

    pub fn fallback(m: impl Into<String>) -> Self {
        UndoError::Fallback(m.into())
    }

    pub fn io(ctx: impl Into<String>, source: io::Error) -> Self {
        UndoError::Io {
            ctx: ctx.into(),
            source,
        }
    }

    pub fn exit_code(&self) -> ExitCode {
        match self {
            UndoError::Fallback(_) => ExitCode::from(FALLBACK_CODE),
            UndoError::Usage(_) => ExitCode::from(2),
            _ => ExitCode::FAILURE,
        }
    }
}

pub trait IoCtx<T> {
    fn ctx(self, ctx: impl Into<String>) -> Result<T>;
}

impl<T> IoCtx<T> for io::Result<T> {
    fn ctx(self, ctx: impl Into<String>) -> Result<T> {
        self.map_err(|e| UndoError::io(ctx, e))
    }
}
