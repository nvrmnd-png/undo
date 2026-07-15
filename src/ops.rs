use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use serde::Serialize;

use crate::error::{IoCtx, Result, UndoError};
use crate::journal::model::{Action, FileState, TrashRef};
use crate::paths::Limits;
use crate::state::{self, Conflict, ConflictKind, Verification};
use crate::trash::Trash;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    Undo,
    Redo,
}

impl Direction {
    pub fn opposite(self) -> Direction {
        match self {
            Direction::Undo => Direction::Redo,
            Direction::Redo => Direction::Undo,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct Change {
    pub kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub from: Option<PathBuf>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub to: Option<PathBuf>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<PathBuf>,
}

impl Change {
    pub fn moved(from: &Path, to: &Path) -> Change {
        Change {
            kind: "move".into(),
            from: Some(from.into()),
            to: Some(to.into()),
            path: None,
        }
    }

    pub fn copied(from: &Path, to: &Path) -> Change {
        Change {
            kind: "copy".into(),
            from: Some(from.into()),
            to: Some(to.into()),
            path: None,
        }
    }

    pub fn trashed(path: &Path, trash_file: &Path) -> Change {
        Change {
            kind: "trash".into(),
            from: Some(path.into()),
            to: Some(trash_file.into()),
            path: None,
        }
    }

    pub fn restored(trash_file: &Path, to: &Path) -> Change {
        Change {
            kind: "restore".into(),
            from: Some(trash_file.into()),
            to: Some(to.into()),
            path: None,
        }
    }

    pub fn simple(kind: &str, path: &Path) -> Change {
        Change {
            kind: kind.into(),
            from: None,
            to: None,
            path: Some(path.into()),
        }
    }
}

impl std::fmt::Display for Change {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match (&self.from, &self.to, &self.path) {
            (Some(from), Some(to), _) => {
                write!(f, "{} {} -> {}", self.kind, from.display(), to.display())
            }
            (_, _, Some(p)) => write!(f, "{} {}", self.kind, p.display()),
            _ => write!(f, "{}", self.kind),
        }
    }
}

pub struct OpCtx<'a> {
    pub trash: &'a Trash,
    pub limits: &'a Limits,
    pub force: bool,
    pub evictions: Vec<TrashRef>,
    pub artifacts: Vec<TrashRef>,
    pub changes: Vec<Change>,
    pub warnings: Vec<String>,
}

impl<'a> OpCtx<'a> {
    pub fn new(trash: &'a Trash, limits: &'a Limits, force: bool) -> Self {
        OpCtx {
            trash,
            limits,
            force,
            evictions: Vec::new(),
            artifacts: Vec::new(),
            changes: Vec::new(),
            warnings: Vec::new(),
        }
    }
}

pub fn process_umask() -> u32 {
    static UMASK: OnceLock<u32> = OnceLock::new();
    *UMASK.get_or_init(|| {
        let probe = rustix::fs::Mode::from_raw_mode(0o022);
        let old = rustix::process::umask(probe);
        rustix::process::umask(old);
        old.as_raw_mode() & 0o777
    })
}

fn set_mode(path: &Path, mode: u32) -> Result<()> {
    fs::set_permissions(path, fs::Permissions::from_mode(mode & 0o7777))
        .ctx(format!("setting mode on {}", path.display()))
}

fn copy_mtime(from_meta: &fs::Metadata, to: &Path) -> Result<()> {
    let ft = filetime::FileTime::from_last_modification_time(from_meta);
    filetime::set_file_mtime(to, ft).ctx(format!("setting mtime on {}", to.display()))
}

pub fn verified_copy_file(src: &Path, dst: &Path) -> Result<()> {
    let src_meta = fs::metadata(src).ctx(format!("inspecting {}", src.display()))?;
    let mut src_f = File::open(src).ctx(format!("opening {}", src.display()))?;
    let mut dst_f = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(dst)
        .ctx(format!("creating {}", dst.display()))?;
    let mut hasher = blake3::Hasher::new();
    let mut buf = vec![0u8; 128 * 1024];
    loop {
        let n = src_f
            .read(&mut buf)
            .ctx(format!("reading {}", src.display()))?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
        dst_f
            .write_all(&buf[..n])
            .ctx(format!("writing {}", dst.display()))?;
    }
    dst_f.sync_all().ctx(format!("syncing {}", dst.display()))?;
    drop(dst_f);
    let src_hash = hasher.finalize().to_hex().to_string();
    let dst_hash = state::hash_file(dst)?;
    if src_hash != dst_hash {
        let _ = fs::remove_file(dst);
        return Err(UndoError::msg(format!(
            "verified copy failed: {} and {} differ after copy",
            src.display(),
            dst.display()
        )));
    }
    set_mode(dst, src_meta.mode())?;
    copy_mtime(&src_meta, dst)?;
    Ok(())
}

fn copy_symlink(src: &Path, dst: &Path) -> Result<()> {
    let target = fs::read_link(src).ctx(format!("reading link {}", src.display()))?;
    std::os::unix::fs::symlink(&target, dst).ctx(format!("creating symlink {}", dst.display()))
}

pub fn verified_copy_tree(src: &Path, dst: &Path) -> Result<()> {
    let src_meta = fs::symlink_metadata(src).ctx(format!("inspecting {}", src.display()))?;
    fs::create_dir(dst).ctx(format!("creating {}", dst.display()))?;
    for entry in fs::read_dir(src).ctx(format!("reading {}", src.display()))? {
        let entry = entry.ctx(format!("reading {}", src.display()))?;
        let from = entry.path();
        let to = dst.join(entry.file_name());
        let meta = entry
            .metadata()
            .ctx(format!("inspecting {}", from.display()))?;
        let ft = meta.file_type();
        if ft.is_dir() {
            verified_copy_tree(&from, &to)?;
        } else if ft.is_file() {
            verified_copy_file(&from, &to)?;
        } else if ft.is_symlink() {
            copy_symlink(&from, &to)?;
        } else {
            return Err(UndoError::msg(format!(
                "cannot copy special file {}",
                from.display()
            )));
        }
    }
    set_mode(dst, src_meta.mode())?;
    copy_mtime(&src_meta, dst)?;
    Ok(())
}

pub fn verified_copy_node(src: &Path, dst: &Path) -> Result<()> {
    let meta = fs::symlink_metadata(src).ctx(format!("inspecting {}", src.display()))?;
    let ft = meta.file_type();
    if ft.is_dir() {
        verified_copy_tree(src, dst)
    } else if ft.is_file() {
        verified_copy_file(src, dst)
    } else if ft.is_symlink() {
        copy_symlink(src, dst)
    } else {
        Err(UndoError::msg(format!(
            "cannot copy special file {}",
            src.display()
        )))
    }
}

pub fn remove_node(path: &Path) -> Result<()> {
    let meta = fs::symlink_metadata(path).ctx(format!("inspecting {}", path.display()))?;
    if meta.file_type().is_dir() {
        fs::remove_dir_all(path).ctx(format!("removing {}", path.display()))
    } else {
        fs::remove_file(path).ctx(format!("removing {}", path.display()))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MoveKind {
    Rename,
    Xdev,
}

pub fn safe_move(src: &Path, dst: &Path) -> Result<MoveKind> {
    match fs::rename(src, dst) {
        Ok(()) => Ok(MoveKind::Rename),
        Err(e) if e.kind() == io::ErrorKind::CrossesDevices => {
            verified_copy_node(src, dst)?;
            remove_node(src)?;
            Ok(MoveKind::Xdev)
        }
        Err(e) => Err(UndoError::io(
            format!("moving {} -> {}", src.display(), dst.display()),
            e,
        )),
    }
}

pub fn copy_file_for_cp(src: &Path, dst: &Path, preserve: bool) -> Result<()> {
    verified_copy_file(src, dst)?;
    let src_meta = fs::metadata(src).ctx(format!("inspecting {}", src.display()))?;
    if preserve {
        let _ = std::os::unix::fs::chown(dst, Some(src_meta.uid()), Some(src_meta.gid()));
    } else {
        set_mode(dst, src_meta.mode() & !process_umask())?;
        filetime::set_file_mtime(dst, filetime::FileTime::now())
            .ctx(format!("setting mtime on {}", dst.display()))?;
    }
    Ok(())
}

pub fn copy_tree_for_cp(src: &Path, dst: &Path, preserve: bool) -> Result<()> {
    verified_copy_tree(src, dst)?;
    if !preserve {
        reset_tree_defaults(dst)?;
    }
    Ok(())
}

fn reset_tree_defaults(root: &Path) -> Result<()> {
    let umask = process_umask();
    let meta = fs::symlink_metadata(root).ctx(format!("inspecting {}", root.display()))?;
    if meta.file_type().is_dir() {
        for entry in fs::read_dir(root).ctx(format!("reading {}", root.display()))? {
            let entry = entry.ctx(format!("reading {}", root.display()))?;
            reset_tree_defaults(&entry.path())?;
        }
    }
    if !meta.file_type().is_symlink() {
        set_mode(root, meta.mode() & !umask)?;
        filetime::set_file_mtime(root, filetime::FileTime::now())
            .ctx(format!("setting mtime on {}", root.display()))?;
    }
    Ok(())
}

fn ensure_parent(path: &Path, warnings: &mut Vec<String>) -> Result<()> {
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
        && fs::symlink_metadata(parent).is_err()
    {
        fs::create_dir_all(parent).ctx(format!("recreating {}", parent.display()))?;
        warnings.push(format!(
            "recreated missing parent directory {} (original metadata not preserved)",
            parent.display()
        ));
    }
    Ok(())
}

fn verify_backup(backup: &Option<TrashRef>, v: &mut Verification, limits: &Limits) {
    if let Some(b) = backup {
        v.merge(state::verify_node(&b.file, &b.state, limits));
        if let Some(tree) = &b.tree {
            v.merge(state::verify_tree(&b.file, tree, limits));
        }
    }
}

fn as_warnings(v: Verification) -> Verification {
    let mut out = Verification {
        warnings: v.warnings,
        conflicts: Vec::new(),
    };
    for c in v.conflicts {
        out.warn(format!("{c} — proceeding with current state"));
    }
    out
}

fn verify_landing_site(path: &Path, backup: &Option<TrashRef>, limits: &Limits) -> Verification {
    match backup {
        Some(b) => state::verify_node(path, &b.state, limits),
        None => state::verify_absent(path),
    }
}

pub fn verify_action(
    action: &Action,
    dir: Direction,
    limits: &Limits,
    removed_siblings: &std::collections::HashSet<PathBuf>,
) -> Verification {
    let mut v = Verification::default();
    match (action, dir) {
        (
            Action::Move {
                src,
                dst,
                post,
                backup,
                ..
            },
            Direction::Undo,
        )
        | (
            Action::MoveXdev {
                src,
                dst,
                post,
                backup,
                ..
            },
            Direction::Undo,
        ) => {
            v.merge(state::verify_node(dst, post, limits));
            v.merge(state::verify_absent(src));
            verify_backup(backup, &mut v, limits);
        }
        (
            Action::Move {
                src,
                dst,
                post,
                backup,
                ..
            },
            Direction::Redo,
        )
        | (
            Action::MoveXdev {
                src,
                dst,
                post,
                backup,
                ..
            },
            Direction::Redo,
        ) => {
            v.merge(state::verify_node(src, post, limits));
            v.merge(verify_landing_site(dst, backup, limits));
        }
        (
            Action::Copy {
                dst, post, backup, ..
            },
            Direction::Undo,
        ) => {
            v.merge(state::verify_node(dst, post, limits));
            verify_backup(backup, &mut v, limits);
        }
        (
            Action::Copy {
                src,
                dst,
                src_state,
                backup,
                ..
            },
            Direction::Redo,
        ) => {
            v.merge(as_warnings(state::verify_node(src, src_state, limits)));
            v.merge(verify_landing_site(dst, backup, limits));
        }
        (
            Action::CopyTree {
                dst,
                dst_root,
                summary,
                ..
            },
            Direction::Undo,
        ) => {
            v.merge(state::verify_node(dst, dst_root, limits));
            v.merge(state::verify_tree(dst, summary, limits));
        }
        (
            Action::CopyTree {
                src,
                src_root,
                summary,
                dst,
                ..
            },
            Direction::Redo,
        ) => {
            v.merge(as_warnings(state::verify_node(src, src_root, limits)));
            if matches!(fs::symlink_metadata(src), Ok(m) if m.is_dir()) {
                v.merge(as_warnings(state::verify_tree(src, summary, limits)));
            }
            v.merge(state::verify_absent(dst));
        }
        (Action::TrashPut { origin, trash }, Direction::Undo) => {
            v.merge(state::verify_absent(origin));
            v.merge(state::verify_node(&trash.file, &trash.state, limits));
            if let Some(tree) = &trash.tree {
                v.merge(state::verify_tree(&trash.file, tree, limits));
            }
        }
        (Action::TrashPut { origin, trash }, Direction::Redo) => {
            v.merge(state::verify_node(origin, &trash.state, limits));
            if let Some(tree) = &trash.tree
                && matches!(fs::symlink_metadata(origin), Ok(m) if m.is_dir())
            {
                v.merge(state::verify_tree(origin, tree, limits));
            }
        }
        (Action::CreateDir { path, .. }, Direction::Undo) => match fs::symlink_metadata(path) {
            Ok(m) if m.is_dir() => match fs::read_dir(path) {
                Ok(rd) => {
                    let foreign = rd.flatten().any(|e| !removed_siblings.contains(&e.path()));
                    if foreign {
                        v.block(Conflict::new(
                            path.clone(),
                            ConflictKind::Modified,
                            "empty directory",
                            "not empty (contains other files)",
                        ));
                    }
                }
                Err(e) => v.block(Conflict::new(
                    path.clone(),
                    ConflictKind::Missing,
                    "readable directory",
                    format!("unreadable: {e}"),
                )),
            },
            Ok(_) => v.block(Conflict::new(
                path.clone(),
                ConflictKind::TypeChanged,
                "directory",
                "non-directory",
            )),
            Err(_) => v.block(Conflict::new(
                path.clone(),
                ConflictKind::Missing,
                "directory",
                "absent",
            )),
        },
        (Action::CreateDir { path, .. }, Direction::Redo) => {
            v.merge(state::verify_absent(path));
        }
        (Action::SetMode { path, old, new }, dir) => {
            let want = match dir {
                Direction::Undo => *new,
                Direction::Redo => *old,
            };
            match state::capture(path, limits) {
                Ok(FileState::File { mode, .. })
                | Ok(FileState::Dir { mode, .. })
                | Ok(FileState::Other { mode, .. }) => {
                    if mode != want {
                        v.block(Conflict::new(
                            path.clone(),
                            ConflictKind::Modified,
                            format!("mode {want:04o}"),
                            format!("mode {mode:04o}"),
                        ));
                    }
                }
                Ok(FileState::Symlink { .. }) => v.block(Conflict::new(
                    path.clone(),
                    ConflictKind::TypeChanged,
                    "non-symlink",
                    "symlink",
                )),
                Ok(FileState::Absent) | Err(_) => v.block(Conflict::new(
                    path.clone(),
                    ConflictKind::Missing,
                    "existing node",
                    "absent",
                )),
            }
        }
        (
            Action::SetOwner {
                path,
                old_uid,
                old_gid,
                new_uid,
                new_gid,
                ..
            },
            dir,
        ) => {
            let (want_uid, want_gid) = match dir {
                Direction::Undo => (*new_uid, *new_gid),
                Direction::Redo => (*old_uid, *old_gid),
            };
            match state::capture(path, limits) {
                Ok(FileState::Absent) | Err(_) => v.block(Conflict::new(
                    path.clone(),
                    ConflictKind::Missing,
                    "existing node",
                    "absent",
                )),
                Ok(FileState::File { uid, gid, .. })
                | Ok(FileState::Dir { uid, gid, .. })
                | Ok(FileState::Symlink { uid, gid, .. })
                | Ok(FileState::Other { uid, gid, .. }) => {
                    if uid != want_uid || gid != want_gid {
                        v.block(Conflict::new(
                            path.clone(),
                            ConflictKind::Modified,
                            format!("owner {want_uid}:{want_gid}"),
                            format!("owner {uid}:{gid}"),
                        ));
                    }
                }
            }
        }
        (
            Action::Symlink {
                link, post, backup, ..
            },
            Direction::Undo,
        ) => {
            v.merge(state::verify_node(link, post, limits));
            verify_backup(backup, &mut v, limits);
        }
        (Action::Symlink { link, backup, .. }, Direction::Redo) => {
            v.merge(verify_landing_site(link, backup, limits));
        }
        (
            Action::Hardlink {
                link, post, backup, ..
            },
            Direction::Undo,
        ) => {
            verify_hardlink_inode(link, post, &mut v, limits);
            verify_backup(backup, &mut v, limits);
        }
        (
            Action::Hardlink {
                src,
                link,
                post,
                backup,
                ..
            },
            Direction::Redo,
        ) => {
            verify_hardlink_inode(src, post, &mut v, limits);
            v.merge(verify_landing_site(link, backup, limits));
        }
    }
    v
}

fn verify_hardlink_inode(path: &Path, post: &FileState, v: &mut Verification, limits: &Limits) {
    let (want_dev, want_ino) = match post {
        FileState::File { dev, ino, .. } => (*dev, *ino),
        _ => {
            v.merge(state::verify_node(path, post, limits));
            return;
        }
    };
    match fs::symlink_metadata(path) {
        Ok(m) if m.is_file() => {
            if m.dev() != want_dev || m.ino() != want_ino {
                v.block(Conflict::new(
                    path.to_path_buf(),
                    ConflictKind::Modified,
                    format!("inode {want_dev}:{want_ino}"),
                    format!("inode {}:{}", m.dev(), m.ino()),
                ));
            }
        }
        Ok(_) => v.block(Conflict::new(
            path.to_path_buf(),
            ConflictKind::TypeChanged,
            "file",
            "non-file",
        )),
        Err(_) => v.block(Conflict::new(
            path.to_path_buf(),
            ConflictKind::Missing,
            "file",
            "absent",
        )),
    }
}

fn vacate_for_redo(path: &Path, backup: &mut Option<TrashRef>, ctx: &mut OpCtx<'_>) -> Result<()> {
    if fs::symlink_metadata(path).is_err() {
        return Ok(());
    }
    let parked = ctx.trash.put(path, ctx.limits)?;
    ctx.changes.push(Change::trashed(path, &parked.file));
    if backup.is_some() {
        *backup = Some(parked);
    } else if ctx.force {
        ctx.evictions.push(parked);
    } else {
        return Err(UndoError::msg(format!(
            "{}: unexpectedly occupied (raced?)",
            path.display()
        )));
    }
    Ok(())
}

fn vacate_for_undo(path: &Path, ctx: &mut OpCtx<'_>) -> Result<()> {
    if fs::symlink_metadata(path).is_err() {
        return Ok(());
    }
    if !ctx.force {
        return Err(UndoError::msg(format!(
            "{}: unexpectedly occupied (raced?)",
            path.display()
        )));
    }
    let parked = ctx.trash.put(path, ctx.limits)?;
    ctx.changes.push(Change::trashed(path, &parked.file));
    ctx.evictions.push(parked);
    Ok(())
}

fn restore_backup(backup: &Option<TrashRef>, ctx: &mut OpCtx<'_>) {
    if let Some(b) = backup {
        match ctx.trash.restore(b) {
            Ok(warnings) => {
                ctx.warnings.extend(warnings);
                ctx.changes.push(Change::restored(&b.file, &b.origin));
            }
            Err(e) => {
                ctx.warnings.push(format!(
                    "could not restore backup {} to {}: {e}",
                    b.file.display(),
                    b.origin.display()
                ));
            }
        }
    }
}

pub fn apply_action(action: &mut Action, dir: Direction, ctx: &mut OpCtx<'_>) -> Result<()> {
    match dir {
        Direction::Undo => apply_undo(action, ctx),
        Direction::Redo => apply_redo(action, ctx),
    }
}

fn apply_undo(action: &mut Action, ctx: &mut OpCtx<'_>) -> Result<()> {
    match action {
        Action::Move {
            src, dst, backup, ..
        }
        | Action::MoveXdev {
            src, dst, backup, ..
        } => {
            if fs::symlink_metadata(&*src).is_ok() {
                vacate_for_undo(src, ctx)?;
            }
            ensure_parent(src, &mut ctx.warnings)?;
            safe_move(dst, src)?;
            ctx.changes.push(Change::moved(dst, src));
            restore_backup(backup, ctx);
        }
        Action::Copy { dst, backup, .. } => {
            let parked = ctx.trash.put(dst, ctx.limits)?;
            ctx.changes.push(Change::trashed(dst, &parked.file));
            ctx.artifacts.push(parked);
            restore_backup(backup, ctx);
        }
        Action::CopyTree { dst, .. } => {
            let parked = ctx.trash.put(dst, ctx.limits)?;
            ctx.changes.push(Change::trashed(dst, &parked.file));
            ctx.artifacts.push(parked);
        }
        Action::TrashPut { origin, trash } => {
            if fs::symlink_metadata(&*origin).is_ok() {
                vacate_for_undo(origin, ctx)?;
            }
            let warnings = ctx.trash.restore(trash)?;
            ctx.warnings.extend(warnings);
            ctx.changes.push(Change::restored(&trash.file, origin));
        }
        Action::CreateDir { path, .. } => {
            let occupied = fs::read_dir(&*path)
                .map(|mut d| d.next().is_some())
                .unwrap_or(false);
            if occupied {
                if !ctx.force {
                    return Err(UndoError::msg(format!(
                        "{}: not empty — refusing to remove",
                        path.display()
                    )));
                }
                let parked = ctx.trash.put(path, ctx.limits)?;
                ctx.changes.push(Change::trashed(path, &parked.file));
                ctx.evictions.push(parked);
            } else {
                fs::remove_dir(&*path).ctx(format!("removing {}", path.display()))?;
                ctx.changes.push(Change::simple("rmdir", path));
            }
        }
        Action::SetMode { path, old, .. } => {
            set_mode(path, *old)?;
            ctx.changes
                .push(Change::simple(&format!("mode {:04o}", old), path));
        }
        Action::SetOwner {
            path,
            old_uid,
            old_gid,
            deref,
            ..
        } => {
            apply_chown(path, *old_uid, *old_gid, *deref)?;
            ctx.changes
                .push(Change::simple(&format!("owner {old_uid}:{old_gid}"), path));
        }
        Action::Symlink { link, backup, .. } | Action::Hardlink { link, backup, .. } => {
            let parked = ctx.trash.put(link, ctx.limits)?;
            ctx.changes.push(Change::trashed(link, &parked.file));
            ctx.artifacts.push(parked);
            restore_backup(backup, ctx);
        }
    }
    Ok(())
}

fn apply_redo(action: &mut Action, ctx: &mut OpCtx<'_>) -> Result<()> {
    match action {
        Action::Move {
            src, dst, backup, ..
        }
        | Action::MoveXdev {
            src, dst, backup, ..
        } => {
            vacate_for_redo(dst, backup, ctx)?;
            ensure_parent(dst, &mut ctx.warnings)?;
            safe_move(src, dst)?;
            ctx.changes.push(Change::moved(src, dst));
        }
        Action::Copy {
            src,
            dst,
            src_state,
            post,
            preserve,
            backup,
        } => {
            vacate_for_redo(dst, backup, ctx)?;
            ensure_parent(dst, &mut ctx.warnings)?;
            copy_file_for_cp(src, dst, *preserve)?;
            ctx.changes.push(Change::copied(src, dst));
            *src_state = state::capture(src, ctx.limits)?;
            *post = state::capture(dst, ctx.limits)?;
        }
        Action::CopyTree {
            src,
            dst,
            src_root,
            dst_root,
            summary,
            ..
        } => {
            if fs::symlink_metadata(&*dst).is_ok() {
                vacate_for_undo(dst, ctx)?;
            }
            ensure_parent(dst, &mut ctx.warnings)?;
            verified_copy_tree(src, dst)?;
            ctx.changes.push(Change::copied(src, dst));
            *src_root = state::capture(src, ctx.limits)?;
            *dst_root = state::capture(dst, ctx.limits)?;
            *summary = state::summarize_tree(dst, ctx.limits)?;
        }
        Action::TrashPut { origin, trash } => {
            let parked = ctx.trash.put(origin, ctx.limits)?;
            ctx.changes.push(Change::trashed(origin, &parked.file));
            *trash = parked;
        }
        Action::CreateDir { path, mode, post } => {
            if fs::symlink_metadata(&*path).is_ok() {
                vacate_for_undo(path, ctx)?;
            }
            ensure_parent(path, &mut ctx.warnings)?;
            fs::create_dir(&*path).ctx(format!("creating {}", path.display()))?;
            set_mode(path, *mode)?;
            *post = state::capture(path, ctx.limits)?;
            ctx.changes.push(Change::simple("mkdir", path));
        }
        Action::SetMode { path, new, .. } => {
            set_mode(path, *new)?;
            ctx.changes
                .push(Change::simple(&format!("mode {:04o}", new), path));
        }
        Action::SetOwner {
            path,
            new_uid,
            new_gid,
            deref,
            ..
        } => {
            apply_chown(path, *new_uid, *new_gid, *deref)?;
            ctx.changes
                .push(Change::simple(&format!("owner {new_uid}:{new_gid}"), path));
        }
        Action::Symlink {
            target,
            link,
            post,
            backup,
        } => {
            vacate_for_redo(link, backup, ctx)?;
            ensure_parent(link, &mut ctx.warnings)?;
            std::os::unix::fs::symlink(&*target, &*link)
                .ctx(format!("creating symlink {}", link.display()))?;
            *post = state::capture(link, ctx.limits)?;
            ctx.changes.push(Change::simple("symlink", link));
        }
        Action::Hardlink {
            src,
            link,
            post,
            backup,
        } => {
            vacate_for_redo(link, backup, ctx)?;
            ensure_parent(link, &mut ctx.warnings)?;
            fs::hard_link(&*src, &*link).ctx(format!("creating hard link {}", link.display()))?;
            *post = state::capture(link, ctx.limits)?;
            ctx.changes.push(Change::simple("hardlink", link));
        }
    }
    Ok(())
}

pub fn apply_chown(path: &Path, uid: u32, gid: u32, deref: bool) -> Result<()> {
    let res = if deref {
        std::os::unix::fs::chown(path, Some(uid), Some(gid))
    } else {
        std::os::unix::fs::lchown(path, Some(uid), Some(gid))
    };
    res.map_err(|e| {
        if e.kind() == io::ErrorKind::PermissionDenied {
            UndoError::msg(format!(
                "changing ownership of {} to {uid}:{gid} requires the same privileges as the original chown",
                path.display()
            ))
        } else {
            UndoError::io(format!("changing ownership of {}", path.display()), e)
        }
    })
}

pub fn absolutize_action(action: &mut Action) -> Result<()> {
    fn abs(p: &mut PathBuf) -> Result<()> {
        *p = crate::paths::absolutize(p)?;
        Ok(())
    }
    fn abs_backup(backup: &mut Option<TrashRef>) -> Result<()> {
        if let Some(b) = backup {
            abs(&mut b.origin)?;
        }
        Ok(())
    }
    match action {
        Action::Move {
            src, dst, backup, ..
        }
        | Action::MoveXdev {
            src, dst, backup, ..
        } => {
            abs(src)?;
            abs(dst)?;
            abs_backup(backup)?;
        }
        Action::Copy {
            src, dst, backup, ..
        } => {
            abs(src)?;
            abs(dst)?;
            abs_backup(backup)?;
        }
        Action::CopyTree { src, dst, .. } => {
            abs(src)?;
            abs(dst)?;
        }
        Action::TrashPut { origin, trash } => {
            abs(origin)?;
            abs(&mut trash.origin)?;
        }
        Action::CreateDir { path, .. }
        | Action::SetMode { path, .. }
        | Action::SetOwner { path, .. } => abs(path)?,
        Action::Symlink { link, backup, .. } => {
            abs(link)?;
            abs_backup(backup)?;
        }
        Action::Hardlink {
            src, link, backup, ..
        } => {
            abs(src)?;
            abs(link)?;
            abs_backup(backup)?;
        }
    }
    Ok(())
}

pub fn describe(action: &Action) -> String {
    match action {
        Action::Move { src, dst, .. } => format!("moved {} -> {}", src.display(), dst.display()),
        Action::MoveXdev { src, dst, .. } => {
            format!(
                "moved {} -> {} (cross-device)",
                src.display(),
                dst.display()
            )
        }
        Action::Copy { src, dst, .. } => format!("copied {} -> {}", src.display(), dst.display()),
        Action::CopyTree {
            src, dst, summary, ..
        } => format!(
            "copied tree {} -> {} ({} files, {} dirs)",
            src.display(),
            dst.display(),
            summary.files,
            summary.dirs
        ),
        Action::TrashPut { origin, trash } => {
            format!("trashed {} -> {}", origin.display(), trash.file.display())
        }
        Action::CreateDir { path, mode, .. } => {
            format!("created directory {} (mode {:04o})", path.display(), mode)
        }
        Action::SetMode { path, old, new } => {
            format!("mode {:04o} -> {:04o} on {}", old, new, path.display())
        }
        Action::SetOwner {
            path,
            old_uid,
            old_gid,
            new_uid,
            new_gid,
            ..
        } => format!(
            "owner {old_uid}:{old_gid} -> {new_uid}:{new_gid} on {}",
            path.display()
        ),
        Action::Symlink { target, link, .. } => {
            format!("symlinked {} -> {}", link.display(), target.display())
        }
        Action::Hardlink { src, link, .. } => {
            format!("hard-linked {} => {}", link.display(), src.display())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};

    static N: AtomicU32 = AtomicU32::new(0);

    struct TempDir(PathBuf);

    impl TempDir {
        fn new() -> TempDir {
            let p = std::env::temp_dir().join(format!(
                "undo-ops-test-{}-{}",
                std::process::id(),
                N.fetch_add(1, Ordering::SeqCst)
            ));
            fs::create_dir_all(&p).unwrap();
            TempDir(p)
        }

        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn verified_copy_preserves_content_mode_mtime() {
        let t = TempDir::new();
        let src = t.path().join("src");
        let dst = t.path().join("dst");
        fs::write(&src, b"payload").unwrap();
        fs::set_permissions(&src, fs::Permissions::from_mode(0o640)).unwrap();
        filetime::set_file_mtime(&src, filetime::FileTime::from_unix_time(1_000_000, 0)).unwrap();

        verified_copy_file(&src, &dst).unwrap();

        assert_eq!(fs::read(&dst).unwrap(), b"payload");
        let meta = fs::metadata(&dst).unwrap();
        assert_eq!(meta.mode() & 0o777, 0o640);
        assert_eq!(meta.mtime(), 1_000_000);
    }

    #[test]
    fn verified_copy_refuses_existing_destination() {
        let t = TempDir::new();
        let src = t.path().join("src");
        let dst = t.path().join("dst");
        fs::write(&src, b"a").unwrap();
        fs::write(&dst, b"b").unwrap();
        assert!(verified_copy_file(&src, &dst).is_err());
        assert_eq!(fs::read(&dst).unwrap(), b"b");
    }

    #[test]
    fn copy_tree_preserves_symlinks() {
        let t = TempDir::new();
        let src = t.path().join("tree");
        fs::create_dir_all(src.join("sub")).unwrap();
        fs::write(src.join("sub/f"), b"x").unwrap();
        std::os::unix::fs::symlink("sub/f", src.join("l")).unwrap();

        let dst = t.path().join("copy");
        verified_copy_tree(&src, &dst).unwrap();

        let link = fs::symlink_metadata(dst.join("l")).unwrap();
        assert!(link.file_type().is_symlink());
        assert_eq!(
            fs::read_link(dst.join("l")).unwrap(),
            PathBuf::from("sub/f")
        );
        assert_eq!(fs::read(dst.join("sub/f")).unwrap(), b"x");
    }

    #[test]
    fn safe_move_same_device_is_rename() {
        let t = TempDir::new();
        let src = t.path().join("a");
        let dst = t.path().join("b");
        fs::write(&src, b"1").unwrap();
        assert_eq!(safe_move(&src, &dst).unwrap(), MoveKind::Rename);
        assert!(!src.exists());
        assert_eq!(fs::read(&dst).unwrap(), b"1");
    }

    #[test]
    fn change_display_is_readable() {
        let c = Change::moved(Path::new("/a"), Path::new("/b"));
        assert_eq!(c.to_string(), "move /a -> /b");
        let c = Change::simple("mkdir", Path::new("/d"));
        assert_eq!(c.to_string(), "mkdir /d");
    }

    #[test]
    fn umask_is_sane() {
        let m = process_umask();
        assert!(m <= 0o777);
        assert_eq!(m, process_umask());
    }
}
