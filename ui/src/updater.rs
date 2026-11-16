//! The update check and the install of a new version.
//!
//! The Tauri updater plugin does the network work and the signature check.
//! This module holds the state of the update for the window (`Updates`),
//! the rules of the checks, and the texts.
//!
//! Rules:
//! - The program asks before it installs. Only the command `install_update`
//!   starts a download, and only while no session runs (`commands`).
//! - The plugin checks the signature of the download with the public key of
//!   `tauri.conf.json`. `Update::download` checks it, and `Update::install`
//!   checks it again before it starts the installer. No code path here runs
//!   a file without these checks.
//! - Only a newer version counts: the plugin compares the versions, and this
//!   program sets no comparator of its own.
//! - The update address is fixed in `tauri.conf.json`. The tests give their
//!   address and key as arguments of `PluginSource::new`, never through an
//!   environment variable.
//!
//! Locks: `Updates::inner` is the last lock. Its holder takes no other lock
//! and calls no other code. A start of a session takes it while it holds
//! the capture lock (see `commands::begin_update_install`).

use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crate::dto::UpdateStatusDto;
use crate::events::UpdateEmit;
use crate::state::lock;

/// The update address. `tauri.conf.json` has it as
/// `plugins.updater.endpoints`, and the program uses that value. A test
/// compares the two.
#[cfg(test)]
pub const ENDPOINT: &str =
    "https://github.com/Superbrain8/edcopilot-voicebridge/releases/latest/download/latest.json";
/// The release page of a version is this text and the version.
pub const RELEASE_PAGE_PREFIX: &str =
    "https://github.com/Superbrain8/edcopilot-voicebridge/releases/tag/v";

/// The time limit of one check.
pub const CHECK_TIMEOUT: Duration = Duration::from_secs(15);
/// The time limit of the download of the installer.
pub const DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(30 * 60);
/// The first automatic check comes this time after the start.
pub const FIRST_CHECK_DELAY: Duration = Duration::from_secs(10);
/// The time between two automatic checks.
pub const CHECK_INTERVAL: Duration = Duration::from_secs(24 * 60 * 60);
/// The time that the window shows `READY` before the program closes.
pub const READY_PAUSE: Duration = Duration::from_secs(2);
/// The longest text of the release notes that the window gets.
pub const MAX_NOTES_CHARS: usize = 8_000;

/// The text of a manual check in a developer build.
pub const DEVELOPER_BUILD: &str = "This is a developer build. It has no updates.";
/// The text of an install while a session runs, starts, or stops.
pub const STOP_FIRST: &str = "Stop the session first.";
/// The text of a session start, a check, or a second install during the
/// download or the install.
pub const IN_PROGRESS: &str = "The update is in progress.";
/// The text of an install without an available update.
pub const NO_UPDATE: &str = "There is no update to install. Check for updates first.";
/// The text of a cancel after the download is complete.
pub const TOO_LATE: &str = "The download is complete. You cannot stop the update now.";

/// The text of a manual check that failed.
pub fn check_failed_text(cause: Cause) -> String {
    format!(
        "The program could not check for updates: {}. Try again later.",
        cause.text()
    )
}

/// The text of a download or an install that failed.
pub fn install_failed_text(cause: Cause) -> String {
    format!(
        "The update did not install: {}. The program is not changed.",
        cause.text()
    )
}

/// The release page of a version.
pub fn release_page(version: &str) -> String {
    format!("{RELEASE_PAGE_PREFIX}{version}")
}

/// Why a check, a download, or an install failed. The window shows the
/// text. It never holds an address or the text of a library error.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cause {
    Timeout,
    Connection,
    Server,
    BadData,
    Download,
    Signature,
    Format,
    InstallerStart,
    Unknown,
}

impl Cause {
    pub fn text(self) -> &'static str {
        match self {
            Cause::Timeout => "the server did not answer in time",
            Cause::Connection => "the program could not connect to the server",
            Cause::Server => "the server did not send the update information",
            Cause::BadData => "the update information from the server is not valid",
            Cause::Download => "the download of the installer failed",
            Cause::Signature => "the signature of the installer is not correct",
            Cause::Format => "the downloaded file is not an installer",
            Cause::InstallerStart => "the installer did not start",
            Cause::Unknown => "an unknown error occurred",
        }
    }
}

