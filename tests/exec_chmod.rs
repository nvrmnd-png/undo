mod common;

use common::TestEnv;
use predicates::prelude::*;

#[test]
fn chmod_octal_roundtrip() {
    let t = TestEnv::new();
    t.write("f", "x");
    t.set_mode("f", 0o644);

    t.exec(&["chmod", "600", "f"]).assert().success();
    assert_eq!(t.mode("f"), 0o600);

    t.undo().assert().success();
    assert_eq!(t.mode("f"), 0o644);

    t.undo().arg("redo").assert().success();
    assert_eq!(t.mode("f"), 0o600);
}

#[test]
fn chmod_symbolic_and_dash_modes() {
    let t = TestEnv::new();
    t.write("f", "x");
    t.set_mode("f", 0o644);

    t.exec(&["chmod", "u+x", "f"]).assert().success();
    assert_eq!(t.mode("f"), 0o744);

    t.exec(&["chmod", "-w", "f"]).assert().success();
    assert_eq!(t.mode("f") & 0o222, 0);

    t.undo().assert().success();
    t.undo().assert().success();
    assert_eq!(t.mode("f"), 0o644);
}

#[test]
fn chmod_recursive_skips_symlinks() {
    let t = TestEnv::new();
    t.write("outside.txt", "keep my mode");
    t.set_mode("outside.txt", 0o664);
    t.write("tree/a.txt", "x");
    t.write("tree/sub/b.txt", "y");
    t.symlink("../../outside.txt", "tree/sub/link");
    t.set_mode("tree/a.txt", 0o644);
    t.set_mode("tree/sub/b.txt", 0o644);

    t.exec(&["chmod", "-R", "u=rwX,go=", "tree"])
        .assert()
        .success();
    assert_eq!(t.mode("tree/a.txt"), 0o600);
    assert_eq!(t.mode("tree/sub/b.txt"), 0o600);
    assert_eq!(
        t.mode("tree"),
        0o700,
        "directories keep their search bit via X"
    );
    assert_eq!(t.mode("outside.txt"), 0o664, "-R must not follow symlinks");

    t.undo().assert().success();
    assert_eq!(t.mode("tree/a.txt"), 0o644);
    assert_eq!(t.mode("tree/sub/b.txt"), 0o644);
}

#[test]
fn chmod_noop_journals_nothing() {
    let t = TestEnv::new();
    t.write("f", "x");
    t.set_mode("f", 0o600);
    t.exec(&["chmod", "600", "f"]).assert().success();
    t.undo()
        .assert()
        .failure()
        .stderr(predicate::str::contains("nothing to undo"));
}

#[test]
fn chmod_undo_refuses_when_mode_drifted() {
    let t = TestEnv::new();
    t.write("f", "x");
    t.set_mode("f", 0o644);
    t.exec(&["chmod", "600", "f"]).assert().success();

    t.set_mode("f", 0o777);
    t.undo()
        .assert()
        .code(1)
        .stderr(predicate::str::contains("mode"));
    assert_eq!(t.mode("f"), 0o777);

    t.undo().arg("--force").assert().success();
    assert_eq!(t.mode("f"), 0o644);
}

#[test]
fn chmod_reference_falls_back() {
    let t = TestEnv::new();
    t.write("f", "x");
    t.exec(&["chmod", "--reference=/etc/passwd", "f"])
        .assert()
        .code(125);
}

#[test]
fn chmod_multiple_clauses() {
    let t = TestEnv::new();
    t.write("f", "x");
    t.set_mode("f", 0o600);
    t.exec(&["chmod", "u=rwx,g=rx,o=", "f"]).assert().success();
    assert_eq!(t.mode("f"), 0o750);
}

#[test]
fn chmod_symlink_operand_changes_target() {
    let t = TestEnv::new();
    t.write("target", "x");
    t.set_mode("target", 0o644);
    t.symlink("target", "link");

    t.exec(&["chmod", "600", "link"]).assert().success();
    assert_eq!(t.mode("target"), 0o600);

    t.undo().assert().success();
    assert_eq!(t.mode("target"), 0o644);
}
