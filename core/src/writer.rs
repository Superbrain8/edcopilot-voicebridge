use std::fmt;
use std::fs::{self, OpenOptions};
use std::io::{self, ErrorKind, Write};
use std::path::{Path, PathBuf};
use std::process;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{bail, Context};

const DEFAULT_TIMEOUT: Duration = Duration::from_secs(2);
const DEFAULT_POLL_INTERVAL: Duration = Duration::from_millis(25);

/// Windows `ERROR_SHARING_VIOLATION`: another process holds the file open.
const ERROR_SHARING_VIOLATION: i32 = 32;

/// Makes temp file names unique within this process.
static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

/// EDCoPilot did not consume the token before the confirm timeout.
///
/// `withdrawn` is true when the token was removed again and cannot run later.
/// It is false when the request file could not be cleared (it holds another
/// token, or the removal failed); the token possibly runs later.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NotConsumed {
    pub withdrawn: bool,
}

impl fmt::Display for NotConsumed {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.withdrawn {
            write!(f, "EDCoPilot did not consume the token; it was withdrawn")
        } else {
            write!(
                f,
                "EDCoPilot did not consume the token; the request file could not be cleared"
            )
        }
    }
}

impl std::error::Error for NotConsumed {}

/// Writes command tokens to the file EDCoPilot polls.
///
/// `send` on one `Writer` is safe to call from several threads: the calls run
/// one after the other. Use one `Writer` for each request file.
pub struct Writer {
    request_path: PathBuf,
    timeout: Duration,
    poll_interval: Duration,
    confirm_timeout: Option<Duration>,
    send_lock: Mutex<()>,
}

impl Writer {
    pub fn new(request_path: PathBuf) -> Writer {
        Writer::with_timeout(request_path, DEFAULT_TIMEOUT, DEFAULT_POLL_INTERVAL)
    }

    pub fn with_timeout(
        request_path: PathBuf,
        timeout: Duration,
        poll_interval: Duration,
    ) -> Writer {
        Writer {
            request_path,
            timeout,
            poll_interval,
            confirm_timeout: Some(timeout),
            send_lock: Mutex::new(()),
        }
    }

    /// Sets how long `send` waits for EDCoPilot to consume the token.
    /// `None`: `send` returns after the write and does not wait.
    pub fn confirm_timeout(mut self, confirm_timeout: Option<Duration>) -> Writer {
        self.confirm_timeout = confirm_timeout;
        self
    }

    /// Writes `token` as plain UTF-8 (no BOM, no newline). Never overwrites a
    /// token EDCoPilot has not consumed; waits up to the timeout, then errors.
    ///
    /// Then waits until EDCoPilot consumes the token. If it does not, the token
    /// is withdrawn, so that it cannot run later, and the error is `NotConsumed`.
    pub fn send(&self, token: &str) -> anyhow::Result<()> {
        if token.is_empty() {
            bail!("refusing to write an empty command token");
        }
        // An empty or relative path would put the temporary file and the
        // token in the current folder. The check comes before any file.
        if !self.request_path.is_absolute() {
            bail!(
                "the request file is not an absolute path: \"{}\"",
                self.request_path.display()
            );
        }
        // The lock holds no data, so a poisoned lock is safe to use.
        let _guard = self.send_lock.lock().unwrap_or_else(|e| e.into_inner());
        // Stage the complete token next to the request file, then link it into
        // place, so EDCoPilot never sees a partly written file.
        let tmp = self.temp_path();
        let result = write_new(&tmp, token).and_then(|()| self.publish(token, &tmp));
        let _ = fs::remove_file(&tmp);
        result?;
        match self.confirm_timeout {
            Some(wait) => self.await_consumption(token, wait),
            None => Ok(()),
        }
    }

