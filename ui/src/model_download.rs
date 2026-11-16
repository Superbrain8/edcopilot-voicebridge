//! The download of the local model.
//!
//! The address and the hash of the archive are constants. The program never
//! takes an address from the web part or from the configuration. The steps:
//! download to `<models>/<name>.part`, check the size and the SHA-256 hash,
//! unpack into `<models>/<name>.unpack`, check the model files, then rename
//! the model folder to `<models>/<name>`. A failed step never leaves a
//! folder at `<models>/<name>` that the program then tries to load.

use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::Arc;
use std::time::{Duration, Instant};

use sha2::{Digest, Sha256};

/// One model archive that the program can download.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ModelSource {
    /// The name of the model folder, and of the folder in the archive.
    pub name: &'static str,
    /// The HTTPS address of the `.tar.bz2` archive.
    pub url: &'static str,
    /// The SHA-256 hash of the archive, in lower case hex.
    pub sha256: &'static str,
    pub archive_bytes: u64,
    /// The sum of the file sizes in the archive.
    pub unpacked_bytes: u64,
}

/// NVIDIA Parakeet TDT 0.6b v3, int8, from the sherpa-onnx release
/// `asr-models`. A download of the archive set these values. The model has
/// the license CC-BY-4.0 (the `README.md` in the archive gives it).
pub const PARAKEET_V3_INT8: ModelSource = ModelSource {
    name: "sherpa-onnx-nemo-parakeet-tdt-0.6b-v3-int8",
    url: "https://github.com/k2-fsa/sherpa-onnx/releases/download/asr-models/sherpa-onnx-nemo-parakeet-tdt-0.6b-v3-int8.tar.bz2",
    sha256: "1495e91b59d7fc53ddca91f3431758ec99f8fa253fdb0e1e851be0f72b14b0f9",
    archive_bytes: 487_211_443,
    unpacked_bytes: 671_341_917,
};

/// The files that the local engine loads from the model folder.
pub const MODEL_FILES: [&str; 4] = [
    "encoder.int8.onnx",
    "decoder.int8.onnx",
    "joiner.int8.onnx",
    "tokens.txt",
];

/// The hosts that the download can use: the release page and the hosts
/// that GitHub redirects to. `github.com` gives HTTP 302 to
/// `release-assets.githubusercontent.com`. `objects.githubusercontent.com`
/// is the older host.
pub const ALLOWED_HOSTS: [&str; 3] = [
    "github.com",
    "release-assets.githubusercontent.com",
    "objects.githubusercontent.com",
];

/// The text of a cancelled download.
pub const CANCELLED: &str = "the download was cancelled";
/// The text of an archive with a wrong hash.
pub const DAMAGED: &str = "the downloaded file is damaged";
/// The text when the size of the file on the server is not the pinned size.
pub const CHANGED: &str = "the model archive on the server changed, a program update is necessary";
/// The text of a download while another download runs, also in another
/// instance of the program.
pub const ALREADY_RUNNING: &str = "a model download is already running";
/// The most entries that the archive can have.
const MAX_ENTRIES: usize = 64;
/// The unpacked files can be this much larger than `unpacked_bytes`.
const UNPACK_MARGIN: u64 = 64 * 1024;

/// The steps of a download, in this order. A download ends with `Done`,
/// `Failed`, or `Cancelled`.
#[derive(Debug, Clone, PartialEq)]
pub enum DownloadEvent {
    Progress { done: u64, total: u64 },
    Verifying,
    Unpacking,
    Done { model_dir: PathBuf },
    Failed(String),
    Cancelled,
}

/// The answer of a server to a request.
pub struct Fetched {
    /// The byte of the archive where `body` starts. It is `from_byte` when
    /// the server sends the rest of the file, and 0 when the server sends
    /// the whole file.
    pub start: u64,
    /// The size of the whole file on the server, when the server tells it
    /// (`Content-Length` of a full answer, or the total of `Content-Range`).
    pub total: Option<u64>,
    pub body: Box<dyn Read + Send>,
}

/// Gets an archive. `from_byte` above 0 asks for the rest of the file from
/// that byte. A server can send the whole file instead.
pub trait Fetch {
    fn get(&self, url: &str, from_byte: u64) -> Result<Fetched, String>;
}

impl<T: Fetch + ?Sized> Fetch for &T {
    fn get(&self, url: &str, from_byte: u64) -> Result<Fetched, String> {
        (**self).get(url, from_byte)
    }
}

/// The free bytes for this user on the disk of `dir`. `None` when the
/// system does not tell it.
#[cfg(windows)]
pub fn free_space(dir: &Path) -> Option<u64> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::GetDiskFreeSpaceExW;
    let wide: Vec<u16> = dir.as_os_str().encode_wide().chain(Some(0)).collect();
    let mut free = 0u64;
    // SAFETY: `wide` ends with 0 and lives during the call. The function
    // writes only `free`. The other pointers are null, which it permits.
    let ok = unsafe {
        GetDiskFreeSpaceExW(
            wide.as_ptr(),
            &mut free,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
        )
    };
    (ok != 0).then_some(free)
}

/// The free bytes on the disk of `dir`. This build does not know them.
#[cfg(not(windows))]
pub fn free_space(_dir: &Path) -> Option<u64> {
    None
}

/// True when `dir` has each model file as a file that is not empty.
pub fn is_complete_model(dir: &Path) -> bool {
    MODEL_FILES
        .iter()
        .all(|name| fs::metadata(dir.join(name)).is_ok_and(|m| m.is_file() && m.len() > 0))
}

/// Downloads the model into `models_dir`. See the module text for the
/// steps. `cancel` stops the download at the next block. The `.part` file
/// stays after a cancel or a broken connection, so that the next call
/// continues from its size.
#[allow(dead_code)] // The commands use `download_model_with`. The real download test uses this.
pub fn download_model(
    source: &ModelSource,
    models_dir: &Path,
    fetch: &dyn Fetch,
    cancel: &AtomicBool,
    on_event: &dyn Fn(DownloadEvent),
) -> Result<PathBuf, String> {
    download_model_with(source, models_dir, fetch, cancel, on_event, &free_space)
}

/// `download_model` with the function that gives the free space.
pub fn download_model_with(
    source: &ModelSource,
    models_dir: &Path,
    fetch: &dyn Fetch,
    cancel: &AtomicBool,
    on_event: &dyn Fn(DownloadEvent),
    free: &dyn Fn(&Path) -> Option<u64>,
) -> Result<PathBuf, String> {
    let result = run(source, models_dir, fetch, cancel, on_event, free);
    match &result {
        Ok(dir) => on_event(DownloadEvent::Done {
            model_dir: dir.clone(),
        }),
        Err(text) if text == CANCELLED => on_event(DownloadEvent::Cancelled),
        Err(text) => on_event(DownloadEvent::Failed(text.clone())),
    }
    result
}

fn run(
    source: &ModelSource,
    models_dir: &Path,
    fetch: &dyn Fetch,
    cancel: &AtomicBool,
    on_event: &dyn Fn(DownloadEvent),
    free: &dyn Fn(&Path) -> Option<u64>,
) -> Result<PathBuf, String> {
    fs::create_dir_all(models_dir).map_err(|e| {
        format!(
            "cannot make the models folder {}: {e}",
            models_dir.display()
        )
    })?;
    // One download for each models folder, also across program instances.
    let _lock = ModelLock::acquire(models_dir, source.name)?;
    // `format!` and not `with_extension`: the name has a dot ("0.6b").
    let path = |suffix: &str| models_dir.join(format!("{}.{suffix}", source.name));
    let target = models_dir.join(source.name);
    let (part, unpack, old) = (path("part"), path("unpack"), path("old"));
    // The leftovers of a crash. An unpack folder is never complete work.
    remove_dir(&unpack)?;
    if !target.exists() && is_complete_model(&old) {
        // A crash between the two renames of `place`. The old folder is a
        // complete model: it comes back, because it can load now.
        fs::rename(&old, &target).map_err(|e| format!("cannot move {}: {e}", old.display()))?;
    }
    remove_dir(&old)?;
    if is_complete_model(&target) {
        remove_file(&part)?;
        return Ok(target);
    }
    let mut have = fs::metadata(&part).map(|m| m.len()).unwrap_or(0);
    if have > source.archive_bytes {
        remove_file(&part)?;
        have = 0;
    }
    check_space(source, models_dir, have, free)?;
    if have < source.archive_bytes {
        fetch_part(source, &part, have, fetch, cancel, on_event)?;
    }
    on_event(DownloadEvent::Verifying);
    verify(source, &part, cancel)?;
    on_event(DownloadEvent::Unpacking);
    let result = unpack_and_place(source, &part, &unpack, &target, &old, cancel);
    // The unpack folder never stays, also after an error.
    let _ = fs::remove_dir_all(&unpack);
    result?;
    remove_file(&part)?;
    Ok(target)
}

