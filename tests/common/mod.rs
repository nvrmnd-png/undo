#![allow(dead_code)]

use std::fs;
use std::path::PathBuf;

use assert_cmd::Command;
use tempfile::TempDir;

pub struct TestEnv {
    pub tmp: TempDir,
    pub home: PathBuf,
    pub work: PathBuf,
    pub data: PathBuf,
}

impl TestEnv {
    pub fn new() -> TestEnv {
        let tmp = TempDir::new().expect("tempdir");
        let home = tmp.path().join("home");
        let work = tmp.path().join("work");
        let data = tmp.path().join("undo-data");
        fs::create_dir_all(&home).unwrap();
        fs::create_dir_all(&work).unwrap();
        TestEnv {
            tmp,
            home,
            work,
            data,
        }
    }

    pub fn undo(&self) -> Command {
        let mut cmd = Command::cargo_bin("undo").expect("binary");
        cmd.current_dir(&self.work)
            .env("HOME", &self.home)
            .env("XDG_DATA_HOME", self.home.join(".local/share"))
            .env("XDG_CONFIG_HOME", self.home.join(".config"))
            .env("UNDO_DATA_DIR", &self.data)
            .env("NO_COLOR", "1")
            .env_remove("UNDO_HASH_MAX_BYTES")
            .env_remove("UNDO_TREE_CAP");
        cmd
    }

    pub fn config_file(&self) -> PathBuf {
        self.home.join(".config/undo/config.toml")
    }

    pub fn write_config(&self, toml: &str) {
        let p = self.config_file();
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(p, toml).unwrap();
    }

    pub fn logfile(&self) -> PathBuf {
        self.data.join("undo.log")
    }

    pub fn exec(&self, args: &[&str]) -> Command {
        let mut cmd = self.undo();
        cmd.arg("exec").arg("--").args(args);
        cmd
    }

    pub fn path(&self, rel: &str) -> PathBuf {
        self.work.join(rel)
    }

    pub fn write(&self, rel: &str, content: &str) {
        let p = self.path(rel);
        if let Some(parent) = p.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(p, content).unwrap();
    }

    pub fn read(&self, rel: &str) -> String {
        fs::read_to_string(self.path(rel)).unwrap_or_else(|e| panic!("reading {rel}: {e}"))
    }

    pub fn mkdir(&self, rel: &str) {
        fs::create_dir_all(self.path(rel)).unwrap();
    }

    pub fn symlink(&self, target: &str, rel: &str) {
        std::os::unix::fs::symlink(target, self.path(rel)).unwrap();
    }

    pub fn exists(&self, rel: &str) -> bool {
        fs::symlink_metadata(self.path(rel)).is_ok()
    }

    pub fn is_dir(&self, rel: &str) -> bool {
        self.path(rel).is_dir()
    }

    pub fn is_symlink(&self, rel: &str) -> bool {
        fs::symlink_metadata(self.path(rel))
            .map(|m| m.file_type().is_symlink())
            .unwrap_or(false)
    }

    pub fn mode(&self, rel: &str) -> u32 {
        use std::os::unix::fs::MetadataExt;
        fs::symlink_metadata(self.path(rel)).unwrap().mode() & 0o7777
    }

    pub fn set_mode(&self, rel: &str, mode: u32) {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(self.path(rel), fs::Permissions::from_mode(mode)).unwrap();
    }

    pub fn trash_files(&self) -> PathBuf {
        self.home.join(".local/share/Trash/files")
    }

    pub fn trash_info(&self) -> PathBuf {
        self.home.join(".local/share/Trash/info")
    }

    pub fn trash_entries(&self) -> Vec<String> {
        match fs::read_dir(self.trash_files()) {
            Ok(rd) => {
                let mut v: Vec<String> = rd
                    .filter_map(|e| e.ok())
                    .map(|e| e.file_name().to_string_lossy().into_owned())
                    .collect();
                v.sort();
                v
            }
            Err(_) => Vec::new(),
        }
    }

    pub fn json(output: &[u8]) -> serde_json::Value {
        serde_json::from_slice(output).expect("valid JSON on stdout")
    }
}
