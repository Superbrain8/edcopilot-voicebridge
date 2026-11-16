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
//! - Only a newer release version counts (`is_offered`): no pre-release, no
//!   equal or older version, and build metadata does not count.
//! - The update address and the key are fixed in `tauri.conf.json`. The
//!   download address must be an asset of the project (`DOWNLOAD_PREFIX`).
//!   Only a test build can give another address, key, or install action
//!   (`PluginSource::for_test`).
//! - The window program has no log file. The Diagnostics screen shows the
//!   result of the last check, and the Status screen the update steps.
//!
//! Locks: `Updates::inner` is the last lock. Its holder takes no other lock
//! and calls no other code. A start of a session takes it while it holds
//! the capture lock (see `commands::begin_update_install`).

use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::task::Poll;
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
/// Each download address in `latest.json` must start with this text: an
/// asset of a release of the project.
pub const DOWNLOAD_PREFIX: &str =
    "https://github.com/Superbrain8/edcopilot-voicebridge/releases/download/";
/// The release page of a version is this text and the version.
pub const RELEASE_PAGE_PREFIX: &str =
    "https://github.com/Superbrain8/edcopilot-voicebridge/releases/tag/v";

/// The time limit of one check.
pub const CHECK_TIMEOUT: Duration = Duration::from_secs(15);
/// The time limit of the download of the installer.
pub const DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(30 * 60);
/// The largest installer that the program downloads, in bytes.
pub const MAX_DOWNLOAD_BYTES: u64 = 200_000_000;
/// The first automatic check comes this time after the start.
pub const FIRST_CHECK_DELAY: Duration = Duration::from_secs(10);
/// The time between two looks at the clock of the automatic check.
pub const TICK: Duration = Duration::from_secs(30 * 60);
/// The time between two automatic checks, by the clock of the computer.
pub const CHECK_INTERVAL_MS: u64 = 24 * 60 * 60 * 1000;
/// The time that the window shows the ready text before the program closes.
pub const READY_PAUSE: Duration = Duration::from_secs(2);
/// False for a normal release: `ui/signing-key.pub` (the key that signs the
/// release) is the key of `tauri.conf.json`. True only for a bridge release
/// of a key change: `tauri.conf.json` has the NEW key, and the release is
/// signed with the OLD key of `ui/signing-key.pub`. A test checks the rule.
/// See "A new key (rotation)" in docs/DEVELOPMENT.md.
pub const BRIDGE_RELEASE: bool = false;
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
/// The text of a signature that is not correct. The user must not install
/// this version by hand: the file can be changed.
pub const SIGNATURE_FAILED: &str = "The update did not install: the signature of the \
     installer is not correct. The program is not changed. Do not install this version by \
     hand. Tell the project on its Issues page.";

/// The text of a manual check that failed.
pub fn check_failed_text(cause: Cause) -> String {
    format!(
        "The program could not check for updates: {}. Try again later.",
        cause.text()
    )
}

/// The text of a download or an install that failed.
pub fn install_failed_text(cause: Cause) -> String {
    if cause == Cause::Signature {
        return SIGNATURE_FAILED.to_string();
    }
    format!(
        "The update did not install: {}. The program is not changed.",
        cause.text()
    )
}

/// The release page of a version.
pub fn release_page(version: &str) -> String {
    format!("{RELEASE_PAGE_PREFIX}{version}")
}

