mod common;

use common::TestEnv;
use predicates::prelude::*;

#[test]
fn init_snippet_wraps_valid_aliases_only() {
    let t = TestEnv::new();
    t.write_config("[plugins]\nmycp = \"cp\"\ntrashit = \"rm\"\nbad = \"frobnicate\"\n");

    let out = t.undo().args(["init", "zsh"]).assert().success();
    let text = String::from_utf8_lossy(&out.get_output().stdout);
    // Valid aliases appended after the static wrappers.
    assert!(text.contains("mycp"), "mycp should be wrapped");
    assert!(text.contains("trashit"), "trashit should be wrapped");
    // Alias with an unknown target is dropped.
    assert!(
        !text.contains("bad"),
        "alias with invalid target must be skipped"
    );
    // The nine built-ins are still present (static part).
    assert!(text.contains("for _undo_cmd in mv cp rm"));
}

#[test]
fn alias_routes_to_builtin_and_is_undoable() {
    let t = TestEnv::new();
    t.write_config("[plugins]\nmycp = \"cp\"\n");
    t.write("a.txt", "hello");

    // mycp behaves like cp.
    t.exec(&["mycp", "a.txt", "b.txt"]).assert().success();
    assert_eq!(t.read("b.txt"), "hello");

    // Journaled under the real command name.
    let out = t.undo().args(["log", "--json"]).assert().success();
    let v = TestEnv::json(&out.get_output().stdout);
    assert_eq!(v["entries"][0]["command"], "cp");

    // And it undoes.
    t.undo().assert().success();
    assert!(!t.exists("b.txt"));
}

#[test]
fn alias_to_rm_uses_trash() {
    let t = TestEnv::new();
    t.write_config("[plugins]\ntrashit = \"rm\"\n");
    t.write("gone.txt", "x");

    t.exec(&["trashit", "gone.txt"]).assert().success();
    assert!(!t.exists("gone.txt"));
    assert_eq!(t.trash_entries().len(), 1);

    t.undo().assert().success();
    assert_eq!(t.read("gone.txt"), "x");
}

#[test]
fn alias_with_bad_target_is_usage_error() {
    let t = TestEnv::new();
    t.write_config("[plugins]\nbad = \"frobnicate\"\n");
    t.write("a.txt", "x");
    t.exec(&["bad", "a.txt", "b.txt"])
        .assert()
        .code(2)
        .stderr(predicate::str::contains("unknown command 'frobnicate'"));
}

#[test]
fn unknown_command_without_alias_is_usage_error() {
    let t = TestEnv::new();
    t.exec(&["definitelynotacommand", "x"])
        .assert()
        .code(2)
        .stderr(predicate::str::contains("unsupported command"));
}

// Network path: hits the real GitHub API. Run with:
//   cargo test --test plugins -- --ignored update_check_against_github
#[test]
#[ignore]
fn update_check_against_github() {
    let t = TestEnv::new();
    // We are on the crate's version; the published latest is older or equal,
    // so --check reports up to date and never downloads.
    t.undo().args(["update", "--check"]).assert().success();
}