fn remove_dir(path: &Path) -> Result<(), String> {
    match fs::remove_dir_all(path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(format!("cannot remove {}: {e}", path.display())),
    }
}

fn remove_file(path: &Path) -> Result<(), String> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(format!("cannot remove {}: {e}", path.display())),
    }
}

fn megabytes(bytes: u64) -> u64 {
    bytes.div_ceil(1_000_000)
}

/// The rest of the archive and the unpacked files must fit on the disk.
fn check_space(
    source: &ModelSource,
    models_dir: &Path,
    have: u64,
    free: &dyn Fn(&Path) -> Option<u64>,
) -> Result<(), String> {
    let needed = source.archive_bytes - have + source.unpacked_bytes;
    match free(models_dir) {
        Some(free) if free <= needed => Err(format!(
            "not enough free disk space in {}: the download needs {} MB, the disk has {} MB",
            models_dir.display(),
            megabytes(needed),
            free / 1_000_000
        )),
        _ => Ok(()),
    }
}

/// The size of one block of the download and of the unpack.
const BLOCK: usize = 64 * 1024;
/// The interval of the progress events, in bytes.
const PROGRESS_STEP: u64 = 1024 * 1024;

/// Appends the rest of the archive to the `.part` file.
fn fetch_part(
    source: &ModelSource,
    part: &Path,
    have: u64,
    fetch: &dyn Fetch,
    cancel: &AtomicBool,
    on_event: &dyn Fn(DownloadEvent),
) -> Result<(), String> {
    if cancel.load(Ordering::SeqCst) {
        return Err(CANCELLED.to_string());
    }
    let total = source.archive_bytes;
    let fetched = fetch.get(source.url, have)?;
    if fetched.total.is_some_and(|size| size != total) {
        // A new file on the server. The `.part` file can still be a part of
        // the pinned file, so it stays.
        return Err(CHANGED.to_string());
    }
    let mut done = if fetched.start == have {
        have
    } else if fetched.start == 0 {
        // The server sends the whole file: start again.
        0
    } else {
        return Err(format!(
            "the server sent the file from byte {} and not from byte {have}",
            fetched.start
        ));
    };
    let mut file = OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(false)
        .open(part)
        .map_err(|e| format!("cannot write {}: {e}", part.display()))?;
    file.set_len(done)
        .and_then(|()| io::Seek::seek(&mut file, io::SeekFrom::Start(done)))
        .map_err(|e| format!("cannot write {}: {e}", part.display()))?;
    on_event(DownloadEvent::Progress { done, total });
    let mut body = fetched.body;
    let mut buf = vec![0u8; BLOCK];
    let mut reported = done;
    while done < total {
        if cancel.load(Ordering::SeqCst) {
            return Err(CANCELLED.to_string());
        }
        let want = usize::try_from(total - done).map_or(BLOCK, |rest| rest.min(BLOCK));
        let n = match body.read(&mut buf[..want]) {
            Ok(0) => break,
            Ok(n) => n,
            Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
            Err(_) if cancel.load(Ordering::SeqCst) => return Err(CANCELLED.to_string()),
            Err(e) => {
                return Err(format!(
                    "the download stopped at {} of {} MB ({e}), try again to continue",
                    done / 1_000_000,
                    megabytes(total)
                ))
            }
        };
        file.write_all(&buf[..n])
            .map_err(|e| format!("cannot write {}: {e}", part.display()))?;
        done += n as u64;
        if done - reported >= PROGRESS_STEP || done == total {
            on_event(DownloadEvent::Progress { done, total });
            reported = done;
        }
    }
    file.sync_all()
        .map_err(|e| format!("cannot write {}: {e}", part.display()))?;
    if done < total {
        return Err(format!(
            "the download stopped at {} of {} MB, try again to continue",
            done / 1_000_000,
            megabytes(total)
        ));
    }
    Ok(())
}

/// Checks the size and the hash of the `.part` file. A wrong file is
/// deleted, so that the next try starts again.
fn verify(source: &ModelSource, part: &Path, cancel: &AtomicBool) -> Result<(), String> {
    let mut file = File::open(part).map_err(|e| format!("cannot read {}: {e}", part.display()))?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; BLOCK];
    let mut size = 0u64;
    loop {
        if cancel.load(Ordering::SeqCst) {
            return Err(CANCELLED.to_string());
        }
        let n = file
            .read(&mut buf)
            .map_err(|e| format!("cannot read {}: {e}", part.display()))?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
        size += n as u64;
    }
    drop(file);
    let hash: String = hasher
        .finalize()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    if size != source.archive_bytes || hash != source.sha256 {
        remove_file(part)?;
        return Err(DAMAGED.to_string());
    }
    Ok(())
}

/// The path of an archive entry below the unpack folder. `None` for a path
/// that is absolute, that has `..` or a drive, or that has a `:` (a
/// Windows file stream).
fn safe_path(path: &Path) -> Option<PathBuf> {
    let mut out = PathBuf::new();
    for part in path.components() {
        match part {
            Component::Normal(name) => {
                if name.to_str().is_none_or(|n| n.contains(':')) {
                    return None;
                }
                out.push(name);
            }
            Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => return None,
        }
    }
    Some(out)
}

/// Unpacks the archive into `unpack`, checks the model files, and renames
/// the model folder to `target`.
fn unpack_and_place(
    source: &ModelSource,
    part: &Path,
    unpack: &Path,
    target: &Path,
    old: &Path,
    cancel: &AtomicBool,
) -> Result<(), String> {
    if unpack.exists() {
        fs::remove_dir_all(unpack)
            .map_err(|e| format!("cannot remove {}: {e}", unpack.display()))?;
    }
    fs::create_dir_all(unpack).map_err(|e| format!("cannot make {}: {e}", unpack.display()))?;
    let file = File::open(part).map_err(|e| format!("cannot read {}: {e}", part.display()))?;
    unpack_archive(file, unpack, cancel, source.unpacked_bytes + UNPACK_MARGIN)?;
    let inner = unpack.join(source.name);
    let model = if inner.is_dir() {
        inner
    } else {
        unpack.to_path_buf()
    };
    if !is_complete_model(&model) {
        return Err(format!(
            "the archive does not have the model files ({})",
            MODEL_FILES.join(", ")
        ));
    }
    place(&model, target, old)
}

/// The lock of the download in one models folder. The lock file
/// `<name>.lock` holds no data: only the lock of the operating system on it
/// counts, as for the session lock of the runtime. The system releases that
/// lock when the file closes, also when the process stops without a clean
/// exit. Thus a lock never stays after a crash. The file itself stays.
pub struct ModelLock {
    _file: File,
}

impl ModelLock {
    pub fn acquire(models_dir: &Path, name: &str) -> Result<ModelLock, String> {
        let path = models_dir.join(format!("{name}.lock"));
        let file = OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(false)
            .open(&path)
            .map_err(|e| format!("cannot open the download lock {}: {e}", path.display()))?;
        match file.try_lock() {
            Ok(()) => Ok(ModelLock { _file: file }),
            Err(std::fs::TryLockError::WouldBlock) => Err(ALREADY_RUNNING.to_string()),
            Err(std::fs::TryLockError::Error(e)) => Err(format!(
                "cannot lock the download lock {}: {e}",
                path.display()
            )),
        }
    }
}

