use std::fmt;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

pub const DETAILS_VERSION: u32 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    PendingExec,
    Applied,
    PendingUndo,
    Undone,
    PendingRedo,
    Superseded,
    Broken,
}

impl Status {
    pub fn as_str(self) -> &'static str {
        match self {
            Status::PendingExec => "pending_exec",
            Status::Applied => "applied",
            Status::PendingUndo => "pending_undo",
            Status::Undone => "undone",
            Status::PendingRedo => "pending_redo",
            Status::Superseded => "superseded",
            Status::Broken => "broken",
        }
    }

    pub fn parse(s: &str) -> Option<Status> {
        Some(match s {
            "pending_exec" => Status::PendingExec,
            "applied" => Status::Applied,
            "pending_undo" => Status::PendingUndo,
            "undone" => Status::Undone,
            "pending_redo" => Status::PendingRedo,
            "superseded" => Status::Superseded,
            "broken" => Status::Broken,
            _ => return None,
        })
    }

    pub fn is_pending(self) -> bool {
        matches!(
            self,
            Status::PendingExec | Status::PendingUndo | Status::PendingRedo
        )
    }
}

impl fmt::Display for Status {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum FileState {
    Absent,
    File {
        mode: u32,
        uid: u32,
        gid: u32,
        size: u64,
        mtime_ms: i64,
        blake3: Option<String>,
        dev: u64,
        ino: u64,
    },
    Dir {
        mode: u32,
        uid: u32,
        gid: u32,
        mtime_ms: i64,
    },
    Symlink {
        target: PathBuf,
        uid: u32,
        gid: u32,
    },
    Other {
        mode: u32,
        uid: u32,
        gid: u32,
    },
}

impl FileState {
    pub fn is_absent(&self) -> bool {
        matches!(self, FileState::Absent)
    }

