use super::*;
use crate::events::tests::FakeEmit;
use std::collections::VecDeque;
use std::sync::atomic::AtomicUsize;

mod plugin;

/// What the fake download does.
pub enum Download {
    /// Sends the bytes in chunks of `chunk` bytes, then gives them.
    Bytes {
        bytes: Vec<u8>,
        chunk: usize,
    },
    Fail(Cause),
    /// Never ends: for the cancel.
    Forever,
}

/// An offered update that records each install.
pub struct FakePending {
    pub download: Download,
    pub install_result: Result<(), Cause>,
    pub installs: Mutex<Vec<Vec<u8>>>,
    pub downloads: AtomicUsize,
}

impl FakePending {
    pub fn new(download: Download) -> Arc<FakePending> {
        Arc::new(FakePending {
            download,
            install_result: Ok(()),
            installs: Mutex::new(Vec::new()),
            downloads: AtomicUsize::new(0),
        })
    }

    pub fn install_count(&self) -> usize {
        lock(&self.installs).len()
    }
}

impl PendingUpdate for FakePending {
    fn download<'a>(&'a self, progress: Progress<'a>) -> BoxFuture<'a, Result<Vec<u8>, Cause>> {
        self.downloads.fetch_add(1, Ordering::SeqCst);
        Box::pin(async move {
            match &self.download {
                Download::Bytes { bytes, chunk } => {
                    for part in bytes.chunks(*chunk) {
                        progress(part.len() as u64, Some(bytes.len() as u64));
                    }
                    Ok(bytes.clone())
                }
                Download::Fail(cause) => Err(*cause),
                Download::Forever => std::future::pending().await,
            }
        })
    }

    fn install(&self, bytes: Vec<u8>) -> Result<(), Cause> {
        lock(&self.installs).push(bytes);
        self.install_result
    }
}

/// One answer of the fake update server.
pub type CheckAnswer = Result<Option<Found>, Cause>;

/// An update server with a list of answers. A clone shares the answers
/// and the count.
#[derive(Default, Clone)]
pub struct FakeSource {
    pub answers: Arc<Mutex<VecDeque<CheckAnswer>>>,
    pub calls: Arc<AtomicUsize>,
}

impl FakeSource {
    pub fn with(answers: Vec<CheckAnswer>) -> FakeSource {
        FakeSource {
            answers: Arc::new(Mutex::new(answers.into())),
            calls: Arc::new(AtomicUsize::new(0)),
        }
    }

    pub fn calls(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }
}

impl UpdateSource for FakeSource {
    fn check(&self) -> BoxFuture<'_, Result<Option<Found>, Cause>> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        let answer = lock(&self.answers).pop_front().unwrap_or(Ok(None));
        Box::pin(async move { answer })
    }
}

pub fn found(version: &str, pending: Arc<dyn PendingUpdate>) -> Found {
    Found {
        version: version.to_string(),
        notes: Some("- Updates inside the program.".to_string()),
        date: Some("2026-12-01T10:00:00Z".to_string()),
        pending,
    }
}

pub fn release_build() -> Updates {
    Updates::new("0.2.0", false)
}

/// An update state of a release build with the offer of version 0.3.0.
pub fn with_offer(pending: Arc<dyn PendingUpdate>) -> Arc<Updates> {
    let updates = Arc::new(release_build());
    let source = FakeSource::with(vec![Ok(Some(found("0.3.0", pending)))]);
    block_on(run_check(&updates, &source, false, &FakeEmit::default())).unwrap();
    assert_eq!(updates.phase(), Phase::Available);
    updates
}

/// One tick of the automatic check with `source` at the time `now`.
pub fn auto(
    updates: &Updates,
    source: &FakeSource,
    enabled: Result<bool, String>,
    emit: &FakeEmit,
    now: u64,
) {
    let source = source.clone();
    let make = move || Ok(Box::new(source.clone()) as Box<dyn UpdateSource>);
    block_on(auto_check(updates, &make, enabled, emit, now));
}

/// A time 24 hours after now: an automatic check is due.
pub fn later() -> u64 {
    now_ms() + CHECK_INTERVAL_MS
}

