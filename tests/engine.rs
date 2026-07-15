mod common;

use std::fs;
use std::os::unix::fs::MetadataExt;

use common::TestEnv;
use predicates::prelude::*;
use undo::journal::Journal;
use undo::journal::model::{Details, Status};

#[test]
fn undo_walks_the_stack_backwards() {
    let t = TestEnv::new();
    t.write("1", "a");
    t.write("2", "b");
    t.exec(&["mv", "1", "one"]).assert().success();
    t.exec(&["mv", "2", "two"]).assert().success();

    t.undo().assert().success();
    assert!(t.exists("2") && t.exists("one"));
    t.undo().assert().success();
    assert!(t.exists("1") && t.exists("2"));

    t.undo().arg("redo").assert().success();
    assert!(t.exists("one"));
    t.undo().arg("redo").assert().success();
    assert!(t.exists("two"));
}

#[test]
fn new_operation_supersedes_the_redo_stack() {
    let t = TestEnv::new();
    t.write("a", "x");
    t.exec(&["mv", "a", "b"]).assert().success();
    t.undo().assert().success();

    t.exec(&["mv", "a", "c"]).assert().success();
    t.undo()
        .arg("redo")
        .assert()
        .failure()
        .stderr(predicate::str::contains("superseded"));

    t.undo()
        .args(["history"])
        .assert()
        .success()
        .stdout(predicate::str::contains("superseded"));
}

#[test]
fn force_evicts_occupier_to_trash_and_records_it() {
    let t = TestEnv::new();
    t.write("a", "original");
    t.exec(&["mv", "a", "b"]).assert().success();

    t.write("a", "squatter");
    t.undo()
        .assert()
        .code(1)
        .stderr(predicate::str::contains("occupied"));

    t.undo().arg("--force").assert().success();
    assert_eq!(t.read("a"), "original");
    assert_eq!(
        t.trash_entries(),
        vec!["a"],
        "the squatter must be in the trash"
    );

    t.undo()
        .args(["show", "1"])
        .assert()
        .success()
        .stdout(predicate::str::contains("evicted by --force"));
}

#[test]
fn foreign_uid_entries_are_invisible_and_unusable() {
    let t = TestEnv::new();
    fs::create_dir_all(&t.data).unwrap();
    let journal = Journal::open(&t.data).unwrap();
    let foreign_uid = undo::paths::euid() + 1;
    let id = journal
        .insert_raw(
            foreign_uid,
            "mv",
            &["mv".into(), "x".into(), "y".into()],
            &t.work,
            Status::Applied,
            &Details::new(),
        )
        .unwrap();
    drop(journal);

    t.undo()
        .assert()
        .failure()
        .stderr(predicate::str::contains("nothing to undo"));
    t.undo()
        .args(["show", &id.to_string()])
        .assert()
        .failure()
        .stderr(predicate::str::contains("no journal entry"));
    t.undo()
        .args(["history"])
        .assert()
        .success()
        .stdout(predicate::str::contains("empty"));
}

#[test]
fn dry_run_changes_nothing() {
    let t = TestEnv::new();
    t.write("a", "x");
    t.exec(&["mv", "a", "b"]).assert().success();

    t.undo()
        .arg("--dry-run")
        .assert()
        .success()
        .stdout(predicate::str::contains("would undo"));
    assert!(t.exists("b"), "dry run must not move anything");

    t.undo().assert().success();
    assert!(t.exists("a"));
}

#[test]
fn crashed_pending_rows_are_swept_to_broken() {
    let t = TestEnv::new();
    fs::create_dir_all(&t.data).unwrap();
    let journal = Journal::open(&t.data).unwrap();
    journal
        .insert_raw(
            undo::paths::euid(),
            "mv",
            &["mv".into(), "a".into(), "b".into()],
            &t.work,
            Status::PendingExec,
            &Details::new(),
        )
        .unwrap();
    drop(journal);

    t.undo().assert().failure().stderr(
        predicate::str::contains("marked broken").or(predicate::str::contains("interrupted")),
    );
    t.undo()
        .args(["history"])
        .assert()
        .success()
        .stdout(predicate::str::contains("broken"));
}

#[test]
fn journal_files_are_private() {
    let t = TestEnv::new();
    t.write("a", "x");
    t.exec(&["mv", "a", "b"]).assert().success();

    let dir_mode = fs::metadata(&t.data).unwrap().mode() & 0o777;
    assert_eq!(dir_mode, 0o700, "data dir must be 0700");
    let db_mode = fs::metadata(t.data.join("journal.db")).unwrap().mode() & 0o777;
    assert_eq!(db_mode, 0o600, "journal must be 0600");
}

#[test]
fn clear_forgets_journal_but_not_trash() {
    let t = TestEnv::new();
    t.write("a", "x");
    t.exec(&["rm", "a"]).assert().success();
    assert_eq!(t.trash_entries().len(), 1);

    t.undo().args(["clear", "--yes"]).assert().success();
    t.undo()
        .args(["history"])
        .assert()
        .success()
        .stdout(predicate::str::contains("empty"));
    assert_eq!(
        t.trash_entries().len(),
        1,
        "clear must never touch the trash"
    );
}

#[test]
fn clear_without_tty_requires_yes() {
    let t = TestEnv::new();
    t.write("a", "x");
    t.exec(&["rm", "a"]).assert().success();
    t.undo().arg("clear").assert().code(2);
    t.undo()
        .args(["history"])
        .assert()
        .success()
        .stdout(predicate::str::contains("rm"));
}

#[test]
fn protected_paths_are_hard_errors_not_fallbacks() {
    let t = TestEnv::new();
    let trash = t.home.join(".local/share/Trash");
    fs::create_dir_all(&trash).unwrap();
    t.exec(&["rm", "-r", trash.to_str().unwrap()])
        .assert()
        .code(2)
        .stderr(predicate::str::contains("refusing"));
    t.exec(&["rm", "-r", "/"]).assert().code(2);
    t.exec(&["rm", "-r", t.home.to_str().unwrap()])
        .assert()
        .code(2);
}

#[test]
fn show_displays_actions_and_trash_locations() {
    let t = TestEnv::new();
    t.write("f", "x");
    t.exec(&["rm", "f"]).assert().success();
    t.undo().args(["show", "1"]).assert().success().stdout(
        predicate::str::contains("trashed")
            .and(predicate::str::contains("Trash/files/f"))
            .and(predicate::str::contains("status:")),
    );
}

#[test]
fn hash_limit_env_var_disables_hashing_but_keeps_verification() {
    let t = TestEnv::new();
    t.write("big.bin", "0123456789");
    t.exec(&["mv", "big.bin", "moved.bin"])
        .env("UNDO_HASH_MAX_BYTES", "4")
        .assert()
        .success();

    t.undo().env("UNDO_HASH_MAX_BYTES", "4").assert().success();
    assert_eq!(t.read("big.bin"), "0123456789");
}
