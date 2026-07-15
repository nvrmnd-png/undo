mod common;

use common::TestEnv;
use predicates::prelude::*;

#[test]
fn mkdir_roundtrip() {
    let t = TestEnv::new();
    t.exec(&["mkdir", "d"]).assert().success();
    assert!(t.is_dir("d"));

    t.undo().assert().success();
    assert!(!t.exists("d"));

    t.undo().arg("redo").assert().success();
    assert!(t.is_dir("d"));
}

#[test]
fn mkdir_parents_journals_only_created_levels() {
    let t = TestEnv::new();
    t.mkdir("a");

    t.exec(&["mkdir", "-p", "a/b/c"]).assert().success();
    assert!(t.is_dir("a/b/c"));

    t.undo().assert().success();
    assert!(!t.exists("a/b"), "created levels must be removed");
    assert!(
        t.is_dir("a"),
        "the pre-existing level must survive the undo"
    );
}

#[test]
fn mkdir_mode_flag() {
    let t = TestEnv::new();
    t.exec(&["mkdir", "-m", "700", "secret"]).assert().success();
    assert_eq!(t.mode("secret"), 0o700);

    t.exec(&["mkdir", "-m", "u=rwx,go=", "secret2"])
        .assert()
        .success();
    assert_eq!(t.mode("secret2"), 0o700);
}

#[test]
fn mkdir_existing_fails_and_journals_nothing() {
    let t = TestEnv::new();
    t.mkdir("d");
    t.exec(&["mkdir", "d"])
        .assert()
        .code(1)
        .stderr(predicate::str::contains("File exists"));
    t.undo()
        .assert()
        .failure()
        .stderr(predicate::str::contains("nothing to undo"));
}

#[test]
fn mkdir_undo_refuses_nonempty_then_force_evicts_to_trash() {
    let t = TestEnv::new();
    t.exec(&["mkdir", "d"]).assert().success();
    t.write("d/keep.txt", "user data");

    t.undo()
        .assert()
        .code(1)
        .stderr(predicate::str::contains("not empty"));
    assert!(t.exists("d/keep.txt"));

    t.undo().arg("--force").assert().success();
    assert!(!t.exists("d"));
    assert_eq!(t.trash_entries(), vec!["d"]);
}

#[test]
fn mkdir_multiple_operands_partial_failure() {
    let t = TestEnv::new();
    t.mkdir("exists");
    t.exec(&["mkdir", "new1", "exists", "new2"])
        .assert()
        .code(1);
    assert!(t.is_dir("new1"));
    assert!(t.is_dir("new2"));

    t.undo().assert().success();
    assert!(!t.exists("new1"));
    assert!(!t.exists("new2"));
    assert!(t.is_dir("exists"));
}
