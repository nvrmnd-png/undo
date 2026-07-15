mod common;

use common::TestEnv;
use predicates::prelude::*;

#[test]
fn cp_file_roundtrip() {
    let t = TestEnv::new();
    t.write("a.txt", "content");

    t.exec(&["cp", "a.txt", "b.txt"]).assert().success();
    assert_eq!(t.read("b.txt"), "content");

    t.undo().assert().success();
    assert!(!t.exists("b.txt"));
    assert_eq!(t.read("a.txt"), "content", "source must never be touched");
    assert_eq!(
        t.trash_entries().len(),
        1,
        "the copy is parked, not deleted"
    );

    t.undo().arg("redo").assert().success();
    assert_eq!(t.read("b.txt"), "content");
}

#[test]
fn cp_overwrite_restores_old_content_on_undo() {
    let t = TestEnv::new();
    t.write("a.txt", "new");
    t.write("b.txt", "old");

    t.exec(&["cp", "a.txt", "b.txt"]).assert().success();
    assert_eq!(t.read("b.txt"), "new");

    t.undo().assert().success();
    assert_eq!(t.read("b.txt"), "old");
    assert_eq!(t.read("a.txt"), "new");
}

#[test]
fn cp_recursive_fresh_tree_roundtrip() {
    let t = TestEnv::new();
    t.write("src/sub/deep.txt", "deep");
    t.write("src/top.txt", "top");
    t.symlink("top.txt", "src/link");

    t.exec(&["cp", "-r", "src", "copy"]).assert().success();
    assert_eq!(t.read("copy/sub/deep.txt"), "deep");
    assert!(
        t.is_symlink("copy/link"),
        "symlinks inside trees stay symlinks"
    );

    t.undo().assert().success();
    assert!(!t.exists("copy"));
    assert_eq!(t.read("src/sub/deep.txt"), "deep");

    t.undo().arg("redo").assert().success();
    assert_eq!(t.read("copy/top.txt"), "top");
}

#[test]
fn cp_merge_into_existing_dir_keeps_preexisting_content_on_undo() {
    let t = TestEnv::new();
    t.write("src/newfile.txt", "new");
    t.write("src/sub/inner.txt", "inner");
    t.write("dst/existing.txt", "keep me");

    t.exec(&["cp", "-r", "src", "dst"]).assert().success();
    assert_eq!(t.read("dst/src/newfile.txt"), "new");
    assert_eq!(t.read("dst/src/sub/inner.txt"), "inner");

    t.undo().assert().success();
    assert!(!t.exists("dst/src"));
    assert_eq!(t.read("dst/existing.txt"), "keep me");
}

#[test]
fn cp_preserve_keeps_mode() {
    let t = TestEnv::new();
    t.write("a.sh", "#!/bin/sh\n");
    t.set_mode("a.sh", 0o750);

    t.exec(&["cp", "-p", "a.sh", "b.sh"]).assert().success();
    assert_eq!(t.mode("b.sh"), 0o750);

    t.exec(&["cp", "a.sh", "c.sh"]).assert().success();
    assert!(t.mode("c.sh") <= 0o750);
}

#[test]
fn cp_dir_without_r_is_omitted() {
    let t = TestEnv::new();
    t.write("dir/f", "x");
    t.exec(&["cp", "dir", "copy"])
        .assert()
        .code(1)
        .stderr(predicate::str::contains("omitting directory"));
    assert!(!t.exists("copy"));
}

#[test]
fn cp_no_dereference_copies_the_link() {
    let t = TestEnv::new();
    t.write("target", "data");
    t.symlink("target", "link");

    t.exec(&["cp", "-P", "link", "copy"]).assert().success();
    assert!(t.is_symlink("copy"));

    t.undo().assert().success();
    assert!(!t.exists("copy"));
    assert!(t.is_symlink("link"));
}

#[test]
fn cp_dereferences_symlink_operand_by_default() {
    let t = TestEnv::new();
    t.write("target", "data");
    t.symlink("target", "link");

    t.exec(&["cp", "link", "copy"]).assert().success();
    assert!(
        !t.is_symlink("copy"),
        "default cp follows the operand symlink"
    );
    assert_eq!(t.read("copy"), "data");
}

#[test]
fn cp_same_file_is_an_error() {
    let t = TestEnv::new();
    t.write("a", "x");
    t.exec(&["cp", "a", "./a"])
        .assert()
        .code(1)
        .stderr(predicate::str::contains("are the same file"));
}

#[test]
fn cp_n_skips_existing() {
    let t = TestEnv::new();
    t.write("a", "new");
    t.write("b", "old");
    t.exec(&["cp", "-n", "a", "b"]).assert().success();
    assert_eq!(t.read("b"), "old");
    t.undo()
        .assert()
        .failure()
        .stderr(predicate::str::contains("nothing to undo"));
}
