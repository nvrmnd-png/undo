mod common;

use common::TestEnv;
use predicates::prelude::*;

#[test]
fn prune_dry_run_changes_nothing() {
    let t = TestEnv::new();
    t.write("a", "x");
    t.exec(&["mv", "a", "b"]).assert().success();

    t.undo()
        .args(["--dry-run", "prune", "--older-than", "0"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Would remove"));

    // Entry still present.
    t.undo()
        .arg("list")
        .assert()
        .success()
        .stdout(predicate::str::contains("mv a b"));
}

#[test]
fn prune_removes_old_entries_but_keeps_trash() {
    let t = TestEnv::new();
    t.write("gone.txt", "precious");
    t.exec(&["rm", "gone.txt"]).assert().success();
    assert_eq!(t.trash_entries().len(), 1);

    t.undo()
        .args(["prune", "--older-than", "0", "--yes"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Removed"));

    // Journal emptied, trash kept (the promise).
    t.undo()
        .args(["history"])
        .assert()
        .success()
        .stdout(predicate::str::contains("empty"));
    assert_eq!(
        t.trash_entries().len(),
        1,
        "prune without --empty-trash must keep trashed files"
    );
}

#[test]
fn prune_empty_trash_deletes_trashed_files() {
    let t = TestEnv::new();
    t.write("gone.txt", "x");
    t.exec(&["rm", "gone.txt"]).assert().success();
    assert_eq!(t.trash_entries().len(), 1);

    t.undo()
        .args(["prune", "--older-than", "0", "--empty-trash", "--yes"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Permanently deleted"));

    assert!(
        t.trash_entries().is_empty(),
        "--empty-trash must permanently delete the trashed file"
    );
}

#[test]
fn prune_nothing_when_entries_are_recent() {
    let t = TestEnv::new();
    t.write("a", "x");
    t.exec(&["mv", "a", "b"]).assert().success();

    // Cutoff of 3650 days keeps everything recent.
    t.undo()
        .args(["prune", "--older-than", "3650", "--yes"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Nothing to prune"));
    t.undo()
        .arg("list")
        .assert()
        .success()
        .stdout(predicate::str::contains("mv a b"));
}

#[test]
fn prune_json_shape() {
    let t = TestEnv::new();
    t.write("a", "x");
    t.exec(&["mv", "a", "b"]).assert().success();

    let out = t
        .undo()
        .args(["prune", "--older-than", "0", "--yes", "--json"])
        .assert()
        .success();
    let v = TestEnv::json(&out.get_output().stdout);
    assert_eq!(v["schema"], 1);
    assert_eq!(v["dry_run"], false);
    assert_eq!(v["removed"], 1);
    assert_eq!(v["empty_trash"], false);
}