/// The restore action of the tests: it counts its calls.
pub fn counting_restore() -> (RestoreUi, Arc<AtomicUsize>) {
    let count = Arc::new(AtomicUsize::new(0));
    let c = Arc::clone(&count);
    (
        Arc::new(move || {
            c.fetch_add(1, Ordering::SeqCst);
        }),
        count,
    )
}

pub fn block_on<F: Future>(future: F) -> F::Output {
    tauri::async_runtime::block_on(future)
}

/// The text of a refused `begin_download`.
pub fn refusal(updates: &Updates) -> String {
    match updates.begin_download() {
        Err(text) => text,
        Ok(_) => panic!("the download was not refused"),
    }
}

fn wait_for(what: &str, mut done: impl FnMut() -> bool) {
    let end = std::time::Instant::now() + Duration::from_secs(10);
    while !done() {
        assert!(std::time::Instant::now() < end, "timed out: {what}");
        std::thread::sleep(Duration::from_millis(5));
    }
}

#[test]
fn test_texts_are_exact() {
    assert_eq!(
        DEVELOPER_BUILD,
        "This is a developer build. It has no updates."
    );
    assert_eq!(STOP_FIRST, "Stop the session first.");
    assert_eq!(IN_PROGRESS, "The update is in progress.");
    assert_eq!(
        check_failed_text(Cause::Timeout),
        "The program could not check for updates: the server did not answer in time. \
         Try again later."
    );
    assert_eq!(
        install_failed_text(Cause::Signature),
        "The update did not install: the signature of the installer is not correct. \
         The program is not changed. Do not install this version by hand. Tell the \
         project on its Issues page."
    );
    assert_eq!(SIGNATURE_FAILED, install_failed_text(Cause::Signature));
    assert_eq!(
        install_failed_text(Cause::Download),
        "The update did not install: the download of the installer failed. \
         The program is not changed."
    );
    assert_eq!(
        release_page("0.3.0"),
        "https://github.com/Superbrain8/edcopilot-voicebridge/releases/tag/v0.3.0"
    );
    // A cause goes into the middle of a sentence: no address, no colon, no
    // full stop, and a small first letter.
    for cause in [
        Cause::Timeout,
        Cause::Connection,
        Cause::Server,
        Cause::BadData,
        Cause::Download,
        Cause::Signature,
        Cause::Format,
        Cause::InstallerStart,
        Cause::Unknown,
    ] {
        let text = cause.text();
        assert!(!text.contains("http") && !text.contains(':') && !text.ends_with('.'));
        assert!(text.chars().next().unwrap().is_lowercase(), "{text}");
    }
}

#[test]
fn test_a_manual_check_without_a_newer_version_is_up_to_date() {
    let updates = release_build();
    let emit = FakeEmit::default();
    let source = FakeSource::with(vec![Ok(None)]);
    let before = now_ms();
    let status = block_on(run_check(&updates, &source, true, &emit)).unwrap();
    assert_eq!(status.phase, "up_to_date");
    assert!(status.checked_at.unwrap() >= before);
    assert_eq!(status.version, None);
    assert_eq!(status.current_version, "0.2.0");
    assert_eq!(emit.phases(), ["checking", "up_to_date"]);
    assert_eq!(source.calls(), 1);
}

#[test]
fn test_a_check_that_finds_a_newer_version_offers_it() {
    let updates = release_build();
    let emit = FakeEmit::default();
    let pending = FakePending::new(Download::Fail(Cause::Download));
    let source = FakeSource::with(vec![Ok(Some(found("0.3.0", pending)))]);
    let status = block_on(run_check(&updates, &source, true, &emit)).unwrap();
    assert_eq!(status.phase, "available");
    assert_eq!(status.version.as_deref(), Some("0.3.0"));
    assert_eq!(
        status.notes.as_deref(),
        Some("- Updates inside the program.")
    );
    assert_eq!(status.date.as_deref(), Some("2026-12-01T10:00:00Z"));
    assert_eq!(
        status.release_page.as_deref(),
        Some("https://github.com/Superbrain8/edcopilot-voicebridge/releases/tag/v0.3.0")
    );
    assert!(!status.hidden);
    assert_eq!(emit.phases(), ["checking", "available"]);
}

