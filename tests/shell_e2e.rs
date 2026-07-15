mod common;

use std::process::Command;

use common::TestEnv;

fn run_in_shell(t: &TestEnv, shell: &str, script: &str) -> Option<(i32, String, String)> {
    if !which(shell) {
        eprintln!("skipping: {shell} is not installed");
        return None;
    }
    let bin = assert_cmd::cargo::cargo_bin("undo");
    let bin_dir = bin.parent().unwrap();
    let path = format!(
        "{}:{}",
        bin_dir.display(),
        std::env::var("PATH").unwrap_or_default()
    );
    let out = Command::new(shell)
        .arg("-c")
        .arg(script)
        .current_dir(&t.work)
        .env("HOME", &t.home)
        .env("XDG_DATA_HOME", t.home.join(".local/share"))
        .env("UNDO_DATA_DIR", &t.data)
        .env("NO_COLOR", "1")
        .env("PATH", path)
        .output()
        .expect("shell spawn");
    Some((
        out.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    ))
}

fn which(bin: &str) -> bool {
    Command::new("sh")
        .arg("-c")
        .arg(format!("command -v {bin}"))
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

fn roundtrip_script(init_shell: &str) -> String {
    format!(
        r#"
eval "$(undo init {init_shell})"
mv a.txt moved.txt || exit 10
test -f moved.txt || exit 11
undo || exit 12
test -f a.txt || exit 13
echo ROUNDTRIP_OK
"#
    )
}

#[test]
fn bash_wrapper_roundtrip() {
    let t = TestEnv::new();
    t.write("a.txt", "x");
    let Some((code, stdout, stderr)) = run_in_shell(&t, "bash", &roundtrip_script("bash")) else {
        return;
    };
    assert_eq!(code, 0, "stdout: {stdout}\nstderr: {stderr}");
    assert!(stdout.contains("ROUNDTRIP_OK"));
    assert!(t.exists("a.txt"));
}

#[test]
fn zsh_wrapper_roundtrip() {
    let t = TestEnv::new();
    t.write("a.txt", "x");
    let Some((code, stdout, stderr)) = run_in_shell(&t, "zsh", &roundtrip_script("zsh")) else {
        return;
    };
    assert_eq!(code, 0, "stdout: {stdout}\nstderr: {stderr}");
    assert!(stdout.contains("ROUNDTRIP_OK"));
}

#[test]
fn fish_wrapper_roundtrip() {
    let t = TestEnv::new();
    t.write("a.txt", "x");
    let script = r#"
undo init fish | source
mv a.txt moved.txt; or exit 10
test -f moved.txt; or exit 11
undo; or exit 12
test -f a.txt; or exit 13
echo ROUNDTRIP_OK
"#;
    let Some((code, stdout, stderr)) = run_in_shell(&t, "fish", script) else {
        return;
    };
    assert_eq!(code, 0, "stdout: {stdout}\nstderr: {stderr}");
    assert!(stdout.contains("ROUNDTRIP_OK"));
}

#[test]
fn bash_wrapper_falls_back_on_unsupported_flags() {
    let t = TestEnv::new();
    t.write("a.txt", "x");
    let script = r#"
eval "$(undo init bash)"
mv --backup=numbered a.txt b.txt || exit 10
test -f b.txt || exit 11
undo 2>/dev/null && exit 12  # nothing journaled → undo must fail
echo FALLBACK_OK
"#;
    let Some((code, stdout, stderr)) = run_in_shell(&t, "bash", script) else {
        return;
    };
    assert_eq!(code, 0, "stdout: {stdout}\nstderr: {stderr}");
    assert!(stdout.contains("FALLBACK_OK"));
    assert!(
        stderr.contains("not journaled"),
        "wrapper must explain the fallback: {stderr}"
    );
}

#[test]
fn wrapper_passes_exit_codes_through() {
    let t = TestEnv::new();
    let script = r#"
eval "$(undo init bash)"
rm no-such-file 2>/dev/null
echo "rc=$?"
"#;
    let Some((code, stdout, _)) = run_in_shell(&t, "bash", script) else {
        return;
    };
    assert_eq!(code, 0);
    assert!(
        stdout.contains("rc=1"),
        "GNU rm exit code must pass through: {stdout}"
    );
}
