use std::fs;
use std::io::{self, Read};
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::{IoCtx, Result, UndoError};
use crate::journal::model::{FileState, TreeSummary};
use crate::paths::Limits;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConflictKind {
    Modified,
    Occupied,
    Missing,
    TypeChanged,
    ForeignUid,
    Broken,
}

impl ConflictKind {
    pub fn as_str(self) -> &'static str {
        match self {
            ConflictKind::Modified => "modified",
            ConflictKind::Occupied => "occupied",
            ConflictKind::Missing => "missing",
            ConflictKind::TypeChanged => "type_changed",
            ConflictKind::ForeignUid => "foreign_uid",
            ConflictKind::Broken => "broken",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Conflict {
    pub path: PathBuf,
    pub kind: ConflictKind,
    pub expected: String,
    pub found: String,
}

impl Conflict {
    pub fn new(
        path: impl Into<PathBuf>,
        kind: ConflictKind,
        expected: impl Into<String>,
        found: impl Into<String>,
    ) -> Self {
        Conflict {
            path: path.into(),
            kind,
            expected: expected.into(),
            found: found.into(),
        }
    }
}

impl std::fmt::Display for Conflict {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{}: {} (expected {}, found {})",
            self.path.display(),
            self.kind.as_str(),
            self.expected,
            self.found
        )
    }
}

#[derive(Debug, Default, Clone)]
pub struct Verification {
    pub warnings: Vec<String>,
    pub conflicts: Vec<Conflict>,
}

impl Verification {
    pub fn ok(&self) -> bool {
        self.conflicts.is_empty()
    }

    pub fn merge(&mut self, other: Verification) {
        self.warnings.extend(other.warnings);
        self.conflicts.extend(other.conflicts);
    }

    pub fn warn(&mut self, msg: impl Into<String>) {
        self.warnings.push(msg.into());
    }

    pub fn block(&mut self, c: Conflict) {
        self.conflicts.push(c);
    }
}

fn mtime_ms(meta: &fs::Metadata) -> i64 {
    meta.mtime() * 1000 + meta.mtime_nsec() / 1_000_000
}

pub fn hash_file(path: &Path) -> Result<String> {
    let mut file = fs::File::open(path).ctx(format!("hashing {}", path.display()))?;
    let mut hasher = blake3::Hasher::new();
    let mut buf = [0u8; 64 * 1024];
    loop {
        let n = file
            .read(&mut buf)
            .ctx(format!("hashing {}", path.display()))?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hasher.finalize().to_hex().to_string())
}

pub fn capture(path: &Path, limits: &Limits) -> Result<FileState> {
    let meta = match fs::symlink_metadata(path) {
        Ok(m) => m,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(FileState::Absent),
        Err(e) => return Err(UndoError::io(format!("inspecting {}", path.display()), e)),
    };
    let ft = meta.file_type();
    if ft.is_symlink() {
        let target = fs::read_link(path).ctx(format!("reading link {}", path.display()))?;
        Ok(FileState::Symlink {
            target,
            uid: meta.uid(),
            gid: meta.gid(),
        })
    } else if ft.is_dir() {
        Ok(FileState::Dir {
            mode: meta.mode() & 0o7777,
            uid: meta.uid(),
            gid: meta.gid(),
            mtime_ms: mtime_ms(&meta),
        })
    } else if ft.is_file() {
        let blake3 = if meta.len() <= limits.hash_max {
            Some(hash_file(path)?)
        } else {
            None
        };
        Ok(FileState::File {
            mode: meta.mode() & 0o7777,
            uid: meta.uid(),
            gid: meta.gid(),
            size: meta.len(),
            mtime_ms: mtime_ms(&meta),
            blake3,
            dev: meta.dev(),
            ino: meta.ino(),
        })
    } else {
        Ok(FileState::Other {
            mode: meta.mode() & 0o7777,
            uid: meta.uid(),
            gid: meta.gid(),
        })
    }
}