/// True when the program offers `release` to a program of version
/// `current`: a release version (no pre-release part) with a higher major,
/// minor, or patch number. Build metadata does not count.
pub fn is_offered(current: &semver::Version, release: &semver::Version) -> bool {
    release.pre.is_empty()
        && (release.major, release.minor, release.patch)
            > (current.major, current.minor, current.patch)
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

/// The progress callback of a download: the size of one more part, and the
/// size of the file, if the server sent it.
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
    /// A check runs, and the window has no offer to show.
    Checking,
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
    /// True while a check runs. An automatic check keeps an offer on the
    /// screen: the phase stays `Available`.
    check_running: bool,
    /// True when a click on "Check for updates" waits for the running check.
    check_manual: bool,
    /// The offered update, with `Available`, `Downloading`, and `Ready`.
    offer: Option<Found>,
    /// The text of the last failed download or install of the offer.
    install_error: Option<String>,
    /// The end of the last check, in milliseconds since 1970.
    checked_at: Option<u64>,
    /// The version of the offer when the user clicked "Later". The note
    /// stays hidden for this version only, until the next manual check.
    hidden_version: Option<String>,
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
                check_running: false,
                check_manual: false,
                offer: None,
                install_error: None,
                checked_at: None,
                hidden_version: None,
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
            Phase::Checking => ("checking", 0, None, None),
            Phase::UpToDate => ("up_to_date", 0, None, None),
            Phase::Available => ("available", 0, None, None),
            Phase::Downloading { received, total } => ("downloading", *received, *total, None),
            Phase::Ready => ("ready", 0, None, None),
            Phase::CheckFailed { cause } => {
                ("check_failed", 0, None, Some(cause.text().to_string()))
            }
        };
        let offer = inner.offer.as_ref();
        let hidden = offer.is_some_and(|o| inner.hidden_version.as_ref() == Some(&o.version));
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
            hidden,
        }
    }

    /// True during the download and the install: a session must not start.
    pub fn blocks_session(&self) -> bool {
        matches!(
            lock(&self.inner).phase,
            Phase::Downloading { .. } | Phase::Ready
        )
    }

    /// True when an automatic check is due at the time `now`: before the
    /// first check, or 24 hours after the end of the last check, by the
    /// clock of the computer.
    pub fn auto_check_due(&self, now: u64) -> bool {
        match lock(&self.inner).checked_at {
            None => true,
            // A time in the future: the clock went back. Check again.
            Some(at) => at > now || now - at >= CHECK_INTERVAL_MS,
        }
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
        if matches!(inner.phase, Phase::Downloading { .. } | Phase::Ready) {
            return if manual {
                Err(IN_PROGRESS.to_string())
            } else {
                Ok(false)
            };
        }
        if manual {
            inner.hidden_version = None;
        }
        if inner.check_running {
            // The running check shows its result to this user too.
            inner.check_manual |= manual;
            return Ok(false);
        }
        inner.check_running = true;
        inner.check_manual = manual;
        // An automatic check keeps an offer on the screen. "Install update"
        // stays possible and uses that offer.
        if manual || inner.phase != Phase::Available {
            inner.phase = Phase::Checking;
        }
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
            let manual = inner.check_manual;
            inner.check_running = false;
            inner.check_manual = false;
            if matches!(inner.phase, Phase::Downloading { .. } | Phase::Ready) {
                // The user started the install of the offer during an
                // automatic check. The install keeps its offer. The result
                // is discarded and does not count as a check.
                None
            } else {
                inner.checked_at = Some(now);
                match result {
                    Ok(None) => {
                        inner.phase = Phase::UpToDate;
                        inner.offer = None;
                        inner.install_error = None;
                        None
                    }
                    Ok(Some(found)) => {
                        let version = capped(&found.version, 64);
                        let same = inner.offer.as_ref().is_some_and(|o| o.version == version);
                        if !same {
                            inner.install_error = None;
                        }
                        inner.phase = Phase::Available;
                        inner.offer = Some(Found {
                            version,
                            notes: found.notes.map(|n| capped(&n, MAX_NOTES_CHARS)),
                            date: found.date.map(|d| capped(&d, 64)),
                            pending: found.pending,
                        });
                        None
                    }
                    Err(cause) => {
                        if !manual && inner.phase == Phase::Available && inner.offer.is_some() {
                            // The offer of the earlier check stays.
                        } else {
                            inner.phase = Phase::CheckFailed { cause };
                            inner.offer = None;
                            inner.install_error = None;
                        }
                        manual.then_some(cause)
                    }
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
    /// was cancelled, the function runs at once. When the download is
    /// complete (`Ready`), the function is dropped and does not run: the
    /// task starts the installer.
    pub fn set_abort(&self, id: u64, abort: Box<dyn FnOnce() + Send>) {
        let late = {
            let mut inner = lock(&self.inner);
            let current = inner.download == id;
            if current && matches!(inner.phase, Phase::Downloading { .. }) {
                inner.abort = Some(abort);
                None
            } else if current && inner.phase == Phase::Ready {
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

    /// "Later": the window hides the note of this version until the next
    /// start of the program or the next manual check. A newer version shows
    /// the note again.
    pub fn hide(&self) {
        let mut inner = lock(&self.inner);
        inner.hidden_version = inner.offer.as_ref().map(|o| o.version.clone());
    }
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
    let status = updates.finish_check(result, now_ms());
    emit.emit_update(&updates.status());
    status
}

/// One tick of the automatic check at the time `now`. `enabled` is
/// `updates.check` of the configuration in use, or the error text when the
/// file does not load. It checks only when the check is due, never in a
/// developer build, never when the switch is off, and never when the file
/// does not load. A failure is quiet: the Diagnostics screen shows it.
/// `source` makes the update server only when the check runs.
pub async fn auto_check(
    updates: &Updates,
    source: &dyn Fn() -> Result<Box<dyn UpdateSource>, Cause>,
    enabled: Result<bool, String>,
    emit: &dyn UpdateEmit,
    now: u64,
) {
    if updates.developer_build() || enabled != Ok(true) || !updates.auto_check_due(now) {
        return;
    }
    match source() {
        Ok(source) => {
            let _ = run_check(updates, &*source, false, emit).await;
        }
        Err(cause) => {
            if updates.begin_check(false) == Ok(true) {
                let _ = updates.finish_check(Err(cause), now);
                emit.emit_update(&updates.status());
            }
        }
    }
}

/// Puts the window back after an installer that did not start: the tray
/// icon and the main window. The tests give a function that records.
pub type RestoreUi = Arc<dyn Fn() + Send + Sync>;

/// Downloads the update of `begin_download`, then starts the installer
/// after `pause`. The installer ends the program. A failure or a cancel
/// leaves the offer. When the installer does not start, `restore` shows the
/// tray icon and the window again. `spawn_blocking` runs the pause and the
/// install off the async threads.
pub async fn run_download(
    updates: Arc<Updates>,
    id: u64,
    pending: Arc<dyn PendingUpdate>,
    emit: Arc<dyn UpdateEmit>,
    pause: Duration,
    restore: RestoreUi,
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
            updates.finish_download(id, Err(cause));
            // The state is new, or the same after a cancel: one more
            // event does no harm.
            emit.emit_update(&updates.status());
            return;
        }
    };
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
    updates.install_failed(id, cause);
    restore();
    emit.emit_update(&updates.status());
}

/// Starts `run_download` as a task, with its cancel.
pub fn start_download(
    updates: Arc<Updates>,
    id: u64,
    pending: Arc<dyn PendingUpdate>,
    emit: Arc<dyn UpdateEmit>,
    pause: Duration,
    restore: RestoreUi,
) {
    let task = tauri::async_runtime::spawn(run_download(
        Arc::clone(&updates),
        id,
        pending,
        emit,
        pause,
        restore,
    ));
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

/// The install action of the tests: it replaces `Update::install`, so that
/// no test runs a file.
#[cfg(test)]
pub type InstallAction = Arc<
    dyn Fn(&tauri_plugin_updater::Update, &[u8]) -> Result<(), tauri_plugin_updater::Error>
        + Send
        + Sync,
>;

/// The update server through the plugin.
pub struct PluginSource {
    updater: tauri_plugin_updater::Updater,
    /// `DOWNLOAD_PREFIX`. Only a test build has another value.
    allowed_prefix: String,
    /// `MAX_DOWNLOAD_BYTES`. Only a test build has another value.
    max_bytes: u64,
    #[cfg(test)]
    install: InstallAction,
}

/// The test server of `PluginSource::for_test`: its address, a key (`None`
/// keeps the key of the configuration), the start of its download
/// addresses, and a size limit.
#[cfg(test)]
pub struct TestServer {
    pub endpoint: url::Url,
    pub pubkey: Option<String>,
    pub allowed_prefix: String,
    pub max_bytes: u64,
}

/// The plugin builder with the rules of the program: the check time limit,
/// the version rule of `is_offered`, and `before_exit`.
fn builder<R: tauri::Runtime, M: tauri::Manager<R>>(
    app: &M,
    before_exit: impl Fn() + Send + Sync + 'static,
) -> tauri_plugin_updater::UpdaterBuilder {
    use tauri_plugin_updater::UpdaterExt;
    app.updater_builder()
        .timeout(CHECK_TIMEOUT)
        .version_comparator(|current, release| is_offered(&current, &release.version))
        .on_before_exit(before_exit)
}

impl PluginSource {
    /// The plugin with the address and the key of `tauri.conf.json`.
    /// `before_exit` runs just before the installer starts.
    pub fn new<R: tauri::Runtime, M: tauri::Manager<R>>(
        app: &M,
        before_exit: impl Fn() + Send + Sync + 'static,
    ) -> Result<PluginSource, Cause> {
        let updater = builder(app, before_exit)
            .build()
            .map_err(|e| cause_of(&e, Stage::Check))?;
        Ok(PluginSource {
            updater,
            allowed_prefix: DOWNLOAD_PREFIX.to_string(),
            max_bytes: MAX_DOWNLOAD_BYTES,
            #[cfg(test)]
            install: Arc::new(|update, bytes| update.install(bytes)),
        })
    }

    /// The plugin with a test server and a test install action.
    #[cfg(test)]
    pub fn for_test<R: tauri::Runtime, M: tauri::Manager<R>>(
        app: &M,
        server: TestServer,
        install: InstallAction,
    ) -> Result<PluginSource, Cause> {
        let mut builder = builder(app, || {})
            .endpoints(vec![server.endpoint])
            .map_err(|e| cause_of(&e, Stage::Check))?;
        if let Some(pubkey) = server.pubkey {
            builder = builder.pubkey(pubkey);
        }
        let updater = builder.build().map_err(|e| cause_of(&e, Stage::Check))?;
        Ok(PluginSource {
            updater,
            allowed_prefix: server.allowed_prefix,
            max_bytes: server.max_bytes,
            install,
        })
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
            let Some(mut update) = update else {
                return Ok(None);
            };
            // Only an asset of a release of the project.
            if !update
                .download_url
                .as_str()
                .starts_with(&self.allowed_prefix)
            {
                return Err(Cause::BadData);
            }
            // The check has its time limit. The download has its own.
            update.timeout = Some(DOWNLOAD_TIMEOUT);
            let date = update
                .raw_json
                .get("pub_date")
                .and_then(|d| d.as_str())
                .map(str::to_string);
            Ok(Some(Found {
                version: update.version.clone(),
                notes: update.body.clone(),
                date,
                pending: Arc::new(PluginPending {
                    update,
                    max_bytes: self.max_bytes,
                    #[cfg(test)]
                    install: Arc::clone(&self.install),
                }),
            }))
        })
    }
}

/// An update of the plugin.
struct PluginPending {
    update: tauri_plugin_updater::Update,
    max_bytes: u64,
    #[cfg(test)]
    install: InstallAction,
}

impl PendingUpdate for PluginPending {
    fn download<'a>(&'a self, progress: Progress<'a>) -> BoxFuture<'a, Result<Vec<u8>, Cause>> {
        Box::pin(async move {
            let max = self.max_bytes;
            let received = AtomicU64::new(0);
            let too_large = AtomicBool::new(false);
            // `download` checks the signature before it gives the bytes.
            let download = self.update.download(
                |chunk, total| {
                    let chunk = chunk as u64;
                    let now = received.fetch_add(chunk, Ordering::SeqCst) + chunk;
                    if now > max || total.is_some_and(|t| t > max) {
                        too_large.store(true, Ordering::SeqCst);
                    }
                    progress(chunk, total);
                },
                || {},
            );
            let mut download = std::pin::pin!(download);
            // A file above the limit stops the download: the drop of the
            // future ends the request.
            std::future::poll_fn(|cx| {
                if too_large.load(Ordering::SeqCst) {
                    return Poll::Ready(Err(Cause::Download));
                }
                match download.as_mut().poll(cx) {
                    Poll::Ready(_) if too_large.load(Ordering::SeqCst) => {
                        Poll::Ready(Err(Cause::Download))
                    }
                    Poll::Ready(result) => {
                        Poll::Ready(result.map_err(|e| cause_of(&e, Stage::Download)))
                    }
                    Poll::Pending if too_large.load(Ordering::SeqCst) => {
                        Poll::Ready(Err(Cause::Download))
                    }
                    Poll::Pending => Poll::Pending,
                }
            })
            .await
        })
    }

    fn install(&self, bytes: Vec<u8>) -> Result<(), Cause> {
        // `install` checks the signature again, starts the installer, and
        // ends the program.
        #[cfg(not(test))]
        let result = self.update.install(&bytes);
        #[cfg(test)]
        let result = (self.install)(&self.update, &bytes);
        result.map_err(|e| cause_of(&e, Stage::Install))
    }
}

/// The state machine with fake servers, and the plugin with a local server.
#[cfg(test)]
pub mod tests;
