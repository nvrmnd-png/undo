mod common;

use common::TestEnv;
use predicates::prelude::*;

#[test]
fn rename_batch_roundtrip() {
    let t = TestEnv::new();
    t.write("a.txt", "a");
    t.write("b.txt", "b");
    t.write("c.md", "c");

    t.exec(&["rename", "s/\\.txt$/.md/", "a.txt", "b.txt"])
        .assert()
        .success();
    assert!(t.exists("a.md") && t.exists("b.md"));
    assert!(!t.exists("a.txt") && !t.exists("b.txt"));

    t.undo().assert().success();
    assert!(t.exists("a.txt") && t.exists("b.txt"));
    assert!(!t.exists("a.md") && !t.exists("b.md"));

    t.undo().arg("redo").assert().success();
    assert!(t.exists("a.md") && t.exists("b.md"));
}

#[test]
fn rename_regex_groups() {
    let t = TestEnv::new();
    t.write("01-intro.txt", "x");
    t.exec(&["rename", r"s/^(\d+)-(\w+)/$2-$1/", "01-intro.txt"])
        .assert()
        .success();
    assert!(t.exists("intro-01.txt"));

    t.undo().assert().success();
    assert!(t.exists("01-intro.txt"));
}

#[test]
fn rename_dry_run_changes_nothing() {
    let t = TestEnv::new();
    t.write("a.txt", "x");
    t.exec(&["rename", "-n", "s/txt/md/", "a.txt"])
        .assert()
        .success()
        .stdout(predicate::str::contains("rename(a.txt, a.md)"));
    assert!(t.exists("a.txt"));
    t.undo()
        .assert()
        .failure()
        .stderr(predicate::str::contains("nothing to undo"));
}

#[test]
fn rename_collision_aborts_whole_batch() {
    let t = TestEnv::new();
    t.write("foo1.txt", "1");
    t.write("foo2.txt", "2");

    t.exec(&["rename", r"s/\d//", "foo1.txt", "foo2.txt"])
        .assert()
        .code(1)
        .stderr(predicate::str::contains("nothing renamed"));
    assert!(t.exists("foo1.txt") && t.exists("foo2.txt"));
    t.undo()
        .assert()
        .failure()
        .stderr(predicate::str::contains("nothing to undo"));
}

#[test]
fn rename_existing_target_needs_force() {
    let t = TestEnv::new();
    t.write("a.txt", "new");
    t.write("a.md", "old");

    t.exec(&["rename", "s/txt/md/", "a.txt"])
        .assert()
        .code(1)
        .stderr(predicate::str::contains("already exists"));
    assert_eq!(t.read("a.md"), "old");

    t.exec(&["rename", "-f", "s/txt/md/", "a.txt"])
        .assert()
        .success();
    assert_eq!(t.read("a.md"), "new");

    t.undo().assert().success();
    assert_eq!(t.read("a.txt"), "new");
    assert_eq!(t.read("a.md"), "old");
}

#[test]
fn rename_unsupported_perl_features_fall_back() {
    let t = TestEnv::new();
    t.write("a", "x");
    for expr in ["y/a/b/", "s/a/b/e", r"s/a/\Ub/", "s/a/b"] {
        t.exec(&["rename", expr, "a"]).assert().code(125);
        assert!(t.exists("a"));
    }
}

#[test]
fn rename_missing_file_aborts_batch() {
    let t = TestEnv::new();
    t.write("real.txt", "x");
    t.exec(&["rename", "s/txt/md/", "ghost.txt", "real.txt"])
        .assert()
        .code(1);
    assert!(
        t.exists("real.txt"),
        "all-or-nothing: nothing may be renamed"
    );
    assert!(!t.exists("real.md"));
}

#[test]
fn rename_case_insensitive_and_global_flags() {
    let t = TestEnv::new();
    t.write("AAA.txt", "x");
    t.exec(&["rename", "s/a/b/gi", "AAA.txt"])
        .assert()
        .success();
    assert!(t.exists("bbb.txt"));
}