/// Unpacks a `.tar.bz2` stream into `dest`. Only files and folders are
/// permitted, and each path must stay below `dest`. The archive can have at
/// most `MAX_ENTRIES` entries and `max_bytes` bytes of files. The hash check
/// comes first, so these limits are a second guard only.
fn unpack_archive(
    stream: impl Read,
    dest: &Path,
    cancel: &AtomicBool,
    max_bytes: u64,
) -> Result<(), String> {
    let broken = |e: io::Error| format!("the archive cannot be unpacked: {e}");
    let mut archive = tar::Archive::new(bzip2::read::BzDecoder::new(io::BufReader::new(stream)));
    let mut buf = vec![0u8; BLOCK];
    let mut written = 0u64;
    for (count, entry) in archive.entries().map_err(broken)?.enumerate() {
        if cancel.load(Ordering::SeqCst) {
            return Err(CANCELLED.to_string());
        }
        if count >= MAX_ENTRIES {
            return Err(format!(
                "the archive has too many entries (more than {MAX_ENTRIES})"
            ));
        }
        let mut entry = entry.map_err(broken)?;
        let kind = entry.header().entry_type();
        if kind.is_pax_global_extensions() {
            // Data about the archive. It makes no file.
            continue;
        }
        let raw = entry.path().map_err(broken)?.into_owned();
        let Some(relative) = safe_path(&raw) else {
            return Err(format!(
                "the archive has an entry outside its folder: {}",
                raw.display()
            ));
        };
        let path = dest.join(&relative);
        if kind.is_dir() {
            fs::create_dir_all(&path)
                .map_err(|e| format!("cannot make {}: {e}", path.display()))?;
            continue;
        }
        if !kind.is_file() {
            return Err(format!(
                "the archive has an entry that is not a file or a folder: {}",
                raw.display()
            ));
        }
        if relative.as_os_str().is_empty() {
            return Err("the archive has a file without a name".to_string());
        }
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .map_err(|e| format!("cannot make {}: {e}", parent.display()))?;
        }
        let mut out =
            File::create(&path).map_err(|e| format!("cannot write {}: {e}", path.display()))?;
        loop {
            if cancel.load(Ordering::SeqCst) {
                return Err(CANCELLED.to_string());
            }
            let n = entry.read(&mut buf).map_err(broken)?;
            if n == 0 {
                break;
            }
            written += n as u64;
            if written > max_bytes {
                return Err("the archive is larger than expected".to_string());
            }
            out.write_all(&buf[..n])
                .map_err(|e| format!("cannot write {}: {e}", path.display()))?;
        }
        // The file is on the disk before the rename of the folder.
        out.sync_all()
            .map_err(|e| format!("cannot write {}: {e}", path.display()))?;
    }
    Ok(())
}

/// Renames `model` to `target`. A folder at `target` that is not complete
/// goes away first. A failure puts it back.
fn place(model: &Path, target: &Path, old: &Path) -> Result<(), String> {
    let had_old = target.exists();
    if had_old {
        if old.exists() {
            fs::remove_dir_all(old).map_err(|e| format!("cannot remove {}: {e}", old.display()))?;
        }
        fs::rename(target, old).map_err(|e| format!("cannot move {}: {e}", target.display()))?;
    }
    if let Err(e) = fs::rename(model, target) {
        if had_old {
            let _ = fs::rename(old, target);
        }
        return Err(format!(
            "cannot move the model to {}: {e}",
            target.display()
        ));
    }
    if had_old {
        let _ = fs::remove_dir_all(old);
    }
    Ok(())
}

/// True when `url` is an HTTPS address of a permitted host.
pub fn url_permitted(url: &url::Url, hosts: &[&str], https_only: bool) -> bool {
    let scheme_ok = url.scheme() == "https" || (!https_only && url.scheme() == "http");
    scheme_ok
        && url.username().is_empty()
        && url.password().is_none()
        && url
            .host_str()
            .is_some_and(|host| hosts.iter().any(|h| host.eq_ignore_ascii_case(h)))
}

/// The User-Agent of the download. It names no person and no computer.
const USER_AGENT: &str = "voicebridge-model-download";
/// The longest wait for the answer of the server and for the next block.
const IDLE_LIMIT: Duration = Duration::from_secs(60);
/// The interval of the checks of the cancel flag during a wait.
const CANCEL_POLL: Duration = Duration::from_millis(200);
/// The most redirects of one request.
const MAX_REDIRECTS: usize = 10;
/// The answer to a range outside the file (HTTP 416).
const RANGE_REFUSED: &str = "the server refused the rest of the file";

/// The download over HTTPS. A thread reads the network, so that a cancel
/// works also while the server sends nothing.
pub struct HttpFetch {
    client: reqwest::blocking::Client,
    hosts: Vec<&'static str>,
    https_only: bool,
    cancel: Arc<AtomicBool>,
    idle_limit: Duration,
}

/// One message of the reader thread.
enum Chunk {
    /// The start byte of the body and the total size of the file.
    Head(Result<(u64, Option<u64>), String>),
    Data(Vec<u8>),
    End,
    Error(String),
}

impl HttpFetch {
    /// The real download: HTTPS, and only `ALLOWED_HOSTS`.
    pub fn new(cancel: Arc<AtomicBool>) -> Result<HttpFetch, String> {
        HttpFetch::with_rules(&ALLOWED_HOSTS, true, cancel, IDLE_LIMIT)
    }

    fn with_rules(
        hosts: &[&'static str],
        https_only: bool,
        cancel: Arc<AtomicBool>,
        idle_limit: Duration,
    ) -> Result<HttpFetch, String> {
        let allowed = hosts.to_vec();
        let policy = reqwest::redirect::Policy::custom(move |attempt| {
            if attempt.previous().len() >= MAX_REDIRECTS {
                attempt.error("too many redirects")
            } else if url_permitted(attempt.url(), &allowed, https_only) {
                attempt.follow()
            } else {
                attempt.error("a redirect to an address that is not permitted")
            }
        });
        let client = reqwest::blocking::Client::builder()
            .user_agent(USER_AGENT)
            .redirect(policy)
            .https_only(https_only)
            .connect_timeout(Duration::from_secs(30))
            // The system ends a dead connection, so that the reader thread
            // and its socket do not stay after a stalled download.
            .tcp_keepalive(Duration::from_secs(30))
            // The whole download takes minutes. The reader thread watches
            // the time between the blocks.
            .timeout(None)
            .build()
            .map_err(|_| "the HTTP client cannot start".to_string())?;
        Ok(HttpFetch {
            client,
            hosts: hosts.to_vec(),
            https_only,
            cancel,
            idle_limit,
        })
    }
}

/// A plain text for an error of the HTTP client. It holds no address.
fn http_error(error: &reqwest::Error) -> String {
    if error.is_redirect() {
        "the server sent a redirect to an address that is not permitted".to_string()
    } else if error.is_timeout() {
        "the server did not answer in time".to_string()
    } else if error.is_connect() {
        "cannot connect to the server, check the network".to_string()
    } else {
        "the connection to the server failed".to_string()
    }
}

/// The total size `T` of a `Content-Range: bytes N-M/T` value. `None` for
/// `*`.
fn range_total(value: &str) -> Option<u64> {
    value.rsplit_once('/')?.1.trim().parse().ok()
}

/// The first byte of a `Content-Range: bytes N-M/T` value.
fn range_start(value: &str) -> Option<u64> {
    value
        .strip_prefix("bytes ")?
        .split('-')
        .next()?
        .trim()
        .parse()
        .ok()
}

impl Fetch for HttpFetch {
    fn get(&self, url: &str, from_byte: u64) -> Result<Fetched, String> {
        let parsed =
            url::Url::parse(url).map_err(|_| "the model address is not valid".to_string())?;
        if !url_permitted(&parsed, &self.hosts, self.https_only) {
            return Err("the model address is not permitted".to_string());
        }
        let mut request = self.client.get(parsed);
        if from_byte > 0 {
            request = request.header(reqwest::header::RANGE, format!("bytes={from_byte}-"));
        }
        let (tx, rx) = mpsc::sync_channel::<Chunk>(16);
        std::thread::spawn(move || read_network(request, from_byte, tx));
        let mut reader = ChannelReader {
            rx,
            rest: Vec::new(),
            at: 0,
            cancel: Arc::clone(&self.cancel),
            idle_limit: self.idle_limit,
            ended: false,
        };
        let (start, total) = match reader.next_chunk()? {
            Some(Chunk::Head(Err(text))) if text == RANGE_REFUSED && from_byte > 0 => {
                // The `.part` file does not fit the file of the server.
                return self.get(url, 0);
            }
            Some(Chunk::Head(head)) => head?,
            _ => return Err("the server did not answer".to_string()),
        };
        Ok(Fetched {
            start,
            total,
            body: Box::new(reader),
        })
    }
}

/// The reader thread: sends the request, then the blocks of the body.
fn read_network(
    request: reqwest::blocking::RequestBuilder,
    from_byte: u64,
    tx: mpsc::SyncSender<Chunk>,
) {
    let response = match request.send() {
        Ok(response) => response,
        Err(error) => {
            let _ = tx.send(Chunk::Head(Err(http_error(&error))));
            return;
        }
    };
    let status = response.status().as_u16();
    let header = |name| {
        response
            .headers()
            .get(name)
            .and_then(|v: &reqwest::header::HeaderValue| v.to_str().ok())
            .map(str::to_string)
    };
    let head = match status {
        200 => Ok((
            0,
            header(reqwest::header::CONTENT_LENGTH).and_then(|v| v.trim().parse().ok()),
        )),
        206 => {
            let range = header(reqwest::header::CONTENT_RANGE);
            let start = range.as_deref().and_then(range_start);
            match start {
                Some(start) if start == from_byte => {
                    Ok((start, range.as_deref().and_then(range_total)))
                }
                _ => Err("the server sent a part of the file that was not asked for".to_string()),
            }
        }
        // The range is outside the file: the `.part` file is wrong.
        416 => Err(RANGE_REFUSED.to_string()),
        _ => Err(format!("the server answered with HTTP status {status}")),
    };
    let ok = head.is_ok();
    if tx.send(Chunk::Head(head)).is_err() || !ok {
        return;
    }
    let mut response = response;
    loop {
        let mut buf = vec![0u8; BLOCK];
        match response.read(&mut buf) {
            Ok(0) => {
                let _ = tx.send(Chunk::End);
                return;
            }
            Ok(n) => {
                buf.truncate(n);
                if tx.send(Chunk::Data(buf)).is_err() {
                    // The download stopped reading: end the thread.
                    return;
                }
            }
            Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
            Err(_) => {
                let _ = tx.send(Chunk::Error("the connection broke".to_string()));
                return;
            }
        }
    }
}

/// The body of an `HttpFetch` answer. It reads the blocks of the reader
/// thread. A cancel or a long wait gives an error.
struct ChannelReader {
    rx: mpsc::Receiver<Chunk>,
    rest: Vec<u8>,
    at: usize,
    cancel: Arc<AtomicBool>,
    idle_limit: Duration,
    ended: bool,
}

impl ChannelReader {
    /// The next message. `None` when the thread ended without a message.
    fn next_chunk(&mut self) -> Result<Option<Chunk>, String> {
        let begin = Instant::now();
        loop {
            if self.cancel.load(Ordering::SeqCst) {
                return Err(CANCELLED.to_string());
            }
            match self.rx.recv_timeout(CANCEL_POLL) {
                Ok(chunk) => return Ok(Some(chunk)),
                Err(mpsc::RecvTimeoutError::Disconnected) => return Ok(None),
                Err(mpsc::RecvTimeoutError::Timeout) => {
                    if begin.elapsed() >= self.idle_limit {
                        return Err("the server stopped sending data".to_string());
                    }
                }
            }
        }
    }
}

impl Read for ChannelReader {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        while self.at >= self.rest.len() {
            if self.ended {
                return Ok(0);
            }
            match self.next_chunk().map_err(io::Error::other)? {
                Some(Chunk::Data(data)) => {
                    self.rest = data;
                    self.at = 0;
                }
                Some(Chunk::End) => self.ended = true,
                Some(Chunk::Error(text)) => return Err(io::Error::other(text)),
                Some(Chunk::Head(_)) | None => {
                    return Err(io::Error::other("the connection broke"));
                }
            }
        }
        let n = buf.len().min(self.rest.len() - self.at);
        buf[..n].copy_from_slice(&self.rest[self.at..self.at + n]);
        self.at += n;
        Ok(n)
    }
}