/// A future that can go to another thread.
pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// The progress callback of a download: the bytes so far and the size, if
/// the server sent it.
pub type Progress<'a> = &'a (dyn Fn(u64, Option<u64>) + Send + Sync);

/// An update that the server offers. The plugin gives the real one.
pub trait PendingUpdate: Send + Sync {
    /// Downloads the installer and checks its signature. `Ok` only with a
    /// correct signature.
    fn download<'a>(&'a self, progress: Progress<'a>) -> BoxFuture<'a, Result<Vec<u8>, Cause>>;
    /// Checks the signature again, starts the installer, and ends the
    /// program. It returns only when the installer did not start.
    fn install(&self, bytes: Vec<u8>) -> Result<(), Cause>;
}

/// The result of a check that found a newer version.
#[derive(Clone)]
pub struct Found {
    pub version: String,
    pub notes: Option<String>,
    /// The publish date as `latest.json` has it (RFC 3339).
    pub date: Option<String>,
    pub pending: Arc<dyn PendingUpdate>,
}

/// The update server. The plugin gives the real one.
pub trait UpdateSource: Send + Sync {
    /// `Ok(None)`: no newer version.
    fn check(&self) -> BoxFuture<'_, Result<Option<Found>, Cause>>;
}

/// The step of the update.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Phase {
    Idle,
    Checking {
        manual: bool,
    },
    UpToDate,
    Available,
    Downloading {
        received: u64,
        total: Option<u64>,
    },
    /// The download and its signature are correct. The installer starts.
    Ready,
    CheckFailed {
        cause: Cause,
    },
}

struct Inner {
    phase: Phase,
    /// The phase before the running check.
    before_check: Phase,
    /// The offered update, with `Available`, `Downloading`, and `Ready`.
    offer: Option<Found>,
    /// The text of the last failed download or install of the offer.
    install_error: Option<String>,
    /// The end of the last check, in milliseconds since 1970.
    checked_at: Option<u64>,
    /// True after "Later", until the next manual check.
    hidden: bool,
    /// The number of the newest download. An older download changes
    /// nothing.
    download: u64,
    /// Stops the running download.
    abort: Option<Box<dyn FnOnce() + Send>>,
}

/// The update state of the window program. `AppState` has one.
pub struct Updates {
    inner: Mutex<Inner>,
    current_version: String,
    developer_build: bool,
    /// The megabyte of the last progress event.
    last_megabyte: AtomicU64,
}

/// The time now in milliseconds since 1970.
pub fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as u64)
}

/// Cuts a text to `max` characters.
fn capped(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        text.to_string()
    } else {
        let mut cut: String = text.chars().take(max.saturating_sub(1)).collect();
        cut.push('…');
        cut
    }
}

impl Updates {
    /// `developer_build`: a build with debug assertions. It never checks.
    pub fn new(current_version: &str, developer_build: bool) -> Updates {
        Updates {
            inner: Mutex::new(Inner {
                phase: Phase::Idle,
                before_check: Phase::Idle,
                offer: None,
                install_error: None,
                checked_at: None,
                hidden: false,
                download: 0,
                abort: None,
            }),
            current_version: current_version.to_string(),
            developer_build,
            last_megabyte: AtomicU64::new(u64::MAX),
        }
    }

    pub fn developer_build(&self) -> bool {
        self.developer_build
    }

    #[cfg(test)]
    pub fn phase(&self) -> Phase {
        lock(&self.inner).phase.clone()
    }