    /// Waits until the request file is gone or empty. On timeout, removes the
    /// token if the file still holds it.
    fn await_consumption(&self, token: &str, wait: Duration) -> anyhow::Result<()> {
        let path = &self.request_path;
        let deadline = Instant::now() + wait;
        loop {
            match fs::metadata(path) {
                Err(e) if e.kind() == ErrorKind::NotFound => return Ok(()),
                Ok(meta) if meta.len() == 0 => return Ok(()),
                _ => {}
            }
            if Instant::now() >= deadline {
                break;
            }
            thread::sleep(self.poll_interval);
        }
        match self.withdraw_if_ours(token) {
            None => Ok(()),
            Some(withdrawn) => Err(NotConsumed { withdrawn })
                .with_context(|| format!("no consumer for {} within {:?}", path.display(), wait)),
        }
    }

    /// The step after the confirm deadline. `None`: the request file is gone
    /// or empty, so the token was consumed. `Some(true)`: the file held our
    /// token and it was removed. `Some(false)`: the file holds another token,
    /// or the removal failed.
    fn withdraw_if_ours(&self, token: &str) -> Option<bool> {
        let path = &self.request_path;
        match fs::read(path) {
            Err(e) if e.kind() == ErrorKind::NotFound => None,
            Ok(bytes) if bytes.is_empty() => None,
            // Remove only our own token. Another tool possibly wrote this one.
            Ok(bytes) if bytes == token.as_bytes() => match fs::remove_file(path) {
                Ok(()) => Some(true),
                Err(e) if e.kind() == ErrorKind::NotFound => None,
                Err(_) => Some(false),
            },
            _ => Some(false),
        }
    }

    fn temp_path(&self) -> PathBuf {
        let name = self.request_path.file_name().unwrap_or_default();
        let n = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
        self.request_path.with_file_name(format!(
            "{}.{}.{}.tmp",
            name.to_string_lossy(),
            process::id(),
            n
        ))
    }

    /// Publishes the staged token, waiting while an unread token is in place.
    fn publish(&self, token: &str, tmp: &Path) -> anyhow::Result<()> {
        let deadline = Instant::now() + self.timeout;
        let mut use_link = true;
        loop {
            self.remove_if_consumed()?;
            if self.try_publish(token, tmp, &mut use_link)? {
                return Ok(());
            }
            if Instant::now() >= deadline {
                bail!(
                    "previous token in {} was not consumed within {:?}",
                    self.request_path.display(),
                    self.timeout
                );
            }
            thread::sleep(self.poll_interval);
        }
    }

    /// Removes the request file when it is empty: EDCoPilot consumed it.
    fn remove_if_consumed(&self) -> anyhow::Result<()> {
        let path = &self.request_path;
        let len = match fs::metadata(path) {
            Ok(meta) => meta.len(),
            Err(e) if e.kind() == ErrorKind::NotFound || is_transient(&e) => return Ok(()),
            Err(e) => return Err(e).with_context(|| format!("reading {}", path.display())),
        };
        if len == 0 {
            match fs::remove_file(path) {
                Ok(()) => {}
                Err(e) if e.kind() == ErrorKind::NotFound || is_transient(&e) => {}
                Err(e) => return Err(e).with_context(|| format!("removing {}", path.display())),
            }
        }
        Ok(())
    }

    /// One publish attempt. `Ok(false)`: the request file is busy, wait and retry.
    fn try_publish(&self, token: &str, tmp: &Path, use_link: &mut bool) -> anyhow::Result<bool> {
        let path = &self.request_path;
        if *use_link {
            // hard_link is atomic and fails when the target exists.
            match fs::hard_link(tmp, path) {
                Ok(()) => return Ok(true),
                Err(e) if e.kind() == ErrorKind::AlreadyExists || is_transient(&e) => {
                    return Ok(false)
                }
                // For example a file system without hard links: write in place.
                Err(_) => *use_link = false,
            }
        }
        match OpenOptions::new().write(true).create_new(true).open(path) {
            Ok(mut file) => {
                file.write_all(token.as_bytes())
                    .with_context(|| format!("writing {}", path.display()))?;
                Ok(true)
            }
            Err(e) if e.kind() == ErrorKind::AlreadyExists || is_transient(&e) => Ok(false),
            Err(e) => Err(e).with_context(|| format!("creating {}", path.display())),
        }
    }
}

