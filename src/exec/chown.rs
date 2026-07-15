use std::fs;
use std::os::unix::fs::MetadataExt;
use std::path::PathBuf;

use clap::Parser;

use super::adapter::{Session, fallback_parse};
use crate::error::{Result, UndoError};
use crate::journal::model::Action;
use crate::ops;

#[derive(Debug, Parser)]
#[command(name = "chown", disable_help_flag = true, disable_version_flag = true)]
struct ChownArgs {
    #[arg(short = 'h', long = "no-dereference")]
    no_deref: bool,
    #[arg(short = 'R', long = "recursive")]
    recursive: bool,
    #[arg(short = 'v', long = "verbose")]
    verbose: bool,
    #[arg(value_name = "OWNER[:GROUP]")]
    spec: String,
    #[arg(value_name = "FILE", num_args = 1..)]
    files: Vec<PathBuf>,
}

#[derive(Debug, Clone, Copy)]
struct OwnerSpec {
    uid: Option<u32>,
    gid: Option<u32>,
}

fn lookup_id(db: &str, name: &str) -> Option<u32> {
    let content = fs::read_to_string(db).ok()?;
    for line in content.lines() {
        let mut fields = line.split(':');
        let entry_name = fields.next()?;
        if entry_name == name {
            return fields.nth(1)?.parse().ok();
        }
    }
    None
}

fn parse_spec(spec: &str) -> Result<OwnerSpec> {
    if spec.contains('.') && !spec.contains(':') {
        return Err(UndoError::fallback(format!(
            "chown: legacy '.' separator in '{spec}' is not supported"
        )));
    }
    let (owner_part, group_part) = match spec.split_once(':') {
        Some((o, g)) => (o, Some(g)),
        None => (spec, None),
    };
    let uid = if owner_part.is_empty() {
        None
    } else if let Ok(n) = owner_part.parse::<u32>() {
        Some(n)
    } else {
        Some(lookup_id("/etc/passwd", owner_part).ok_or_else(|| {
            UndoError::fallback(format!(
                "chown: user '{owner_part}' not found in /etc/passwd (NSS lookups are not supported)"
            ))
        })?)
    };
    let gid = match group_part {
        None | Some("") => None,
        Some(g) => {
            if let Ok(n) = g.parse::<u32>() {
                Some(n)
            } else {
                Some(lookup_id("/etc/group", g).ok_or_else(|| {
                    UndoError::fallback(format!(
                        "chown: group '{g}' not found in /etc/group (NSS lookups are not supported)"
                    ))
                })?)
            }
        }
    };
    if uid.is_none() && gid.is_none() {
        return Err(UndoError::fallback(format!("chown: invalid spec '{spec}'")));
    }
    Ok(OwnerSpec { uid, gid })
}

pub fn run(argv: &[String]) -> Result<u8> {
    let args = ChownArgs::try_parse_from(argv).map_err(|e| fallback_parse("chown", e))?;
    let spec = parse_spec(&args.spec)?;

    let mut session = Session::new("chown", argv)?;
    for f in &args.files {
        session.protect(f)?;
    }

    let mut targets: Vec<PathBuf> = Vec::new();
    for operand in &args.files {
        let meta = match fs::symlink_metadata(operand) {
            Ok(m) => m,
            Err(_) => {
                session.err(format!(
                    "cannot access '{}': No such file or directory",
                    operand.display()
                ));
                continue;
            }
        };
        let path = if meta.file_type().is_symlink() && !args.no_deref {
            match fs::canonicalize(operand) {
                Ok(resolved) => resolved,
                Err(_) => {
                    session.err(format!(
                        "cannot dereference '{}': dangling symlink",
                        operand.display()
                    ));
                    continue;
                }
            }
        } else {
            operand.clone()
        };
        targets.push(path.clone());
        if args.recursive
            && fs::symlink_metadata(&path)
                .map(|m| m.is_dir())
                .unwrap_or(false)
        {
            collect_recursive(&path, &mut targets, session.limits.tree_cap)?;
        }
    }

    for path in targets {
        let meta = match fs::symlink_metadata(&path) {
            Ok(m) => m,
            Err(e) => {
                session.err(format!("cannot access '{}': {e}", path.display()));
                continue;
            }
        };
        let (old_uid, old_gid) = (meta.uid(), meta.gid());
        let new_uid = spec.uid.unwrap_or(old_uid);
        let new_gid = spec.gid.unwrap_or(old_gid);
        if new_uid == old_uid && new_gid == old_gid {
            if args.verbose {
                println!(
                    "ownership of '{}' retained as {old_uid}:{old_gid}",
                    path.display()
                );
            }
            continue;
        }
        session.begin()?;
        match ops::apply_chown(&path, new_uid, new_gid, false) {
            Ok(()) => {
                session.record(Action::SetOwner {
                    path: path.clone(),
                    old_uid,
                    old_gid,
                    new_uid,
                    new_gid,
                    deref: false,
                })?;
                if args.verbose {
                    println!(
                        "changed ownership of '{}' from {old_uid}:{old_gid} to {new_uid}:{new_gid}",
                        path.display()
                    );
                }
            }
            Err(e) => {
                session.err(format!("{e}"));
            }
        }
    }

    session.finish()
}

fn collect_recursive(root: &PathBuf, targets: &mut Vec<PathBuf>, cap: u64) -> Result<()> {
    let entries =
        fs::read_dir(root).map_err(|e| UndoError::io(format!("reading {}", root.display()), e))?;
    for entry in entries {
        let entry = entry.map_err(|e| UndoError::io(format!("reading {}", root.display()), e))?;
        let path = entry.path();
        let meta = entry
            .metadata()
            .map_err(|e| UndoError::io(format!("inspecting {}", path.display()), e))?;
        targets.push(path.clone());
        if targets.len() as u64 > cap {
            return Err(UndoError::fallback(format!(
                "chown: recursion over {cap} nodes is not journaled"
            )));
        }
        if meta.is_dir() {
            collect_recursive(&path, targets, cap)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numeric_specs() {
        let s = parse_spec("1000").unwrap();
        assert_eq!((s.uid, s.gid), (Some(1000), None));
        let s = parse_spec("1000:100").unwrap();
        assert_eq!((s.uid, s.gid), (Some(1000), Some(100)));
        let s = parse_spec(":100").unwrap();
        assert_eq!((s.uid, s.gid), (None, Some(100)));
    }

    #[test]
    fn root_resolves_from_passwd() {
        let s = parse_spec("root").unwrap();
        assert_eq!(s.uid, Some(0));
        let s = parse_spec(":root").unwrap();
        assert_eq!(s.gid, Some(0));
    }

    #[test]
    fn rejects_odd_specs() {
        assert!(parse_spec("owner.group").is_err());
        assert!(parse_spec(":").is_err());
        assert!(parse_spec("definitely-not-a-user-xyz").is_err());
    }
}