    /// The state for the window.
    pub fn status(&self) -> UpdateStatusDto {
        let inner = lock(&self.inner);
        let (phase, received, total, cause) = match &inner.phase {
            Phase::Idle => ("idle", 0, None, None),
            Phase::Checking { .. } => ("checking", 0, None, None),
            Phase::UpToDate => ("up_to_date", 0, None, None),
            Phase::Available => ("available", 0, None, None),
            Phase::Downloading { received, total } => ("downloading", *received, *total, None),
            Phase::Ready => ("ready", 0, None, None),
            Phase::CheckFailed { cause } => {
                ("check_failed", 0, None, Some(cause.text().to_string()))
            }
        };
        let offer = inner.offer.as_ref();
        UpdateStatusDto {
            current_version: self.current_version.clone(),
            developer_build: self.developer_build,
            phase: phase.to_string(),
            version: offer.map(|o| o.version.clone()),
            notes: offer.and_then(|o| o.notes.clone()),
            date: offer.and_then(|o| o.date.clone()),
            release_page: offer.map(|o| release_page(&o.version)),
            received,
            total,
            cause,
            error: inner.install_error.clone(),
            checked_at: inner.checked_at,
            hidden: inner.hidden,
        }
    }

    /// True during the download and the install: a session must not start.
    pub fn blocks_session(&self) -> bool {
        matches!(
            lock(&self.inner).phase,
            Phase::Downloading { .. } | Phase::Ready
        )
    }

    /// Marks the start of a check. `Ok(false)`: no check now (a check runs
    /// already, or an automatic check during the update). A manual check
    /// during the update and a manual check of a developer build give the
    /// text.
    pub fn begin_check(&self, manual: bool) -> Result<bool, String> {
        if self.developer_build {
            return if manual {
                Err(DEVELOPER_BUILD.to_string())
            } else {
                Ok(false)
            };
        }
        let mut inner = lock(&self.inner);
        match inner.phase {
            Phase::Downloading { .. } | Phase::Ready => {
                return if manual {
                    Err(IN_PROGRESS.to_string())
                } else {
                    Ok(false)
                };
            }
            Phase::Checking { .. } => {
                if manual {
                    // The running check shows its result to this user too.
                    inner.hidden = false;
                    inner.phase = Phase::Checking { manual: true };
                }
                return Ok(false);
            }
            _ => {}
        }
        if manual {
            inner.hidden = false;
        }
        inner.before_check = inner.phase.clone();
        inner.phase = Phase::Checking { manual };
        Ok(true)
    }

    /// Ends the check of `begin_check`. A manual check that failed gives
    /// its text as the error. An automatic check that failed keeps an
    /// update that an earlier check found.
    pub fn finish_check(
        &self,
        result: Result<Option<Found>, Cause>,
        now: u64,
    ) -> Result<UpdateStatusDto, String> {
        let failed = {
            let mut inner = lock(&self.inner);
            let manual = matches!(inner.phase, Phase::Checking { manual: true });
            inner.checked_at = Some(now);
            match result {
                Ok(None) => {
                    inner.phase = Phase::UpToDate;
                    inner.offer = None;
                    inner.install_error = None;
                    None
                }
                Ok(Some(found)) => {
                    let same = inner
                        .offer
                        .as_ref()
                        .is_some_and(|o| o.version == found.version);
                    if !same {
                        inner.install_error = None;
                    }
                    inner.phase = Phase::Available;
                    inner.offer = Some(Found {
                        version: capped(&found.version, 64),
                        notes: found.notes.map(|n| capped(&n, MAX_NOTES_CHARS)),
                        date: found.date.map(|d| capped(&d, 64)),
                        pending: found.pending,
                    });
                    None
                }
                Err(cause) => {
                    if !manual && inner.before_check == Phase::Available && inner.offer.is_some() {
                        inner.phase = Phase::Available;
                    } else {
                        inner.phase = Phase::CheckFailed { cause };
                        inner.offer = None;
                        inner.install_error = None;
                    }
                    manual.then_some(cause)
                }
            }
        };
        match failed {
            Some(cause) => Err(check_failed_text(cause)),
            None => Ok(self.status()),
        }
    }