#[test]
fn test_a_failed_manual_check_gives_the_text() {
    let updates = release_build();
    let emit = FakeEmit::default();
    let source = FakeSource::with(vec![Err(Cause::Connection)]);
    let error = block_on(run_check(&updates, &source, true, &emit)).unwrap_err();
    assert_eq!(
        error,
        "The program could not check for updates: the program could not connect to \
         the server. Try again later."
    );
    let status = updates.status();
    assert_eq!(status.phase, "check_failed");
    assert_eq!(
        status.cause.as_deref(),
        Some("the program could not connect to the server")
    );
    assert!(status.checked_at.is_some());
    assert_eq!(emit.phases(), ["checking", "check_failed"]);
}

#[test]
fn test_a_failed_automatic_check_is_quiet() {
    let updates = release_build();
    let emit = FakeEmit::default();
    let source = FakeSource::with(vec![Err(Cause::Timeout)]);
    auto(&updates, &source, Ok(true), &emit, later());
    assert_eq!(source.calls(), 1);
    // No error goes back. Only the Diagnostics screen shows the cause.
    let status = updates.status();
    assert_eq!(status.phase, "check_failed");
    assert_eq!(status.error, None);
}

#[test]
fn test_a_failed_automatic_check_keeps_an_earlier_offer() {
    let pending = FakePending::new(Download::Fail(Cause::Download));
    let updates = with_offer(pending);
    let source = FakeSource::with(vec![Err(Cause::Timeout)]);
    auto(&updates, &source, Ok(true), &FakeEmit::default(), later());
    let status = updates.status();
    assert_eq!(status.phase, "available");
    assert_eq!(status.version.as_deref(), Some("0.3.0"));
    // A manual check that fails drops the offer: the user sees the failure.
    let source = FakeSource::with(vec![Err(Cause::Timeout)]);
    block_on(run_check(&updates, &source, true, &FakeEmit::default())).unwrap_err();
    assert_eq!(updates.status().phase, "check_failed");
    assert_eq!(updates.status().version, None);
}

#[test]
fn test_the_switch_off_stops_only_the_automatic_check() {
    let updates = release_build();
    let source = FakeSource::with(vec![Ok(None)]);
    auto(&updates, &source, Ok(false), &FakeEmit::default(), later());
    // A configuration that does not load: no automatic check either.
    auto(
        &updates,
        &source,
        Err("bad file".into()),
        &FakeEmit::default(),
        later(),
    );
    assert_eq!(source.calls(), 0);
    assert_eq!(updates.status().phase, "idle");
    // The button still works.
    block_on(run_check(&updates, &source, true, &FakeEmit::default())).unwrap();
    assert_eq!(source.calls(), 1);
    assert_eq!(updates.status().phase, "up_to_date");
}

#[test]
fn test_a_developer_build_never_checks() {
    let updates = Updates::new("0.2.0", true);
    let emit = FakeEmit::default();
    let source = FakeSource::with(vec![Ok(None)]);
    auto(&updates, &source, Ok(true), &emit, later());
    let error = block_on(run_check(&updates, &source, true, &emit)).unwrap_err();
    assert_eq!(error, DEVELOPER_BUILD);
    assert_eq!(source.calls(), 0);
    assert!(emit.phases().is_empty());
    let status = updates.status();
    assert!(status.developer_build);
    assert_eq!(status.phase, "idle");
}

#[test]
fn test_later_hides_only_that_version_until_the_next_manual_check() {
    let pending = FakePending::new(Download::Fail(Cause::Download));
    let updates = with_offer(pending.clone());
    updates.hide();
    assert!(updates.status().hidden);
    // The next automatic check finds the same version: still hidden.
    let source = FakeSource::with(vec![Ok(Some(found("0.3.0", pending.clone())))]);
    auto(&updates, &source, Ok(true), &FakeEmit::default(), later());
    assert!(updates.status().hidden);
    // A newer version shows the note again.
    let source = FakeSource::with(vec![Ok(Some(found("0.4.0", pending.clone())))]);
    auto(
        &updates,
        &source,
        Ok(true),
        &FakeEmit::default(),
        later() + CHECK_INTERVAL_MS,
    );
    let status = updates.status();
    assert_eq!(status.version.as_deref(), Some("0.4.0"));
    assert!(!status.hidden);
    updates.hide();
    // A manual check shows the note again.
    let source = FakeSource::with(vec![Ok(Some(found("0.4.0", pending)))]);
    let status = block_on(run_check(&updates, &source, true, &FakeEmit::default())).unwrap();
    assert!(!status.hidden);
    assert_eq!(status.phase, "available");
}