pub fn summarize_tree(root: &Path, limits: &Limits) -> Result<TreeSummary> {
    let mut summary = TreeSummary {
        files: 0,
        dirs: 0,
        bytes: 0,
        capped: false,
    };
    let mut stack = vec![root.to_path_buf()];
    let mut seen: u64 = 0;
    while let Some(dir) = stack.pop() {
        let entries = fs::read_dir(&dir).ctx(format!("walking {}", dir.display()))?;
        for entry in entries {
            let entry = entry.ctx(format!("walking {}", dir.display()))?;
            seen += 1;
            if seen > limits.tree_cap {
                summary.capped = true;
                return Ok(summary);
            }
            let meta = entry
                .metadata()
                .ctx(format!("walking {}", entry.path().display()))?;
            if meta.file_type().is_dir() {
                summary.dirs += 1;
                stack.push(entry.path());
            } else {
                summary.files += 1;
                if meta.file_type().is_file() {
                    summary.bytes += meta.len();
                }
            }
        }
    }
    Ok(summary)
}

pub fn capture_with_tree(path: &Path, limits: &Limits) -> Result<(FileState, Option<TreeSummary>)> {
    let state = capture(path, limits)?;
    let tree = match &state {
        FileState::Dir { .. } => Some(summarize_tree(path, limits)?),
        _ => None,
    };
    Ok((state, tree))
}

fn describe_current(path: &Path) -> String {
    match fs::symlink_metadata(path) {
        Ok(m) if m.file_type().is_symlink() => "symlink".into(),
        Ok(m) if m.is_dir() => "directory".into(),
        Ok(m) if m.is_file() => format!("file ({} bytes)", m.len()),
        Ok(_) => "special file".into(),
        Err(_) => "absent".into(),
    }
}

pub fn verify_node(path: &Path, expected: &FileState, limits: &Limits) -> Verification {
    let mut v = Verification::default();
    let current = match capture(path, limits) {
        Ok(c) => c,
        Err(e) => {
            v.block(Conflict::new(
                path,
                ConflictKind::Missing,
                expected.kind_name(),
                format!("unreadable: {e}"),
            ));
            return v;
        }
    };

    match (expected, &current) {
        (FileState::Absent, FileState::Absent) => {}
        (FileState::Absent, _) => {
            v.block(Conflict::new(
                path,
                ConflictKind::Occupied,
                "absent",
                describe_current(path),
            ));
        }
        (_, FileState::Absent) => {
            v.block(Conflict::new(
                path,
                ConflictKind::Missing,
                expected.kind_name(),
                "absent",
            ));
        }
        (
            FileState::File {
                size,
                mtime_ms,
                blake3,
                mode,
                uid,
                gid,
                ..
            },
            FileState::File {
                size: cur_size,
                mtime_ms: cur_mtime,
                blake3: cur_hash,
                mode: cur_mode,
                uid: cur_uid,
                gid: cur_gid,
                ..
            },
        ) => {
            match (blake3, cur_hash) {
                (Some(exp), Some(cur)) => {
                    if exp != cur {
                        v.block(Conflict::new(
                            path,
                            ConflictKind::Modified,
                            format!("blake3 {}", &exp[..exp.len().min(12)]),
                            format!("blake3 {}", &cur[..cur.len().min(12)]),
                        ));
                        return v;
                    }
                    if mtime_ms != cur_mtime || mode != cur_mode {
                        v.warn(format!(
                            "{}: metadata drifted since recording (content verified)",
                            path.display()
                        ));
                    }
                }
                _ => {
                    if size != cur_size || mtime_ms != cur_mtime {
                        v.block(Conflict::new(
                            path,
                            ConflictKind::Modified,
                            format!("{size} bytes, mtime {mtime_ms}"),
                            format!("{cur_size} bytes, mtime {cur_mtime}"),
                        ));
                        return v;
                    }
                }
            }
            if uid != cur_uid || gid != cur_gid {
                v.warn(format!(
                    "{}: ownership drifted since recording",
                    path.display()
                ));
            }
        }
        (FileState::Dir { mode, .. }, FileState::Dir { mode: cur_mode, .. }) => {
            if mode != cur_mode {
                v.warn(format!(
                    "{}: directory mode drifted since recording",
                    path.display()
                ));
            }
        }
        (
            FileState::Symlink { target, .. },
            FileState::Symlink {
                target: cur_target, ..
            },
        ) => {
            if target != cur_target {
                v.block(Conflict::new(
                    path,
                    ConflictKind::Modified,
                    format!("symlink -> {}", target.display()),
                    format!("symlink -> {}", cur_target.display()),
                ));
            }
        }
        (FileState::Other { .. }, FileState::Other { .. }) => {}
        _ => {
            v.block(Conflict::new(
                path,
                ConflictKind::TypeChanged,
                expected.kind_name(),
                current.kind_name(),
            ));
        }
    }
    v
}