    /// Marks the start of the download of the offered update. The caller
    /// makes sure that no session runs (`commands::begin_update_install`).
    pub fn begin_download(&self) -> Result<(u64, Arc<dyn PendingUpdate>), String> {
        let mut inner = lock(&self.inner);
        match inner.phase {
            Phase::Downloading { .. } | Phase::Ready => return Err(IN_PROGRESS.to_string()),
            Phase::Available => {}
            _ => return Err(NO_UPDATE.to_string()),
        }
        let Some(pending) = inner.offer.as_ref().map(|o| Arc::clone(&o.pending)) else {
            return Err(NO_UPDATE.to_string());
        };
        inner.download += 1;
        inner.phase = Phase::Downloading {
            received: 0,
            total: None,
        };
        inner.install_error = None;
        inner.abort = None;
        self.last_megabyte.store(u64::MAX, Ordering::SeqCst);
        Ok((inner.download, pending))
    }

    /// Keeps the function that stops the download `id`. When that download
    /// is not the running one any more, the function runs at once.
    pub fn set_abort(&self, id: u64, abort: Box<dyn FnOnce() + Send>) {
        let late = {
            let mut inner = lock(&self.inner);
            if inner.download == id && matches!(inner.phase, Phase::Downloading { .. }) {
                inner.abort = Some(abort);
                None
            } else {
                Some(abort)
            }
        };
        if let Some(abort) = late {
            abort();
        }
    }

    /// The progress of download `id`. True when the window must get an
    /// event: at most one for each megabyte.
    pub fn progress(&self, id: u64, received: u64, total: Option<u64>) -> bool {
        let mut inner = lock(&self.inner);
        if inner.download != id || !matches!(inner.phase, Phase::Downloading { .. }) {
            return false;
        }
        inner.phase = Phase::Downloading { received, total };
        let megabyte = received / 1_000_000;
        self.last_megabyte.swap(megabyte, Ordering::SeqCst) != megabyte
    }

    /// The end of download `id`. `Ok`: the signature is correct, the phase
    /// is `Ready`, and the result is true: the installer may start. A
    /// failure gives the offer again with the text.
    pub fn finish_download(&self, id: u64, result: Result<(), Cause>) -> bool {
        let mut inner = lock(&self.inner);
        if inner.download != id || !matches!(inner.phase, Phase::Downloading { .. }) {
            return false;
        }
        inner.abort = None;
        match result {
            Ok(()) => {
                inner.phase = Phase::Ready;
                true
            }
            Err(cause) => {
                inner.phase = Phase::Available;
                inner.install_error = Some(install_failed_text(cause));
                false
            }
        }
    }

    /// The installer of download `id` did not start.
    pub fn install_failed(&self, id: u64, cause: Cause) {
        let mut inner = lock(&self.inner);
        if inner.download == id && inner.phase == Phase::Ready {
            inner.phase = Phase::Available;
            inner.install_error = Some(install_failed_text(cause));
        }
    }

    /// Stops the download. The offer stays. Without a download it does
    /// nothing. After the download it is too late.
    pub fn cancel(&self) -> Result<(), String> {
        let abort = {
            let mut inner = lock(&self.inner);
            match inner.phase {
                Phase::Downloading { .. } => {
                    inner.download += 1;
                    inner.phase = Phase::Available;
                    inner.install_error = None;
                    inner.abort.take()
                }
                Phase::Ready => return Err(TOO_LATE.to_string()),
                _ => None,
            }
        };
        // The plugin keeps the download in memory: nothing stays on the disk.
        if let Some(abort) = abort {
            abort();
        }
        Ok(())
    }

    /// "Later": the window hides the note until the next start of the
    /// program or the next manual check.
    pub fn hide(&self) {
        lock(&self.inner).hidden = true;
    }
}

/// One line for the log of the program: version numbers and results only.
fn log(line: &str) {
    eprintln!("update: {line}");
}

/// Runs one check with `source`, and sends the new state to the window.
/// `manual` is a click on "Check for updates".
pub async fn run_check(
    updates: &Updates,
    source: &dyn UpdateSource,
    manual: bool,
    emit: &dyn UpdateEmit,
) -> Result<UpdateStatusDto, String> {
    if !updates.begin_check(manual)? {
        return Ok(updates.status());
    }
    emit.emit_update(&updates.status());
    let result = source.check().await;
    match &result {
        Ok(None) => log(&format!(
            "no newer version than {}",
            updates.current_version
        )),
        Ok(Some(found)) => log(&format!(
            "version {} is available, this is version {}",
            found.version, updates.current_version
        )),
        Err(cause) => log(&format!("the check failed: {}", cause.text())),
    }
    let status = updates.finish_check(result, now_ms());
    emit.emit_update(&updates.status());
    status
}