#[test]
fn test_a_correct_download_gets_ready_and_starts_the_installer() {
    let bytes = vec![7u8; 2_500_000];
    let pending = FakePending::new(Download::Bytes {
        bytes: bytes.clone(),
        chunk: 100_000,
    });
    let updates = with_offer(pending.clone());
    let emit = Arc::new(FakeEmit::default());
    let (id, offered) = updates.begin_download().unwrap();
    block_on(run_download(
        Arc::clone(&updates),
        id,
        offered,
        emit.clone(),
        Duration::ZERO,
        counting_restore().0,
    ));
    assert_eq!(updates.phase(), Phase::Ready);
    assert_eq!(*lock(&pending.installs), vec![bytes]);
    // One progress event for each megabyte, then ready.
    assert_eq!(
        emit.phases(),
        ["downloading", "downloading", "downloading", "ready"]
    );
    let events = lock(&emit.0);
    assert_eq!(events[2]["received"], 2_000_000);
    assert_eq!(events[2]["total"], 2_500_000);
}

#[test]
fn test_a_failed_download_offers_the_update_again_and_installs_nothing() {
    let pending = FakePending::new(Download::Fail(Cause::Signature));
    let updates = with_offer(pending.clone());
    let emit = Arc::new(FakeEmit::default());
    let (id, offered) = updates.begin_download().unwrap();
    assert!(updates.blocks_session());
    block_on(run_download(
        Arc::clone(&updates),
        id,
        offered,
        emit.clone(),
        Duration::ZERO,
        counting_restore().0,
    ));
    assert_eq!(pending.install_count(), 0);
    let status = updates.status();
    assert_eq!(status.phase, "available");
    assert_eq!(status.error.as_deref(), Some(SIGNATURE_FAILED));
    assert!(!updates.blocks_session());
    // A new try clears the text.
    updates.begin_download().unwrap();
    assert_eq!(updates.status().error, None);
}

#[test]
fn test_an_installer_that_does_not_start_offers_the_update_again() {
    let pending = Arc::new(FakePending {
        download: Download::Bytes {
            bytes: vec![1, 2, 3],
            chunk: 3,
        },
        install_result: Err(Cause::InstallerStart),
        installs: Mutex::new(Vec::new()),
        downloads: AtomicUsize::new(0),
    });
    let updates = with_offer(pending.clone());
    let (id, offered) = updates.begin_download().unwrap();
    let (restore, restored) = counting_restore();
    block_on(run_download(
        Arc::clone(&updates),
        id,
        offered,
        Arc::new(FakeEmit::default()),
        Duration::ZERO,
        restore,
    ));
    assert_eq!(pending.install_count(), 1);
    // The tray icon and the window come back.
    assert_eq!(restored.load(Ordering::SeqCst), 1);
    assert!(!updates.blocks_session());
    let status = updates.status();
    assert_eq!(status.phase, "available");
    assert_eq!(
        status.error.as_deref(),
        Some(
            "The update did not install: the installer did not start. \
             The program is not changed."
        )
    );
}

#[test]
fn test_cancel_stops_the_download_and_keeps_the_offer() {
    let pending = FakePending::new(Download::Forever);
    let updates = with_offer(pending.clone());
    let emit = Arc::new(FakeEmit::default());
    let (id, offered) = updates.begin_download().unwrap();
    // One download at a time.
    assert_eq!(refusal(&updates), IN_PROGRESS);
    start_download(
        Arc::clone(&updates),
        id,
        offered,
        emit,
        Duration::ZERO,
        counting_restore().0,
    );
    wait_for("the download starts", || {
        pending.downloads.load(Ordering::SeqCst) == 1
    });
    updates.cancel().unwrap();
    assert_eq!(updates.phase(), Phase::Available);
    assert!(!updates.blocks_session());
    assert_eq!(updates.status().error, None);
    // A late result of the stopped download changes nothing.
    assert!(!updates.finish_download(id, Ok(())));
    assert!(!updates.progress(id, 5, None));
    assert_eq!(updates.phase(), Phase::Available);
    assert_eq!(pending.install_count(), 0);
    // A cancel without a download does nothing.
    updates.cancel().unwrap();
    // The user can try again.
    let (next, _) = updates.begin_download().unwrap();
    assert!(next > id);
}

