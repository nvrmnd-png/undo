use std::fs::{File, OpenOptions};
use std::os::unix::fs::OpenOptionsExt;
use std::path::Path;
use std::time::{Duration, Instant};

use rustix::fs::{FlockOperation, flock};

use crate::error::{IoCtx, Result, UndoError};
use crate::paths;

const LOCK_TIMEOUT: Duration = Duration::from_secs(10);
const LOCK_RETRY: Duration = Duration::from_millis(50);

#[derive(Debug)]
pub struct LockGuard {
    file: File,
}

pub fn acquire(data_dir: &Path) -> Result<LockGuard> {
    let path = paths::lock_path(data_dir);
    let file = OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .mode(0o600)
        .open(&path)
        .ctx(format!("opening lock file {}", path.display()))?;

    let deadline = Instant::now() + LOCK_TIMEOUT;
    loop {
        match flock(&file, FlockOperation::NonBlockingLockExclusive) {
            Ok(()) => return Ok(LockGuard { file }),
            Err(rustix::io::Errno::WOULDBLOCK) => {
                if Instant::now() >= deadline {
                    return Err(UndoError::msg(
                        "another undo instance is running (lock held for >10s)",
                    ));
                }
                std::thread::sleep(LOCK_RETRY);
            }
            Err(e) => {
                return Err(UndoError::io(
                    format!("locking {}", path.display()),
                    std::io::Error::from(e),
                ));
            }
        }
    }
}

impl Drop for LockGuard {
    fn drop(&mut self) {
        let _ = flock(&self.file, FlockOperation::Unlock);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lock_is_exclusive_across_open_file_descriptions() {
        let dir = std::env::temp_dir().join(format!("undo-lock-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let guard = acquire(&dir).unwrap();

        let path = paths::lock_path(&dir);
        let other = OpenOptions::new().write(true).open(&path).unwrap();
        let res = flock(&other, FlockOperation::NonBlockingLockExclusive);
        assert!(res.is_err(), "second flock unexpectedly succeeded");

        drop(guard);
        flock(&other, FlockOperation::NonBlockingLockExclusive).unwrap();
        flock(&other, FlockOperation::Unlock).unwrap();
        let _ = std::fs::remove_dir_all(&dir);
    }
}
