mod common;

use common::TestEnv;
use predicates::prelude::*;

#[test]
fn search_finds_by_filename() {
    let t = TestEnv::new();
    t.write("report.txt", "x");
    t.mkdir("archive");
    t.exec(&["mv", "report.txt", "archive/"]).assert().success();
    t.write("note.md", "y");
    t.exec(&["rm", "note.md"]).assert().success();

    t.undo()
        .args(["search", "report"])
        .assert()
        .success()
        .stdout(predicate::str::contains("report.txt"));

    t.undo()
        .args(["search", "note"])
        .assert()
        .success()
        .stdout(predicate::str::contains("note.md"));

    t.undo()
        .args(["search", "zzznope"])
        .assert()
        .success()
        .stdout(predicate::str::contains("No journal entries match"));
}

#[test]
fn search_json_shape() {
    let t = TestEnv::new();
    t.write("uniquename.txt", "x");
    t.exec(&["mv", "uniquename.txt", "renamed.txt"])
        .assert()
        .success();

    let out = t
        .undo()
        .args(["search", "uniquename", "--json"])
        .assert()
        .success();
    let v = TestEnv::json(&out.get_output().stdout);
    assert_eq!(v["schema"], 1);
    assert_eq!(v["needle"], "uniquename");
    assert!(!v["entries"].as_array().unwrap().is_empty());
}

#[test]
fn log_shows_time_command_and_file() {
    let t = TestEnv::new();
    t.write("a.txt", "x");
    t.mkdir("d");
    t.exec(&["mv", "a.txt", "d/"]).assert().success();

    t.undo()
        .arg("log")
        .assert()
        .success()
        .stdout(predicate::str::contains("mv").and(predicate::str::contains("a.txt")));
}

#[test]
fn log_json_has_files() {
    let t = TestEnv::new();
    t.write("gone.txt", "x");
    t.exec(&["rm", "gone.txt"]).assert().success();

    let out = t.undo().args(["log", "--json"]).assert().success();
    let v = TestEnv::json(&out.get_output().stdout);
    assert_eq!(v["schema"], 1);
    let e0 = &v["entries"][0];
    assert_eq!(e0["command"], "rm");
    assert!(
        e0["files"]
            .as_array()
            .unwrap()
            .iter()
            .any(|f| f.as_str().unwrap().contains("gone.txt"))
    );
}

#[test]
fn log_empty_journal() {
    let t = TestEnv::new();
    t.undo()
        .arg("log")
        .assert()
        .success()
        .stdout(predicate::str::contains("empty"));
}