#[test]
fn test_an_abort_after_the_cancel_runs_at_once() {
    let pending = FakePending::new(Download::Forever);
    let updates = with_offer(pending);
    let (id, _) = updates.begin_download().unwrap();
    updates.cancel().unwrap();
    let ran = Arc::new(AtomicUsize::new(0));
    let r = ran.clone();
    updates.set_abort(
        id,
        Box::new(move || {
            r.fetch_add(1, Ordering::SeqCst);
        }),
    );
    assert_eq!(ran.load(Ordering::SeqCst), 1);
}

#[test]
fn test_cancel_after_the_download_is_too_late() {
    let pending = FakePending::new(Download::Forever);
    let updates = with_offer(pending);
    let (id, _) = updates.begin_download().unwrap();
    assert!(updates.finish_download(id, Ok(())));
    assert_eq!(updates.cancel().unwrap_err(), TOO_LATE);
    assert_eq!(updates.phase(), Phase::Ready);
    assert!(updates.blocks_session());
}

#[test]
fn test_no_check_and_no_second_install_during_the_update() {
    let pending = FakePending::new(Download::Forever);
    let updates = with_offer(pending);
    updates.begin_download().unwrap();
    let source = FakeSource::with(vec![Ok(None)]);
    let error = block_on(run_check(&updates, &source, true, &FakeEmit::default())).unwrap_err();
    assert_eq!(error, IN_PROGRESS);
    auto(&updates, &source, Ok(true), &FakeEmit::default(), later());
    assert_eq!(source.calls(), 0);
    assert_eq!(refusal(&updates), IN_PROGRESS);
}

#[test]
fn test_install_without_an_offer_is_refused() {
    let updates = release_build();
    assert_eq!(refusal(&updates), NO_UPDATE);
    let source = FakeSource::with(vec![Ok(None)]);
    block_on(run_check(&updates, &source, true, &FakeEmit::default())).unwrap();
    assert_eq!(refusal(&updates), NO_UPDATE);
}

#[test]
fn test_long_notes_are_cut() {
    let updates = release_build();
    let mut offer = found("0.3.0", FakePending::new(Download::Forever));
    offer.notes = Some("x".repeat(MAX_NOTES_CHARS + 10));
    let source = FakeSource::with(vec![Ok(Some(offer))]);
    let status = block_on(run_check(&updates, &source, true, &FakeEmit::default())).unwrap();
    let notes = status.notes.unwrap();
    assert_eq!(notes.chars().count(), MAX_NOTES_CHARS);
    assert!(notes.ends_with('…'));
}

/// The configuration of the plugin: the fixed address, the key of the
/// project, and no dangerous switch.
#[test]
fn test_the_plugin_configuration_is_safe() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let config: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(dir.join("tauri.conf.json")).unwrap())
            .unwrap();
    let updater = &config["plugins"]["updater"];
    assert_eq!(updater["endpoints"], serde_json::json!([ENDPOINT]));
    assert!(ENDPOINT.starts_with("https://"));
    assert_eq!(
        updater["pubkey"],
        "dW50cnVzdGVkIGNvbW1lbnQ6IG1pbmlzaWduIHB1YmxpYyBrZXk6IDU1RDM2RDRGMTAzQzRFQzUKUldURlRqd1FUMjNUVmF5amVua21IVjRSemgreGNBd0ZsKytjaE5iWGVqMWJPUVJ6Ni93T21adUsK"
    );
    assert_eq!(updater["requireSignedVersion"], true);
    assert_eq!(updater["windows"]["installMode"], "passive");
    let keys: Vec<&String> = updater.as_object().unwrap().keys().collect();
    assert_eq!(
        keys.len(),
        4,
        "no other key, for example no dangerous switch and no allowDowngrades: {keys:?}"
    );
    // The build never needs the key: the release workflow signs the finished
    // installer in a separate job with `tauri signer sign`.
    assert_eq!(config["bundle"]["createUpdaterArtifacts"], false);
    // The page gets no updater permission.
    for file in std::fs::read_dir(dir.join("capabilities")).unwrap() {
        let text = std::fs::read_to_string(file.unwrap().path()).unwrap();
        assert!(!text.contains("updater"), "{text}");
    }
}

