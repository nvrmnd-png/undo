mod common;

use std::fs;
use std::os::unix::fs::MetadataExt;

use common::TestEnv;

#[test]
fn trashinfo_matches_the_spec() {
    let t = TestEnv::new();
    t.write("my file.txt", "x");
    t.exec(&["rm", "my file.txt"]).assert().success();

    let info_dir = t.trash_info();
    let entries: Vec<_> = fs::read_dir(&info_dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .collect();
    assert_eq!(entries.len(), 1);
    let name = entries[0].file_name().to_string_lossy().into_owned();
    assert!(
        name.ends_with(".trashinfo"),
        "info file must end in .trashinfo: {name}"
    );

    let body = fs::read_to_string(entries[0].path()).unwrap();
    assert!(body.starts_with("[Trash Info]\n"), "bad header: {body}");
    let path_line = body.lines().find(|l| l.starts_with("Path=")).unwrap();
    assert!(
        path_line.contains("my%20file.txt"),
        "bad Path line: {path_line}"
    );
    assert!(
        path_line.starts_with("Path=/"),
        "Path must be absolute: {path_line}"
    );
    let date_line = body
        .lines()
        .find(|l| l.starts_with("DeletionDate="))
        .unwrap();
    let date = date_line.trim_start_matches("DeletionDate=");
    assert_eq!(
        date.len(),
        19,
        "DeletionDate must be YYYY-MM-DDThh:mm:ss: {date}"
    );
}

#[test]
fn restore_removes_the_trashinfo() {
    let t = TestEnv::new();
    t.write("f", "x");
    t.exec(&["rm", "f"]).assert().success();
    assert_eq!(fs::read_dir(t.trash_info()).unwrap().count(), 1);

    t.undo().assert().success();
    assert_eq!(fs::read_dir(t.trash_info()).unwrap().count(), 0);
    assert_eq!(fs::read_dir(t.trash_files()).unwrap().count(), 0);
}

#[test]
fn collision_names_follow_the_dot_n_scheme() {
    let t = TestEnv::new();
    for _ in 0..3 {
        t.write("f", "x");
        t.exec(&["rm", "f"]).assert().success();
    }
    assert_eq!(t.trash_entries(), vec!["f", "f.2", "f.3"]);

    let mut infos: Vec<String> = fs::read_dir(t.trash_info())
        .unwrap()
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    infos.sort();
    assert_eq!(infos, vec!["f.2.trashinfo", "f.3.trashinfo", "f.trashinfo"]);
}

#[test]
fn stray_payload_without_info_is_never_clobbered() {
    let t = TestEnv::new();
    fs::create_dir_all(t.trash_files()).unwrap();
    fs::write(t.trash_files().join("f"), "stray data").unwrap();

    t.write("f", "ours");
    t.exec(&["rm", "f"]).assert().success();

    assert_eq!(
        fs::read_to_string(t.trash_files().join("f")).unwrap(),
        "stray data",
        "the stray payload must survive"
    );
    assert_eq!(t.trash_entries(), vec!["f", "f.2"]);
}

#[test]
fn trash_root_is_private() {
    let t = TestEnv::new();
    t.write("f", "x");
    t.exec(&["rm", "f"]).assert().success();
    let mode = fs::metadata(t.home.join(".local/share/Trash"))
        .unwrap()
        .mode()
        & 0o777;
    assert_eq!(mode, 0o700);
}