pub fn verify_tree(path: &Path, expected: &TreeSummary, limits: &Limits) -> Verification {
    let mut v = Verification::default();
    if expected.capped {
        v.warn(format!(
            "{}: tree was recorded capped at {} nodes — verification is shallow",
            path.display(),
            limits.tree_cap
        ));
        return v;
    }
    match summarize_tree(path, limits) {
        Ok(cur) => {
            if cur.files != expected.files
                || cur.dirs != expected.dirs
                || cur.bytes != expected.bytes
            {
                v.block(Conflict::new(
                    path,
                    ConflictKind::Modified,
                    format!(
                        "{} files, {} dirs, {} bytes",
                        expected.files, expected.dirs, expected.bytes
                    ),
                    format!(
                        "{} files, {} dirs, {} bytes",
                        cur.files, cur.dirs, cur.bytes
                    ),
                ));
            }
        }
        Err(e) => {
            v.block(Conflict::new(
                path,
                ConflictKind::Missing,
                "directory tree",
                format!("unreadable: {e}"),
            ));
        }
    }
    v
}

pub fn verify_absent(path: &Path) -> Verification {
    let mut v = Verification::default();
    if fs::symlink_metadata(path).is_ok() {
        v.block(Conflict::new(
            path,
            ConflictKind::Occupied,
            "absent",
            describe_current(path),
        ));
    }
    v
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write as _;

    fn limits() -> Limits {
        Limits {
            hash_max: 1024 * 1024,
            tree_cap: 1000,
        }
    }

    fn tmp() -> tempfile_lite::TempDir {
        tempfile_lite::TempDir::new()
    }

    mod tempfile_lite {
        use std::path::{Path, PathBuf};
        use std::sync::atomic::{AtomicU32, Ordering};

        static N: AtomicU32 = AtomicU32::new(0);

        pub struct TempDir(PathBuf);

        impl TempDir {
            pub fn new() -> TempDir {
                let p = std::env::temp_dir().join(format!(
                    "undo-state-test-{}-{}",
                    std::process::id(),
                    N.fetch_add(1, Ordering::SeqCst)
                ));
                std::fs::create_dir_all(&p).unwrap();
                TempDir(p)
            }

            pub fn path(&self) -> &Path {
                &self.0
            }
        }

        impl Drop for TempDir {
            fn drop(&mut self) {
                let _ = std::fs::remove_dir_all(&self.0);
            }
        }
    }

    #[test]
    fn capture_absent_file_dir_symlink() {
        let t = tmp();
        let missing = t.path().join("missing");
        assert_eq!(capture(&missing, &limits()).unwrap(), FileState::Absent);

        let file = t.path().join("f");
        std::fs::File::create(&file)
            .unwrap()
            .write_all(b"hello")
            .unwrap();
        match capture(&file, &limits()).unwrap() {
            FileState::File { size, blake3, .. } => {
                assert_eq!(size, 5);
                assert!(blake3.is_some());
            }
            other => panic!("expected file, got {other:?}"),
        }

        let dir = t.path().join("d");
        std::fs::create_dir(&dir).unwrap();
        assert!(matches!(
            capture(&dir, &limits()).unwrap(),
            FileState::Dir { .. }
        ));

        let link = t.path().join("l");
        std::os::unix::fs::symlink("f", &link).unwrap();
        match capture(&link, &limits()).unwrap() {
            FileState::Symlink { target, .. } => assert_eq!(target, PathBuf::from("f")),
            other => panic!("expected symlink, got {other:?}"),
        }
    }

    #[test]
    fn large_files_skip_hashing() {
        let t = tmp();
        let file = t.path().join("big");
        std::fs::File::create(&file)
            .unwrap()
            .write_all(b"0123456789")
            .unwrap();
        let tiny = Limits {
            hash_max: 4,
            tree_cap: 1000,
        };
        match capture(&file, &tiny).unwrap() {
            FileState::File { blake3, .. } => assert!(blake3.is_none()),
            other => panic!("expected file, got {other:?}"),
        }
    }

    #[test]
    fn verify_detects_content_change() {
        let t = tmp();
        let file = t.path().join("f");
        std::fs::File::create(&file)
            .unwrap()
            .write_all(b"one")
            .unwrap();
        let recorded = capture(&file, &limits()).unwrap();

        std::fs::File::create(&file)
            .unwrap()
            .write_all(b"two")
            .unwrap();
        let v = verify_node(&file, &recorded, &limits());
        assert!(!v.ok());
        assert_eq!(v.conflicts[0].kind, ConflictKind::Modified);
    }

    #[test]
    fn verify_detects_type_change_and_occupation() {
        let t = tmp();
        let file = t.path().join("f");
        std::fs::File::create(&file)
            .unwrap()
            .write_all(b"x")
            .unwrap();
        let recorded = capture(&file, &limits()).unwrap();

        std::fs::remove_file(&file).unwrap();
        std::fs::create_dir(&file).unwrap();
        let v = verify_node(&file, &recorded, &limits());
        assert_eq!(v.conflicts[0].kind, ConflictKind::TypeChanged);

        let v = verify_absent(&file);
        assert_eq!(v.conflicts[0].kind, ConflictKind::Occupied);
        std::fs::remove_dir(&file).unwrap();
        assert!(verify_absent(&file).ok());
    }

    #[test]
    fn verify_symlink_by_target_string() {
        let t = tmp();
        let link = t.path().join("l");
        std::os::unix::fs::symlink("a", &link).unwrap();
        let recorded = capture(&link, &limits()).unwrap();

        std::fs::remove_file(&link).unwrap();
        std::os::unix::fs::symlink("b", &link).unwrap();
        let v = verify_node(&link, &recorded, &limits());
        assert!(!v.ok());
        assert_eq!(v.conflicts[0].kind, ConflictKind::Modified);
    }

    #[test]
    fn tree_summary_counts_and_verifies() {
        let t = tmp();
        let root = t.path().join("tree");
        std::fs::create_dir_all(root.join("sub")).unwrap();
        std::fs::File::create(root.join("a"))
            .unwrap()
            .write_all(b"aa")
            .unwrap();
        std::fs::File::create(root.join("sub/b"))
            .unwrap()
            .write_all(b"bbb")
            .unwrap();
        std::os::unix::fs::symlink("a", root.join("l")).unwrap();

        let s = summarize_tree(&root, &limits()).unwrap();
        assert_eq!((s.files, s.dirs, s.bytes, s.capped), (3, 1, 5, false));

        assert!(verify_tree(&root, &s, &limits()).ok());
        std::fs::File::create(root.join("new")).unwrap();
        assert!(!verify_tree(&root, &s, &limits()).ok());
    }

    #[test]
    fn tree_cap_marks_capped() {
        let t = tmp();
        let root = t.path().join("tree");
        std::fs::create_dir(&root).unwrap();
        for i in 0..10 {
            std::fs::File::create(root.join(format!("f{i}"))).unwrap();
        }
        let capped = Limits {
            hash_max: 0,
            tree_cap: 3,
        };
        let s = summarize_tree(&root, &capped).unwrap();
        assert!(s.capped);
    }
}