/// A download that is complete is not stopped by a late abort handle: the
/// task starts the installer.
#[test]
fn test_set_abort_after_the_download_does_not_abort() {
    let pending = FakePending::new(Download::Forever);
    let updates = with_offer(pending);
    let (id, _) = updates.begin_download().unwrap();
    assert!(updates.finish_download(id, Ok(())));
    let ran = Arc::new(AtomicUsize::new(0));
    let r = ran.clone();
    updates.set_abort(
        id,
        Box::new(move || {
            r.fetch_add(1, Ordering::SeqCst);
        }),
    );
    assert_eq!(ran.load(Ordering::SeqCst), 0);
    assert_eq!(updates.phase(), Phase::Ready);
}

/// The automatic check follows the clock of the computer: one check, then
/// the next one 24 hours after the end of the last check.
#[test]
fn test_the_automatic_check_is_due_by_the_clock() {
    let updates = release_build();
    let source = FakeSource::with(vec![Ok(None), Ok(None), Ok(None)]);
    let emit = FakeEmit::default();
    auto(&updates, &source, Ok(true), &emit, 1_000);
    assert_eq!(source.calls(), 1);
    // The check sets the time of its end: now_ms of the test computer.
    let end = updates.status().checked_at.unwrap();
    auto(&updates, &source, Ok(true), &emit, end + 60_000);
    auto(
        &updates,
        &source,
        Ok(true),
        &emit,
        end + CHECK_INTERVAL_MS - 1,
    );
    assert_eq!(source.calls(), 1);
    auto(&updates, &source, Ok(true), &emit, end + CHECK_INTERVAL_MS);
    assert_eq!(source.calls(), 2);
    let end = updates.status().checked_at.unwrap();
    assert!(!updates.auto_check_due(end + 1));
    assert!(updates.auto_check_due(end + CHECK_INTERVAL_MS));
}

/// A switch from off to on applies at the next tick.
#[test]
fn test_the_switch_on_applies_at_the_next_tick() {
    let updates = release_build();
    let source = FakeSource::with(vec![Ok(None)]);
    let emit = FakeEmit::default();
    auto(&updates, &source, Ok(false), &emit, 1_000);
    assert_eq!(source.calls(), 0);
    auto(
        &updates,
        &source,
        Ok(true),
        &emit,
        1_000 + TICK.as_millis() as u64,
    );
    assert_eq!(source.calls(), 1);
}

/// A failure to make the update server is quiet and shows on Diagnostics.
#[test]
fn test_no_update_server_is_a_quiet_failure() {
    let updates = release_build();
    let make = || Err::<Box<dyn UpdateSource>, Cause>(Cause::Unknown);
    block_on(auto_check(
        &updates,
        &make,
        Ok(true),
        &FakeEmit::default(),
        0,
    ));
    let status = updates.status();
    assert_eq!(status.phase, "check_failed");
    assert_eq!(status.cause.as_deref(), Some("an unknown error occurred"));
}

/// An automatic check keeps the offer on the screen while it runs, and the
/// install uses that offer.
#[test]
fn test_an_automatic_check_keeps_the_offer_on_the_screen() {
    let pending = FakePending::new(Download::Forever);
    let updates = with_offer(pending.clone());
    assert!(updates.begin_check(false).unwrap());
    assert_eq!(updates.status().phase, "available");
    assert!(!updates.status().hidden);
    // Install during the check: the offer of the earlier check.
    updates.begin_download().unwrap();
    // The end of the check does not change the install.
    let other = FakePending::new(Download::Forever);
    updates
        .finish_check(Ok(Some(found("0.5.0", other))), 5)
        .unwrap();
    let status = updates.status();
    assert_eq!(status.phase, "downloading");
    assert_eq!(status.version.as_deref(), Some("0.3.0"));
    // The discarded result does not count as a check.
    assert_ne!(status.checked_at, Some(5));
    // A manual check shows "checking".
    let fresh = with_offer(pending);
    assert!(fresh.begin_check(true).unwrap());
    assert_eq!(fresh.status().phase, "checking");
}

