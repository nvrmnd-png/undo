mod common;

use std::fs;

use common::TestEnv;
use predicates::prelude::*;

fn own_groups() -> Vec<u32> {
    let status = fs::read_to_string("/proc/self/status").unwrap_or_default();
    status
        .lines()
        .find(|l| l.starts_with("Groups:"))
        .map(|l| {
            l.trim_start_matches("Groups:")
                .split_whitespace()
                .filter_map(|g| g.parse().ok())
                .collect()
        })
        .unwrap_or_default()
}

#[test]
fn chown_noop_journals_nothing() {
    let t = TestEnv::new();
    t.write("f", "x");
    use std::os::unix::fs::MetadataExt;
    let meta = fs::metadata(t.path("f")).unwrap();
    let spec = format!("{}:{}", meta.uid(), meta.gid());

    t.exec(&["chown", &spec, "f"]).assert().success();
    t.undo()
        .assert()
        .failure()
        .stderr(predicate::str::contains("nothing to undo"));
}

#[test]
fn chown_group_change_roundtrip_if_possible() {
    use std::os::unix::fs::MetadataExt;
    let t = TestEnv::new();
    t.write("f", "x");
    let old_gid = fs::metadata(t.path("f")).unwrap().gid();
    let Some(other) = own_groups().into_iter().find(|g| *g != old_gid) else {
        eprintln!("skipping: user has no second group");
        return;
    };

    t.exec(&["chown", &format!(":{other}"), "f"])
        .assert()
        .success();
    assert_eq!(fs::metadata(t.path("f")).unwrap().gid(), other);

    t.undo().assert().success();
    assert_eq!(fs::metadata(t.path("f")).unwrap().gid(), old_gid);

    t.undo().arg("redo").assert().success();
    assert_eq!(fs::metadata(t.path("f")).unwrap().gid(), other);
}

#[test]
fn chown_to_root_fails_without_privileges() {
    if undo::paths::euid() == 0 {
        eprintln!("skipping: running as root");
        return;
    }
    let t = TestEnv::new();
    t.write("f", "x");
    t.exec(&["chown", "root", "f"])
        .assert()
        .code(1)
        .stderr(predicate::str::contains("requires the same privileges"));
    t.undo()
        .assert()
        .failure()
        .stderr(predicate::str::contains("nothing to undo"));
}

#[test]
fn chown_unknown_name_falls_back() {
    let t = TestEnv::new();
    t.write("f", "x");
    t.exec(&["chown", "definitely-not-a-user-xyz", "f"])
        .assert()
        .code(125);
    assert!(t.exists("f"));
}

#[test]
fn chown_legacy_dot_syntax_falls_back() {
    let t = TestEnv::new();
    t.write("f", "x");
    t.exec(&["chown", "user.group", "f"]).assert().code(125);
}

#[test]
fn chown_recursive_collects_tree_noop() {
    use std::os::unix::fs::MetadataExt;
    let t = TestEnv::new();
    t.write("tree/a", "x");
    t.write("tree/sub/b", "y");
    let meta = fs::metadata(t.path("tree/a")).unwrap();
    let spec = format!("{}:{}", meta.uid(), meta.gid());

    t.exec(&["chown", "-R", &spec, "tree"]).assert().success();
    t.undo()
        .assert()
        .failure()
        .stderr(predicate::str::contains("nothing to undo"));
}