    pub fn kind_name(&self) -> &'static str {
        match self {
            FileState::Absent => "absent",
            FileState::File { .. } => "file",
            FileState::Dir { .. } => "directory",
            FileState::Symlink { .. } => "symlink",
            FileState::Other { .. } => "special file",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TreeSummary {
    pub files: u64,
    pub dirs: u64,
    pub bytes: u64,
    pub capped: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TrashRef {
    pub origin: PathBuf,
    pub file: PathBuf,
    pub info: PathBuf,
    pub state: FileState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tree: Option<TreeSummary>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Action {
    Move {
        src: PathBuf,
        dst: PathBuf,
        pre: FileState,
        post: FileState,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        backup: Option<TrashRef>,
    },
    MoveXdev {
        src: PathBuf,
        dst: PathBuf,
        pre: FileState,
        post: FileState,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        backup: Option<TrashRef>,
    },
    Copy {
        src: PathBuf,
        dst: PathBuf,
        src_state: FileState,
        post: FileState,
        #[serde(default)]
        preserve: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        backup: Option<TrashRef>,
    },
    CopyTree {
        src: PathBuf,
        dst: PathBuf,
        src_root: FileState,
        dst_root: FileState,
        summary: TreeSummary,
        #[serde(default)]
        preserve: bool,
    },
    TrashPut {
        origin: PathBuf,
        trash: TrashRef,
    },
    CreateDir {
        path: PathBuf,
        mode: u32,
        post: FileState,
    },
    SetMode {
        path: PathBuf,
        old: u32,
        new: u32,
    },
    SetOwner {
        path: PathBuf,
        old_uid: u32,
        old_gid: u32,
        new_uid: u32,
        new_gid: u32,
        deref: bool,
    },
    Symlink {
        target: PathBuf,
        link: PathBuf,
        post: FileState,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        backup: Option<TrashRef>,
    },
    Hardlink {
        src: PathBuf,
        link: PathBuf,
        post: FileState,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        backup: Option<TrashRef>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Details {
    pub v: u32,
    pub actions: Vec<Action>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub force_evictions: Vec<TrashRef>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub undo_artifacts: Vec<TrashRef>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub broken_at: Option<usize>,
}

impl Details {
    pub fn new() -> Self {
        Details {
            v: DETAILS_VERSION,
            actions: Vec::new(),
            force_evictions: Vec::new(),
            undo_artifacts: Vec::new(),
            broken_at: None,
        }
    }

    pub fn trash_refs(&self) -> Vec<&TrashRef> {
        let mut refs = Vec::new();
        for action in &self.actions {
            match action {
                Action::Move { backup, .. }
                | Action::MoveXdev { backup, .. }
                | Action::Copy { backup, .. }
                | Action::Symlink { backup, .. }
                | Action::Hardlink { backup, .. } => {
                    if let Some(b) = backup {
                        refs.push(b);
                    }
                }
                Action::TrashPut { trash, .. } => refs.push(trash),
                _ => {}
            }
        }
        refs.extend(self.force_evictions.iter());
        refs.extend(self.undo_artifacts.iter());
        refs
    }
}

impl Default for Details {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Clone)]
pub struct Operation {
    pub id: i64,
    pub ts_ms: i64,
    pub uid: u32,
    pub username: String,
    pub cwd: PathBuf,
    pub command: String,
    pub argv: Vec<String>,
    pub status: Status,
    pub details: Details,
    pub undo_ts_ms: Option<i64>,
    pub redo_ts_ms: Option<i64>,
}

impl Operation {
    pub fn summary(&self) -> String {
        self.argv
            .iter()
            .map(|a| {
                if a.is_empty() || a.contains(|c: char| c.is_whitespace() || "\"'\\$`".contains(c))
                {
                    format!("'{}'", a.replace('\'', r"'\''"))
                } else {
                    a.clone()
                }
            })
            .collect::<Vec<_>>()
            .join(" ")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file_state() -> FileState {
        FileState::File {
            mode: 0o644,
            uid: 1000,
            gid: 1000,
            size: 42,
            mtime_ms: 1_752_000_000_000,
            blake3: Some("deadbeef".into()),
            dev: 1,
            ino: 2,
        }
    }

    fn trash_ref() -> TrashRef {
        TrashRef {
            origin: "/home/x/f".into(),
            file: "/home/x/.local/share/Trash/files/f".into(),
            info: "/home/x/.local/share/Trash/info/f.trashinfo".into(),
            state: file_state(),
            tree: None,
        }
    }

    #[test]
    fn details_roundtrip_all_action_variants() {
        let actions = vec![
            Action::Move {
                src: "/a".into(),
                dst: "/b".into(),
                pre: file_state(),
                post: file_state(),
                backup: Some(trash_ref()),
            },
            Action::MoveXdev {
                src: "/a".into(),
                dst: "/b".into(),
                pre: file_state(),
                post: file_state(),
                backup: None,
            },
            Action::Copy {
                src: "/a".into(),
                dst: "/b".into(),
                src_state: file_state(),
                post: file_state(),
                preserve: true,
                backup: None,
            },
            Action::CopyTree {
                src: "/a".into(),
                dst: "/b".into(),
                src_root: FileState::Dir {
                    mode: 0o755,
                    uid: 0,
                    gid: 0,
                    mtime_ms: 0,
                },
                dst_root: FileState::Dir {
                    mode: 0o755,
                    uid: 0,
                    gid: 0,
                    mtime_ms: 0,
                },
                summary: TreeSummary {
                    files: 3,
                    dirs: 1,
                    bytes: 10,
                    capped: false,
                },
                preserve: false,
            },
            Action::TrashPut {
                origin: "/a".into(),
                trash: trash_ref(),
            },
            Action::CreateDir {
                path: "/d".into(),
                mode: 0o755,
                post: FileState::Dir {
                    mode: 0o755,
                    uid: 0,
                    gid: 0,
                    mtime_ms: 0,
                },
            },
            Action::SetMode {
                path: "/f".into(),
                old: 0o644,
                new: 0o600,
            },
            Action::SetOwner {
                path: "/f".into(),
                old_uid: 1000,
                old_gid: 1000,
                new_uid: 1000,
                new_gid: 100,
                deref: false,
            },
            Action::Symlink {
                target: "../x".into(),
                link: "/l".into(),
                post: FileState::Symlink {
                    target: "../x".into(),
                    uid: 1000,
                    gid: 1000,
                },
                backup: None,
            },
            Action::Hardlink {
                src: "/a".into(),
                link: "/l".into(),
                post: file_state(),
                backup: Some(trash_ref()),
            },
        ];
        let details = Details {
            actions,
            ..Details::new()
        };
        let json = serde_json::to_string(&details).unwrap();
        let back: Details = serde_json::from_str(&json).unwrap();
        assert_eq!(details, back);
    }

    #[test]
    fn status_string_roundtrip() {
        for s in [
            Status::PendingExec,
            Status::Applied,
            Status::PendingUndo,
            Status::Undone,
            Status::PendingRedo,
            Status::Superseded,
            Status::Broken,
        ] {
            assert_eq!(Status::parse(s.as_str()), Some(s));
        }
        assert_eq!(Status::parse("nope"), None);
    }

    #[test]
    fn summary_quotes_awkward_args() {
        let op = Operation {
            id: 1,
            ts_ms: 0,
            uid: 1000,
            username: "u".into(),
            cwd: "/".into(),
            command: "mv".into(),
            argv: vec!["mv".into(), "a file".into(), "b".into()],
            status: Status::Applied,
            details: Details::new(),
            undo_ts_ms: None,
            redo_ts_ms: None,
        };
        assert_eq!(op.summary(), "mv 'a file' b");
    }
}
