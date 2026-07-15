mod common;

use common::TestEnv;
use predicates::prelude::*;

#[test]
fn undo_json_report_shape() {
    let t = TestEnv::new();
    t.write("a", "x");
    t.exec(&["mv", "a", "b"]).assert().success();

    let out = t.undo().arg("--json").assert().success();
    let v = TestEnv::json(&out.get_output().stdout);
    assert_eq!(v["schema"], 1);
    assert_eq!(v["ok"], true);
    assert_eq!(v["action"], "undo");
    assert_eq!(v["entry"]["command"], "mv");
    assert_eq!(v["entry"]["status"], "undone");
    assert!(v["changes"].as_array().is_some_and(|c| !c.is_empty()));
    assert_eq!(v["changes"][0]["kind"], "move");
}

#[test]
fn refusal_json_has_conflicts_and_exit_1() {
    let t = TestEnv::new();
    t.write("a", "x");
    t.exec(&["mv", "a", "b"]).assert().success();
    t.write("b", "tampered");

    let out = t.undo().arg("--json").assert().code(1);
    let v = TestEnv::json(&out.get_output().stdout);
    assert_eq!(v["ok"], false);
    let conflicts = v["conflicts"].as_array().unwrap();
    assert!(!conflicts.is_empty());
    assert_eq!(conflicts[0]["kind"], "modified");
}

#[test]
fn list_json_shape() {
    let t = TestEnv::new();
    t.write("a", "x");
    t.write("c", "y");
    t.exec(&["mv", "a", "b"]).assert().success();
    t.exec(&["mv", "c", "d"]).assert().success();
    t.undo().assert().success();

    let out = t.undo().args(["list", "--json"]).assert().success();
    let v = TestEnv::json(&out.get_output().stdout);
    assert_eq!(v["schema"], 1);
    assert_eq!(v["undo_stack"].as_array().unwrap().len(), 1);
    assert_eq!(v["redo_stack"].as_array().unwrap().len(), 1);
    assert_eq!(v["undo_stack"][0]["id"], 1);
    assert_eq!(v["redo_stack"][0]["id"], 2);
}

#[test]
fn history_and_show_json() {
    let t = TestEnv::new();
    t.write("f", "x");
    t.exec(&["rm", "f"]).assert().success();

    let out = t.undo().args(["history", "--json"]).assert().success();
    let v = TestEnv::json(&out.get_output().stdout);
    assert_eq!(v["entries"][0]["command"], "rm");
    assert!(v["entries"][0]["argv"].as_array().is_some());

    let out = t.undo().args(["show", "1", "--json"]).assert().success();
    let v = TestEnv::json(&out.get_output().stdout);
    assert_eq!(v["details"]["actions"][0]["kind"], "trash_put");
    assert!(
        v["details"]["actions"][0]["trash"]["file"]
            .as_str()
            .is_some()
    );
}

#[test]
fn yaml_output_parses_shape() {
    let t = TestEnv::new();
    t.write("f", "x");
    t.exec(&["rm", "f"]).assert().success();

    t.undo()
        .args(["history", "--yaml"])
        .assert()
        .success()
        .stdout(
            predicate::str::contains("schema: 1")
                .and(predicate::str::contains("entries:"))
                .and(predicate::str::contains("command: rm")),
        );
}

#[test]
fn json_and_yaml_conflict() {
    let t = TestEnv::new();
    t.undo()
        .args(["list", "--json", "--yaml"])
        .assert()
        .failure();
}

#[test]
fn exec_and_tui_reject_machine_output() {
    let t = TestEnv::new();
    t.write("a", "x");
    t.undo()
        .args(["--json", "exec", "--", "rm", "a"])
        .assert()
        .code(2);
    assert!(t.exists("a"));
    t.undo().args(["--json", "tui"]).assert().code(2);
}

#[test]
fn clear_json() {
    let t = TestEnv::new();
    t.write("a", "x");
    t.exec(&["rm", "a"]).assert().success();
    let out = t
        .undo()
        .args(["clear", "--yes", "--json"])
        .assert()
        .success();
    let v = TestEnv::json(&out.get_output().stdout);
    assert_eq!(v["ok"], true);
    assert_eq!(v["cleared"], 1);
}

#[test]
fn machine_stdout_is_a_single_document() {
    let t = TestEnv::new();
    t.write("a", "x");
    t.exec(&["mv", "a", "b"]).assert().success();
    let out = t.undo().arg("--json").assert().success();
    TestEnv::json(&out.get_output().stdout);
}
