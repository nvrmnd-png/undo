mod common;

use common::TestEnv;
use predicates::prelude::*;

#[test]
fn config_show_defaults() {
    let t = TestEnv::new();
    t.undo().args(["config", "show"]).assert().success().stdout(
        predicate::str::contains("max_age_days")
            .and(predicate::str::contains("90"))
            .and(predicate::str::contains("logging")),
    );
}

#[test]
fn config_show_json_reflects_file() {
    let t = TestEnv::new();
    t.write_config("[cleanup]\nenabled = true\nmax_age_days = 7\n[logging]\nenabled = true\n");
    let out = t
        .undo()
        .args(["config", "show", "--json"])
        .assert()
        .success();
    let v = TestEnv::json(&out.get_output().stdout);
    assert_eq!(v["schema"], 1);
    assert_eq!(v["config"]["cleanup"]["enabled"], true);
    assert_eq!(v["config"]["cleanup"]["max_age_days"], 7);
    assert_eq!(v["config"]["logging"]["enabled"], true);
}

#[test]
fn config_reset_writes_defaults() {
    let t = TestEnv::new();
    t.write_config("[cleanup]\nmax_age_days = 1\n");
    t.undo()
        .args(["config", "reset", "--yes"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Reset configuration"));
    // After reset the value is back to the default.
    let out = t
        .undo()
        .args(["config", "show", "--json"])
        .assert()
        .success();
    let v = TestEnv::json(&out.get_output().stdout);
    assert_eq!(v["config"]["cleanup"]["max_age_days"], 90);
}

#[test]
fn excluded_path_falls_back_silently() {
    let t = TestEnv::new();
    t.mkdir("skip");
    t.write("skip/junk.txt", "x");
    let abs_skip = t.path("skip");
    t.write_config(&format!(
        "[exclude]\npaths = [\"{}\"]\n",
        abs_skip.display()
    ));

    // exec on an excluded path returns 126 (silent) and does not act or journal.
    t.exec(&["rm", "skip/junk.txt"]).assert().code(126);
    assert!(
        t.exists("skip/junk.txt"),
        "exec must not touch excluded paths"
    );
    t.undo()
        .args(["history"])
        .assert()
        .success()
        .stdout(predicate::str::contains("empty"));
}

#[test]
fn logging_appends_to_logfile_when_enabled() {
    let t = TestEnv::new();
    t.write_config("[logging]\nenabled = true\n");
    t.write("a.txt", "x");
    t.exec(&["mv", "a.txt", "b.txt"]).assert().success();

    assert!(t.logfile().exists(), "logfile should be created");
    let log = std::fs::read_to_string(t.logfile()).unwrap();
    assert!(
        log.contains("\tmv\t"),
        "log line should record the command: {log}"
    );
    assert!(log.contains("a.txt"));
}

#[test]
fn logging_off_by_default() {
    let t = TestEnv::new();
    t.write("a.txt", "x");
    t.exec(&["mv", "a.txt", "b.txt"]).assert().success();
    assert!(!t.logfile().exists(), "no logfile without logging enabled");
}