/// The automatic check. `enabled` is `updates.check` of the configuration
/// in use, or the error text when the file does not load. It never checks
/// in a developer build, when the switch is off, or when the file does not
/// load. A failure is quiet: one line in the log.
pub async fn auto_check(
    updates: &Updates,
    source: &dyn UpdateSource,
    enabled: Result<bool, String>,
    emit: &dyn UpdateEmit,
) {
    if updates.developer_build() {
        log("no automatic check: this is a developer build");
        return;
    }
    match enabled {
        Ok(true) => {
            let _ = run_check(updates, source, false, emit).await;
        }
        Ok(false) => log("no automatic check: updates.check is false"),
        Err(_) => log("no automatic check: the configuration does not load"),
    }
}

/// Downloads the update of `begin_download`, then starts the installer
/// after `pause`. The installer ends the program. A failure or a cancel
/// leaves the offer. `spawn_blocking` runs the pause and the install off
/// the async threads.
pub async fn run_download(
    updates: Arc<Updates>,
    id: u64,
    pending: Arc<dyn PendingUpdate>,
    emit: Arc<dyn UpdateEmit>,
    pause: Duration,
) {
    let received = AtomicU64::new(0);
    let progress = |chunk: u64, total: Option<u64>| {
        let now = received.fetch_add(chunk, Ordering::SeqCst) + chunk;
        if updates.progress(id, now, total) {
            emit.emit_update(&updates.status());
        }
    };
    let result = pending.download(&progress).await;
    let bytes = match result {
        Ok(bytes) => {
            if !updates.finish_download(id, Ok(())) {
                return;
            }
            bytes
        }
        Err(cause) => {
            log(&format!("the download failed: {}", cause.text()));
            updates.finish_download(id, Err(cause));
            // The state is new, or the same after a cancel: one more
            // event does no harm.
            emit.emit_update(&updates.status());
            return;
        }
    };
    log("the signature is correct, the installer starts");
    emit.emit_update(&updates.status());
    let install = {
        let pending = Arc::clone(&pending);
        tauri::async_runtime::spawn_blocking(move || {
            std::thread::sleep(pause);
            pending.install(bytes)
        })
    };
    let cause = match install.await {
        Ok(Ok(())) => return,
        Ok(Err(cause)) => cause,
        Err(_) => Cause::Unknown,
    };
    log(&format!("the install failed: {}", cause.text()));
    updates.install_failed(id, cause);
    emit.emit_update(&updates.status());
}

/// Starts `run_download` as a task, with its cancel.
pub fn start_download(
    updates: Arc<Updates>,
    id: u64,
    pending: Arc<dyn PendingUpdate>,
    emit: Arc<dyn UpdateEmit>,
    pause: Duration,
) {
    let task =
        tauri::async_runtime::spawn(run_download(Arc::clone(&updates), id, pending, emit, pause));
    updates.set_abort(id, Box::new(move || task.abort()));
}

/// The cause of an error of the plugin.
pub fn cause_of(error: &tauri_plugin_updater::Error, stage: Stage) -> Cause {
    use tauri_plugin_updater::Error as E;
    match error {
        E::Reqwest(e) if e.is_timeout() => Cause::Timeout,
        E::Reqwest(e) if stage == Stage::Check && e.is_decode() => Cause::BadData,
        E::Reqwest(e) if stage == Stage::Check && e.is_connect() => Cause::Connection,
        E::Reqwest(_) if stage == Stage::Check => Cause::Connection,
        E::Reqwest(_) | E::Network(_) => Cause::Download,
        E::ReleaseNotFound => Cause::Server,
        E::Serialization(_)
        | E::Semver(_)
        | E::UrlParse(_)
        | E::TargetNotFound(_)
        | E::TargetsNotFound(_) => Cause::BadData,
        E::Minisign(_)
        | E::Base64(_)
        | E::SignatureUtf8(_)
        | E::SignedVersionMismatch { .. }
        | E::MissingSignedVersion => Cause::Signature,
        E::InvalidUpdaterFormat | E::BinaryNotFoundInArchive => Cause::Format,
        E::Io(_) if stage == Stage::Install => Cause::InstallerStart,
        _ => Cause::Unknown,
    }
}