#[cfg(test)]
pub mod tests {
    use super::*;
    use std::sync::Mutex;

    use crate::state::lock;

    /// The content of each model file in the test archive.
    const CONTENT: &[u8] = b"model data";
    const NAME: &str = "test-model-0.6b";

    /// One entry of a test archive.
    pub enum Entry<'a> {
        File(&'a str, &'a [u8]),
        Dir(&'a str),
        Symlink(&'a str, &'a str),
        /// A file with a name that `tar::Header::set_path` refuses.
        RawName(&'a str, &'a [u8]),
    }

    /// A `.tar.bz2` archive with the entries.
    pub fn archive(entries: &[Entry]) -> Vec<u8> {
        let encoder = bzip2::write::BzEncoder::new(Vec::new(), bzip2::Compression::fast());
        let mut builder = tar::Builder::new(encoder);
        for entry in entries {
            let mut header = tar::Header::new_gnu();
            header.set_mode(0o644);
            match entry {
                Entry::File(path, data) => {
                    header.set_entry_type(tar::EntryType::Regular);
                    header.set_size(data.len() as u64);
                    builder.append_data(&mut header, path, *data).unwrap();
                }
                Entry::Dir(path) => {
                    header.set_entry_type(tar::EntryType::Directory);
                    header.set_size(0);
                    builder.append_data(&mut header, path, io::empty()).unwrap();
                }
                Entry::Symlink(path, to) => {
                    header.set_entry_type(tar::EntryType::Symlink);
                    header.set_size(0);
                    builder.append_link(&mut header, path, to).unwrap();
                }
                Entry::RawName(path, data) => {
                    header.set_entry_type(tar::EntryType::Regular);
                    header.set_size(data.len() as u64);
                    let name = &mut header.as_gnu_mut().unwrap().name;
                    name[..path.len()].copy_from_slice(path.as_bytes());
                    header.set_cksum();
                    builder.append(&header, *data).unwrap();
                }
            }
        }
        builder.into_inner().unwrap().finish().unwrap()
    }

    /// The archive of a good model in the folder `NAME`.
    pub fn good_archive() -> Vec<u8> {
        let mut entries = vec![Entry::Dir("test-model-0.6b/")];
        let paths: Vec<String> = MODEL_FILES.iter().map(|f| format!("{NAME}/{f}")).collect();
        entries.extend(paths.iter().map(|p| Entry::File(p, CONTENT)));
        entries.push(Entry::File("test-model-0.6b/test_wavs/en.wav", b"RIFF"));
        archive(&entries)
    }

    fn hex_hash(data: &[u8]) -> String {
        Sha256::digest(data)
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect()
    }

    /// A source with the size and the hash of `data`.
    pub fn source_for(data: &[u8]) -> ModelSource {
        ModelSource {
            name: NAME,
            url: "https://github.com/example/model.tar.bz2",
            sha256: Box::leak(hex_hash(data).into_boxed_str()),
            archive_bytes: data.len() as u64,
            unpacked_bytes: 1000,
        }
    }

    /// A server in memory. It records the `from_byte` of each call.
    #[derive(Default)]
    pub struct FakeFetch {
        pub data: Vec<u8>,
        pub calls: Mutex<Vec<u64>>,
        /// False: the server always sends the whole file.
        pub ranges: bool,
        /// The connection breaks after this number of bytes of each call.
        pub break_after: Mutex<Option<usize>>,
        /// The error of `get`.
        pub error: Option<String>,
        /// While true, the body sends nothing (a slow server).
        pub hold: Arc<AtomicBool>,
        /// The total size that the server tells. `None`: the size of `data`.
        pub total: Option<Option<u64>>,
        /// The fetch panics (a bug in the download code).
        pub panic: bool,
    }

    impl FakeFetch {
        pub fn new(data: Vec<u8>) -> FakeFetch {
            FakeFetch {
                data,
                ranges: true,
                ..FakeFetch::default()
            }
        }

        pub fn calls(&self) -> Vec<u64> {
            lock(&self.calls).clone()
        }
    }

    /// The body of a `FakeFetch` answer: small blocks, and an error after
    /// `limit` bytes.
    struct FakeBody {
        data: Vec<u8>,
        at: usize,
        limit: Option<usize>,
        hold: Arc<AtomicBool>,
    }