/// Creates `path` (it must not exist) and writes `token` to it.
fn write_new(path: &Path, token: &str) -> anyhow::Result<()> {
    OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .and_then(|mut file| file.write_all(token.as_bytes()))
        .with_context(|| format!("writing {}", path.display()))
}

/// Errors that clear by themselves while EDCoPilot consumes the previous token:
/// a delete-pending file, or (Windows) a sharing violation.
fn is_transient(e: &io::Error) -> bool {
    e.kind() == ErrorKind::PermissionDenied
        || (cfg!(windows) && e.raw_os_error() == Some(ERROR_SHARING_VIOLATION))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::time::Instant;

    fn writer(dir: &tempfile::TempDir, timeout_ms: u64) -> (Writer, PathBuf) {
        let path = dir.path().join("EDCoPilot.request.txt");
        let w = Writer::with_timeout(
            path.clone(),
            Duration::from_millis(timeout_ms),
            Duration::from_millis(10),
        )
        .confirm_timeout(None);
        (w, path)
    }

    #[test]
    fn test_writes_exact_bytes() {
        let dir = tempfile::tempdir().unwrap();
        let (w, path) = writer(&dir, 200);
        w.send("OpenChartLogPanel").unwrap();
        assert_eq!(fs::read(&path).unwrap(), b"OpenChartLogPanel");
    }

    #[test]
    fn test_quotes_spaces_and_utf8_are_byte_exact() {
        let dir = tempfile::tempdir().unwrap();
        let (w, path) = writer(&dir, 200);
        w.send("SailTo:\"TRV 48213\"").unwrap();
        assert_eq!(fs::read(&path).unwrap(), b"SailTo:\"TRV 48213\"");
        fs::remove_file(&path).unwrap();
        w.send("Zürich Ω").unwrap();
        assert_eq!(fs::read(&path).unwrap(), "Zürich Ω".as_bytes());
    }

    #[test]
    fn test_does_not_overwrite_unread() {
        let dir = tempfile::tempdir().unwrap();
        let (w, path) = writer(&dir, 200);
        fs::write(&path, "First").unwrap();
        assert!(w.send("Second").is_err());
        assert_eq!(fs::read(&path).unwrap(), b"First");
    }

    #[test]
    fn test_waits_then_writes_when_consumed() {
        let dir = tempfile::tempdir().unwrap();
        let (w, path) = writer(&dir, 2000);
        fs::write(&path, "First").unwrap();
        let p = path.clone();
        let h = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(100));
            fs::remove_file(p).unwrap();
        });
        w.send("Second").unwrap();
        h.join().unwrap();
        assert_eq!(fs::read(&path).unwrap(), b"Second");
    }

    #[test]
    fn test_empty_existing_file_is_replaced() {
        let dir = tempfile::tempdir().unwrap();
        let (w, path) = writer(&dir, 200);
        fs::write(&path, "").unwrap();
        w.send("Token").unwrap();
        assert_eq!(fs::read(&path).unwrap(), b"Token");
    }

    #[test]
    fn test_missing_parent_dir_fails_fast() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("missing").join("EDCoPilot.request.txt");
        let w = Writer::with_timeout(path, Duration::from_secs(5), Duration::from_millis(10));
        let start = Instant::now();
        assert!(w.send("Token").is_err());
        assert!(start.elapsed() < Duration::from_secs(1));
    }

    #[test]
    fn test_empty_token_is_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let (w, path) = writer(&dir, 200);
        assert!(w.send("").is_err());
        assert!(!path.exists());
    }

    #[test]
    fn test_empty_or_relative_request_path_is_refused_before_any_file() {
        for path in ["", "vb-writer-test-relative/EDCoPilot.request.txt"] {
            let w = Writer::with_timeout(
                PathBuf::from(path),
                Duration::from_millis(50),
                Duration::from_millis(10),
            );
            let error = w.send("Guide:Panel:ChartLog").unwrap_err().to_string();
            assert!(error.contains("not an absolute path"), "{path:?}: {error}");
        }
        // The relative folder is never made in the current folder.
        assert!(!Path::new("vb-writer-test-relative").exists());
    }

    fn dir_entries(dir: &tempfile::TempDir) -> Vec<String> {
        let mut names: Vec<String> = fs::read_dir(dir.path())
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
    }

    #[test]
    fn test_no_temp_file_left_after_success() {
        let dir = tempfile::tempdir().unwrap();
        let (w, _) = writer(&dir, 200);
        w.send("Token").unwrap();
        assert_eq!(dir_entries(&dir), ["EDCoPilot.request.txt"]);
    }

    #[test]
    fn test_no_temp_file_left_after_timeout() {
        let dir = tempfile::tempdir().unwrap();
        let (w, path) = writer(&dir, 100);
        fs::write(&path, "First").unwrap();
        assert!(w.send("Second").is_err());
        assert_eq!(dir_entries(&dir), ["EDCoPilot.request.txt"]);
    }

    fn confirming_writer(dir: &tempfile::TempDir, confirm_ms: u64) -> (Writer, PathBuf) {
        let path = dir.path().join("EDCoPilot.request.txt");
        let w = Writer::with_timeout(
            path.clone(),
            Duration::from_millis(200),
            Duration::from_millis(5),
        )
        .confirm_timeout(Some(Duration::from_millis(confirm_ms)));
        (w, path)
    }

    /// Acts as EDCoPilot: waits for the request file, then runs `consume` on it.
    fn consumer(
        path: PathBuf,
        consume: impl FnOnce(&Path) + Send + 'static,
    ) -> std::thread::JoinHandle<Vec<u8>> {
        std::thread::spawn(move || loop {
            if let Ok(bytes) = fs::read(&path) {
                if !bytes.is_empty() {
                    consume(&path);
                    return bytes;
                }
            }
            std::thread::sleep(Duration::from_millis(2));
        })
    }

    #[test]
    fn test_confirmed_when_consumer_deletes_the_file() {
        let dir = tempfile::tempdir().unwrap();
        let (w, path) = confirming_writer(&dir, 2000);
        let h = consumer(path.clone(), |p| fs::remove_file(p).unwrap());
        w.send("OpenChartLogPanel").unwrap();
        assert_eq!(h.join().unwrap(), b"OpenChartLogPanel");
        assert!(!path.exists());
    }

    #[test]
    fn test_confirmed_when_consumer_empties_the_file() {
        let dir = tempfile::tempdir().unwrap();
        let (w, path) = confirming_writer(&dir, 2000);
        let h = consumer(path.clone(), |p| fs::write(p, "").unwrap());
        w.send("Token").unwrap();
        h.join().unwrap();
    }

    #[test]
    fn test_unconsumed_token_is_withdrawn() {
        let dir = tempfile::tempdir().unwrap();
        let (w, path) = confirming_writer(&dir, 100);
        let err = w.send("Token").unwrap_err();
        let not_consumed = err
            .downcast_ref::<NotConsumed>()
            .expect("NotConsumed error");
        assert!(not_consumed.withdrawn);
        assert!(
            !path.exists(),
            "a stale token must not stay in the request file"
        );
        assert_eq!(dir_entries(&dir), Vec::<String>::new());
    }

    /// Our token was consumed and another tool wrote its own token before the
    /// deadline: the step after the deadline must not remove that token.
    #[test]
    fn test_foreign_token_is_not_withdrawn() {
        let dir = tempfile::tempdir().unwrap();
        let (w, path) = confirming_writer(&dir, 100);
        fs::write(&path, "Other").unwrap();
        assert_eq!(w.withdraw_if_ours("Token"), Some(false));
        assert_eq!(fs::read(&path).unwrap(), b"Other");
    }

    #[test]
    fn test_withdraw_if_ours_removes_our_token() {
        let dir = tempfile::tempdir().unwrap();
        let (w, path) = confirming_writer(&dir, 100);
        fs::write(&path, "Token").unwrap();
        assert_eq!(w.withdraw_if_ours("Token"), Some(true));
        assert!(!path.exists());
    }

    #[test]
    fn test_withdraw_if_ours_sees_a_consumed_file() {
        let dir = tempfile::tempdir().unwrap();
        let (w, path) = confirming_writer(&dir, 100);
        assert_eq!(w.withdraw_if_ours("Token"), None);
        fs::write(&path, "").unwrap();
        assert_eq!(w.withdraw_if_ours("Token"), None);
    }

    #[test]
    fn test_default_writer_confirms() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("EDCoPilot.request.txt");
        let w = Writer::with_timeout(
            path.clone(),
            Duration::from_millis(100),
            Duration::from_millis(5),
        );
        assert!(w.send("Token").is_err());
        assert!(!path.exists());
    }

    #[test]
    fn test_concurrent_sends_deliver_every_token_once() {
        let dir = tempfile::tempdir().unwrap();
        let (w, path) = confirming_writer(&dir, 2000);
        let w = std::sync::Arc::new(w);
        let seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let (seen2, stop2, p) = (seen.clone(), stop.clone(), path.clone());
        let edcopilot = std::thread::spawn(move || {
            while !stop2.load(Ordering::Relaxed) {
                if let Ok(bytes) = fs::read(&p) {
                    if !bytes.is_empty() && fs::remove_file(&p).is_ok() {
                        seen2
                            .lock()
                            .unwrap()
                            .push(String::from_utf8(bytes).unwrap());
                    }
                }
                std::thread::sleep(Duration::from_millis(3));
            }
        });
        let senders: Vec<_> = (0..4)
            .map(|i| {
                let w = w.clone();
                std::thread::spawn(move || w.send(&format!("Token{i}")).unwrap())
            })
            .collect();
        for s in senders {
            s.join().unwrap();
        }
        stop.store(true, Ordering::Relaxed);
        edcopilot.join().unwrap();
        let mut seen = seen.lock().unwrap().clone();
        seen.sort();
        assert_eq!(seen, ["Token0", "Token1", "Token2", "Token3"]);
    }

    #[test]
    fn test_is_transient() {
        assert!(is_transient(&io::Error::from(ErrorKind::PermissionDenied)));
        assert!(!is_transient(&io::Error::from(ErrorKind::NotFound)));
        assert!(!is_transient(&io::Error::from(ErrorKind::AlreadyExists)));
        assert_eq!(
            is_transient(&io::Error::from_raw_os_error(32)),
            cfg!(windows)
        );
    }

    /// While EDCoPilot holds the consumed (empty) file open, removing it fails with
    /// a sharing violation (OS error 32). `send` must wait, not give up.
    #[cfg(windows)]
    #[test]
    fn test_sharing_violation_is_retried() {
        use std::os::windows::fs::OpenOptionsExt;
        let dir = tempfile::tempdir().unwrap();
        let (w, path) = writer(&dir, 2000);
        fs::write(&path, "").unwrap();
        let handle = fs::OpenOptions::new()
            .read(true)
            .share_mode(0)
            .open(&path)
            .unwrap();
        let h = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(100));
            drop(handle);
        });
        w.send("Second").unwrap();
        h.join().unwrap();
        assert_eq!(fs::read(&path).unwrap(), b"Second");
    }
}