/// The step of an error of the plugin.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stage {
    Check,
    Download,
    Install,
}

/// The function that starts the installer. The real one is
/// `Update::install`: it checks the signature again, starts the installer,
/// and ends the program. The tests give a function that only records.
pub type InstallAction = Arc<
    dyn Fn(&tauri_plugin_updater::Update, &[u8]) -> Result<(), tauri_plugin_updater::Error>
        + Send
        + Sync,
>;

/// The real install: the plugin.
pub fn plugin_install() -> InstallAction {
    Arc::new(|update, bytes| update.install(bytes))
}

/// The update server through the plugin.
pub struct PluginSource {
    updater: tauri_plugin_updater::Updater,
    install: InstallAction,
}

/// An address and a public key for a test server. `pubkey` `None` keeps
/// the key of the configuration.
pub struct TestServer {
    pub endpoint: url::Url,
    pub pubkey: Option<String>,
}

impl PluginSource {
    /// The plugin with the address and the key of `tauri.conf.json`.
    /// `test` replaces them (only the tests give it). `before_exit` runs
    /// just before the installer starts and the program ends.
    pub fn new<R: tauri::Runtime, M: tauri::Manager<R>>(
        app: &M,
        test: Option<TestServer>,
        before_exit: impl Fn() + Send + Sync + 'static,
        install: InstallAction,
    ) -> Result<PluginSource, Cause> {
        use tauri_plugin_updater::UpdaterExt;
        let mut builder = app
            .updater_builder()
            .timeout(CHECK_TIMEOUT)
            .on_before_exit(before_exit);
        if let Some(test) = test {
            builder = builder
                .endpoints(vec![test.endpoint])
                .map_err(|e| cause_of(&e, Stage::Check))?;
            if let Some(pubkey) = test.pubkey {
                builder = builder.pubkey(pubkey);
            }
        }
        let updater = builder.build().map_err(|e| cause_of(&e, Stage::Check))?;
        Ok(PluginSource { updater, install })
    }
}

impl UpdateSource for PluginSource {
    fn check(&self) -> BoxFuture<'_, Result<Option<Found>, Cause>> {
        Box::pin(async move {
            let update = self
                .updater
                .check()
                .await
                .map_err(|e| cause_of(&e, Stage::Check))?;
            Ok(update.map(|mut update| {
                // The check has its time limit. The download has its own.
                update.timeout = Some(DOWNLOAD_TIMEOUT);
                let date = update
                    .raw_json
                    .get("pub_date")
                    .and_then(|d| d.as_str())
                    .map(str::to_string);
                Found {
                    version: update.version.clone(),
                    notes: update.body.clone(),
                    date,
                    pending: Arc::new(PluginPending {
                        update,
                        install: Arc::clone(&self.install),
                    }),
                }
            }))
        })
    }
}

/// An update of the plugin.
struct PluginPending {
    update: tauri_plugin_updater::Update,
    install: InstallAction,
}

impl PendingUpdate for PluginPending {
    fn download<'a>(&'a self, progress: Progress<'a>) -> BoxFuture<'a, Result<Vec<u8>, Cause>> {
        Box::pin(async move {
            // `download` checks the signature before it gives the bytes.
            self.update
                .download(|chunk, total| progress(chunk as u64, total), || {})
                .await
                .map_err(|e| cause_of(&e, Stage::Download))
        })
    }

    fn install(&self, bytes: Vec<u8>) -> Result<(), Cause> {
        (self.install)(&self.update, &bytes).map_err(|e| cause_of(&e, Stage::Install))
    }
}

/// The state machine with fake servers, and the plugin with a local server.
#[cfg(test)]
pub mod tests;