    impl Read for FakeBody {
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            while self.hold.load(Ordering::SeqCst) {
                std::thread::sleep(Duration::from_millis(1));
            }
            if self.limit.is_some_and(|limit| self.at >= limit) {
                return Err(io::Error::other("connection reset"));
            }
            let end = self
                .data
                .len()
                .min(self.at + 7)
                .min(self.limit.unwrap_or(usize::MAX));
            let n = (end - self.at).min(buf.len());
            buf[..n].copy_from_slice(&self.data[self.at..self.at + n]);
            self.at += n;
            Ok(n)
        }
    }

    impl Fetch for FakeFetch {
        fn get(&self, _url: &str, from_byte: u64) -> Result<Fetched, String> {
            lock(&self.calls).push(from_byte);
            assert!(!self.panic, "a bug in the fetch");
            if let Some(error) = &self.error {
                return Err(error.clone());
            }
            let start = if self.ranges { from_byte } else { 0 };
            let data = self.data[start as usize..].to_vec();
            Ok(Fetched {
                start,
                total: self.total.unwrap_or(Some(self.data.len() as u64)),
                body: Box::new(FakeBody {
                    data,
                    at: 0,
                    limit: lock(&self.break_after).take(),
                    hold: Arc::clone(&self.hold),
                }),
            })
        }
    }

    /// The events of one download.
    #[derive(Default)]
    pub struct Events(pub Mutex<Vec<DownloadEvent>>);

    impl Events {
        pub fn last(&self) -> DownloadEvent {
            lock(&self.0).last().cloned().expect("no event")
        }
    }

    fn plenty(_: &Path) -> Option<u64> {
        Some(u64::MAX)
    }

    fn download(
        source: &ModelSource,
        dir: &Path,
        fetch: &FakeFetch,
        cancel: &AtomicBool,
        events: &Events,
    ) -> Result<PathBuf, String> {
        download_model_with(
            source,
            dir,
            fetch,
            cancel,
            &|e| lock(&events.0).push(e),
            &plenty,
        )
    }

    /// The names in a folder, sorted, without the lock file. The lock file
    /// stays, as the session lock file does.
    fn names(dir: &Path) -> Vec<String> {
        let mut list: Vec<String> = fs::read_dir(dir)
            .map(|r| {
                r.map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
                    .filter(|n| n != &format!("{NAME}.lock"))
                    .collect()
            })
            .unwrap_or_default();
        list.sort();
        list
    }

    fn write_model(dir: &Path, content: &[u8]) {
        fs::create_dir_all(dir).unwrap();
        for file in MODEL_FILES {
            fs::write(dir.join(file), content).unwrap();
        }
    }

    #[test]
    fn test_changed_size_on_the_server_fails_before_a_write_and_keeps_the_part_file() {
        let dir = tempfile::tempdir().unwrap();
        let data = good_archive();
        let source = source_for(&data);
        let part = dir.path().join(format!("{NAME}.part"));
        fs::write(&part, &data[..30]).unwrap();
        let mut fetch = FakeFetch::new(data.clone());
        fetch.total = Some(Some(data.len() as u64 + 5));
        let events = Events::default();
        let error = download(
            &source,
            dir.path(),
            &fetch,
            &AtomicBool::new(false),
            &events,
        )
        .unwrap_err();
        assert_eq!(error, CHANGED);
        assert_eq!(events.last(), DownloadEvent::Failed(CHANGED.to_string()));
        assert!(!lock(&events.0)
            .iter()
            .any(|e| matches!(e, DownloadEvent::Progress { .. })));
        assert_eq!(fs::read(&part).unwrap(), &data[..30]);
        // A server that does not tell the size: the hash check decides.
        fetch.total = Some(None);
        download(
            &source,
            dir.path(),
            &fetch,
            &AtomicBool::new(false),
            &events,
        )
        .unwrap();
    }

    #[test]
    fn test_leftovers_of_a_crash_are_removed() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join(NAME);
        let unpack = dir.path().join(format!("{NAME}.unpack"));
        let old = dir.path().join(format!("{NAME}.old"));
        let part = dir.path().join(format!("{NAME}.part"));
        // A complete model: the leftovers go, and nothing is fetched.
        write_model(&target, b"m");
        write_model(&unpack, b"u");
        write_model(&old, b"o");
        fs::write(&part, b"p").unwrap();
        let fetch = FakeFetch::new(good_archive());
        let events = Events::default();
        let source = source_for(&fetch.data);
        download(
            &source,
            dir.path(),
            &fetch,
            &AtomicBool::new(false),
            &events,
        )
        .unwrap();
        assert_eq!(names(dir.path()), [NAME]);
        assert!(fetch.calls().is_empty());
        // A crash between the two renames: `.old` is a complete model and
        // the target is absent. The complete `.old` comes back.
        fs::rename(&target, &old).unwrap();
        write_model(&unpack, b"u");
        let model = download(
            &source,
            dir.path(),
            &fetch,
            &AtomicBool::new(false),
            &events,
        )
        .unwrap();
        assert_eq!(fs::read(model.join("tokens.txt")).unwrap(), b"m");
        assert_eq!(names(dir.path()), [NAME]);
        assert!(fetch.calls().is_empty());
        // An `.old` that is not complete goes, and the download runs.
        fs::remove_dir_all(&target).unwrap();
        fs::create_dir(&old).unwrap();
        fs::write(old.join("tokens.txt"), b"o").unwrap();
        download(
            &source,
            dir.path(),
            &fetch,
            &AtomicBool::new(false),
            &events,
        )
        .unwrap();
        assert_eq!(names(dir.path()), [NAME]);
        assert_eq!(fetch.calls(), [0]);
    }

    #[test]
    fn test_second_instance_gets_already_running() {
        let dir = tempfile::tempdir().unwrap();
        let held = ModelLock::acquire(dir.path(), NAME).unwrap();
        let fetch = FakeFetch::new(good_archive());
        let source = source_for(&fetch.data);
        let events = Events::default();
        let error = download(
            &source,
            dir.path(),
            &fetch,
            &AtomicBool::new(false),
            &events,
        )
        .unwrap_err();
        assert_eq!(error, ALREADY_RUNNING);
        assert!(fetch.calls().is_empty());
        // The system frees the lock when the handle closes, also after a
        // crash. The file itself stays.
        drop(held);
        download(
            &source,
            dir.path(),
            &fetch,
            &AtomicBool::new(false),
            &events,
        )
        .unwrap();
        assert!(dir.path().join(format!("{NAME}.lock")).is_file());
    }

    #[test]
    fn test_unpack_stops_at_too_many_bytes_and_too_many_entries() {
        let mut big_entries: Vec<String> =
            MODEL_FILES.iter().map(|f| format!("{NAME}/{f}")).collect();
        big_entries.push(format!("{NAME}/big.bin"));
        let big = vec![0u8; 200 * 1024];
        let mut entries: Vec<Entry> = big_entries[..4]
            .iter()
            .map(|p| Entry::File(p, CONTENT))
            .collect();
        entries.push(Entry::File(&big_entries[4], &big));
        let too_big = archive(&entries);
        let names_many: Vec<String> = (0..MAX_ENTRIES + 1)
            .map(|i| format!("{NAME}/f{i}"))
            .collect();
        let many = archive(
            &names_many
                .iter()
                .map(|p| Entry::File(p, b"x"))
                .collect::<Vec<_>>(),
        );
        for (data, text) in [
            (too_big, "larger than expected"),
            (many, "too many entries"),
        ] {
            let dir = tempfile::tempdir().unwrap();
            let source = source_for(&data);
            let fetch = FakeFetch::new(data);
            let events = Events::default();
            let error = download(
                &source,
                dir.path(),
                &fetch,
                &AtomicBool::new(false),
                &events,
            )
            .unwrap_err();
            assert!(error.contains(text), "{error}");
            assert_eq!(names(dir.path()), [format!("{NAME}.part")]);
        }
    }

    #[test]
    fn test_place_puts_the_old_folder_back_when_the_rename_fails() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join(NAME);
        let old = dir.path().join(format!("{NAME}.old"));
        fs::create_dir(&target).unwrap();
        fs::write(target.join("tokens.txt"), b"old").unwrap();
        // The model folder is absent: the second rename fails.
        let error = place(&dir.path().join("absent"), &target, &old).unwrap_err();
        assert!(error.starts_with("cannot move the model"), "{error}");
        assert_eq!(fs::read(target.join("tokens.txt")).unwrap(), b"old");
        assert!(!old.exists());
        assert_eq!(names(dir.path()), [NAME]);
    }

    #[test]
    fn test_good_archive_gives_the_model_folder_and_no_part_file() {
        let dir = tempfile::tempdir().unwrap();
        let models = dir.path().join("models");
        let data = good_archive();
        let source = source_for(&data);
        let fetch = FakeFetch::new(data);
        let events = Events::default();
        let model = download(&source, &models, &fetch, &AtomicBool::new(false), &events).unwrap();
        assert_eq!(model, models.join(NAME));
        assert!(is_complete_model(&model));
        assert_eq!(fs::read(model.join("tokens.txt")).unwrap(), CONTENT);
        assert!(model.join("test_wavs/en.wav").is_file());
        // Only the model folder stays.
        assert_eq!(names(&models), [NAME]);
        assert_eq!(fetch.calls(), [0]);
        let events = lock(&events.0).clone();
        let total = source.archive_bytes;
        assert_eq!(events[0], DownloadEvent::Progress { done: 0, total });
        let n = events.len();
        assert_eq!(
            events[n - 4..],
            [
                DownloadEvent::Progress { done: total, total },
                DownloadEvent::Verifying,
                DownloadEvent::Unpacking,
                DownloadEvent::Done { model_dir: model }
            ]
        );
    }

    #[test]
    fn test_wrong_hash_is_refused_and_nothing_is_unpacked() {
        let dir = tempfile::tempdir().unwrap();
        let data = good_archive();
        let mut source = source_for(&data);
        source.sha256 = "00";
        let fetch = FakeFetch::new(data);
        let events = Events::default();
        let error = download(
            &source,
            dir.path(),
            &fetch,
            &AtomicBool::new(false),
            &events,
        )
        .unwrap_err();
        assert_eq!(error, DAMAGED);
        assert_eq!(events.last(), DownloadEvent::Failed(DAMAGED.to_string()));
        assert!(!lock(&events.0).contains(&DownloadEvent::Unpacking));
        // No part file, no folder: the next try starts again.
        assert!(names(dir.path()).is_empty(), "{:?}", names(dir.path()));
        // The next try with the right hash works and starts from 0.
        let source = source_for(&fetch.data);
        download(
            &source,
            dir.path(),
            &fetch,
            &AtomicBool::new(false),
            &events,
        )
        .unwrap();
        assert_eq!(fetch.calls(), [0, 0]);
    }

    #[test]
    fn test_archive_entry_with_dot_dot_is_refused() {
        let good: Vec<String> = MODEL_FILES.iter().map(|f| format!("{NAME}/{f}")).collect();
        let bad_entries = [
            Entry::RawName("test-model-0.6b/../../escaped.txt", b"bad"),
            Entry::RawName("../escaped.txt", b"bad"),
            Entry::RawName("/escaped.txt", b"bad"),
            Entry::RawName("C:/escaped.txt", b"bad"),
            Entry::RawName("test-model-0.6b/tokens.txt:stream", b"bad"),
            Entry::Symlink("test-model-0.6b/link", "../../escaped.txt"),
        ];
        for bad in bad_entries {
            let dir = tempfile::tempdir().unwrap();
            let models = dir.path().join("a").join("models");
            let mut entries: Vec<Entry> = good.iter().map(|p| Entry::File(p, CONTENT)).collect();
            entries.insert(1, bad);
            let data = archive(&entries);
            let source = source_for(&data);
            let fetch = FakeFetch::new(data);
            let events = Events::default();
            let error =
                download(&source, &models, &fetch, &AtomicBool::new(false), &events).unwrap_err();
            assert!(
                error.contains("outside its folder") || error.contains("not a file or a folder"),
                "{error}"
            );
            assert!(matches!(events.last(), DownloadEvent::Failed(_)));
            // Nothing outside the models folder, and no model folder.
            assert_eq!(names(dir.path()), ["a"]);
            assert_eq!(names(&dir.path().join("a")), ["models"]);
            assert!(!models.join(NAME).exists());
            assert!(!models.join(format!("{NAME}.unpack")).exists());
        }
    }

    #[test]
    fn test_cancel_stops_and_keeps_the_part_file() {
        let dir = tempfile::tempdir().unwrap();
        let data = good_archive();
        let source = source_for(&data);
        let fetch = FakeFetch::new(data);
        let cancel = AtomicBool::new(false);
        let events = Events::default();
        // The cancel comes with the first progress event.
        let result = download_model_with(
            &source,
            dir.path(),
            &fetch,
            &cancel,
            &|e| {
                if matches!(e, DownloadEvent::Progress { .. }) {
                    cancel.store(true, Ordering::SeqCst);
                }
                lock(&events.0).push(e);
            },
            &plenty,
        );
        assert_eq!(result.unwrap_err(), CANCELLED);
        assert_eq!(events.last(), DownloadEvent::Cancelled);
        assert!(dir.path().join(format!("{NAME}.part")).is_file());
        assert!(!dir.path().join(NAME).exists());
    }

    #[test]
    fn test_cancel_during_the_unpack_leaves_no_model_folder() {
        let dir = tempfile::tempdir().unwrap();
        let data = good_archive();
        let source = source_for(&data);
        let fetch = FakeFetch::new(data);
        let cancel = AtomicBool::new(false);
        let events = Events::default();
        let result = download_model_with(
            &source,
            dir.path(),
            &fetch,
            &cancel,
            &|e| {
                if e == DownloadEvent::Unpacking {
                    cancel.store(true, Ordering::SeqCst);
                }
                lock(&events.0).push(e);
            },
            &plenty,
        );
        assert_eq!(result.unwrap_err(), CANCELLED);
        assert_eq!(names(dir.path()), [format!("{NAME}.part")]);
    }

    #[test]
    fn test_second_call_continues_from_the_part_file() {
        let dir = tempfile::tempdir().unwrap();
        let data = good_archive();
        let source = source_for(&data);
        let fetch = FakeFetch::new(data);
        *lock(&fetch.break_after) = Some(40);
        let events = Events::default();
        let error = download(
            &source,
            dir.path(),
            &fetch,
            &AtomicBool::new(false),
            &events,
        )
        .unwrap_err();
        assert!(error.contains("try again to continue"), "{error}");
        assert!(!dir.path().join(NAME).exists());
        let part = dir.path().join(format!("{NAME}.part"));
        let size = fs::metadata(&part).unwrap().len();
        assert_eq!(size, 40);
        download(
            &source,
            dir.path(),
            &fetch,
            &AtomicBool::new(false),
            &events,
        )
        .unwrap();
        assert_eq!(fetch.calls(), [0, size]);
        assert!(is_complete_model(&dir.path().join(NAME)));
        assert!(!part.exists());
    }

    #[test]
    fn test_server_without_ranges_starts_again() {
        let dir = tempfile::tempdir().unwrap();
        let data = good_archive();
        let source = source_for(&data);
        let mut fetch = FakeFetch::new(data);
        fetch.ranges = false;
        *lock(&fetch.break_after) = Some(40);
        let events = Events::default();
        download(
            &source,
            dir.path(),
            &fetch,
            &AtomicBool::new(false),
            &events,
        )
        .unwrap_err();
        download(
            &source,
            dir.path(),
            &fetch,
            &AtomicBool::new(false),
            &events,
        )
        .unwrap();
        assert_eq!(fetch.calls(), [0, 40]);
        assert!(is_complete_model(&dir.path().join(NAME)));
    }

    #[test]
    fn test_part_file_with_wrong_bytes_is_deleted_and_the_next_try_starts_again() {
        let dir = tempfile::tempdir().unwrap();
        let data = good_archive();
        let source = source_for(&data);
        let part = dir.path().join(format!("{NAME}.part"));
        fs::write(&part, vec![0u8; 30]).unwrap();
        let fetch = FakeFetch::new(data);
        let events = Events::default();
        let error = download(
            &source,
            dir.path(),
            &fetch,
            &AtomicBool::new(false),
            &events,
        )
        .unwrap_err();
        assert_eq!(error, DAMAGED);
        assert!(!part.exists());
        download(
            &source,
            dir.path(),
            &fetch,
            &AtomicBool::new(false),
            &events,
        )
        .unwrap();
        assert_eq!(fetch.calls(), [30, 0]);
    }

    #[test]
    fn test_existing_model_folder_is_not_downloaded_again() {
        let dir = tempfile::tempdir().unwrap();
        let model = dir.path().join(NAME);
        fs::create_dir(&model).unwrap();
        for file in MODEL_FILES {
            fs::write(model.join(file), b"the user's model").unwrap();
        }
        // An archive that is damaged: it is never fetched.
        let mut source = source_for(b"x");
        source.sha256 = "00";
        let fetch = FakeFetch::new(b"x".to_vec());
        let events = Events::default();
        let result = download(
            &source,
            dir.path(),
            &fetch,
            &AtomicBool::new(false),
            &events,
        );
        assert_eq!(result.unwrap(), model);
        assert!(fetch.calls().is_empty());
        assert_eq!(
            fs::read(model.join("tokens.txt")).unwrap(),
            b"the user's model"
        );
        assert_eq!(names(dir.path()), [NAME]);
        assert_eq!(
            lock(&events.0).clone(),
            [DownloadEvent::Done { model_dir: model }]
        );
    }

    #[test]
    fn test_incomplete_model_folder_stays_after_a_failed_download_and_goes_after_a_good_one() {
        let dir = tempfile::tempdir().unwrap();
        let model = dir.path().join(NAME);
        fs::create_dir(&model).unwrap();
        fs::write(model.join("tokens.txt"), b"old").unwrap();
        let data = good_archive();
        let mut source = source_for(&data);
        source.sha256 = "00";
        let fetch = FakeFetch::new(data);
        let events = Events::default();
        download(
            &source,
            dir.path(),
            &fetch,
            &AtomicBool::new(false),
            &events,
        )
        .unwrap_err();
        assert_eq!(fs::read(model.join("tokens.txt")).unwrap(), b"old");
        let source = source_for(&fetch.data);
        download(
            &source,
            dir.path(),
            &fetch,
            &AtomicBool::new(false),
            &events,
        )
        .unwrap();
        assert_eq!(fs::read(model.join("tokens.txt")).unwrap(), CONTENT);
        assert_eq!(names(dir.path()), [NAME]);
    }

    #[test]
    fn test_archive_without_the_model_files_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let data = archive(&[Entry::File("test-model-0.6b/README.md", b"text")]);
        let source = source_for(&data);
        let fetch = FakeFetch::new(data);
        let events = Events::default();
        let error = download(
            &source,
            dir.path(),
            &fetch,
            &AtomicBool::new(false),
            &events,
        )
        .unwrap_err();
        assert!(error.contains("does not have the model files"), "{error}");
        // The checked archive stays, so that a second try does not
        // download it again. No model folder and no unpack folder stay.
        assert_eq!(names(dir.path()), [format!("{NAME}.part")]);
    }

    #[test]
    fn test_fetch_error_gives_a_plain_message_and_no_model_folder() {
        let dir = tempfile::tempdir().unwrap();
        let data = good_archive();
        let source = source_for(&data);
        let mut fetch = FakeFetch::new(data);
        fetch.error = Some("cannot connect to the server, check the network".into());
        let events = Events::default();
        let error = download(
            &source,
            dir.path(),
            &fetch,
            &AtomicBool::new(false),
            &events,
        )
        .unwrap_err();
        assert_eq!(error, "cannot connect to the server, check the network");
        assert_eq!(events.last(), DownloadEvent::Failed(error));
        assert!(!dir.path().join(NAME).exists());
    }

    #[test]
    fn test_not_enough_space_is_reported_before_the_download() {
        let dir = tempfile::tempdir().unwrap();
        let data = good_archive();
        let source = source_for(&data);
        let fetch = FakeFetch::new(data);
        let needed = source.archive_bytes + source.unpacked_bytes;
        let events = Events::default();
        let error = download_model_with(
            &source,
            dir.path(),
            &fetch,
            &AtomicBool::new(false),
            &|e| lock(&events.0).push(e),
            &|_| Some(needed),
        )
        .unwrap_err();
        assert!(error.starts_with("not enough free disk space"), "{error}");
        assert!(fetch.calls().is_empty());
        assert!(names(dir.path()).is_empty());
        // One byte more is enough.
        download_model_with(
            &source,
            dir.path(),
            &fetch,
            &AtomicBool::new(false),
            &|e| lock(&events.0).push(e),
            &|_| Some(needed + 1),
        )
        .unwrap();
    }

    #[test]
    fn test_safe_path_refuses_paths_outside_the_folder() {
        assert_eq!(
            safe_path(Path::new("a/b.txt")),
            Some(PathBuf::from("a/b.txt"))
        );
        assert_eq!(safe_path(Path::new("./a")), Some(PathBuf::from("a")));
        for bad in [
            "../a",
            "a/../../b",
            "/a",
            "\\a",
            "C:\\a",
            "C:a",
            "a/b:c",
            "\\\\server\\share\\a",
        ] {
            assert_eq!(safe_path(Path::new(bad)), None, "{bad}");
        }
    }

    #[test]
    fn test_only_https_and_the_listed_hosts_are_permitted() {
        let ok = |u: &str| url_permitted(&url::Url::parse(u).unwrap(), &ALLOWED_HOSTS, true);
        assert!(ok(PARAKEET_V3_INT8.url));
        assert!(ok("https://release-assets.githubusercontent.com/x?sig=1"));
        assert!(ok("https://objects.githubusercontent.com/x"));
        assert!(!ok("http://github.com/x"));
        assert!(!ok("https://github.com.evil.example/x"));
        assert!(!ok("https://evil.example/github.com"));
        assert!(!ok("https://user:pw@github.com/x"));
        assert!(!ok("ftp://github.com/x"));
    }

    #[test]
    fn test_the_pinned_source_is_https_and_has_a_full_hash() {
        let s = PARAKEET_V3_INT8;
        assert!(s.url.starts_with("https://github.com/"));
        assert!(s.url.ends_with(&format!("{}.tar.bz2", s.name)));
        assert_eq!(s.sha256.len(), 64);
        assert!(s
            .sha256
            .chars()
            .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()));
        assert!(s.archive_bytes > 0 && s.unpacked_bytes > s.archive_bytes);
    }

    /// A local HTTP server for `HttpFetch`. Each request gets the answer of
    /// `answer`. The server ends when the test drops it.
    struct Server {
        server: Arc<tiny_http::Server>,
        port: u16,
        thread: Option<std::thread::JoinHandle<()>>,
    }

    type Answer = dyn Fn(&tiny_http::Request, u16) -> tiny_http::ResponseBox + Send + Sync;

    impl Server {
        fn start(answer: Arc<Answer>) -> Server {
            let server = Arc::new(tiny_http::Server::http("127.0.0.1:0").unwrap());
            let port = server.server_addr().to_ip().unwrap().port();
            let s = Arc::clone(&server);
            let thread = std::thread::spawn(move || {
                for request in s.incoming_requests() {
                    let response = answer(&request, port);
                    let _ = request.respond(response);
                }
            });
            Server {
                server,
                port,
                thread: Some(thread),
            }
        }

        fn url(&self, path: &str) -> String {
            format!("http://127.0.0.1:{}{path}", self.port)
        }
    }

    impl Drop for Server {
        fn drop(&mut self) {
            self.server.unblock();
            if let Some(thread) = self.thread.take() {
                let _ = thread.join();
            }
        }
    }

    fn header(name: &str, value: &str) -> tiny_http::Header {
        tiny_http::Header::from_bytes(name.as_bytes(), value.as_bytes()).unwrap()
    }

    /// `/file` sends `BODY` (with ranges), `/to-file` redirects to `/file`,
    /// `/away` redirects to the host `localhost`.
    fn test_server() -> Server {
        const BODY: &[u8] = b"0123456789abcdefghij";
        Server::start(Arc::new(
            |request: &tiny_http::Request, port| match request.url() {
                "/file" => {
                    let range = request
                        .headers()
                        .iter()
                        .find(|h| h.field.equiv("Range"))
                        .and_then(|h| {
                            h.value
                                .as_str()
                                .strip_prefix("bytes=")?
                                .strip_suffix('-')?
                                .parse::<usize>()
                                .ok()
                        });
                    match range {
                        Some(from) => tiny_http::Response::from_data(&BODY[from..])
                            .with_status_code(206)
                            .with_header(header(
                                "Content-Range",
                                &format!("bytes {from}-{}/{}", BODY.len() - 1, BODY.len()),
                            ))
                            .boxed(),
                        None => tiny_http::Response::from_data(BODY).boxed(),
                    }
                }
                "/to-file" => tiny_http::Response::empty(302)
                    .with_header(header("Location", &format!("http://127.0.0.1:{port}/file")))
                    .boxed(),
                "/away" => tiny_http::Response::empty(302)
                    .with_header(header("Location", &format!("http://localhost:{port}/file")))
                    .boxed(),
                // A range is outside this file: 416, then the whole file.
                "/no-range" => {
                    if request.headers().iter().any(|h| h.field.equiv("Range")) {
                        tiny_http::Response::empty(416).boxed()
                    } else {
                        tiny_http::Response::from_data(BODY).boxed()
                    }
                }
                "/wrong-start" => tiny_http::Response::from_data(&BODY[3..])
                    .with_status_code(206)
                    .with_header(header(
                        "Content-Range",
                        &format!("bytes 3-{}/{}", BODY.len() - 1, BODY.len()),
                    ))
                    .boxed(),
                _ => tiny_http::Response::empty(404).boxed(),
            },
        ))
    }

    fn local_fetch(cancel: Arc<AtomicBool>, idle: Duration) -> HttpFetch {
        HttpFetch::with_rules(&["127.0.0.1"], false, cancel, idle).unwrap()
    }

    fn read_all(mut fetched: Fetched) -> Vec<u8> {
        let mut out = Vec::new();
        fetched.body.read_to_end(&mut out).unwrap();
        out
    }

    #[test]
    fn test_http_fetch_gives_the_total_size_of_the_server() {
        let server = test_server();
        let fetch = local_fetch(Arc::default(), IDLE_LIMIT);
        assert_eq!(fetch.get(&server.url("/file"), 0).unwrap().total, Some(20));
        assert_eq!(fetch.get(&server.url("/file"), 15).unwrap().total, Some(20));
        assert_eq!(range_total("bytes 15-19/20"), Some(20));
        assert_eq!(range_total("bytes 15-19/*"), None);
    }

    #[test]
    fn test_http_fetch_starts_again_after_416_and_refuses_a_wrong_start() {
        let server = test_server();
        let fetch = local_fetch(Arc::default(), IDLE_LIMIT);
        let fetched = fetch.get(&server.url("/no-range"), 15).unwrap();
        assert_eq!(fetched.start, 0);
        assert_eq!(read_all(fetched), b"0123456789abcdefghij");
        let error = fetch.get(&server.url("/wrong-start"), 15).err().unwrap();
        assert_eq!(
            error,
            "the server sent a part of the file that was not asked for"
        );
    }

    /// The bytes that `StallBody` sends before its silence. It is more than
    /// the write buffer of the server, so that the bytes go out at once.
    const BEFORE_SILENCE: usize = 256 * 1024;

    /// A body that sends `BEFORE_SILENCE` bytes, then waits until `gate` is
    /// false.
    struct StallBody {
        sent: usize,
        gate: Arc<AtomicBool>,
    }

    impl Read for StallBody {
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            if self.sent < BEFORE_SILENCE {
                let n = buf.len().min(BEFORE_SILENCE - self.sent);
                buf[..n].fill(b'a');
                self.sent += n;
                return Ok(n);
            }
            while self.gate.load(Ordering::SeqCst) {
                std::thread::sleep(Duration::from_millis(5));
            }
            Ok(0)
        }
    }

    #[test]
    fn test_http_fetch_stops_after_silence_between_blocks() {
        let gate = Arc::new(AtomicBool::new(true));
        let held = Arc::clone(&gate);
        let server = Server::start(Arc::new(move |_: &tiny_http::Request, _| {
            let body = StallBody {
                sent: 0,
                gate: Arc::clone(&held),
            };
            tiny_http::Response::new(200.into(), Vec::new(), body, Some(2 * BEFORE_SILENCE), None)
                .boxed()
        }));
        // Declared after the server, so it opens the gate before the
        // server waits for its thread, also when an assert fails.
        let _open = OpenGate(Arc::clone(&gate));
        let fetch = local_fetch(Arc::default(), Duration::from_millis(300));
        let mut fetched = fetch.get(&server.url("/file"), 0).unwrap();
        let mut buf = [0u8; 4096];
        let begin = Instant::now();
        let mut got = Vec::new();
        let error = loop {
            match fetched.body.read(&mut buf) {
                Ok(0) => panic!("the body ended"),
                Ok(n) => got.extend_from_slice(&buf[..n]),
                Err(e) => break e.to_string(),
            }
        };
        assert!(!got.is_empty() && got.len() <= BEFORE_SILENCE);
        assert_eq!(error, "the server stopped sending data");
        assert!(begin.elapsed() < Duration::from_secs(5));
    }

    /// Opens a gate of a test server when it is dropped.
    struct OpenGate(Arc<AtomicBool>);

    impl Drop for OpenGate {
        fn drop(&mut self) {
            self.0.store(false, Ordering::SeqCst);
        }
    }

    #[test]
    fn test_http_fetch_follows_a_permitted_redirect_and_asks_for_the_rest() {
        let server = test_server();
        let fetch = local_fetch(Arc::default(), IDLE_LIMIT);
        let fetched = fetch.get(&server.url("/to-file"), 0).unwrap();
        assert_eq!(fetched.start, 0);
        assert_eq!(read_all(fetched), b"0123456789abcdefghij");
        let fetched = fetch.get(&server.url("/file"), 15).unwrap();
        assert_eq!(fetched.start, 15);
        assert_eq!(read_all(fetched), b"fghij");
        let error = fetch.get(&server.url("/nothing"), 0).err().unwrap();
        assert_eq!(error, "the server answered with HTTP status 404");
    }

    #[test]
    fn test_http_fetch_refuses_a_redirect_to_another_host() {
        let server = test_server();
        let fetch = local_fetch(Arc::default(), IDLE_LIMIT);
        let error = fetch.get(&server.url("/away"), 0).err().unwrap();
        assert_eq!(
            error,
            "the server sent a redirect to an address that is not permitted"
        );
        // The first address must be permitted too.
        let error = fetch
            .get(&format!("http://localhost:{}/file", server.port), 0)
            .err()
            .unwrap();
        assert_eq!(error, "the model address is not permitted");
        // The real rules permit no http address.
        let real = HttpFetch::new(Arc::default()).unwrap();
        let error = real.get(&server.url("/file"), 0).err().unwrap();
        assert_eq!(error, "the model address is not permitted");
    }

    #[test]
    fn test_http_fetch_stops_at_a_cancel_and_after_a_long_silence() {
        let gate = Arc::new(AtomicBool::new(true));
        let held = Arc::clone(&gate);
        // The server waits before its answer, as a stalled connection does.
        let server = Server::start(Arc::new(move |_: &tiny_http::Request, _| {
            while held.load(Ordering::SeqCst) {
                std::thread::sleep(Duration::from_millis(5));
            }
            tiny_http::Response::from_data(b"late".to_vec()).boxed()
        }));
        let cancel = Arc::new(AtomicBool::new(false));
        let fetch = local_fetch(Arc::clone(&cancel), Duration::from_millis(300));
        let begin = Instant::now();
        let error = fetch.get(&server.url("/file"), 0).err().unwrap();
        assert_eq!(error, "the server stopped sending data");
        assert!(begin.elapsed() < Duration::from_secs(5));
        let stopper = {
            let cancel = Arc::clone(&cancel);
            std::thread::spawn(move || {
                std::thread::sleep(Duration::from_millis(100));
                cancel.store(true, Ordering::SeqCst);
            })
        };
        let slow = local_fetch(Arc::clone(&cancel), IDLE_LIMIT);
        let begin = Instant::now();
        let error = slow.get(&server.url("/file"), 0).err().unwrap();
        assert_eq!(error, CANCELLED);
        assert!(begin.elapsed() < Duration::from_secs(5));
        stopper.join().unwrap();
        gate.store(false, Ordering::SeqCst);
    }

    /// A speech clip of the private folder `VOICEBRIDGE_STT_FIXTURES`
    /// (default `private/stt_fixtures` in the workspace; a relative path
    /// starts at the workspace folder). The repository has no recordings.
    /// `None` (with a note on stderr): the clip is absent.
    fn speech_clip(name: &str) -> Option<std::path::PathBuf> {
        let folder = std::env::var_os("VOICEBRIDGE_STT_FIXTURES")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| std::path::PathBuf::from("private/stt_fixtures"));
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join(folder)
            .join(name);
        if path.is_file() {
            Some(path)
        } else {
            eprintln!("skipped: no speech clip at {}", path.display());
            None
        }
    }

    /// Downloads the real archive into a temporary folder and loads the
    /// model. It needs the network and about 1.2 GB of free disk space,
    /// and the speech clip of `speech_clip`.
    #[test]
    #[ignore]
    fn the_real_archive_downloads_and_the_model_loads() {
        use edcopilot_voicebridge_stt::{wav, ParakeetEngine, SttEngine};
        let Some(clip) = speech_clip("en_set_course_home.wav") else {
            return;
        };
        let dir = tempfile::tempdir().unwrap();
        let cancel = Arc::new(AtomicBool::new(false));
        let fetch = HttpFetch::new(Arc::clone(&cancel)).unwrap();
        let model = download_model(&PARAKEET_V3_INT8, dir.path(), &fetch, &cancel, &|e| {
            if !matches!(e, DownloadEvent::Progress { .. }) {
                eprintln!("{e:?}");
            }
        })
        .unwrap();
        let mut engine = ParakeetEngine::new(&model, 2, None).unwrap();
        let text = engine
            .transcribe(&wav::read_file(&clip).unwrap())
            .unwrap()
            .text;
        assert!(text.to_lowercase().contains("course"), "{text:?}");
    }
}
