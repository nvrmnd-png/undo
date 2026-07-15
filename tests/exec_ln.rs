mod common;

use std::fs;
use std::os::unix::fs::MetadataExt;

use common::TestEnv;
use predicates::prelude::*;

#[test]
fn ln_symlink_roundtrip() {
    let t = TestEnv::new();
    t.write("target.txt", "data");

    t.exec(&["ln", "-s", "target.txt", "link"])
        .assert()
        .success();
    assert!(t.is_symlink("link"));
    assert_eq!(
        fs::read_link(t.path("link")).unwrap().to_str(),
        Some("target.txt")
    );

    t.undo().assert().success();
    assert!(!t.exists("link"));
    assert_eq!(t.read("target.txt"), "data");

    t.undo().arg("redo").assert().success();
    assert!(t.is_symlink("link"));
}

#[test]
fn ln_dangling_symlink_is_fine() {
    let t = TestEnv::new();
    t.exec(&["ln", "-s", "nowhere", "link"]).assert().success();
    assert!(t.is_symlink("link"));
    t.undo().assert().success();
    assert!(!t.exists("link"));
}

#[test]
fn ln_hardlink_roundtrip_same_inode() {
    let t = TestEnv::new();
    t.write("a", "shared");

    t.exec(&["ln", "a", "l"]).assert().success();
    let ino_a = fs::metadata(t.path("a")).unwrap().ino();
    let ino_l = fs::metadata(t.path("l")).unwrap().ino();
    assert_eq!(ino_a, ino_l);

    t.undo().assert().success();
    assert!(!t.exists("l"));
    assert_eq!(t.read("a"), "shared");

    t.undo().arg("redo").assert().success();
    assert_eq!(fs::metadata(t.path("l")).unwrap().ino(), ino_a);
}

#[test]
fn ln_force_parks_victim_and_undo_restores() {
    let t = TestEnv::new();
    t.write("target", "t");
    t.write("existing", "old content");

    t.exec(&["ln", "-sf", "target", "existing"])
        .assert()
        .success();
    assert!(t.is_symlink("existing"));

    t.undo().assert().success();
    assert!(!t.is_symlink("existing"));
    assert_eq!(t.read("existing"), "old content");
}

#[test]
fn ln_without_force_refuses_existing() {
    let t = TestEnv::new();
    t.write("target", "t");
    t.write("existing", "x");
    t.exec(&["ln", "-s", "target", "existing"])
        .assert()
        .code(1)
        .stderr(predicate::str::contains("File exists"));
    assert_eq!(t.read("existing"), "x");
}

#[test]
fn ln_into_directory() {
    let t = TestEnv::new();
    t.write("a.txt", "1");
    t.write("b.txt", "2");
    t.mkdir("links");

    t.exec(&["ln", "-s", "-t", "links", "a.txt", "b.txt"])
        .assert()
        .success();
    assert!(t.is_symlink("links/a.txt"));
    assert!(t.is_symlink("links/b.txt"));

    t.undo().assert().success();
    assert!(!t.exists("links/a.txt"));
    assert!(!t.exists("links/b.txt"));
}

#[test]
fn ln_hardlink_missing_source_fails() {
    let t = TestEnv::new();
    t.exec(&["ln", "ghost", "l"])
        .assert()
        .code(1)
        .stderr(predicate::str::contains("No such file or directory"));
}

#[test]
fn ln_undo_refuses_if_link_was_retargeted() {
    let t = TestEnv::new();
    t.write("t1", "1");
    t.write("t2", "2");
    t.exec(&["ln", "-s", "t1", "link"]).assert().success();

    fs::remove_file(t.path("link")).unwrap();
    t.symlink("t2", "link");

    t.undo().assert().code(1);
    assert!(t.is_symlink("link"));

    t.undo().arg("--force").assert().success();
    assert!(!t.exists("link"));
}
