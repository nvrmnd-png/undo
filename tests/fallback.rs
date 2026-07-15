mod common;

use common::TestEnv;
use predicates::prelude::*;

fn assert_fallback(t: &TestEnv, args: &[&str]) {
    t.exec(args).assert().code(125);
}

fn assert_journal_empty(t: &TestEnv) {
    t.undo()
        .args(["history"])
        .assert()
        .success()
        .stdout(predicate::str::contains("empty"));
}

#[test]
fn unsupported_flags_fall_back_per_command() {
    let t = TestEnv::new();
    t.write("a", "x");
    t.write("b", "y");
    t.mkdir("d");

    assert_fallback(&t, &["mv", "--backup", "a", "b"]);
    assert_fallback(&t, &["mv", "-u", "a", "b"]);
    assert_fallback(&t, &["cp", "--reflink=auto", "a", "b"]);
    assert_fallback(&t, &["cp", "-l", "a", "c"]);
    assert_fallback(&t, &["rm", "--one-file-system", "a"]);
    assert_fallback(&t, &["mkdir", "--context", "x"]);
    assert_fallback(&t, &["rmdir", "--unknown-flag", "d"]);
    assert_fallback(&t, &["chmod", "--reference=/etc/passwd", "a"]);
    assert_fallback(&t, &["chown", "--from=0:0", "0", "a"]);
    assert_fallback(&t, &["ln", "-b", "a", "l"]);
    assert_fallback(&t, &["rename", "y/a/b/", "a"]);

    assert_eq!(t.read("a"), "x");
    assert_eq!(t.read("b"), "y");
    assert!(!t.exists("c") && !t.exists("l") && !t.exists("x"));
    assert_journal_empty(&t);
}

#[test]
fn help_requests_fall_back_to_the_real_tool() {
    let t = TestEnv::new();
    assert_fallback(&t, &["mv", "--help"]);
    assert_fallback(&t, &["rm", "--version"]);
    assert_journal_empty(&t);
}

#[test]
fn missing_operands_fall_back() {
    let t = TestEnv::new();
    assert_fallback(&t, &["mv"]);
    assert_fallback(&t, &["mv", "only-one"]);
    assert_fallback(&t, &["rm"]);
    assert_fallback(&t, &["chmod", "644"]);
    assert_journal_empty(&t);
}

#[test]
fn non_utf8_arguments_fall_back() {
    use std::ffi::OsStr;
    use std::os::unix::ffi::OsStrExt;

    let t = TestEnv::new();
    let mut cmd = t.undo();
    cmd.arg("exec")
        .arg("--")
        .arg("rm")
        .arg(OsStr::from_bytes(b"\xff\xfe"));
    cmd.assert().code(125);
    assert_journal_empty(&t);
}

#[test]
fn unsupported_command_is_a_usage_error_not_a_fallback() {
    let t = TestEnv::new();
    t.exec(&["dd", "if=/dev/zero"]).assert().code(2);
}

#[test]
fn fallback_is_silent_on_stderr_for_the_wrapper() {
    let t = TestEnv::new();
    t.write("a", "x");
    t.exec(&["mv", "--backup", "a", "b"])
        .assert()
        .code(125)
        .stderr(predicate::str::is_empty());
}
