mod common;

use common::TestEnv;
use predicates::prelude::*;

#[test]
fn mv_roundtrip_undo_redo() {
    let t = TestEnv::new();
    t.write("a.txt", "hallo");
    t.mkdir("archive");

    t.exec(&["mv", "a.txt", "archive/"]).assert().success();
    assert!(!t.exists("a.txt"));
    assert_eq!(t.read("archive/a.txt"), "hallo");

    t.undo().assert().success();
    assert_eq!(t.read("a.txt"), "hallo");
    assert!(!t.exists("archive/a.txt"));

    t.undo().arg("redo").assert().success();
    assert!(!t.exists("a.txt"));
    assert_eq!(t.read("archive/a.txt"), "hallo");
}

#[test]
fn mv_overwrite_parks_victim_and_undo_restores_both() {
    let t = TestEnv::new();
    t.write("a.txt", "new");
    t.write("b.txt", "old");

    t.exec(&["mv", "a.txt", "b.txt"]).assert().success();
    assert_eq!(t.read("b.txt"), "new");
    assert_eq!(t.trash_entries().len(), 1, "victim should be in the trash");

    t.undo().assert().success();
    assert_eq!(t.read("a.txt"), "new");
    assert_eq!(t.read("b.txt"), "old");
    assert!(
        t.trash_entries().is_empty(),
        "backup should be restored out of the trash"
    );
}

#[test]
fn mv_no_clobber_skips_and_journals_nothing() {
    let t = TestEnv::new();
    t.write("a.txt", "new");
    t.write("b.txt", "old");

    t.exec(&["mv", "-n", "a.txt", "b.txt"]).assert().success();
    assert_eq!(t.read("a.txt"), "new");
    assert_eq!(t.read("b.txt"), "old");

    t.undo()
        .assert()
        .failure()
        .stderr(predicate::str::contains("nothing to undo"));
}

#[test]
fn mv_target_directory_flag() {
    let t = TestEnv::new();
    t.write("a", "1");
    t.write("b", "2");
    t.mkdir("dest");

    t.exec(&["mv", "-t", "dest", "a", "b"]).assert().success();
    assert_eq!(t.read("dest/a"), "1");
    assert_eq!(t.read("dest/b"), "2");

    t.undo().assert().success();
    assert_eq!(t.read("a"), "1");
    assert_eq!(t.read("b"), "2");
    assert!(!t.exists("dest/a"));
}

#[test]
fn mv_missing_source_fails_like_gnu() {
    let t = TestEnv::new();
    t.mkdir("dest");
    t.exec(&["mv", "ghost.txt", "dest/"])
        .assert()
        .code(1)
        .stderr(predicate::str::contains("cannot stat 'ghost.txt'"));
}

#[test]
fn mv_same_file_is_an_error() {
    let t = TestEnv::new();
    t.write("a.txt", "x");
    t.exec(&["mv", "a.txt", "./a.txt"])
        .assert()
        .code(1)
        .stderr(predicate::str::contains("are the same file"));
    assert_eq!(t.read("a.txt"), "x");
}

#[test]
fn mv_directory_roundtrip() {
    let t = TestEnv::new();
    t.write("dir/inner.txt", "content");
    t.mkdir("dest");

    t.exec(&["mv", "dir", "dest/"]).assert().success();
    assert_eq!(t.read("dest/dir/inner.txt"), "content");

    t.undo().assert().success();
    assert_eq!(t.read("dir/inner.txt"), "content");
    assert!(!t.exists("dest/dir"));
}

#[test]
fn mv_undo_refuses_modified_file_then_force_wins() {
    let t = TestEnv::new();
    t.write("a.txt", "original");
    t.mkdir("archive");
    t.exec(&["mv", "a.txt", "archive/"]).assert().success();

    t.write("archive/a.txt", "tampered");
    t.undo()
        .assert()
        .code(1)
        .stderr(predicate::str::contains("modified"));
    assert_eq!(t.read("archive/a.txt"), "tampered");

    t.undo().arg("--force").assert().success();
    assert_eq!(t.read("a.txt"), "tampered");
}

#[test]
fn mv_verbose_prints_renames() {
    let t = TestEnv::new();
    t.write("a", "1");
    t.exec(&["mv", "-v", "a", "b"])
        .assert()
        .success()
        .stdout(predicate::str::contains("renamed 'a' -> 'b'"));
}

#[test]
fn mv_symlink_operand_moves_the_link() {
    let t = TestEnv::new();
    t.write("target.txt", "data");
    t.symlink("target.txt", "link");
    t.mkdir("dest");

    t.exec(&["mv", "link", "dest/"]).assert().success();
    assert!(t.is_symlink("dest/link"));
    assert_eq!(t.read("target.txt"), "data");

    t.undo().assert().success();
    assert!(t.is_symlink("link"));
    assert!(!t.exists("dest/link"));
}
