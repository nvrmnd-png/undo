mod common;

use common::TestEnv;
use predicates::prelude::*;

#[test]
fn rmdir_roundtrip_preserves_mode() {
    let t = TestEnv::new();
    t.mkdir("d");
    t.set_mode("d", 0o750);

    t.exec(&["rmdir", "d"]).assert().success();
    assert!(!t.exists("d"));
    assert_eq!(t.trash_entries(), vec!["d"]);

    t.undo().assert().success();
    assert!(t.is_dir("d"));
    assert_eq!(t.mode("d"), 0o750);
}

#[test]
fn rmdir_refuses_non_empty() {
    let t = TestEnv::new();
    t.write("d/f", "x");
    t.exec(&["rmdir", "d"])
        .assert()
        .code(1)
        .stderr(predicate::str::contains("Directory not empty"));
    assert!(t.exists("d/f"));
}

#[test]
fn rmdir_ignore_fail_on_non_empty() {
    let t = TestEnv::new();
    t.write("d/f", "x");
    t.exec(&["rmdir", "--ignore-fail-on-non-empty", "d"])
        .assert()
        .success();
    assert!(t.exists("d/f"));
}

#[test]
fn rmdir_parents_removes_chain_and_undo_restores_it() {
    let t = TestEnv::new();
    t.mkdir("a/b/c");

    t.exec(&["rmdir", "-p", "a/b/c"]).assert().success();
    assert!(!t.exists("a"));

    t.undo().assert().success();
    assert!(t.is_dir("a/b/c"));
}

#[test]
fn rmdir_parents_stops_at_non_empty_ancestor() {
    let t = TestEnv::new();
    t.mkdir("a/b/c");
    t.write("a/keep.txt", "x");

    t.exec(&["rmdir", "-p", "a/b/c"]).assert().code(1);
    assert!(!t.exists("a/b"));
    assert!(t.exists("a/keep.txt"));
}

#[test]
fn rmdir_rejects_symlink_to_dir() {
    let t = TestEnv::new();
    t.mkdir("real");
    t.symlink("real", "link");
    t.exec(&["rmdir", "link"])
        .assert()
        .code(1)
        .stderr(predicate::str::contains("Not a directory"));
    assert!(t.is_dir("real"));
    assert!(t.is_symlink("link"));
}
