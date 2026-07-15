mod common;

use common::TestEnv;
use predicates::prelude::*;

#[test]
fn rm_file_goes_to_trash_and_undo_restores() {
    let t = TestEnv::new();
    t.write("f.txt", "precious");
    t.set_mode("f.txt", 0o640);

    t.exec(&["rm", "f.txt"]).assert().success();
    assert!(!t.exists("f.txt"));
    assert_eq!(t.trash_entries(), vec!["f.txt"]);

    t.undo().assert().success();
    assert_eq!(t.read("f.txt"), "precious");
    assert_eq!(
        t.mode("f.txt"),
        0o640,
        "mode must survive the trash roundtrip"
    );
    assert!(t.trash_entries().is_empty());
}

#[test]
fn rm_recursive_tree_roundtrip_with_symlink() {
    let t = TestEnv::new();
    t.write("tree/sub/deep.txt", "deep");
    t.write("tree/top.txt", "top");
    t.symlink("top.txt", "tree/link");

    t.exec(&["rm", "-r", "tree"]).assert().success();
    assert!(!t.exists("tree"));
    assert_eq!(t.trash_entries(), vec!["tree"]);

    t.undo().assert().success();
    assert_eq!(t.read("tree/sub/deep.txt"), "deep");
    assert!(t.is_symlink("tree/link"));
}

#[test]
fn rm_refuses_directory_without_r() {
    let t = TestEnv::new();
    t.write("dir/f", "x");
    t.exec(&["rm", "dir"])
        .assert()
        .code(1)
        .stderr(predicate::str::contains("Is a directory"));
    assert!(t.exists("dir/f"));
    t.undo()
        .assert()
        .failure()
        .stderr(predicate::str::contains("nothing to undo"));
}

#[test]
fn rm_force_ignores_missing_but_plain_rm_complains() {
    let t = TestEnv::new();
    t.exec(&["rm", "-f", "ghost"]).assert().success();
    t.exec(&["rm", "ghost"])
        .assert()
        .code(1)
        .stderr(predicate::str::contains("No such file or directory"));
}

#[test]
fn rm_d_removes_only_empty_dirs() {
    let t = TestEnv::new();
    t.mkdir("empty");
    t.write("full/f", "x");

    t.exec(&["rm", "-d", "empty"]).assert().success();
    assert!(!t.exists("empty"));

    t.exec(&["rm", "-d", "full"])
        .assert()
        .code(1)
        .stderr(predicate::str::contains("Directory not empty"));
    assert!(t.exists("full/f"));
}

#[test]
fn rm_collisions_get_distinct_trash_names() {
    let t = TestEnv::new();
    for content in ["one", "two", "three"] {
        t.write("f.txt", content);
        t.exec(&["rm", "f.txt"]).assert().success();
    }
    assert_eq!(t.trash_entries(), vec!["f.txt", "f.txt.2", "f.txt.3"]);

    t.undo().assert().success();
    assert_eq!(t.read("f.txt"), "three");
}

#[test]
fn rm_symlink_removes_link_not_target() {
    let t = TestEnv::new();
    t.write("target", "data");
    t.symlink("target", "link");

    t.exec(&["rm", "link"]).assert().success();
    assert!(!t.exists("link"));
    assert_eq!(t.read("target"), "data");

    t.undo().assert().success();
    assert!(t.is_symlink("link"));
}

#[test]
fn rm_redo_trashes_again() {
    let t = TestEnv::new();
    t.write("f", "x");
    t.exec(&["rm", "f"]).assert().success();
    t.undo().assert().success();
    assert_eq!(t.read("f"), "x");

    t.undo().arg("redo").assert().success();
    assert!(!t.exists("f"));
    assert!(!t.trash_entries().is_empty());
}

#[test]
fn rm_interactive_decline_via_stdin() {
    let t = TestEnv::new();
    t.write("f", "x");
    t.exec(&["rm", "-i", "f"])
        .write_stdin("n\n")
        .assert()
        .success();
    assert!(t.exists("f"));

    t.exec(&["rm", "-i", "f"])
        .write_stdin("y\n")
        .assert()
        .success();
    assert!(!t.exists("f"));
}
