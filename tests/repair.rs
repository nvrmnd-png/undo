mod common;

use std::fs;

use common::TestEnv;
use predicates::prelude::*;

fn corrupt_db(t: &TestEnv) {
    let db = t.data.join("journal.db");
    let mut bytes = fs::read(&db).expect("read journal.db");
    for b in bytes.iter_mut().skip(200).take(3800) {
        *b = 0;
    }
    fs::write(&db, &bytes).expect("write corrupt journal.db");
}

fn corrupt_backups(t: &TestEnv) -> usize {
    fs::read_dir(&t.data)
        .unwrap()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_name().to_string_lossy().contains("corrupt"))
        .count()
}

#[test]
fn repair_healthy_is_noop() {
    let t = TestEnv::new();
    t.write("a", "x");
    t.exec(&["mv", "a", "b"]).assert().success();

    t.undo()
        .arg("repair")
        .assert()
        .success()
        .stdout(predicate::str::contains("healthy"));
    assert_eq!(corrupt_backups(&t), 0);
}

#[test]
fn corrupt_db_hints_at_repair() {
    let t = TestEnv::new();
    t.write("a", "x");
    t.exec(&["mv", "a", "b"]).assert().success();
    corrupt_db(&t);

    t.undo()
        .arg("list")
        .assert()
        .failure()
        .stderr(predicate::str::contains("undo repair"));
}

#[test]
fn repair_rebuilds_corrupt_db_and_keeps_trash() {
    let t = TestEnv::new();
    t.write("a.txt", "precious");
    t.exec(&["rm", "a.txt"]).assert().success();
    assert_eq!(
        t.trash_entries().len(),
        1,
        "one file should be in the trash"
    );

    corrupt_db(&t);

    t.undo().args(["repair", "--yes"]).assert().success();

    // Healthy again, backup written, trash untouched.
    t.undo()
        .arg("repair")
        .assert()
        .success()
        .stdout(predicate::str::contains("healthy"));
    assert_eq!(corrupt_backups(&t), 1, "the damaged db should be backed up");
    assert_eq!(
        t.trash_entries().len(),
        1,
        "repair must not touch the trash"
    );

    // The fresh journal is usable.
    t.write("c.txt", "x");
    t.exec(&["mv", "c.txt", "d.txt"]).assert().success();
    t.undo()
        .arg("list")
        .assert()
        .success()
        .stdout(predicate::str::contains("mv c.txt d.txt"));
}

#[test]
fn repair_without_tty_needs_yes() {
    let t = TestEnv::new();
    t.write("a", "x");
    t.exec(&["mv", "a", "b"]).assert().success();
    corrupt_db(&t);

    // No --yes, no tty → refuses (usage error), leaves db as-is.
    t.undo().arg("repair").assert().code(2);
    assert_eq!(corrupt_backups(&t), 0);
}