/// The version rule: only a newer release version. A pre-release and build
/// metadata do not count.
#[test]
fn test_only_a_newer_release_version_is_offered() {
    let v = |text: &str| semver::Version::parse(text).unwrap();
    let current = v("0.2.0");
    assert!(is_offered(&current, &v("0.2.1")));
    assert!(is_offered(&current, &v("0.3.0")));
    assert!(is_offered(&current, &v("1.0.0")));
    assert!(is_offered(&current, &v("0.3.0+build.7")));
    assert!(!is_offered(&current, &v("0.2.0")));
    assert!(!is_offered(&current, &v("0.1.9")));
    assert!(!is_offered(&current, &v("0.2.0+build.7")));
    assert!(!is_offered(&current, &v("0.3.0-beta.1")));
    assert!(!is_offered(&current, &v("1.0.0-rc.1")));
}

/// The public key of tauri.conf.json is a minisign public key: base64 of a
/// text with the minisign header line, and a key line of 42 bytes
/// (algorithm "Ed", 8 bytes key id, 32 bytes Ed25519 key).
#[test]
fn test_the_project_public_key_decodes_to_a_minisign_key() {
    use base64::Engine;
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let config: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(dir.join("tauri.conf.json")).unwrap())
            .unwrap();
    let pubkey = config["plugins"]["updater"]["pubkey"].as_str().unwrap();
    let engine = base64::engine::general_purpose::STANDARD;
    let text = String::from_utf8(engine.decode(pubkey).unwrap()).unwrap();
    let mut lines = text.lines();
    let header = lines.next().unwrap();
    assert!(
        header.starts_with("untrusted comment: minisign public key: "),
        "{header}"
    );
    let key = engine.decode(lines.next().unwrap().trim()).unwrap();
    assert_eq!(key.len(), 42);
    assert_eq!(&key[..2], b"Ed");
    // The key id of the header is the id in the key, as hex, last byte first.
    let id: String = key[2..10]
        .iter()
        .rev()
        .map(|b| format!("{b:02X}"))
        .collect();
    assert!(header.ends_with(&id), "{header} {id}");
}

/// A last check in the future (a clock that went back) does not stop the
/// automatic check.
#[test]
fn test_a_last_check_in_the_future_is_due() {
    let updates = release_build();
    updates.begin_check(false).unwrap();
    updates.finish_check(Ok(None), 5_000_000).unwrap();
    assert!(!updates.auto_check_due(5_000_001));
    assert!(updates.auto_check_due(4_999_999));
    assert!(updates.auto_check_due(1_000));
}

/// An automatic check that ends during a download discards its result and
/// does not count as a check: the next tick checks again.
#[test]
fn test_a_discarded_check_does_not_set_the_time() {
    let pending = FakePending::new(Download::Forever);
    let updates = with_offer(pending);
    let before = updates.status().checked_at;
    assert!(updates.begin_check(false).unwrap());
    let (id, _) = updates.begin_download().unwrap();
    updates.finish_check(Ok(None), 9).unwrap();
    assert_eq!(updates.status().checked_at, before);
    // The download fails: the offer is back, and the check is due.
    updates.finish_download(id, Err(Cause::Download));
    assert_eq!(updates.status().phase, "available");
    assert!(updates.auto_check_due(before.unwrap() + CHECK_INTERVAL_MS));
}

/// `ui/signing-key.pub` holds the public key that signs the releases. The
/// release job checks the signature with it. Normally it is the key of
/// tauri.conf.json. Only a bridge release (`BRIDGE_RELEASE`) has a new key
/// in tauri.conf.json and the old key in the file.
#[test]
fn test_the_signing_key_file_follows_the_bridge_rule() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let config: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(dir.join("tauri.conf.json")).unwrap())
            .unwrap();
    let in_config = config["plugins"]["updater"]["pubkey"].as_str().unwrap();
    let signing = std::fs::read_to_string(dir.join("signing-key.pub")).unwrap();
    let signing = signing.trim();
    assert!(!signing.is_empty());
    if BRIDGE_RELEASE {
        assert_ne!(signing, in_config, "a bridge release has a new key");
    } else {
        assert_eq!(
            signing, in_config,
            "the releases sign with the key of the program"
        );
    }
}
