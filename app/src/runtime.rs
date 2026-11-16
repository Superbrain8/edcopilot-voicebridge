//! The voice session with its threads. It reports what happens to an
//! `EventSink`, and it prints nothing.
//!
//! Threads: audio callback -> input channel -> builder thread -> clip
//! channel (bounded) -> worker thread with the `Session`. The talk button
//! thread also sends into the input channel. A vocabulary thread reloads the
//! names after file events. A control thread starts the microphone and the
//! talk button, waits for the stop, and stops the other threads. An event
//! thread is the only thread that calls the sink.

use std::any::Any;
use std::collections::{BTreeSet, VecDeque};
use std::ffi::OsString;
use std::fs::{File, OpenOptions, TryLockError};
use std::panic::{self, AssertUnwindSafe};
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender, SyncSender, TrySendError};
use std::sync::Arc;
use std::thread::{self, JoinHandle, ThreadId};
use std::time::{Duration, Instant, SystemTime};

use anyhow::bail;
use edcopilot_voicebridge_audio::{
    list_input_devices, select_device_name, spawn_talk_button, ButtonSource, Capture, EnergyVad,
    JoystickButton, KeyboardKey, Segmenter, SegmenterConfig, TalkButtonHandle, TalkEvent,
};
use edcopilot_voicebridge_core::{watch_dynamic_vocab, Pipeline};
use edcopilot_voicebridge_stt::{AudioClip, SttEngine};

use crate::commands_source::{profile_change, save_report, ProfileWatch, PROFILE_CHANGED};
use crate::config::{build_pipeline, load_vocabulary, Config, LoadReport};
use crate::diagnostics::{levels, Diagnostics, Levels, Marks};
use crate::run::{
    diagnostics_lines, engine_problems, hotword_confusions, make_engine_with_line, mode_line,
    mode_of, session_options, talk_source, TalkSource,
};
use crate::session::{Outcome, Session};

/// The lines of the start, for the status of the session.
#[derive(Debug, Clone)]
pub struct StartInfo {
    pub report: LoadReport,
    /// The name of the speech engine.
    pub engine: String,
    pub mode: Mode,
    /// The mode with the talk button, for example `push to talk, key "F13"`.
    pub mode_line: String,
    /// `None`: the default input device.
    pub input_device: Option<String>,
    /// The processor threads of the local engine. `None` for the cloud engine.
    pub threads: Option<usize>,
    pub dry_run: bool,
    /// The hotword line of the local engine. Empty for the cloud engine.
    pub hotwords_line: String,
    /// The lines of the utterance log and the audio capture, when set.
    pub diagnostics_lines: Vec<String>,
}

/// The result of one utterance.
#[derive(Debug, Clone)]
pub struct UtteranceReport {
    pub outcome: Outcome,
    /// The time of the transcription, with the second pass when it ran.
    pub transcribe: Duration,
    /// The time from the end of the clip to the result.
    pub total: Duration,
    /// The length of the clip.
    pub clip: Duration,
    pub levels: Levels,
    pub marks: Marks,
    /// The name of the saved clip, when the audio capture is on.
    pub audio_file: Option<String>,
}

/// Why the session stopped.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StopReason {
    /// `Runtime::stop`.
    Requested,
    /// A fatal error of the audio stream, for example a removed microphone.
    DeviceError(String),
    /// The worker thread with the speech engine stopped with a panic.
    EngineError(String),
    /// The builder thread stopped with a panic.
    InputError(String),
}

/// What happens in the session. No text holds an API key.
#[derive(Debug, Clone)]
pub enum RuntimeEvent {
    /// The complete text of a warning line, as the console prints it on
    /// stderr. It also comes for a clip that the full clip channel dropped
    /// ("worker busy") and for a result of the second pass that the session
    /// did not use.
    Warning(String),
    /// The session runs. It is the first event after the warnings of the
    /// start. In a box: the start lines with the load report are large.
    Started(Box<StartInfo>),
    /// The talk button went down (`true`) or up (`false`).
    Talk(bool),
    /// The result of one clip. It comes after the clip save and the line of
    /// the utterance log, because the report has the name of the saved clip.
    Utterance(UtteranceReport),
    /// A clip that was not sent: it waited too long for the worker, or the
    /// stop came between its match and its send. The console prints
    /// "utterance dropped: <reason>".
    Dropped { reason: String },
    /// The vocabulary files changed and the names were loaded again.
    VocabularyReloaded { files: usize, names: usize },
    /// The last event. It comes exactly once.
    Stopped { reason: StopReason, glitches: u64 },
}

/// Gets the events of a runtime.
///
/// One thread of the runtime calls `event`, one event at a time, in the
/// order of the events. A slow sink does not stop the audio path: the
/// events wait in a queue. A panic in `event` loses that event only.
/// `event` can stop or drop its runtime (see `Runtime::stop`).
pub trait EventSink: Send + Sync + 'static {
    fn event(&self, event: RuntimeEvent);
}

pub struct RuntimeOptions {
    /// Match, but do not write the request file.
    pub dry_run: bool,
}

/// Gives the API key for the name of a variable.
pub type KeyLookup = Arc<dyn Fn(&str) -> Option<String> + Send + Sync>;

/// The problems of a start that failed.
#[derive(Debug, Clone)]
pub struct StartError {
    pub problems: Vec<String>,
    /// A start step failed (the data, the device, the engine, the microphone
    /// or the talk button), not the check of the configuration. The console
    /// prints these problems as `error:` lines, as before the runtime.
    pub(crate) step_failed: bool,
    /// The stop flag was set during the start. The console stops with exit
    /// code 0 and prints nothing, as for a stop after the start.
    pub(crate) cancelled: bool,
}

impl StartError {
    fn problems(problems: Vec<String>) -> StartError {
        StartError {
            problems,
            step_failed: false,
            cancelled: false,
        }
    }

    fn step(problem: String) -> StartError {
        StartError {
            problems: vec![problem],
            step_failed: true,
            cancelled: false,
        }
    }

    fn cancelled() -> StartError {
        StartError {
            problems: vec!["the start was cancelled".to_string()],
            step_failed: false,
            cancelled: true,
        }
    }
}

/// `Err` when the stop flag is set: the start stops at this step.
fn check_cancel(stop: &AtomicBool) -> Result<(), StartError> {
    if stop.load(Ordering::SeqCst) {
        Err(StartError::cancelled())
    } else {
        Ok(())
    }
}

/// A running voice session.
pub struct Runtime {
    /// Set at the first stop signal. The builder and the worker then drop
    /// the queued work, so no token goes out after the stop.
    stopping: Arc<AtomicBool>,
    control: Sender<StopReason>,
    control_thread: Option<JoinHandle<()>>,
    event_thread: Option<JoinHandle<()>>,
    /// The thread that calls the sink. `stop` on this thread does not wait
    /// for it, because a thread that waits for itself never continues.
    event_thread_id: ThreadId,
}

impl Runtime {
    /// Checks the configuration, loads the data, makes the engine, and starts
    /// the threads. A problem gives `Err` with all problem texts, and no
    /// thread runs.
    pub fn start(
        config: &Config,
        options: RuntimeOptions,
        key_lookup: KeyLookup,
        sink: Arc<dyn EventSink>,
    ) -> Result<Runtime, StartError> {
        let stop = Arc::new(AtomicBool::new(false));
        Runtime::start_with_stop(config, options, key_lookup, sink, stop)
    }

    /// `start` with a stop flag of the caller. The runtime uses this flag as
    /// its stop flag. When the flag is set during the start, the start stops
    /// at the next step (the model load itself does not stop) and gives
    /// `Err` with the problem "the start was cancelled". After the start, a
    /// set flag makes the threads drop their queued work, as `stop` does.
    /// The caller must still call `stop` to end the session.
    pub fn start_with_stop(
        config: &Config,
        options: RuntimeOptions,
        key_lookup: KeyLookup,
        sink: Arc<dyn EventSink>,
        stop: Arc<AtomicBool>,
    ) -> Result<Runtime, StartError> {
        Runtime::start_on(config, options, key_lookup, sink, stop, Box::new(Machine))
    }

    /// `start_with_stop` with the engine, the microphone and the talk button
    /// of `hardware`.
    pub(crate) fn start_on(
        config: &Config,
        options: RuntimeOptions,
        key_lookup: KeyLookup,
        sink: Arc<dyn EventSink>,
        stop: Arc<AtomicBool>,
        hardware: Box<dyn Hardware>,
    ) -> Result<Runtime, StartError> {
        let (event_tx, event_rx) = mpsc::channel::<RuntimeEvent>();
        let event_thread = thread::spawn(move || {
            for event in event_rx {
                // A panic in the sink loses this event only. The next
                // events, and `Stopped`, still go to the sink.
                let _ = panic::catch_unwind(AssertUnwindSafe(|| sink.event(event)));
            }
        });
        let event_thread_id = event_thread.thread().id();
        match launch(
            config,
            &options,
            &key_lookup,
            Events(event_tx),
            stop,
            hardware,
        ) {
            Ok(parts) => Ok(Runtime {
                stopping: parts.stopping,
                control: parts.control,
                control_thread: Some(parts.control_thread),
                event_thread: Some(event_thread),
                event_thread_id,
            }),
            Err(error) => {
                // All senders are gone: the event thread gives the warnings
                // of the start to the sink and ends.
                let _ = event_thread.join();
                Err(error)
            }
        }
    }

    /// Asks the threads to stop and waits for them. Safe to call twice.
    ///
    /// This call blocks until the clip in flight is complete. With the cloud
    /// engine that can be the full time-out of the engine. Thus do not hold a
    /// lock that the sink also takes while you call `stop`.
    ///
    /// After the return, the sink got `Stopped`. Exception: a call from
    /// inside the sink (on the event thread, also through a drop of the
    /// `Runtime`) does not wait for the event thread. Then `Stopped` comes
    /// after the sink returns.
    pub fn stop(&mut self) {
        self.stopping.store(true, Ordering::SeqCst);
        let _ = self.control.send(StopReason::Requested);
        if let Some(thread) = self.control_thread.take() {
            let _ = thread.join();
        }
        if let Some(thread) = self.event_thread.take() {
            if thread::current().id() != self.event_thread_id {
                let _ = thread.join();
            }
        }
    }

    /// False after the first stop signal, also after an error stop.
    pub fn is_running(&self) -> bool {
        !self.stopping.load(Ordering::SeqCst)
    }
}

impl Drop for Runtime {
    fn drop(&mut self) {
        self.stop();
    }
}

/// The sender of the events. A send to a sink that is gone does nothing.
#[derive(Clone)]
pub(crate) struct Events(Sender<RuntimeEvent>);

impl Events {
    fn send(&self, event: RuntimeEvent) {
        let _ = self.0.send(event);
    }

    /// Events that go nowhere, for tests of one loop.
    #[cfg(test)]
    fn none() -> Events {
        Events(mpsc::channel().0)
    }
}

/// The lock of one session for each request file.
///
/// The lock file is `voicebridge-<hash>.lock` in a folder of the user (see
/// `user_lock_dir`), never in the EDCoPilot folder. The hash comes from the
/// request file path. The lock file holds no data: only the lock of the
/// operating system on it counts. The system releases that lock when the
/// file closes, also when the process stops without a clean exit. The file
/// itself stays.
struct SessionLock {
    _file: File,
}

impl SessionLock {
    /// Makes the lock folder when it is absent. A folder that cannot be made
    /// or that is not safe refuses the start (fail closed).
    fn acquire(lock_dir: &Path, request_file: &Path) -> Result<SessionLock, String> {
        prepare_lock_dir(lock_dir)?;
        let path = lock_file(lock_dir, request_file);
        let mut options = OpenOptions::new();
        // Never truncate: the file holds no data, and a link is not followed.
        options.create(true).write(true).truncate(false);
        private_no_follow(&mut options);
        let file = options
            .open(&path)
            .map_err(|e| format!("cannot open the session lock {}: {e}", path.display()))?;
        match file.try_lock() {
            Ok(()) => Ok(SessionLock { _file: file }),
            Err(TryLockError::WouldBlock) => Err(format!(
                "a voice session for {} is already running",
                request_file.display()
            )),
            Err(TryLockError::Error(e)) => Err(format!(
                "cannot lock the session lock {}: {e}",
                path.display()
            )),
        }
    }
}

/// The per-user folder of the session lock files. `var` reads an environment
/// variable. Windows: `%LOCALAPPDATA%\VoiceBridge`. Other systems:
/// `$XDG_RUNTIME_DIR/voicebridge`, else `$XDG_STATE_HOME/voicebridge`, else
/// `$HOME/.local/state/voicebridge`. A value that is not an absolute path is
/// not used. There is no shared folder as a fallback: without a per-user
/// folder the result is an error, and the start is refused.
fn user_lock_dir(var: &dyn Fn(&str) -> Option<OsString>) -> Result<PathBuf, String> {
    let absolute = |name: &str| var(name).map(PathBuf::from).filter(|p| p.is_absolute());
    #[cfg(windows)]
    let dir = absolute("LOCALAPPDATA")
        .map(|d| d.join("VoiceBridge"))
        .ok_or_else(|| {
            "cannot find the folder for the session lock: LOCALAPPDATA is not set".to_string()
        });
    #[cfg(not(windows))]
    let dir = absolute("XDG_RUNTIME_DIR")
        .or_else(|| absolute("XDG_STATE_HOME"))
        .or_else(|| absolute("HOME").map(|home| home.join(".local").join("state")))
        .map(|d| d.join("voicebridge"))
        .ok_or_else(|| {
            "cannot find a per-user folder for the session lock: \
             XDG_RUNTIME_DIR, XDG_STATE_HOME and HOME are not set"
                .to_string()
        });
    dir
}

/// The folder of the saved profile report: the folder of the session lock
/// (`user_lock_dir`) of this user.
pub fn report_dir() -> Result<PathBuf, String> {
    user_lock_dir(&|name| std::env::var_os(name))
}

/// Makes the lock folder when it is absent (mode 0700 on Unix). On Unix the
/// folder must be a real folder, not a link, owned by this user, and other
/// users must not be able to write in it.
fn prepare_lock_dir(dir: &Path) -> Result<(), String> {
    make_private_dir(dir)
        .map_err(|e| format!("cannot open the session lock folder {}: {e}", dir.display()))?;
    #[cfg(unix)]
    check_private_dir(dir)?;
    Ok(())
}

#[cfg(unix)]
fn make_private_dir(dir: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::DirBuilderExt;
    std::fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(dir)
}

#[cfg(not(unix))]
fn make_private_dir(dir: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)
}

#[cfg(unix)]
fn check_private_dir(dir: &Path) -> Result<(), String> {
    use std::os::unix::fs::MetadataExt;
    let refuse = |why: &str| {
        Err(format!(
            "the session lock folder {} is not safe: {why}",
            dir.display()
        ))
    };
    let meta = std::fs::symlink_metadata(dir)
        .map_err(|e| format!("cannot read the session lock folder {}: {e}", dir.display()))?;
    if meta.file_type().is_symlink() {
        return refuse("it is a symbolic link");
    }
    if !meta.is_dir() {
        return refuse("it is not a folder");
    }
    if meta.mode() & 0o022 != 0 {
        return refuse("other users can write in it");
    }
    let me =
        current_uid().map_err(|e| format!("cannot find the user for the session lock: {e}"))?;
    if meta.uid() != me {
        return refuse("another user owns it");
    }
    Ok(())
}

/// The user of the process. std has no `getuid`, and `libc::getuid` needs
/// `unsafe`. A new file belongs to the user of the process, so the owner of
/// a new anonymous temporary file gives the user.
#[cfg(unix)]
fn current_uid() -> std::io::Result<u32> {
    use std::os::unix::fs::MetadataExt;
    Ok(tempfile::tempfile()?.metadata()?.uid())
}

/// The lock file is open only for this user and does not follow a link.
#[cfg(unix)]
fn private_no_follow(options: &mut OpenOptions) {
    use std::os::unix::fs::OpenOptionsExt;
    options.mode(0o600).custom_flags(libc::O_NOFOLLOW);
}

/// The lock file does not follow a reparse point (a link). The folder is in
/// the profile of the user.
#[cfg(windows)]
fn private_no_follow(options: &mut OpenOptions) {
    use std::os::windows::fs::OpenOptionsExt;
    use windows_sys::Win32::Storage::FileSystem::FILE_FLAG_OPEN_REPARSE_POINT;
    options.custom_flags(FILE_FLAG_OPEN_REPARSE_POINT);
}

#[cfg(not(any(unix, windows)))]
fn private_no_follow(_options: &mut OpenOptions) {}

/// The lock file for a request file. Two spellings of one path give the same
/// lock file: relative or absolute, with `.` or `..` parts, and on Windows
/// with other upper and lower case. The request file need not exist.
fn lock_file(lock_dir: &Path, request_file: &Path) -> PathBuf {
    let key = lock_key(request_file);
    lock_dir.join(format!("voicebridge-{:016x}.lock", fnv1a(key.as_bytes())))
}

/// The absolute path without `.` and `..` parts. When the folder exists, it
/// is the real path of the folder (links resolved), and the file name.
fn lock_key(request_file: &Path) -> String {
    let absolute = std::path::absolute(request_file).unwrap_or_else(|_| request_file.into());
    let mut clean = PathBuf::new();
    for part in absolute.components() {
        match part {
            Component::CurDir => {}
            Component::ParentDir => {
                clean.pop();
            }
            other => clean.push(other),
        }
    }
    if let (Some(folder), Some(name)) = (clean.parent(), clean.file_name()) {
        if let Ok(real) = std::fs::canonicalize(folder) {
            clean = real.join(name);
        }
    }
    let key = clean.to_string_lossy().into_owned();
    // Windows paths do not see a difference in case.
    #[cfg(windows)]
    let key = key.to_lowercase();
    key
}

/// The 64-bit FNV-1a hash. It is the same in each build, so the console
/// program and the user interface find the same lock file.
fn fnv1a(bytes: &[u8]) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for &byte in bytes {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

/// Where the runtime gets its speech engine, its microphone and its talk
/// button. The tests use a fake machine.
pub(crate) trait Hardware: Send + 'static {
    /// The folder of the session lock files. `Err` refuses the start.
    fn lock_dir(&self) -> Result<PathBuf, String>;

    /// The input device of the configuration. `None`: the default device.
    fn input_device(&self, config: &Config) -> Result<Option<String>, String>;

    /// The engine and its hotword line (empty when the engine has none).
    fn engine(
        &mut self,
        config: &Config,
        key_lookup: &dyn Fn(&str) -> Option<String>,
    ) -> anyhow::Result<(Box<dyn SttEngine>, String)>;

    /// Starts the talk button. It sends its events into `inputs`. Dropping
    /// the result stops it.
    fn talk_button(
        &mut self,
        config: &Config,
        inputs: Sender<Input>,
    ) -> anyhow::Result<Box<dyn Any + Send>>;

    /// Starts the microphone on the control thread. It sends its samples
    /// into `inputs` and nothing else. Dropping the result stops it.
    fn capture(
        &mut self,
        device: Option<&str>,
        glitches: Arc<AtomicU64>,
        inputs: Sender<Input>,
        on_error: Box<dyn FnMut(String) + Send>,
    ) -> anyhow::Result<Box<dyn Any>>;
}

/// The real engine, microphone and talk button.
struct Machine;

impl Hardware for Machine {
    /// The per-user folder of `user_lock_dir`, never the EDCoPilot folder.
    fn lock_dir(&self) -> Result<PathBuf, String> {
        user_lock_dir(&|name| std::env::var_os(name))
    }

    fn input_device(&self, config: &Config) -> Result<Option<String>, String> {
        select_device_name(Some(&config.microphone.device), &list_input_devices())
    }

    fn engine(
        &mut self,
        config: &Config,
        key_lookup: &dyn Fn(&str) -> Option<String>,
    ) -> anyhow::Result<(Box<dyn SttEngine>, String)> {
        let (engine, line) = make_engine_with_line(config, key_lookup)?;
        Ok((engine, line.unwrap_or_default()))
    }

    fn talk_button(
        &mut self,
        config: &Config,
        inputs: Sender<Input>,
    ) -> anyhow::Result<Box<dyn Any + Send>> {
        Ok(Box::new(start_talk_button(config, inputs)?))
    }

    fn capture(
        &mut self,
        device: Option<&str>,
        glitches: Arc<AtomicU64>,
        inputs: Sender<Input>,
        on_error: Box<dyn FnMut(String) + Send>,
    ) -> anyhow::Result<Box<dyn Any>> {
        let capture = Capture::start_counted(
            device,
            glitches,
            // No lock, no print, no event and no file access here: only a send.
            move |samples| {
                let _ = inputs.send(Input::Samples(samples.to_vec()));
            },
            on_error,
        )?;
        Ok(Box::new(capture))
    }
}

/// What `launch` gives to the `Runtime`.
struct Launched {
    stopping: Arc<AtomicBool>,
    control: Sender<StopReason>,
    control_thread: JoinHandle<()>,
}

/// The steps of the start, in the order of the console program before the
/// runtime. `events` is the only event sender that stays after the start:
/// it goes to the control thread.
fn launch(
    config: &Config,
    options: &RuntimeOptions,
    key_lookup: &KeyLookup,
    events: Events,
    stopping: Arc<AtomicBool>,
    mut hardware: Box<dyn Hardware>,
) -> Result<Launched, StartError> {
    let mut problems = config.validate();
    problems.extend(engine_problems(config, &**key_lookup));
    let request_file = &config.edcopilot.request_file;
    let lock_dir = hardware.lock_dir();
    let lock = match lock_dir
        .clone()
        .and_then(|dir| SessionLock::acquire(&dir, request_file))
    {
        Ok(lock) => Some(lock),
        Err(problem) => {
            problems.push(problem);
            None
        }
    };
    let Some(lock) = lock.filter(|_| problems.is_empty()) else {
        return Err(StartError::problems(problems));
    };

    check_cancel(&stopping)?;
    // The watch takes the stamp before the read: a change during the build
    // also gives the warning.
    let profile = &config.edcopilot.profile;
    let mut profile_watch = (!profile.as_os_str().is_empty()).then(|| ProfileWatch::new(profile));
    let (pipeline, report) =
        build_pipeline(config).map_err(|e| StartError::step(format!("{e:#}")))?;
    for problem in report.problems.iter().chain(report.warnings()) {
        events.send(RuntimeEvent::Warning(format!("warning: {problem}")));
    }
    let built = &report.profile;
    if let Some(watch) = profile_watch.as_mut() {
        watch.set_session_hash(&built.source.sha256);
    }
    // The difference to the saved report, before the start saves the new
    // report. Else a change while no session ran is never shown. The line
    // is the line of `check`, with no prefix.
    if let Some(change) = profile_change(built, lock_dir.as_deref().ok(), profile) {
        events.send(RuntimeEvent::Warning(change.line()));
    }
    let profile_report = report.profile.clone();
    let profile_path = profile.clone();
    let checked_slots = report.checked_slots.clone();
    let vocabulary_warnings = report.vocabulary_warnings.clone();
    let confusions = hotword_confusions(config, &pipeline);
    if !confusions.is_empty() {
        return Err(StartError::problems(confusions));
    }
    check_cancel(&stopping)?;
    let pipeline = Arc::new(pipeline);
    let mode = mode_of(config);
    let device = hardware.input_device(config).map_err(StartError::step)?;
    // A start error of the engine stops the start before any thread starts.
    let (engine, hotwords_line) = hardware
        .engine(config, &**key_lookup)
        .map_err(|e| StartError::step(format!("{e:#}")))?;
    // The model load can take seconds: a stop during it ends the start here.
    check_cancel(&stopping)?;
    let info = StartInfo {
        report,
        engine: engine.name().to_string(),
        mode,
        mode_line: mode_line(config, mode),
        input_device: device.clone(),
        threads: (config.stt.engine == "parakeet").then_some(config.stt.parakeet.threads),
        dry_run: options.dry_run,
        hotwords_line,
        diagnostics_lines: diagnostics_lines(config),
    };

    let vocab = watch_vocab(&config.edcopilot.dynamic_vars_dir, &events);
    let after_start = events.clone();
    let warnings = events.clone();
    let diagnostics = Diagnostics::with_warnings(
        &config.diagnostics.log_file,
        &config.diagnostics.save_audio_dir,
        move |line| warnings.send(RuntimeEvent::Warning(line)),
    );
    let notes = events.clone();
    let session = Session::new(
        engine,
        Arc::clone(&pipeline),
        session_options(config, options.dry_run),
    )
    .with_notes(move |line| notes.send(RuntimeEvent::Warning(line)))
    .with_stop(Arc::clone(&stopping));

    let (control_tx, control_rx) = mpsc::channel::<StopReason>();
    let (ready_tx, ready_rx) = mpsc::channel::<Result<(), StartError>>();
    let parts = SessionParts {
        config: config.clone(),
        mode,
        device,
        info,
        session,
        diagnostics,
        utterances: utterance_builder(config, mode),
        pipeline,
        vocab,
        checked_slots,
        vocabulary_warnings,
        profile_watch,
        lock,
        events,
        stopping: Arc::clone(&stopping),
        control_tx: control_tx.clone(),
        control_rx,
        ready: ready_tx,
    };
    let control_thread = thread::spawn(move || control_loop(parts, hardware));
    match ready_rx.recv() {
        Ok(Ok(())) => {
            // Only a session start writes the report: `check` shows the
            // difference to it.
            if let Ok(dir) = &lock_dir {
                if let Err(e) = save_report(dir, &profile_report, &profile_path) {
                    after_start.send(RuntimeEvent::Warning(format!("warning: {e:#}")));
                }
            }
            Ok(Launched {
                stopping,
                control: control_tx,
                control_thread,
            })
        }
        Ok(Err(error)) => {
            let _ = control_thread.join();
            Err(error)
        }
        Err(_) => {
            let _ = control_thread.join();
            Err(StartError::step(
                "the session stopped during the start".to_string(),
            ))
        }
    }
}

/// All that the control thread needs.
struct SessionParts {
    config: Config,
    mode: Mode,
    device: Option<String>,
    info: StartInfo,
    session: Session,
    diagnostics: Diagnostics,
    utterances: UtteranceBuilder,
    pipeline: Arc<Pipeline>,
    vocab: Option<(Box<dyn Any + Send>, Receiver<()>)>,
    /// The slots whose files the vocabulary reload checks.
    checked_slots: BTreeSet<String>,
    /// The warnings of the vocabulary check at the start.
    vocabulary_warnings: Vec<String>,
    /// The watch of the profile file. `None` when the path of the profile
    /// is empty.
    profile_watch: Option<ProfileWatch>,
    lock: SessionLock,
    events: Events,
    stopping: Arc<AtomicBool>,
    control_tx: Sender<StopReason>,
    control_rx: Receiver<StopReason>,
    /// `Ok` when the session runs, or the problem of the start.
    ready: Sender<Result<(), StartError>>,
}

/// Sends a stop reason when its thread ends with a panic.
struct PanicGuard {
    control: Sender<StopReason>,
    stopping: Arc<AtomicBool>,
    reason: StopReason,
}

impl Drop for PanicGuard {
    fn drop(&mut self) {
        if thread::panicking() {
            // Set the flag first, so that no other thread takes more work.
            self.stopping.store(true, Ordering::SeqCst);
            let _ = self.control.send(self.reason.clone());
        }
    }
}

/// Joins its thread when it is dropped, also during a panic.
struct JoinOnDrop(Option<JoinHandle<()>>);

impl Drop for JoinOnDrop {
    fn drop(&mut self) {
        if let Some(thread) = self.0.take() {
            let _ = thread.join();
        }
    }
}

/// Sets the stop flag when it is dropped.
struct SetOnDrop(Arc<AtomicBool>);

impl Drop for SetOnDrop {
    fn drop(&mut self) {
        self.0.store(true, Ordering::SeqCst);
    }
}

/// The parts of a running session. The fields drop in this order, also
/// during a panic: the stop flag, the microphone, the talk button, then the
/// threads. Dropping the input senders ends the builder thread, then the
/// worker thread. Only a clip in flight can still finish.
struct Running {
    _stop: SetOnDrop,
    _capture: Box<dyn Any>,
    _talk: Option<Box<dyn Any + Send>>,
    _builder: JoinOnDrop,
    _worker: JoinOnDrop,
    _watcher: Option<Box<dyn Any + Send>>,
    _vocab: JoinOnDrop,
}

/// The end of the control thread, also after a panic: it releases the lock,
/// then it sends `Stopped` when `Started` went out.
struct Finish {
    events: Events,
    lock: Option<SessionLock>,
    started: bool,
    reason: Option<StopReason>,
    glitches: u64,
}

impl Drop for Finish {
    fn drop(&mut self) {
        drop(self.lock.take());
        if self.started {
            let reason = self.reason.take().unwrap_or_else(|| {
                StopReason::InputError(
                    "the session control stopped with an internal error".to_string(),
                )
            });
            self.events.send(RuntimeEvent::Stopped {
                reason,
                glitches: self.glitches,
            });
        }
    }
}

/// The control thread. It keeps the microphone on the thread that made it.
fn control_loop(parts: SessionParts, mut hardware: Box<dyn Hardware>) {
    let SessionParts {
        config,
        mode,
        device,
        info,
        session,
        diagnostics,
        utterances,
        pipeline,
        vocab,
        checked_slots,
        vocabulary_warnings,
        profile_watch,
        lock,
        events,
        stopping,
        control_tx,
        control_rx,
        ready,
    } = parts;
    // Declared first, so that it drops last.
    let mut finish = Finish {
        events: events.clone(),
        lock: Some(lock),
        started: false,
        reason: None,
        glitches: 0,
    };
    let (input_tx, input_rx) = mpsc::channel();
    let talk = match mode {
        Mode::PushToTalk => match hardware.talk_button(&config, input_tx.clone()) {
            Ok(talk) => Some(talk),
            Err(e) => {
                let _ = ready.send(Err(StartError::step(format!("{e:#}"))));
                return;
            }
        },
        Mode::AlwaysListening => None,
    };
    // The capture counts non-fatal stream errors here. The builder reads it.
    let glitches = Arc::new(AtomicU64::new(0));
    let flag = Arc::clone(&stopping);
    let stop_tx = control_tx.clone();
    let on_error = Box::new(move |error: String| {
        // Set the flag first, so that the worker takes no more clips.
        flag.store(true, Ordering::SeqCst);
        let _ = stop_tx.send(StopReason::DeviceError(error));
    });
    let capture =
        match hardware.capture(device.as_deref(), Arc::clone(&glitches), input_tx, on_error) {
            Ok(capture) => capture,
            Err(e) => {
                drop(talk);
                let _ = ready.send(Err(StartError::step(format!("{e:#}"))));
                return;
            }
        };
    if let Err(cancelled) = check_cancel(&stopping) {
        drop(capture);
        drop(talk);
        let _ = ready.send(Err(cancelled));
        return;
    }

    // The threads that send events start after `Started`.
    events.send(RuntimeEvent::Started(Box::new(info)));
    finish.started = true;
    let _ = ready.send(Ok(()));
    let (watcher, rx) = match vocab {
        Some((watcher, rx)) => (Some(watcher), Some(rx)),
        None => (None, None),
    };
    let vocab = (rx.is_some() || profile_watch.is_some()).then(|| {
        start_vocab_reload(
            VocabReload {
                dir: config.edcopilot.dynamic_vars_dir.clone(),
                checked_slots,
                warnings: vocabulary_warnings,
                profile_watch,
            },
            rx,
            pipeline,
            events.clone(),
            Arc::clone(&stopping),
        )
    });
    let (clip_tx, clip_rx) = mpsc::sync_channel(CLIP_CHANNEL_CAPACITY);
    let worker = {
        let guard = PanicGuard {
            control: control_tx.clone(),
            stopping: Arc::clone(&stopping),
            reason: StopReason::EngineError(
                "the speech engine stopped with an internal error".to_string(),
            ),
        };
        let flag = Arc::clone(&stopping);
        let events = events.clone();
        thread::spawn(move || {
            let _guard = guard;
            worker_loop(session, clip_rx, &flag, Instant::now, diagnostics, &events);
        })
    };
    let builder = {
        let guard = PanicGuard {
            control: control_tx.clone(),
            stopping: Arc::clone(&stopping),
            reason: StopReason::InputError(
                "the audio input stopped with an internal error".to_string(),
            ),
        };
        let flag = Arc::clone(&stopping);
        let glitches = Arc::clone(&glitches);
        let events = events.clone();
        thread::spawn(move || {
            let _guard = guard;
            builder_loop(
                utterances,
                input_rx,
                clip_tx,
                &flag,
                &glitches,
                Instant::now,
                &events,
            );
        })
    };
    drop(control_tx);
    drop(events);
    let running = Running {
        _stop: SetOnDrop(Arc::clone(&stopping)),
        _capture: capture,
        _talk: talk,
        _builder: JoinOnDrop(Some(builder)),
        _worker: JoinOnDrop(Some(worker)),
        _watcher: watcher,
        _vocab: JoinOnDrop(vocab),
    };

    // `Runtime` keeps a sender, so the receive ends only with a reason.
    let reason = control_rx.recv().unwrap_or(StopReason::Requested);
    stopping.store(true, Ordering::SeqCst);
    finish.glitches = glitches.load(Ordering::Relaxed);
    finish.reason = Some(reason);
    drop(running);
    // `finish` drops here: the lock goes free, then `Stopped` goes out.
}

/// A push-to-talk clip shorter than this is dropped (300 ms).
pub const PTT_MIN_SAMPLES: usize = 4_800;
/// A push-to-talk clip stops at this length (15 s).
pub const PTT_MAX_SAMPLES: usize = 240_000;
/// Clips that wait for the worker. A full channel drops the new clip.
pub const CLIP_CHANNEL_CAPACITY: usize = 4;
/// A clip that waited longer than this for the worker is dropped: a command
/// that fires long after the user spoke is a wrong action.
pub const MAX_CLIP_WAIT: Duration = Duration::from_secs(5);
/// Quiet time after the last vocabulary file event before the reload.
pub const VOCAB_DEBOUNCE: Duration = Duration::from_millis(500);

/// Poll interval of the talk button.
const TALK_POLL: Duration = Duration::from_millis(10);
/// How often the vocabulary thread checks the debouncer.
const VOCAB_POLL: Duration = Duration::from_millis(100);

/// Gives one signal when `quiet` passed after the last `touch`.
pub struct Debouncer {
    quiet: Duration,
    last: Option<Instant>,
}

impl Debouncer {
    pub fn new(quiet: Duration) -> Debouncer {
        Debouncer { quiet, last: None }
    }

    /// Records an event. The quiet time starts again.
    pub fn touch(&mut self, now: Instant) {
        self.last = Some(now);
    }

    /// `true` one time, when `quiet` passed after the last `touch`.
    pub fn fire(&mut self, now: Instant) -> bool {
        match self.last {
            Some(t) if now.saturating_duration_since(t) >= self.quiet => {
                self.last = None;
                true
            }
            _ => false,
        }
    }
}

/// How the program finds the start and end of an utterance.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    PushToTalk,
    AlwaysListening,
}

/// Samples in one millisecond at 16 kHz.
const SAMPLES_PER_MS: usize = 16;

/// The number of 16 kHz samples in `ms` milliseconds.
pub fn ms_to_samples(ms: u32) -> usize {
    ms as usize * SAMPLES_PER_MS
}

/// One push-to-talk recording.
struct Recording {
    /// Pre-roll, then the samples since `Pressed`, up to `PTT_MAX_SAMPLES`.
    samples: Vec<f32>,
    /// Samples that arrived between `Pressed` and `Released`. The minimum
    /// press length applies to this count only.
    held: usize,
    /// After `Released`: the tail samples that must still arrive.
    tail_left: Option<usize>,
}

impl Recording {
    fn push(&mut self, samples: &[f32]) {
        let room = PTT_MAX_SAMPLES.saturating_sub(self.samples.len());
        self.samples
            .extend_from_slice(&samples[..room.min(samples.len())]);
    }

    fn into_clip(self) -> AudioClip {
        AudioClip {
            samples: self.samples,
        }
    }
}

/// Turns samples and talk button events into clips.
///
/// Push-to-talk: a clip comes out of `on_talk` (at `Released` without a
/// tail, or at a `Pressed` during the tail) or out of `on_samples` (when
/// the last tail sample arrives).
pub struct UtteranceBuilder {
    mode: Mode,
    segmenter: Segmenter,
    /// Push-to-talk: samples of audio before the press.
    pre_roll: usize,
    /// Push-to-talk: samples of audio after the release.
    release_tail: usize,
    /// Push-to-talk: the newest samples while no recording runs, at most
    /// `pre_roll`. It is empty while a recording runs, so it never holds a
    /// sample that is also in a clip.
    ring: VecDeque<f32>,
    recording: Option<Recording>,
}

impl UtteranceBuilder {
    /// The segmenter is used only in `AlwaysListening` mode. No pre-roll
    /// and no tail: see `with_push_to_talk_timing`.
    pub fn new(mode: Mode, segmenter: Segmenter) -> UtteranceBuilder {
        UtteranceBuilder {
            mode,
            segmenter,
            pre_roll: 0,
            release_tail: 0,
            ring: VecDeque::new(),
            recording: None,
        }
    }

    /// Push-to-talk: the recording starts with the last `pre_roll` samples
    /// before `Pressed` and ends when `release_tail` samples arrived after
    /// `Released`. The tail counts samples, not time, so that it also covers
    /// the samples that were late in the audio path. 0 switches a part off.
    pub fn with_push_to_talk_timing(mut self, pre_roll: usize, release_tail: usize) -> Self {
        self.pre_roll = pre_roll;
        self.release_tail = release_tail;
        // No allocation here: the ring grows to `pre_roll` with the samples.
        self.ring = VecDeque::new();
        self
    }

    /// The number of samples in the pre-roll buffer now.
    pub fn pre_roll_len(&self) -> usize {
        self.ring.len()
    }

    /// The configured tail length as a time.
    pub fn release_tail(&self) -> Duration {
        Duration::from_millis((self.release_tail / SAMPLES_PER_MS) as u64)
    }

    /// True while a clip waits for its tail samples.
    pub fn in_tail(&self) -> bool {
        self.recording
            .as_ref()
            .is_some_and(|r| r.tail_left.is_some())
    }

    /// Gives the clip that waits in its tail now, with the tail samples that
    /// arrived. For a stream that stopped sending samples.
    pub fn finish_tail(&mut self) -> Option<AudioClip> {
        if !self.in_tail() {
            return None;
        }
        self.recording.take().map(Recording::into_clip)
    }

    /// Keeps the last `pre_roll` samples.
    fn keep_pre_roll(&mut self, samples: &[f32]) {
        if self.pre_roll == 0 {
            return;
        }
        if samples.len() >= self.pre_roll {
            self.ring.clear();
            self.ring.extend(&samples[samples.len() - self.pre_roll..]);
        } else {
            let over = (self.ring.len() + samples.len()).saturating_sub(self.pre_roll);
            self.ring.drain(..over);
            self.ring.extend(samples);
        }
    }

    /// Push-to-talk: `Pressed` starts a recording. `Released` drops a short
    /// press, gives the clip when there is no tail, or starts the tail.
    pub fn on_talk(&mut self, event: TalkEvent) -> Option<AudioClip> {
        if self.mode != Mode::PushToTalk {
            return None;
        }
        match event {
            TalkEvent::Pressed => {
                // A press during the tail gives that clip now. Two presses are
                // two utterances: the user started the next command, and one
                // clip with two commands matches neither of them. A second
                // `Pressed` without `Released` drops the old recording.
                let finished = match self.recording.take() {
                    Some(r) if r.tail_left.is_some() => Some(r.into_clip()),
                    _ => None,
                };
                // After a recording the ring is empty, so a press during the
                // tail starts with no pre-roll and no sample of that clip.
                self.recording = Some(Recording {
                    samples: self.ring.drain(..).collect(),
                    held: 0,
                    tail_left: None,
                });
                finished
            }
            TalkEvent::Released => {
                let recording = self.recording.as_mut()?;
                if recording.tail_left.is_some() {
                    return None;
                }
                if recording.held < PTT_MIN_SAMPLES {
                    self.recording = None;
                    return None;
                }
                // A full clip cannot take tail samples: no wait for them.
                if self.release_tail == 0 || recording.samples.len() >= PTT_MAX_SAMPLES {
                    return self.recording.take().map(Recording::into_clip);
                }
                recording.tail_left = Some(self.release_tail);
                None
            }
        }
    }

    /// Push-to-talk: keeps the samples while the button is down and during
    /// the tail, up to the maximum, and gives the clip when its tail is
    /// complete. Without a recording, the samples go to the pre-roll.
    /// Always-listening: gives the utterances that ended in these samples.
    pub fn on_samples(&mut self, samples: &[f32]) -> Vec<AudioClip> {
        match self.mode {
            Mode::PushToTalk => {
                let mut clips = Vec::new();
                let mut rest = samples;
                if let Some(recording) = self.recording.as_mut() {
                    match recording.tail_left {
                        None => {
                            recording.held += samples.len();
                            recording.push(samples);
                            rest = &[];
                        }
                        Some(left) => {
                            let n = left.min(samples.len());
                            recording.push(&samples[..n]);
                            rest = &samples[n..];
                            recording.tail_left = Some(left - n);
                        }
                    }
                }
                if self
                    .recording
                    .as_ref()
                    .is_some_and(|r| r.tail_left == Some(0))
                {
                    clips.extend(self.recording.take().map(Recording::into_clip));
                }
                if self.recording.is_none() {
                    self.keep_pre_roll(rest);
                }
                clips
            }
            Mode::AlwaysListening => self
                .segmenter
                .push(samples)
                .into_iter()
                .map(|samples| AudioClip { samples })
                .collect(),
        }
    }
}

/// True when the glitch counter rose between the start and the end of a clip.
fn clip_had_glitch(at_start: u64, at_end: u64) -> bool {
    at_end > at_start
}

/// Messages into the builder thread.
pub(crate) enum Input {
    Samples(Vec<f32>),
    /// A talk button event and the time the button thread saw it.
    Talk(TalkEvent, Instant),
}

/// The utterance builder of the session. The pre-roll and the tail apply only in
/// push-to-talk mode, and only after `validate` checked their range.
fn utterance_builder(config: &Config, mode: Mode) -> UtteranceBuilder {
    let segmenter = Segmenter::new(
        Box::new(EnergyVad::new(config.microphone.vad_threshold_dbfs)),
        SegmenterConfig::default(),
    );
    let builder = UtteranceBuilder::new(mode, segmenter);
    match mode {
        Mode::PushToTalk => {
            let ptt = &config.push_to_talk;
            builder.with_push_to_talk_timing(
                ms_to_samples(ptt.pre_roll_ms),
                ms_to_samples(ptt.release_tail_ms),
            )
        }
        Mode::AlwaysListening => builder,
    }
}

fn start_talk_button(config: &Config, inputs: Sender<Input>) -> anyhow::Result<TalkButtonHandle> {
    let source: Box<dyn ButtonSource> = match talk_source(config) {
        Some(TalkSource::Joystick(j)) => Box::new(JoystickButton::new(&j.device, j.button)?),
        Some(TalkSource::Key(key)) => Box::new(KeyboardKey::parse(key)?),
        None => bail!("push_to_talk needs a key or a joystick"),
    };
    Ok(spawn_talk_button(source, TALK_POLL, move |event| {
        let _ = inputs.send(Input::Talk(event, Instant::now()));
    }))
}

/// Time after the end of the tail before the builder gives a clip whose
/// tail samples did not arrive, for example after the stream stalled.
const TAIL_MARGIN: Duration = Duration::from_secs(1);

/// A clip with the time the builder made it.
struct QueuedClip {
    clip: AudioClip,
    made: Instant,
    /// The audio stream lost samples while the clip was recorded.
    glitch: bool,
}

/// The builder thread: inputs in, clips out. After `stop` is set, it drops
/// the remaining inputs and gives no clip, also not a clip in its tail. When
/// the inputs close, a clip in its tail is dropped with the builder.
///
/// `glitches` is the running count of non-fatal stream errors. The count at
/// the start of a clip is taken at the key press (push-to-talk) or at the
/// end of the clip before (always-listening). A clip whose count rose until
/// it was complete gets the glitch mark.
///
/// Events: `Talk` for each talk button event, and a `Warning` for each clip
/// that the full clip channel drops.
///
/// A clip in its tail has a deadline: release + tail + `TAIL_MARGIN` by the
/// clock `now`. When its tail samples did not arrive until then (a stalled
/// stream), the builder gives the clip with the samples that it has. The
/// `made` stamp of a push-to-talk clip is release + tail at the latest, so
/// that the worker drops a clip that was complete only much later.
fn builder_loop(
    mut builder: UtteranceBuilder,
    inputs: Receiver<Input>,
    clips: SyncSender<QueuedClip>,
    stop: &AtomicBool,
    glitches: &AtomicU64,
    now: impl Fn() -> Instant,
    events: &Events,
) {
    let tail = builder.release_tail();
    let mut at_start = glitches.load(Ordering::Relaxed);
    // The time of the `Released` event of the current recording.
    let mut released: Option<Instant> = None;
    loop {
        // `None`: the deadline of the tail passed.
        let input = match released.filter(|_| builder.in_tail()) {
            Some(at) => {
                let left = (at + tail + TAIL_MARGIN).saturating_duration_since(now());
                if left.is_zero() {
                    None
                } else {
                    match inputs.recv_timeout(left) {
                        Ok(input) => Some(input),
                        // Check the clock again.
                        Err(RecvTimeoutError::Timeout) => continue,
                        Err(RecvTimeoutError::Disconnected) => return,
                    }
                }
            }
            None => match inputs.recv() {
                Ok(input) => Some(input),
                Err(_) => return,
            },
        };
        if stop.load(Ordering::SeqCst) {
            return;
        }
        let pressed = matches!(input, Some(Input::Talk(TalkEvent::Pressed, _)));
        let ready = match input {
            None => builder.finish_tail().into_iter().collect(),
            Some(Input::Samples(samples)) => builder.on_samples(&samples),
            Some(Input::Talk(event, at)) => {
                events.send(RuntimeEvent::Talk(event == TalkEvent::Pressed));
                // A second `Released` during the tail does not move the deadline.
                if event == TalkEvent::Released && !builder.in_tail() {
                    released = Some(at);
                }
                builder.on_talk(event).into_iter().collect()
            }
        };
        // The clip is complete now: the total time starts here, but for a
        // push-to-talk clip not later than release + tail.
        let made = match released {
            Some(at) => now().min(at + tail),
            None => now(),
        };
        // First the mark of the clips that are ready, then the start count
        // of the new recording.
        let glitch = if ready.is_empty() {
            false
        } else {
            let at_end = glitches.load(Ordering::Relaxed);
            let glitch = clip_had_glitch(at_start, at_end);
            at_start = at_end;
            glitch
        };
        if pressed {
            at_start = glitches.load(Ordering::Relaxed);
            released = None;
        }
        for clip in ready {
            if stop.load(Ordering::SeqCst) {
                return;
            }
            match clips.try_send(QueuedClip { clip, made, glitch }) {
                Ok(()) => {}
                Err(TrySendError::Full(queued)) => events.send(RuntimeEvent::Warning(format!(
                    "worker busy, utterance of {} ms dropped",
                    queued.clip.duration().as_millis()
                ))),
                Err(TrySendError::Disconnected(_)) => return,
            }
        }
    }
}

/// The worker thread: transcribes, matches and sends each clip. After `stop`
/// is set, it drops the queued clips. A clip in flight can still finish. A
/// clip that waited longer than `MAX_CLIP_WAIT` by the clock `now` is dropped.
/// The utterance log and the audio capture run here, after the send.
/// Events: `Dropped` for a clip that waited too long, `Utterance` for each
/// other clip, after the log line.
fn worker_loop(
    mut session: Session,
    clips: Receiver<QueuedClip>,
    stop: &AtomicBool,
    now: impl Fn() -> Instant,
    mut diagnostics: Diagnostics,
    events: &Events,
) {
    for QueuedClip { clip, made, glitch } in clips {
        if stop.load(Ordering::SeqCst) {
            return;
        }
        let clip_levels = levels(&clip.samples);
        let waited = now().saturating_duration_since(made);
        if waited > MAX_CLIP_WAIT {
            events.send(RuntimeEvent::Dropped {
                reason: format!("waited {} ms", waited.as_millis()),
            });
            // One time for the clip name and the log line.
            let stamp = SystemTime::now();
            let audio = diagnostics.save_clip(&clip, stamp);
            diagnostics.log(
                stamp,
                None,
                (Duration::ZERO, Duration::ZERO),
                audio.as_deref(),
                clip_levels,
                Marks {
                    glitch,
                    hotwords: false,
                },
            );
            continue;
        }
        // A command of the second pass is sent only while the clip is young enough.
        let outcome = session.on_utterance_until(&clip, Some(made + MAX_CLIP_WAIT), &now);
        // From the moment the clip was complete: the wait in the queue counts.
        let total = Instant::now().saturating_duration_since(made);
        let transcribe = session.last_transcribe_time().unwrap_or_default();
        let marks = Marks {
            glitch,
            hotwords: session.last_used_hotwords(),
        };
        if session.last_halted_by_stop() {
            // The stop came between the match and the send: nothing went out.
            events.send(RuntimeEvent::Dropped {
                reason: "the session stops".to_string(),
            });
            let stamp = SystemTime::now();
            let audio = diagnostics.save_clip(&clip, stamp);
            diagnostics.log(
                stamp,
                None,
                (transcribe, total),
                audio.as_deref(),
                clip_levels,
                marks,
            );
            continue;
        }
        // One time for the clip name and the log line.
        let stamp = SystemTime::now();
        let audio = diagnostics.save_clip(&clip, stamp);
        diagnostics.log(
            stamp,
            Some(&outcome),
            (transcribe, total),
            audio.as_deref(),
            clip_levels,
            marks,
        );
        events.send(RuntimeEvent::Utterance(UtteranceReport {
            outcome,
            transcribe,
            total,
            clip: clip.duration(),
            levels: clip_levels,
            marks,
            audio_file: audio,
        }));
    }
}

/// Watches the vocabulary folder. Gives the watcher and the receiver of
/// its file events. Keep the watcher alive. `None`: the watcher did not
/// start, and a `Warning` went out.
fn watch_vocab(dir: &Path, events: &Events) -> Option<(Box<dyn Any + Send>, Receiver<()>)> {
    let (tx, rx) = mpsc::channel::<()>();
    match watch_dynamic_vocab(dir, move || {
        let _ = tx.send(());
    }) {
        Ok(watcher) => Some((Box::new(watcher), rx)),
        Err(e) => {
            events.send(RuntimeEvent::Warning(format!(
                "warning: cannot watch the vocabulary directory {}, no reload: {e}",
                dir.display()
            )));
            None
        }
    }
}

/// What the vocabulary thread watches and checks.
struct VocabReload {
    /// The vocabulary folder.
    dir: PathBuf,
    /// The slots whose files the reload checks.
    checked_slots: BTreeSet<String>,
    /// The warnings of the vocabulary check that went out. A reload sends
    /// only the new warnings.
    warnings: Vec<String>,
    /// The watch of the profile file. `None`: no watch.
    profile_watch: Option<ProfileWatch>,
}

/// How often the vocabulary thread checks the size and the time of the
/// profile file.
const PROFILE_POLL: Duration = Duration::from_millis(500);

/// Reloads the vocabulary after `VOCAB_DEBOUNCE` of quiet after the file
/// events of `rx`, and checks the profile file each `PROFILE_POLL`. A
/// change of the profile gives one warning: the session keeps its
/// commands. The thread ends when `rx` closes or when `stop` is set.
/// Without `rx` (no vocabulary watcher) only the profile check runs.
fn start_vocab_reload(
    mut reload: VocabReload,
    rx: Option<Receiver<()>>,
    pipeline: Arc<Pipeline>,
    events: Events,
    stop: Arc<AtomicBool>,
) -> JoinHandle<()> {
    thread::spawn(move || {
        // Without a watcher, a channel that never gives a signal keeps the
        // poll interval.
        let (_keep, idle) = mpsc::channel::<()>();
        let rx = rx.unwrap_or(idle);
        let mut debouncer = Debouncer::new(VOCAB_DEBOUNCE);
        let mut next_profile_poll = Instant::now() + PROFILE_POLL;
        loop {
            match rx.recv_timeout(VOCAB_POLL) {
                Ok(()) => debouncer.touch(Instant::now()),
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => return,
            }
            if stop.load(Ordering::SeqCst) {
                return;
            }
            if let Some(watch) = reload.profile_watch.as_mut() {
                if Instant::now() >= next_profile_poll {
                    next_profile_poll = Instant::now() + PROFILE_POLL;
                    if watch.poll() {
                        events.send(RuntimeEvent::Warning(format!("warning: {PROFILE_CHANGED}")));
                    }
                }
            }
            if debouncer.fire(Instant::now()) {
                let (vocab, warnings) = load_vocabulary(&reload.dir, &reload.checked_slots);
                for warning in warnings.iter().filter(|w| !reload.warnings.contains(w)) {
                    events.send(RuntimeEvent::Warning(format!("warning: {warning}")));
                }
                reload.warnings = warnings;
                let files = vocab.len();
                let names: usize = vocab.values().map(Vec::len).sum();
                pipeline.set_dynamic_vocab(vocab);
                events.send(RuntimeEvent::VocabularyReloaded { files, names });
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::SessionOptions;
    use edcopilot_voicebridge_audio::{Vad, FRAME_SAMPLES};
    use edcopilot_voicebridge_stt::Transcript;
    use std::collections::VecDeque;
    use std::sync::atomic::AtomicUsize;

    #[test]
    fn test_debouncer() {
        let t0 = Instant::now();
        let ms = Duration::from_millis;
        let mut d = Debouncer::new(VOCAB_DEBOUNCE);
        assert!(!d.fire(t0));
        assert!(!d.fire(t0 + ms(10_000)));

        d.touch(t0);
        assert!(!d.fire(t0 + ms(499)));
        assert!(d.fire(t0 + ms(500)));
        assert!(!d.fire(t0 + ms(501)));

        d.touch(t0);
        d.touch(t0 + ms(400));
        assert!(!d.fire(t0 + ms(899)));
        assert!(d.fire(t0 + ms(900)));
        assert!(!d.fire(t0 + ms(2_000)));
    }

    fn push_to_talk() -> UtteranceBuilder {
        let segmenter = Segmenter::new(Box::new(EnergyVad::new(-40.0)), SegmenterConfig::default());
        UtteranceBuilder::new(Mode::PushToTalk, segmenter)
    }

    /// Feeds `n` samples of value `v` in blocks of 160 and returns all clips.
    fn feed(b: &mut UtteranceBuilder, n: usize, v: f32) -> Vec<AudioClip> {
        let block = vec![v; 160];
        let mut out = Vec::new();
        let mut left = n;
        while left > 0 {
            let take = left.min(block.len());
            out.extend(b.on_samples(&block[..take]));
            left -= take;
        }
        out
    }

    #[test]
    fn test_push_to_talk_keeps_only_audio_between_press_and_release() {
        let mut b = push_to_talk();
        assert!(feed(&mut b, 4_000, 0.5).is_empty());
        assert_eq!(b.on_talk(TalkEvent::Pressed), None);
        assert!(feed(&mut b, 8_000, 0.25).is_empty());
        let clip = b.on_talk(TalkEvent::Released).expect("a clip");
        assert_eq!(clip.samples.len(), 8_000);
        assert!(clip.samples.iter().all(|&s| s == 0.25));
        // Samples after the release go nowhere.
        assert!(feed(&mut b, 4_000, 0.5).is_empty());
        assert_eq!(b.on_talk(TalkEvent::Released), None);
    }

    #[test]
    fn test_push_to_talk_drops_a_short_press() {
        let mut b = push_to_talk();
        b.on_talk(TalkEvent::Pressed);
        feed(&mut b, 1_600, 0.25);
        assert_eq!(b.on_talk(TalkEvent::Released), None);
        // The next press starts empty.
        b.on_talk(TalkEvent::Pressed);
        feed(&mut b, PTT_MIN_SAMPLES, 0.25);
        let clip = b.on_talk(TalkEvent::Released).expect("a clip");
        assert_eq!(clip.samples.len(), PTT_MIN_SAMPLES);
    }

    #[test]
    fn test_push_to_talk_stops_at_15_seconds() {
        let mut b = push_to_talk();
        b.on_talk(TalkEvent::Pressed);
        assert!(feed(&mut b, 20 * 16_000, 0.25).is_empty());
        let clip = b.on_talk(TalkEvent::Released).expect("a clip");
        assert_eq!(clip.samples.len(), 240_000);
    }

    const PRE: usize = 4_000;
    const TAIL: usize = 4_800;

    fn timed(pre: usize, tail: usize) -> UtteranceBuilder {
        push_to_talk().with_push_to_talk_timing(pre, tail)
    }

    /// Feeds the samples `start..start + n` as values, in blocks of 160.
    fn feed_ramp(b: &mut UtteranceBuilder, start: usize, n: usize) -> Vec<AudioClip> {
        let all: Vec<f32> = (start..start + n).map(|i| i as f32).collect();
        let mut out = Vec::new();
        for block in all.chunks(160) {
            out.extend(b.on_samples(block));
        }
        out
    }

    #[test]
    fn test_pre_roll_holds_the_last_samples_before_the_press() {
        let mut b = timed(PRE, 0);
        // 10_000 samples before the press, values 0..10_000.
        assert!(feed_ramp(&mut b, 0, 10_000).is_empty());
        assert_eq!(b.on_talk(TalkEvent::Pressed), None);
        feed(&mut b, 8_000, -1.0);
        let clip = b.on_talk(TalkEvent::Released).expect("a clip");
        assert_eq!(clip.samples.len(), PRE + 8_000);
        let expected: Vec<f32> = (10_000 - PRE..10_000).map(|i| i as f32).collect();
        assert_eq!(&clip.samples[..PRE], &expected[..]);
        assert!(clip.samples[PRE..].iter().all(|&s| s == -1.0));
    }

    #[test]
    fn test_ring_buffer_never_holds_more_than_the_pre_roll() {
        let mut b = timed(PRE, TAIL);
        for n in [1, 159, 160, 3_999, 4_000, 4_001, 20_000] {
            b.on_samples(&vec![0.1; n]);
            assert!(b.pre_roll_len() <= PRE, "{}", b.pre_roll_len());
        }
        assert_eq!(b.pre_roll_len(), PRE);
        // A large block keeps only its end.
        b.on_samples(&vec![0.2; 50_000]);
        assert_eq!(b.pre_roll_len(), PRE);
    }

    #[test]
    fn test_clip_comes_out_after_the_tail_and_holds_it() {
        let mut b = timed(0, TAIL);
        b.on_talk(TalkEvent::Pressed);
        feed(&mut b, 8_000, 0.25);
        assert_eq!(
            b.on_talk(TalkEvent::Released),
            None,
            "no clip before the tail"
        );
        assert!(feed(&mut b, TAIL - 1, 0.5).is_empty());
        // The last tail sample arrives in a block with more samples.
        let clips = b.on_samples(&[0.5, 0.9, 0.9]);
        assert_eq!(clips.len(), 1);
        let clip = &clips[0];
        assert_eq!(clip.samples.len(), 8_000 + TAIL);
        assert!(clip.samples[..8_000].iter().all(|&s| s == 0.25));
        assert!(clip.samples[8_000..].iter().all(|&s| s == 0.5));
        // Nothing more comes.
        assert!(feed(&mut b, 20_000, 0.5).is_empty());
        assert_eq!(b.on_talk(TalkEvent::Released), None);
    }

    #[test]
    fn test_zero_pre_roll_and_tail_give_the_old_behaviour() {
        let mut old = push_to_talk();
        let mut zero = timed(0, 0);
        for b in [&mut old, &mut zero] {
            assert!(feed_ramp(b, 0, 4_000).is_empty());
            assert_eq!(b.on_talk(TalkEvent::Pressed), None);
            assert!(feed_ramp(b, 4_000, 8_000).is_empty());
        }
        let a = old.on_talk(TalkEvent::Released).expect("a clip");
        let b = zero.on_talk(TalkEvent::Released).expect("a clip");
        assert_eq!(a, b);
        let expected: Vec<f32> = (4_000..12_000).map(|i| i as f32).collect();
        assert_eq!(b.samples, expected);
        assert_eq!(zero.pre_roll_len(), 0);
        assert!(feed(&mut zero, 4_000, 0.5).is_empty());
    }

    #[test]
    fn test_short_press_gives_no_clip_with_pre_roll_and_tail() {
        let mut b = timed(PRE, TAIL);
        feed(&mut b, 20_000, 0.1);
        b.on_talk(TalkEvent::Pressed);
        feed(&mut b, PTT_MIN_SAMPLES - 1, 0.25);
        assert_eq!(b.on_talk(TalkEvent::Released), None);
        assert!(feed(&mut b, 2 * TAIL, 0.1).is_empty());
        // Exactly the minimum is valid.
        b.on_talk(TalkEvent::Pressed);
        feed(&mut b, PTT_MIN_SAMPLES, 0.25);
        assert_eq!(b.on_talk(TalkEvent::Released), None);
        let clips = feed(&mut b, TAIL, 0.1);
        assert_eq!(clips.len(), 1);
        assert_eq!(clips[0].samples.len(), PRE + PTT_MIN_SAMPLES + TAIL);
    }

    #[test]
    fn test_press_during_the_tail_gives_the_clip_at_once() {
        let mut b = timed(PRE, TAIL);
        feed(&mut b, 10_000, 0.1);
        b.on_talk(TalkEvent::Pressed);
        feed(&mut b, 8_000, 0.25);
        assert_eq!(b.on_talk(TalkEvent::Released), None);
        feed(&mut b, 1_000, 0.5);
        let first = b.on_talk(TalkEvent::Pressed).expect("the first clip");
        assert_eq!(first.samples.len(), PRE + 8_000 + 1_000);
        // The new recording has no pre-roll: no sample of the first clip.
        feed(&mut b, 6_000, 0.75);
        assert_eq!(b.on_talk(TalkEvent::Released), None);
        let clips = feed(&mut b, TAIL, 0.9);
        assert_eq!(clips.len(), 1);
        let second = &clips[0];
        assert_eq!(second.samples.len(), 6_000 + TAIL);
        assert!(second.samples[..6_000].iter().all(|&s| s == 0.75));
    }

    #[test]
    fn test_cap_holds_for_the_whole_clip() {
        let mut b = timed(PRE, TAIL);
        feed(&mut b, PRE, 0.1);
        b.on_talk(TalkEvent::Pressed);
        feed(&mut b, PTT_MAX_SAMPLES - PRE - 100, 0.25);
        assert_eq!(b.on_talk(TalkEvent::Released), None);
        // The tail ends after TAIL samples, also when the cap drops some of them.
        let clips = feed(&mut b, TAIL, 0.5);
        assert_eq!(clips.len(), 1);
        assert_eq!(clips[0].samples.len(), PTT_MAX_SAMPLES);
        assert!(clips[0].samples[..PRE].iter().all(|&s| s == 0.1));
    }

    /// Gives the scripted answers one per frame, then `false`.
    struct ScriptVad {
        answers: VecDeque<bool>,
    }

    impl Vad for ScriptVad {
        fn is_speech(&mut self, _frame: &[f32]) -> bool {
            self.answers.pop_front().unwrap_or(false)
        }
    }

    #[test]
    fn test_always_listening_uses_the_segmenter() {
        let parts = [(false, 10), (true, 30), (false, 25)];
        let mut answers = VecDeque::new();
        for (speech, n) in parts {
            answers.extend(std::iter::repeat_n(speech, n));
        }
        let segmenter = Segmenter::new(Box::new(ScriptVad { answers }), SegmenterConfig::default());
        let mut b = UtteranceBuilder::new(Mode::AlwaysListening, segmenter);
        assert_eq!(b.on_talk(TalkEvent::Pressed), None);
        let clips = feed(&mut b, 65 * FRAME_SAMPLES, 0.1);
        assert_eq!(b.on_talk(TalkEvent::Released), None);
        assert_eq!(clips.len(), 1);
        // Pre-roll 10, speech 30, end silence 20 frames.
        assert_eq!(clips[0].samples.len(), 60 * FRAME_SAMPLES);
    }

    /// Hears "help tide log" for each clip and counts the calls.
    struct CountingEngine {
        calls: Arc<AtomicUsize>,
    }

    impl SttEngine for CountingEngine {
        fn name(&self) -> &str {
            "counting"
        }

        fn transcribe(&mut self, _clip: &AudioClip) -> anyhow::Result<Transcript> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            Ok(Transcript {
                text: "help tide log".to_string(),
            })
        }
    }

    /// A session with a `CountingEngine` and its call counter. The writer
    /// does not wait for a consumer.
    fn session_writing_to(request: &Path) -> (Session, Arc<AtomicUsize>) {
        use edcopilot_voicebridge_core::{AliasMap, Matcher, Writer};
        let commands = crate::commands_source::test_support::fixture_loaded_commands();
        let matcher = Matcher::new(commands, Default::default(), AliasMap::new(), 0.8);
        let writer = Writer::new(request.to_path_buf()).confirm_timeout(None);
        let calls = Arc::new(AtomicUsize::new(0));
        let engine = CountingEngine {
            calls: Arc::clone(&calls),
        };
        let options = SessionOptions {
            wake_words: Vec::new(),
            exact_single_word: true,
            dry_run: false,
            hotwords: Vec::new(),
        };
        let session = Session::new(
            Box::new(engine),
            Arc::new(Pipeline::new(matcher, writer)),
            options,
        );
        (session, calls)
    }

    /// A one-second clip that the builder made at `made`.
    fn one_second_clip(made: Instant) -> QueuedClip {
        QueuedClip {
            glitch: false,
            clip: AudioClip {
                samples: vec![0.0; 16_000],
            },
            made,
        }
    }

    #[test]
    fn test_worker_sends_a_queued_clip() {
        let dir = tempfile::tempdir().unwrap();
        let request = dir.path().join("EDCoPilot.request.txt");
        let (tx, rx) = mpsc::sync_channel(CLIP_CHANNEL_CAPACITY);
        tx.send(one_second_clip(Instant::now())).unwrap();
        drop(tx);
        let (session, calls) = session_writing_to(&request);
        worker_loop(
            session,
            rx,
            &AtomicBool::new(false),
            Instant::now,
            Diagnostics::off(),
            &Events::none(),
        );
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert!(request.exists());
    }

    #[test]
    fn test_worker_drops_a_clip_that_waited_too_long() {
        let dir = tempfile::tempdir().unwrap();
        let request = dir.path().join("EDCoPilot.request.txt");
        let made = Instant::now();
        let (tx, rx) = mpsc::sync_channel(CLIP_CHANNEL_CAPACITY);
        tx.send(one_second_clip(made)).unwrap();
        drop(tx);
        let (session, calls) = session_writing_to(&request);
        let late = move || made + MAX_CLIP_WAIT + Duration::from_millis(1);
        worker_loop(
            session,
            rx,
            &AtomicBool::new(false),
            late,
            Diagnostics::off(),
            &Events::none(),
        );
        assert_eq!(calls.load(Ordering::SeqCst), 0, "the engine was called");
        assert!(!request.exists(), "an old clip was sent");
    }

    #[test]
    fn test_worker_processes_a_clip_at_the_wait_limit() {
        let dir = tempfile::tempdir().unwrap();
        let request = dir.path().join("EDCoPilot.request.txt");
        let made = Instant::now();
        let (tx, rx) = mpsc::sync_channel(CLIP_CHANNEL_CAPACITY);
        tx.send(one_second_clip(made)).unwrap();
        drop(tx);
        let (session, calls) = session_writing_to(&request);
        let at_limit = move || made + MAX_CLIP_WAIT;
        worker_loop(
            session,
            rx,
            &AtomicBool::new(false),
            at_limit,
            Diagnostics::off(),
            &Events::none(),
        );
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert!(request.exists());
    }

    #[test]
    fn test_worker_drops_queued_clips_after_stop() {
        let dir = tempfile::tempdir().unwrap();
        let request = dir.path().join("EDCoPilot.request.txt");
        let (tx, rx) = mpsc::sync_channel(CLIP_CHANNEL_CAPACITY);
        for _ in 0..CLIP_CHANNEL_CAPACITY {
            tx.send(one_second_clip(Instant::now())).unwrap();
        }
        drop(tx);
        let stop = AtomicBool::new(true);
        let (session, calls) = session_writing_to(&request);
        worker_loop(
            session,
            rx,
            &stop,
            Instant::now,
            Diagnostics::off(),
            &Events::none(),
        );
        assert_eq!(calls.load(Ordering::SeqCst), 0);
        assert!(!request.exists(), "a token was sent after the stop");
    }

    fn queued_press(inputs: &Sender<Input>) {
        inputs
            .send(Input::Talk(TalkEvent::Pressed, Instant::now()))
            .unwrap();
        inputs.send(Input::Samples(vec![0.25; 8_000])).unwrap();
        inputs
            .send(Input::Talk(TalkEvent::Released, Instant::now()))
            .unwrap();
    }

    #[test]
    fn test_builder_forwards_a_clip() {
        let (input_tx, input_rx) = mpsc::channel();
        let (clip_tx, clip_rx) = mpsc::sync_channel(CLIP_CHANNEL_CAPACITY);
        queued_press(&input_tx);
        drop(input_tx);
        builder_loop(
            push_to_talk(),
            input_rx,
            clip_tx,
            &AtomicBool::new(false),
            &AtomicU64::new(0),
            Instant::now,
            &Events::none(),
        );
        assert_eq!(clip_rx.try_iter().count(), 1);
    }

    #[test]
    fn test_builder_drops_queued_inputs_after_stop() {
        let (input_tx, input_rx) = mpsc::channel();
        let (clip_tx, clip_rx) = mpsc::sync_channel(CLIP_CHANNEL_CAPACITY);
        queued_press(&input_tx);
        drop(input_tx);
        builder_loop(
            push_to_talk(),
            input_rx,
            clip_tx,
            &AtomicBool::new(true),
            &AtomicU64::new(0),
            Instant::now,
            &Events::none(),
        );
        assert_eq!(
            clip_rx.try_iter().count(),
            0,
            "a clip was made after the stop"
        );
    }

    #[test]
    fn test_builder_forwards_a_clip_after_its_tail() {
        let (input_tx, input_rx) = mpsc::channel();
        let (clip_tx, clip_rx) = mpsc::sync_channel(CLIP_CHANNEL_CAPACITY);
        queued_press(&input_tx);
        input_tx.send(Input::Samples(vec![0.5; TAIL])).unwrap();
        drop(input_tx);
        builder_loop(
            timed(PRE, TAIL),
            input_rx,
            clip_tx,
            &AtomicBool::new(false),
            &AtomicU64::new(0),
            Instant::now,
            &Events::none(),
        );
        let clips: Vec<QueuedClip> = clip_rx.try_iter().collect();
        assert_eq!(clips.len(), 1);
        assert_eq!(clips[0].clip.samples.len(), 8_000 + TAIL);
    }

    #[test]
    fn test_builder_drops_a_clip_in_its_tail_when_the_inputs_close() {
        let (input_tx, input_rx) = mpsc::channel();
        let (clip_tx, clip_rx) = mpsc::sync_channel(CLIP_CHANNEL_CAPACITY);
        queued_press(&input_tx);
        input_tx.send(Input::Samples(vec![0.5; TAIL - 1])).unwrap();
        drop(input_tx);
        builder_loop(
            timed(PRE, TAIL),
            input_rx,
            clip_tx,
            &AtomicBool::new(false),
            &AtomicU64::new(0),
            Instant::now,
            &Events::none(),
        );
        assert_eq!(clip_rx.try_iter().count(), 0);
    }

    #[test]
    fn test_builder_gives_no_clip_when_stop_is_set_during_the_tail() {
        // A rendezvous channel: a send returns only when the builder takes
        // the input, so the builder finished the input before it.
        let (input_tx, input_rx) = mpsc::sync_channel(0);
        let (clip_tx, clip_rx) = mpsc::sync_channel(CLIP_CHANNEL_CAPACITY);
        let stop = AtomicBool::new(false);
        thread::scope(|scope| {
            let stop = &stop;
            scope.spawn(move || {
                builder_loop(
                    timed(PRE, TAIL),
                    input_rx,
                    clip_tx,
                    stop,
                    &AtomicU64::new(0),
                    Instant::now,
                    &Events::none(),
                )
            });
            input_tx
                .send(Input::Talk(TalkEvent::Pressed, Instant::now()))
                .unwrap();
            input_tx.send(Input::Samples(vec![0.25; 8_000])).unwrap();
            input_tx
                .send(Input::Talk(TalkEvent::Released, Instant::now()))
                .unwrap();
            input_tx.send(Input::Samples(vec![0.5; 100])).unwrap();
            // The release is done and the clip waits in its tail.
            stop.store(true, Ordering::SeqCst);
            // The builder can end before it takes this input.
            let _ = input_tx.send(Input::Samples(vec![0.5; 2 * TAIL]));
            drop(input_tx);
        });
        assert_eq!(
            clip_rx.try_iter().count(),
            0,
            "a clip was given after the stop"
        );
    }

    /// The tail of `TAIL` samples as a time.
    const TAIL_TIME: Duration = Duration::from_millis(300);

    #[test]
    fn test_builder_gives_a_stalled_tail_at_the_deadline() {
        let t0 = Instant::now();
        let released = t0 + Duration::from_secs(1);
        let deadline = released + TAIL_TIME + TAIL_MARGIN;
        // The test clock: t0 plus this many milliseconds.
        let offset = AtomicU64::new(0);
        let (input_tx, input_rx) = mpsc::sync_channel(0);
        let (clip_tx, clip_rx) = mpsc::sync_channel(CLIP_CHANNEL_CAPACITY);
        let stop = AtomicBool::new(false);
        let glitches = AtomicU64::new(0);
        thread::scope(|scope| {
            let (offset, stop, glitches) = (&offset, &stop, &glitches);
            let clock = move || t0 + Duration::from_millis(offset.load(Ordering::SeqCst));
            scope.spawn(move || {
                builder_loop(
                    timed(0, TAIL),
                    input_rx,
                    clip_tx,
                    stop,
                    glitches,
                    clock,
                    &Events::none(),
                )
            });
            input_tx.send(Input::Talk(TalkEvent::Pressed, t0)).unwrap();
            input_tx.send(Input::Samples(vec![0.25; 8_000])).unwrap();
            input_tx
                .send(Input::Talk(TalkEvent::Released, released))
                .unwrap();
            // 100 tail samples arrive, then the stream stalls.
            input_tx.send(Input::Samples(vec![0.5; 100])).unwrap();
            // Just before the deadline. An empty block wakes the builder.
            let before = (deadline - t0).as_millis() as u64 - 1;
            offset.store(before, Ordering::SeqCst);
            input_tx.send(Input::Samples(Vec::new())).unwrap();
            assert!(clip_rx.try_recv().is_err(), "a clip before the deadline");
            // At the deadline the builder gives the clip with what it has.
            offset.store(before + 1, Ordering::SeqCst);
            let queued = clip_rx
                .recv_timeout(Duration::from_secs(10))
                .expect("the clip at the deadline");
            assert_eq!(queued.clip.samples.len(), 8_100);
            assert_eq!(queued.made, released + TAIL_TIME);
            drop(input_tx);
        });
    }

    #[test]
    fn test_late_clip_is_stamped_at_release_plus_tail_and_dropped() {
        let t0 = Instant::now();
        let released = t0 + Duration::from_secs(1);
        // The builder runs a minute late, for example after a stall.
        let late = released + Duration::from_secs(60);
        let (input_tx, input_rx) = mpsc::channel();
        let (clip_tx, clip_rx) = mpsc::sync_channel(CLIP_CHANNEL_CAPACITY);
        input_tx.send(Input::Talk(TalkEvent::Pressed, t0)).unwrap();
        input_tx.send(Input::Samples(vec![0.25; 8_000])).unwrap();
        input_tx
            .send(Input::Talk(TalkEvent::Released, released))
            .unwrap();
        input_tx.send(Input::Samples(vec![0.5; TAIL])).unwrap();
        drop(input_tx);
        builder_loop(
            timed(0, TAIL),
            input_rx,
            clip_tx,
            &AtomicBool::new(false),
            &AtomicU64::new(0),
            move || late,
            &Events::none(),
        );
        let clips: Vec<QueuedClip> = clip_rx.try_iter().collect();
        assert_eq!(clips.len(), 1);
        assert_eq!(clips[0].made, released + TAIL_TIME);

        let dir = tempfile::tempdir().unwrap();
        let request = dir.path().join("EDCoPilot.request.txt");
        let (tx, rx) = mpsc::sync_channel(CLIP_CHANNEL_CAPACITY);
        for queued in clips {
            tx.send(queued).unwrap();
        }
        drop(tx);
        let (session, calls) = session_writing_to(&request);
        worker_loop(
            session,
            rx,
            &AtomicBool::new(false),
            move || late,
            Diagnostics::off(),
            &Events::none(),
        );
        assert_eq!(calls.load(Ordering::SeqCst), 0, "the engine was called");
        assert!(!request.exists(), "a late clip was sent");
    }

    #[test]
    fn test_tail_in_time_gives_the_whole_clip_stamped_at_its_end() {
        let t0 = Instant::now();
        let released = t0 + Duration::from_secs(1);
        let now = released + TAIL_TIME + Duration::from_millis(50);
        let (input_tx, input_rx) = mpsc::channel();
        let (clip_tx, clip_rx) = mpsc::sync_channel(CLIP_CHANNEL_CAPACITY);
        input_tx.send(Input::Talk(TalkEvent::Pressed, t0)).unwrap();
        input_tx.send(Input::Samples(vec![0.25; 8_000])).unwrap();
        input_tx
            .send(Input::Talk(TalkEvent::Released, released))
            .unwrap();
        input_tx.send(Input::Samples(vec![0.5; TAIL])).unwrap();
        drop(input_tx);
        builder_loop(
            timed(0, TAIL),
            input_rx,
            clip_tx,
            &AtomicBool::new(false),
            &AtomicU64::new(0),
            move || now,
            &Events::none(),
        );
        let clips: Vec<QueuedClip> = clip_rx.try_iter().collect();
        assert_eq!(clips.len(), 1);
        assert_eq!(clips[0].clip.samples.len(), 8_000 + TAIL);
        assert_eq!(clips[0].made, released + TAIL_TIME);
    }

    #[test]
    fn test_glitch_mark_of_a_clip_finished_by_the_next_press() {
        let (input_tx, input_rx) = mpsc::sync_channel(0);
        let (clip_tx, clip_rx) = mpsc::sync_channel(CLIP_CHANNEL_CAPACITY);
        let stop = AtomicBool::new(false);
        let glitches = AtomicU64::new(0);
        thread::scope(|scope| {
            let (stop, glitches) = (&stop, &glitches);
            scope.spawn(move || {
                builder_loop(
                    timed(0, TAIL),
                    input_rx,
                    clip_tx,
                    stop,
                    glitches,
                    Instant::now,
                    &Events::none(),
                )
            });
            let press = |event| Input::Talk(event, Instant::now());
            input_tx.send(press(TalkEvent::Pressed)).unwrap();
            input_tx.send(Input::Samples(vec![0.25; 8_000])).unwrap();
            input_tx.send(press(TalkEvent::Released)).unwrap();
            input_tx.send(Input::Samples(vec![0.5; 100])).unwrap();
            // A glitch during the tail of the first clip.
            glitches.fetch_add(1, Ordering::SeqCst);
            input_tx.send(press(TalkEvent::Pressed)).unwrap();
            input_tx.send(Input::Samples(vec![0.25; 8_000])).unwrap();
            input_tx.send(press(TalkEvent::Released)).unwrap();
            input_tx.send(Input::Samples(vec![0.5; TAIL])).unwrap();
            drop(input_tx);
        });
        let marks: Vec<bool> = clip_rx.try_iter().map(|q| q.glitch).collect();
        assert_eq!(marks, vec![true, false]);
    }

    #[test]
    fn test_full_clip_at_release_is_given_at_once() {
        let mut b = timed(PRE, TAIL);
        b.on_talk(TalkEvent::Pressed);
        feed(&mut b, PTT_MAX_SAMPLES + 10, 0.25);
        let clip = b
            .on_talk(TalkEvent::Released)
            .expect("no wait for the tail");
        assert_eq!(clip.samples.len(), PTT_MAX_SAMPLES);
        assert!(!b.in_tail());
    }

    #[test]
    fn test_timing_applies_only_in_push_to_talk_mode() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("voicebridge.toml");
        std::fs::write(&file, "[edcopilot]\ndir = 'ed'\n").unwrap();
        let mut config = Config::load(&file).unwrap();
        let ptt = utterance_builder(&config, Mode::PushToTalk);
        assert_eq!(ptt.release_tail(), TAIL_TIME);
        config.push_to_talk.pre_roll_ms = u32::MAX;
        config.push_to_talk.release_tail_ms = u32::MAX;
        let listening = utterance_builder(&config, Mode::AlwaysListening);
        assert_eq!(listening.release_tail(), Duration::ZERO);
        assert_eq!(listening.pre_roll, 0);
    }

    #[test]
    fn test_worker_logs_each_utterance_and_saves_the_audio() {
        let dir = tempfile::tempdir().unwrap();
        let request = dir.path().join("EDCoPilot.request.txt");
        let log = dir.path().join("utterances.tsv");
        let clips = dir.path().join("clips");
        let made = Instant::now();
        let (tx, rx) = mpsc::sync_channel(CLIP_CHANNEL_CAPACITY);
        tx.send(one_second_clip(made)).unwrap();
        let late = made.checked_sub(MAX_CLIP_WAIT + Duration::from_secs(1));
        if let Some(late) = late {
            tx.send(one_second_clip(late)).unwrap();
        }
        drop(tx);
        let (session, _calls) = session_writing_to(&request);
        let diagnostics = Diagnostics::new(&log, &clips);
        worker_loop(
            session,
            rx,
            &AtomicBool::new(false),
            Instant::now,
            diagnostics,
            &Events::none(),
        );
        let text = std::fs::read_to_string(&log).unwrap();
        let lines: Vec<Vec<&str>> = text.lines().map(|l| l.split('\t').collect()).collect();
        assert_eq!(lines.len(), 1 + usize::from(late.is_some()), "{text}");
        assert_eq!(lines[0][1], "sent");
        assert_eq!(lines[0][2], "help tide log");
        assert_eq!(lines[0][3], "Guide:Panel:TideLog");
        assert_eq!(lines[0][4], "1.00");
        if late.is_some() {
            assert_eq!(lines[1][1], "dropped");
        }
        for line in &lines {
            assert_eq!(line.len(), 11, "{text}");
            assert!(clips.join(line[7]).is_file(), "{text}");
            // The clip is silence: both levels are the floor.
            assert_eq!(&line[8..], ["-120.0", "-120.0", "very_quiet"], "{text}");
            // One time for the log line and the clip name.
            let stamp: String = line[0].chars().filter(char::is_ascii_digit).collect();
            assert!(
                line[7].starts_with(&format!("utt-{}T{}Z-", &stamp[..8], &stamp[8..])),
                "{text}"
            );
        }
    }

    #[test]
    fn test_glitch_decision() {
        assert!(!clip_had_glitch(3, 3));
        assert!(clip_had_glitch(3, 4));
    }
}

#[cfg(test)]
mod runtime_tests {
    use super::*;
    use std::any::Any;
    use std::path::{Path, PathBuf};
    use std::sync::atomic::AtomicU64;
    use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
    use std::sync::{Arc, Mutex};
    use std::time::{Duration, Instant};

    use edcopilot_voicebridge_audio::TalkEvent;
    use edcopilot_voicebridge_stt::{AudioClip, FakeEngine, SttEngine, Transcript};

    use crate::config::Config;
    use crate::session::Outcome;

    /// The time a test waits for one event.
    const WAIT: Duration = Duration::from_secs(20);

    /// Sends each event into a channel.
    struct Recorder(Mutex<Sender<RuntimeEvent>>);

    impl EventSink for Recorder {
        fn event(&self, event: RuntimeEvent) {
            let _ = self.0.lock().unwrap().send(event);
        }
    }

    fn recorder() -> (Arc<dyn EventSink>, Receiver<RuntimeEvent>) {
        let (tx, rx) = mpsc::channel();
        (Arc::new(Recorder(Mutex::new(tx))), rx)
    }

    /// The next event that is not a warning.
    fn next_event(rx: &Receiver<RuntimeEvent>) -> RuntimeEvent {
        loop {
            match rx.recv_timeout(WAIT).expect("an event") {
                RuntimeEvent::Warning(_) => continue,
                event => return event,
            }
        }
    }

    /// The engine of a fake machine.
    enum EngineChoice {
        Given(Box<dyn SttEngine>),
        /// The engine of the configuration, as on a real machine.
        Real,
    }

    type ErrorCallback = Box<dyn FnMut(String) + Send>;

    /// The inputs and the error callback of the fake microphone.
    #[derive(Clone, Default)]
    struct Wires {
        inputs: Arc<Mutex<Option<Sender<Input>>>>,
        on_error: Arc<Mutex<Option<ErrorCallback>>>,
    }

    impl Wires {
        fn send(&self, input: Input) {
            let inputs = self.inputs.lock().unwrap();
            inputs
                .as_ref()
                .expect("the capture started")
                .send(input)
                .unwrap();
        }

        /// One push-to-talk press of 0.5 s.
        fn press(&self) {
            self.send(Input::Talk(TalkEvent::Pressed, Instant::now()));
            self.send(Input::Samples(vec![0.25; 8_000]));
            self.send(Input::Talk(TalkEvent::Released, Instant::now()));
        }

        fn device_error(&self, text: &str) {
            let mut on_error = self.on_error.lock().unwrap();
            (on_error.as_mut().expect("the capture started"))(text.to_string());
        }
    }

    /// A machine with no microphone and no talk button.
    struct FakeMachine {
        engine: Option<EngineChoice>,
        wires: Wires,
        fault: Option<Fault>,
        lock_dir: PathBuf,
        /// Set during the engine start, as a Ctrl+C during the model load.
        stop_at_engine: Option<Arc<AtomicBool>>,
    }

    /// The lock folder of the tests, which also holds the saved profile
    /// report: one folder for each test (from the name of the test
    /// thread), so that the report of one test never reaches another test.
    /// Two starts of one test share the folder and its locks.
    fn test_lock_dir() -> PathBuf {
        let name: String = std::thread::current()
            .name()
            .unwrap_or("main")
            .chars()
            .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
            .collect();
        let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../target/test-locks")
            .join(name);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// A failure of the fake machine on the control thread.
    #[derive(Clone, Copy, Debug)]
    enum Fault {
        /// The talk button does not start.
        TalkButton,
        /// The microphone does not start.
        Capture,
        /// The start of the microphone panics.
        PanicAtCapture,
        /// The stop of the microphone panics.
        PanicAtStop,
    }

    impl Hardware for FakeMachine {
        fn lock_dir(&self) -> Result<PathBuf, String> {
            Ok(self.lock_dir.clone())
        }

        fn input_device(&self, _config: &Config) -> Result<Option<String>, String> {
            Ok(None)
        }

        fn engine(
            &mut self,
            config: &Config,
            key_lookup: &dyn Fn(&str) -> Option<String>,
        ) -> anyhow::Result<(Box<dyn SttEngine>, String)> {
            if let Some(stop) = &self.stop_at_engine {
                stop.store(true, Ordering::SeqCst);
            }
            match self.engine.take().expect("one engine") {
                EngineChoice::Given(engine) => Ok((engine, "hotwords: off".to_string())),
                EngineChoice::Real => crate::run::make_engine_with_line(config, key_lookup)
                    .map(|(engine, line)| (engine, line.unwrap_or_default())),
            }
        }

        fn talk_button(
            &mut self,
            _config: &Config,
            _inputs: Sender<Input>,
        ) -> anyhow::Result<Box<dyn Any + Send>> {
            if matches!(self.fault, Some(Fault::TalkButton)) {
                anyhow::bail!("joystick backend failed to start: test");
            }
            Ok(Box::new(()))
        }

        fn capture(
            &mut self,
            _device: Option<&str>,
            _glitches: Arc<AtomicU64>,
            inputs: Sender<Input>,
            on_error: Box<dyn FnMut(String) + Send>,
        ) -> anyhow::Result<Box<dyn Any>> {
            match self.fault {
                Some(Fault::Capture) => anyhow::bail!("cannot open the test microphone"),
                Some(Fault::PanicAtCapture) => panic!("the test microphone panics"),
                _ => {}
            }
            *self.wires.inputs.lock().unwrap() = Some(inputs);
            *self.wires.on_error.lock().unwrap() = Some(on_error);
            Ok(Box::new(FakeCapture {
                wires: self.wires.clone(),
                panic_at_stop: matches!(self.fault, Some(Fault::PanicAtStop)),
            }))
        }
    }

    /// As a real capture, the fake drops its input sender when it stops.
    struct FakeCapture {
        wires: Wires,
        panic_at_stop: bool,
    }

    impl Drop for FakeCapture {
        fn drop(&mut self) {
            self.wires.inputs.lock().unwrap().take();
            if self.panic_at_stop {
                panic!("the stop of the test microphone panics");
            }
        }
    }

    fn fake_machine(engine: EngineChoice) -> (FakeMachine, Wires) {
        let wires = Wires::default();
        let machine = FakeMachine {
            engine: Some(engine),
            wires: wires.clone(),
            fault: None,
            lock_dir: test_lock_dir(),
            stop_at_engine: None,
        };
        (machine, wires)
    }

    fn faulty_machine(fault: Fault) -> FakeMachine {
        let (mut machine, _wires) = fake_machine(fake_engine(&[]));
        machine.fault = Some(fault);
        machine
    }

    fn fake_engine(replies: &[&str]) -> EngineChoice {
        let replies = replies.iter().map(|r| Ok(r.to_string())).collect();
        EngineChoice::Given(Box::new(FakeEngine::new(replies)))
    }

    /// A folder with an EDCoPilot folder, a model folder and a configuration
    /// for push-to-talk without pre-roll and tail. `extra` goes at the end.
    fn setup(extra: &str) -> (tempfile::TempDir, Config) {
        setup_ed("", extra)
    }

    /// As `setup`, with `ed_extra` in the `[edcopilot]` table.
    fn setup_ed(ed_extra: &str, extra: &str) -> (tempfile::TempDir, Config) {
        let dir = tempfile::tempdir().unwrap();
        let ed = dir.path().join("ed");
        // The invented EDCoPilot folder of the fixture: the program reads
        // its profile.
        edcopilot_voicebridge_core::profile::fixture::write_fixture_edcopilot(&ed);
        std::fs::create_dir_all(dir.path().join("model")).unwrap();
        let core = Path::new(env!("CARGO_MANIFEST_DIR")).join("../core");
        let text = format!(
            "[edcopilot]\ndir = '{}'\n{ed_extra}\
             [data]\naliases_dir = '{}'\n\
             sound_alikes = '{}'\nsynonyms = '{}'\nfillers = '{}'\n\
             [push_to_talk]\npre_roll_ms = 0\nrelease_tail_ms = 0\n\
             {extra}",
            ed.display(),
            core.join("aliases").display(),
            core.join("data/sound_alikes.toml").display(),
            core.join("data/synonyms.toml").display(),
            core.join("data/fillers.toml").display(),
        );
        let text = if text.contains("[stt") {
            text
        } else {
            format!("{text}[stt.parakeet]\nmodel_dir = 'model'\nhotwords_file = ''\n")
        };
        let file = dir.path().join("voicebridge.toml");
        std::fs::write(&file, text).unwrap();
        let config = Config::load(&file).unwrap();
        (dir, config)
    }

    fn no_key() -> KeyLookup {
        Arc::new(|_: &str| None)
    }

    fn dry_run() -> RuntimeOptions {
        RuntimeOptions { dry_run: true }
    }

    fn start(
        config: &Config,
        machine: FakeMachine,
        key_lookup: KeyLookup,
        sink: Arc<dyn EventSink>,
    ) -> Result<Runtime, StartError> {
        let stop = Arc::new(AtomicBool::new(false));
        Runtime::start_on(config, dry_run(), key_lookup, sink, stop, Box::new(machine))
    }

    fn request_file(dir: &Path) -> PathBuf {
        dir.join("ed").join("EDCoPilot.request.txt")
    }

    #[test]
    fn test_start_with_a_configuration_problem_gives_all_problems_and_no_thread() {
        let (_dir, config) = setup(
            "[matching]\nthreshold = 7.0\n\
             [stt.parakeet]\nmodel_dir = 'absent'\nhotwords_file = ''\n",
        );
        let (machine, wires) = fake_machine(fake_engine(&["help tide log"]));
        let (sink, events) = recorder();
        let error = start(&config, machine, no_key(), sink)
            .err()
            .expect("a start with problems is an error");
        assert!(error.problems.len() >= 2, "{:?}", error.problems);
        assert!(error.problems.iter().any(|p| p.contains("threshold")));
        assert!(error.problems.iter().any(|p| p.contains("model_dir")));
        // The start gave back the sink: no thread holds it.
        let all: Vec<RuntimeEvent> = events.try_iter().collect();
        assert!(
            !all.iter().any(|e| matches!(e, RuntimeEvent::Started(_))),
            "{all:?}"
        );
        assert!(matches!(
            events.recv_timeout(Duration::ZERO),
            Err(RecvTimeoutError::Disconnected)
        ));
        assert!(
            wires.on_error.lock().unwrap().is_none(),
            "the capture started"
        );
    }

    #[test]
    fn test_events_arrive_in_order() {
        let (dir, config) = setup("");
        let (machine, wires) = fake_machine(fake_engine(&["help tide log"]));
        let (sink, events) = recorder();
        let mut runtime = start(&config, machine, no_key(), sink).unwrap();
        assert!(runtime.is_running());
        match next_event(&events) {
            RuntimeEvent::Started(info) => {
                assert!(info.dry_run);
                assert_eq!(info.engine, "fake");
                assert_eq!(info.mode, Mode::PushToTalk);
                // The records of the fixture without the number commands
                // that the check of the runtime grammar takes out.
                assert_eq!(
                    info.report.commands,
                    edcopilot_voicebridge_core::profile::fixture::fixture_counts().loaded
                );
            }
            other => panic!("{other:?}"),
        }
        wires.press();
        assert!(matches!(next_event(&events), RuntimeEvent::Talk(true)));
        assert!(matches!(next_event(&events), RuntimeEvent::Talk(false)));
        match next_event(&events) {
            RuntimeEvent::Utterance(report) => {
                assert!(
                    matches!(&report.outcome, Outcome::DryRun { token, .. } if token == "Guide:Panel:TideLog"),
                    "{:?}",
                    report.outcome
                );
                assert_eq!(report.clip, Duration::from_millis(500));
            }
            other => panic!("{other:?}"),
        }
        runtime.stop();
        assert!(!runtime.is_running());
        assert!(matches!(
            next_event(&events),
            RuntimeEvent::Stopped {
                reason: StopReason::Requested,
                glitches: 0
            }
        ));
        // Nothing after `Stopped`: the sink is gone.
        assert!(events.recv_timeout(Duration::from_millis(200)).is_err());
        assert!(!request_file(dir.path()).exists(), "the dry run sent");
    }

    #[test]
    fn test_stop_twice_is_safe() {
        let (_dir, config) = setup("");
        let (machine, _wires) = fake_machine(fake_engine(&[]));
        let (sink, events) = recorder();
        let mut runtime = start(&config, machine, no_key(), sink).unwrap();
        runtime.stop();
        runtime.stop();
        assert!(!runtime.is_running());
        drop(runtime);
        let stopped = events
            .try_iter()
            .filter(|e| matches!(e, RuntimeEvent::Stopped { .. }))
            .count();
        assert_eq!(stopped, 1);
    }

    /// An engine that fails hard: its thread panics.
    struct BrokenEngine;

    impl SttEngine for BrokenEngine {
        fn name(&self) -> &str {
            "broken"
        }

        fn transcribe(&mut self, _clip: &AudioClip) -> anyhow::Result<Transcript> {
            panic!("the model is gone");
        }
    }

    #[test]
    fn test_engine_failure_gives_stopped_with_the_reason_and_start_works_again() {
        let (_dir, config) = setup("");
        let (machine, wires) = fake_machine(EngineChoice::Given(Box::new(BrokenEngine)));
        let (sink, events) = recorder();
        let mut runtime = start(&config, machine, no_key(), sink).unwrap();
        assert!(matches!(next_event(&events), RuntimeEvent::Started(_)));
        wires.press();
        let reason = loop {
            match next_event(&events) {
                RuntimeEvent::Stopped { reason, .. } => break reason,
                RuntimeEvent::Talk(_) => continue,
                other => panic!("{other:?}"),
            }
        };
        match reason {
            StopReason::EngineError(text) => assert!(!text.is_empty()),
            other => panic!("{other:?}"),
        }
        assert!(!runtime.is_running());
        // A stop after the error stop is safe.
        runtime.stop();

        let (machine, wires) = fake_machine(fake_engine(&["help tide log"]));
        let (sink, events) = recorder();
        let mut again = start(&config, machine, no_key(), sink).unwrap();
        assert!(matches!(next_event(&events), RuntimeEvent::Started(_)));
        wires.press();
        loop {
            match next_event(&events) {
                RuntimeEvent::Utterance(_) => break,
                RuntimeEvent::Talk(_) => continue,
                other => panic!("{other:?}"),
            }
        }
        again.stop();
    }

    #[test]
    fn test_device_error_gives_stopped_with_the_reason() {
        let (_dir, config) = setup("");
        let (machine, wires) = fake_machine(fake_engine(&[]));
        let (sink, events) = recorder();
        let mut runtime = start(&config, machine, no_key(), sink).unwrap();
        assert!(matches!(next_event(&events), RuntimeEvent::Started(_)));
        wires.device_error("Microphone: the device is gone");
        match next_event(&events) {
            RuntimeEvent::Stopped {
                reason: StopReason::DeviceError(text),
                ..
            } => assert_eq!(text, "Microphone: the device is gone"),
            other => panic!("{other:?}"),
        }
        assert!(!runtime.is_running());
        runtime.stop();
    }

    #[test]
    fn test_second_runtime_is_refused_while_the_first_runs() {
        let (_dir, config) = setup("");
        let (machine_a, _wires) = fake_machine(fake_engine(&[]));
        let (sink, _events) = recorder();
        let mut first = start(&config, machine_a, no_key(), sink).unwrap();

        let (machine_b, wires_b) = fake_machine(fake_engine(&[]));
        let (sink, events) = recorder();
        let error = start(&config, machine_b, no_key(), sink)
            .err()
            .expect("a second session is refused");
        assert!(
            error.problems.iter().any(|p| p.contains("already running")),
            "{:?}",
            error.problems
        );
        assert!(!events
            .try_iter()
            .any(|e| matches!(e, RuntimeEvent::Started(_))));
        assert!(wires_b.on_error.lock().unwrap().is_none());
        assert!(first.is_running());

        first.stop();
        let (machine_c, _wires) = fake_machine(fake_engine(&[]));
        let (sink, _events) = recorder();
        let mut third = start(&config, machine_c, no_key(), sink).unwrap();
        third.stop();
    }

    #[test]
    fn test_no_event_text_holds_the_key() {
        const SECRET: &str = "SECRET";
        let lookup: KeyLookup =
            Arc::new(|name: &str| (name == "VB_TEST_KEY").then(|| "sk-test-SECRET".to_string()));
        // Nothing listens on port 9: each transcription fails.
        let (dir, config) = setup(
            "[stt]\nengine = 'openai_compatible'\n\
             [stt.openai_compatible]\napi_key_env = 'VB_TEST_KEY'\n\
             base_url = 'http://127.0.0.1:9/v1'\ntimeout_seconds = 2\n",
        );
        let (machine, wires) = fake_machine(EngineChoice::Real);
        let (sink, events) = recorder();
        let mut runtime = start(&config, machine, Arc::clone(&lookup), sink).unwrap();
        wires.press();
        let mut all = Vec::new();
        loop {
            let event = events.recv_timeout(WAIT).expect("an event");
            let utterance = matches!(event, RuntimeEvent::Utterance(_));
            all.push(event);
            if utterance {
                break;
            }
        }
        wires.device_error("Microphone: the device is gone");
        runtime.stop();
        all.extend(events.iter());
        assert!(
            all.iter().any(|e| matches!(
                e,
                RuntimeEvent::Utterance(UtteranceReport {
                    outcome: Outcome::EngineError { .. },
                    ..
                })
            )),
            "{all:?}"
        );
        assert!(all
            .iter()
            .any(|e| matches!(e, RuntimeEvent::Stopped { .. })));
        for event in &all {
            let text = format!("{event:?}");
            assert!(!text.contains(SECRET), "{text}");
        }
        drop(dir);

        // An engine that does not start: the problem texts hold no key.
        let (_dir, config) = setup(
            "[stt]\nengine = 'openai_compatible'\n\
             [stt.openai_compatible]\napi_key_env = 'VB_TEST_KEY'\nbase_url = ''\n",
        );
        let (machine, _wires) = fake_machine(EngineChoice::Real);
        let (sink, events) = recorder();
        let error = start(&config, machine, lookup, sink)
            .err()
            .expect("an empty base_url does not start");
        assert!(!error.problems.is_empty());
        for problem in &error.problems {
            assert!(!problem.contains(SECRET), "{problem}");
        }
        for event in events.try_iter() {
            let text = format!("{event:?}");
            assert!(!text.contains(SECRET), "{text}");
        }
    }

    /// Owns the runtime and stops or drops it at `Stopped`, on the event
    /// thread. Then it signals `done`.
    struct OwnerSink {
        runtime: Mutex<Option<Runtime>>,
        drop_it: bool,
        done: Mutex<Sender<()>>,
    }

    impl EventSink for OwnerSink {
        fn event(&self, event: RuntimeEvent) {
            if !matches!(event, RuntimeEvent::Stopped { .. }) {
                return;
            }
            let runtime = self.runtime.lock().unwrap().take();
            if let Some(mut runtime) = runtime {
                if !self.drop_it {
                    runtime.stop();
                }
                drop(runtime);
            }
            let _ = self.done.lock().unwrap().send(());
        }
    }

    /// The sink stops or drops its own runtime. The test fails by time-out
    /// when that hangs.
    fn stop_from_inside_the_sink(drop_it: bool) {
        let (_dir, config) = setup("");
        let (machine, wires) = fake_machine(fake_engine(&[]));
        let (done_tx, done_rx) = mpsc::channel();
        let sink = Arc::new(OwnerSink {
            runtime: Mutex::new(None),
            drop_it,
            done: Mutex::new(done_tx),
        });
        let runtime = start(&config, machine, no_key(), sink.clone()).unwrap();
        *sink.runtime.lock().unwrap() = Some(runtime);
        wires.device_error("Microphone: the device is gone");
        assert!(
            done_rx.recv_timeout(WAIT).is_ok(),
            "the stop inside the sink hangs"
        );
    }

    #[test]
    fn test_stop_from_inside_the_sink_does_not_hang() {
        stop_from_inside_the_sink(false);
    }

    #[test]
    fn test_drop_from_inside_the_sink_does_not_hang() {
        stop_from_inside_the_sink(true);
    }

    /// Panics at `Started`, and records the events.
    struct PanicSink(Mutex<Sender<RuntimeEvent>>);

    impl EventSink for PanicSink {
        fn event(&self, event: RuntimeEvent) {
            let started = matches!(event, RuntimeEvent::Started(_));
            let _ = self.0.lock().unwrap().send(event);
            if started {
                panic!("the sink failed");
            }
        }
    }

    #[test]
    fn test_a_panic_in_the_sink_does_not_stop_the_events() {
        let (_dir, config) = setup("");
        let (machine, wires) = fake_machine(fake_engine(&["help tide log"]));
        let (tx, events) = mpsc::channel();
        let sink = Arc::new(PanicSink(Mutex::new(tx)));
        let mut runtime = start(&config, machine, no_key(), sink).unwrap();
        assert!(matches!(next_event(&events), RuntimeEvent::Started(_)));
        wires.press();
        assert!(matches!(next_event(&events), RuntimeEvent::Talk(true)));
        runtime.stop();
        let rest: Vec<RuntimeEvent> = events.iter().collect();
        let stopped = rest
            .iter()
            .filter(|e| matches!(e, RuntimeEvent::Stopped { .. }))
            .count();
        assert_eq!(stopped, 1, "{rest:?}");
    }

    /// A start that fails on the control thread gives `StartError`. The sink
    /// gets no `Started` and no `Stopped`, and the sink channel closes.
    /// A start after it works.
    fn start_fails_on_the_control_thread(fault: Fault, text: &str) {
        let (_dir, config) = setup("");
        let (sink, events) = recorder();
        let error = start(&config, faulty_machine(fault), no_key(), sink)
            .err()
            .expect("the start fails");
        assert!(error.step_failed, "{error:?}");
        assert!(
            error.problems.iter().any(|p| p.contains(text)),
            "{fault:?}: {:?}",
            error.problems
        );
        let all: Vec<RuntimeEvent> = events.try_iter().collect();
        assert!(
            all.iter().all(|e| matches!(e, RuntimeEvent::Warning(_))),
            "{all:?}"
        );
        assert!(matches!(
            events.recv_timeout(Duration::ZERO),
            Err(RecvTimeoutError::Disconnected)
        ));
        let (machine, _wires) = fake_machine(fake_engine(&[]));
        let (sink, _events) = recorder();
        let mut again = start(&config, machine, no_key(), sink).expect("a start after it");
        again.stop();
    }

    #[test]
    fn test_talk_button_failure_gives_start_error_and_start_works_again() {
        start_fails_on_the_control_thread(Fault::TalkButton, "joystick backend");
    }

    #[test]
    fn test_capture_failure_gives_start_error_and_start_works_again() {
        start_fails_on_the_control_thread(Fault::Capture, "test microphone");
    }

    #[test]
    fn test_control_thread_panic_during_the_start_gives_start_error() {
        start_fails_on_the_control_thread(Fault::PanicAtCapture, "stopped during the start");
    }

    #[test]
    fn test_control_thread_panic_at_the_stop_gives_one_stopped_and_frees_the_lock() {
        let (_dir, config) = setup("");
        let (sink, events) = recorder();
        let mut runtime =
            start(&config, faulty_machine(Fault::PanicAtStop), no_key(), sink).unwrap();
        runtime.stop();
        runtime.stop();
        let all: Vec<RuntimeEvent> = events.iter().collect();
        let stopped: Vec<&RuntimeEvent> = all
            .iter()
            .filter(|e| matches!(e, RuntimeEvent::Stopped { .. }))
            .collect();
        assert_eq!(stopped.len(), 1, "{all:?}");
        assert!(
            matches!(all.last(), Some(RuntimeEvent::Stopped { .. })),
            "{all:?}"
        );
        let (machine, _wires) = fake_machine(fake_engine(&[]));
        let (sink, _events) = recorder();
        let mut again = start(&config, machine, no_key(), sink).expect("the lock is free");
        again.stop();
    }

    /// A configuration in its own folder with this request file.
    fn setup_request(request: &Path) -> (tempfile::TempDir, Config) {
        setup_ed(&format!("request_file = '{}'\n", request.display()), "")
    }

    fn quiet_start(config: &Config) -> Result<Runtime, StartError> {
        let (machine, _wires) = fake_machine(fake_engine(&[]));
        let (sink, _events) = recorder();
        start(config, machine, no_key(), sink)
    }

    #[test]
    fn test_same_request_file_from_two_configuration_folders_is_refused() {
        let ed = tempfile::tempdir().unwrap();
        let request = ed.path().join("EDCoPilot.request.txt");
        let (_a, config_a) = setup_request(&request);
        let (_b, config_b) = setup_request(&request);
        let mut first = quiet_start(&config_a).unwrap();
        let error = quiet_start(&config_b)
            .err()
            .expect("the same request file is refused");
        let expected = format!(
            "a voice session for {} is already running",
            request.display()
        );
        assert_eq!(error.problems, [expected]);
        first.stop();
        quiet_start(&config_b).expect("free after the stop").stop();
        assert!(!request.exists(), "the lock wrote the request file");
        let names: Vec<_> = std::fs::read_dir(ed.path()).unwrap().collect();
        assert!(
            names.is_empty(),
            "a file in the EDCoPilot folder: {names:?}"
        );
    }

    #[test]
    fn test_different_request_files_both_start() {
        let (_a, config_a) = setup("");
        let (_b, config_b) = setup("");
        let mut first = quiet_start(&config_a).unwrap();
        let mut second = quiet_start(&config_b).unwrap();
        first.stop();
        second.stop();
    }

    #[test]
    fn test_other_spelling_of_the_request_file_is_the_same_lock() {
        let ed = tempfile::tempdir().unwrap();
        std::fs::create_dir(ed.path().join("sub")).unwrap();
        let request = ed.path().join("EDCoPilot.request.txt");
        let dotted = ed
            .path()
            .join("sub")
            .join("..")
            .join(".")
            .join("EDCoPilot.request.txt");
        let dir = test_lock_dir();
        assert_eq!(lock_file(&dir, &request), lock_file(&dir, &dotted));
        if cfg!(windows) {
            let upper = PathBuf::from(request.to_string_lossy().to_uppercase());
            assert_eq!(lock_file(&dir, &request), lock_file(&dir, &upper));
        }
        let other = ed.path().join("other.request.txt");
        assert_ne!(lock_file(&dir, &request), lock_file(&dir, &other));

        let (_a, config_a) = setup_request(&request);
        let (_b, config_b) = setup_request(&dotted);
        let mut first = quiet_start(&config_a).unwrap();
        let error = quiet_start(&config_b).err().expect("the same lock");
        assert!(error.problems[0].contains("already running"), "{error:?}");
        first.stop();
    }

    #[test]
    fn test_lock_is_free_after_an_error_stop_and_after_a_failed_start() {
        let ed = tempfile::tempdir().unwrap();
        let request = ed.path().join("EDCoPilot.request.txt");
        let (_a, config) = setup_request(&request);
        // An error stop.
        let (machine, wires) = fake_machine(fake_engine(&[]));
        let (sink, events) = recorder();
        let mut runtime = start(&config, machine, no_key(), sink).unwrap();
        wires.device_error("Microphone: the device is gone");
        while !matches!(next_event(&events), RuntimeEvent::Stopped { .. }) {}
        quiet_start(&config)
            .expect("free after the error stop")
            .stop();
        runtime.stop();
        // A failed start: a configuration problem, then the microphone.
        let (_b, broken) = setup_ed(
            &format!("request_file = '{}'\n", request.display()),
            "[matching]\nthreshold = 7.0\n",
        );
        assert!(quiet_start(&broken).is_err());
        quiet_start(&config)
            .expect("free after a configuration problem")
            .stop();
        let (sink, _events) = recorder();
        assert!(start(&config, faulty_machine(Fault::Capture), no_key(), sink).is_err());
        quiet_start(&config)
            .expect("free after a failed microphone")
            .stop();
    }

    #[test]
    fn test_lock_folder_that_cannot_be_written_refuses_the_start() {
        let (dir, config) = setup("");
        let file = dir.path().join("a-file");
        std::fs::write(&file, "x").unwrap();
        let (mut machine, wires) = fake_machine(fake_engine(&[]));
        // A folder below a file cannot exist.
        machine.lock_dir = file.join("locks");
        let (sink, _events) = recorder();
        let error = start(&config, machine, no_key(), sink)
            .err()
            .expect("no lock, no session");
        assert_eq!(error.problems.len(), 1, "{error:?}");
        assert!(
            error.problems[0].starts_with("cannot open the session lock"),
            "{error:?}"
        );
        assert!(wires.on_error.lock().unwrap().is_none());
    }

    #[test]
    fn test_stop_flag_during_the_start_cancels_the_start() {
        let (_dir, config) = setup("");
        let stop = Arc::new(AtomicBool::new(false));
        let (mut machine, wires) = fake_machine(fake_engine(&[]));
        machine.stop_at_engine = Some(Arc::clone(&stop));
        let (sink, events) = recorder();
        let error = Runtime::start_on(
            &config,
            dry_run(),
            no_key(),
            sink,
            Arc::clone(&stop),
            Box::new(machine),
        )
        .err()
        .expect("a stop during the start cancels it");
        assert!(error.cancelled, "{error:?}");
        assert_eq!(error.problems, ["the start was cancelled"]);
        assert!(
            wires.on_error.lock().unwrap().is_none(),
            "the capture started"
        );
        let all: Vec<RuntimeEvent> = events.try_iter().collect();
        assert!(
            all.iter().all(|e| matches!(e, RuntimeEvent::Warning(_))),
            "{all:?}"
        );
        assert!(matches!(
            events.recv_timeout(Duration::ZERO),
            Err(RecvTimeoutError::Disconnected)
        ));
        quiet_start(&config).expect("the lock is free").stop();
    }

    /// A variable lookup with the given variables only.
    fn vars(pairs: &[(&str, &Path)]) -> impl Fn(&str) -> Option<std::ffi::OsString> {
        let pairs: Vec<(String, PathBuf)> = pairs
            .iter()
            .map(|(name, path)| (name.to_string(), path.to_path_buf()))
            .collect();
        move |name: &str| {
            pairs
                .iter()
                .find(|(n, _)| n == name)
                .map(|(_, p)| p.clone().into_os_string())
        }
    }

    #[cfg(windows)]
    #[test]
    fn test_default_lock_folder_is_under_localappdata_and_is_made() {
        let local = tempfile::tempdir().unwrap();
        let dir = user_lock_dir(&vars(&[("LOCALAPPDATA", local.path())])).unwrap();
        assert_eq!(dir, local.path().join("VoiceBridge"));
        assert!(!dir.exists());
        let ed = tempfile::tempdir().unwrap();
        let request = ed.path().join("EDCoPilot.request.txt");
        let lock = SessionLock::acquire(&dir, &request).unwrap();
        assert!(lock_file(&dir, &request).is_file());
        drop(lock);
        // The lock file holds no data.
        assert_eq!(
            std::fs::metadata(lock_file(&dir, &request)).unwrap().len(),
            0
        );
        // No per-user folder: no shared folder instead.
        let error = user_lock_dir(&vars(&[])).unwrap_err();
        assert!(error.contains("LOCALAPPDATA"), "{error}");
        let error = user_lock_dir(&vars(&[("LOCALAPPDATA", Path::new("relative"))])).unwrap_err();
        assert!(error.contains("LOCALAPPDATA"), "{error}");
    }

    #[cfg(unix)]
    #[test]
    fn test_unix_lock_folder_comes_from_the_variables_in_order() {
        let run = Path::new("/run/user/1000");
        let state = Path::new("/home/u/state");
        let home = Path::new("/home/u");
        let all = vars(&[
            ("XDG_RUNTIME_DIR", run),
            ("XDG_STATE_HOME", state),
            ("HOME", home),
        ]);
        assert_eq!(user_lock_dir(&all).unwrap(), run.join("voicebridge"));
        let two = vars(&[("XDG_STATE_HOME", state), ("HOME", home)]);
        assert_eq!(user_lock_dir(&two).unwrap(), state.join("voicebridge"));
        let one = vars(&[("HOME", home)]);
        assert_eq!(
            user_lock_dir(&one).unwrap(),
            home.join(".local/state/voicebridge")
        );
        // A relative value is not used.
        let relative = vars(&[("XDG_RUNTIME_DIR", Path::new("run")), ("HOME", home)]);
        assert_eq!(
            user_lock_dir(&relative).unwrap(),
            home.join(".local/state/voicebridge")
        );
        assert!(user_lock_dir(&vars(&[])).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn test_unix_new_lock_folder_is_private() {
        use std::os::unix::fs::PermissionsExt;
        let base = tempfile::tempdir().unwrap();
        let dir = base.path().join("state").join("voicebridge");
        let request = base.path().join("EDCoPilot.request.txt");
        drop(SessionLock::acquire(&dir, &request).unwrap());
        let mode = std::fs::metadata(&dir).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o700, "{mode:o}");
    }

    #[cfg(unix)]
    #[test]
    fn test_unix_lock_folder_that_is_a_symbolic_link_is_refused() {
        let base = tempfile::tempdir().unwrap();
        let real = base.path().join("real");
        std::fs::create_dir(&real).unwrap();
        let link = base.path().join("link");
        std::os::unix::fs::symlink(&real, &link).unwrap();
        let request = base.path().join("EDCoPilot.request.txt");
        let error = SessionLock::acquire(&link, &request).err().unwrap();
        assert!(error.contains("symbolic link"), "{error}");
        assert_eq!(std::fs::read_dir(&real).unwrap().count(), 0);
    }

    #[cfg(unix)]
    #[test]
    fn test_unix_lock_folder_that_others_can_write_is_refused() {
        use std::os::unix::fs::PermissionsExt;
        let base = tempfile::tempdir().unwrap();
        let dir = base.path().join("open");
        std::fs::create_dir(&dir).unwrap();
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o777)).unwrap();
        let request = base.path().join("EDCoPilot.request.txt");
        let error = SessionLock::acquire(&dir, &request).err().unwrap();
        assert!(error.contains("other users can write"), "{error}");
    }

    #[cfg(unix)]
    #[test]
    fn test_unix_lock_file_that_is_a_symbolic_link_is_not_followed() {
        use std::os::unix::fs::PermissionsExt;
        let base = tempfile::tempdir().unwrap();
        let dir = base.path().join("locks");
        std::fs::create_dir(&dir).unwrap();
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700)).unwrap();
        let request = base.path().join("EDCoPilot.request.txt");
        let target = base.path().join("target.txt");
        std::fs::write(&target, "keep").unwrap();
        std::os::unix::fs::symlink(&target, lock_file(&dir, &request)).unwrap();
        assert!(SessionLock::acquire(&dir, &request).is_err());
        assert_eq!(std::fs::read_to_string(&target).unwrap(), "keep");
    }

    /// Hears "help tide log" and sets the stop flag, as a Stop button
    /// during the transcription.
    struct StopDuringTranscription(Arc<AtomicBool>);

    impl SttEngine for StopDuringTranscription {
        fn name(&self) -> &str {
            "stop"
        }

        fn transcribe(&mut self, _clip: &AudioClip) -> anyhow::Result<Transcript> {
            self.0.store(true, Ordering::SeqCst);
            Ok(Transcript {
                text: "help tide log".to_string(),
            })
        }
    }

    #[test]
    fn test_worker_drops_a_clip_when_the_stop_comes_before_the_send() {
        use crate::session::SessionOptions;
        use edcopilot_voicebridge_core::{AliasMap, Matcher, Writer};
        let dir = tempfile::tempdir().unwrap();
        let request = dir.path().join("EDCoPilot.request.txt");
        let log = dir.path().join("utterances.tsv");
        let commands = crate::commands_source::test_support::fixture_loaded_commands();
        let matcher = Matcher::new(commands, Default::default(), AliasMap::new(), 0.8);
        let writer = Writer::new(request.clone()).confirm_timeout(None);
        let stop = Arc::new(AtomicBool::new(false));
        let options = SessionOptions {
            wake_words: Vec::new(),
            exact_single_word: true,
            dry_run: false,
            hotwords: Vec::new(),
        };
        let session = Session::new(
            Box::new(StopDuringTranscription(Arc::clone(&stop))),
            Arc::new(Pipeline::new(matcher, writer)),
            options,
        )
        .with_stop(Arc::clone(&stop));
        let (tx, rx) = mpsc::sync_channel(CLIP_CHANNEL_CAPACITY);
        tx.send(QueuedClip {
            clip: AudioClip {
                samples: vec![0.0; 16_000],
            },
            made: Instant::now(),
            glitch: false,
        })
        .unwrap();
        drop(tx);
        let (events_tx, events_rx) = mpsc::channel();
        worker_loop(
            session,
            rx,
            &stop,
            Instant::now,
            Diagnostics::new(&log, Path::new("")),
            &Events(events_tx),
        );
        let all: Vec<RuntimeEvent> = events_rx.try_iter().collect();
        assert!(
            matches!(&all[..], [RuntimeEvent::Dropped { reason }] if reason == "the session stops"),
            "{all:?}"
        );
        assert!(!request.exists(), "a command went out after the stop");
        let text = std::fs::read_to_string(&log).unwrap();
        assert_eq!(text.lines().count(), 1, "{text}");
        assert_eq!(text.split('\t').nth(1), Some("dropped"), "{text}");
    }

    /// A key for the cloud engine of the fixture configuration.
    fn test_key() -> KeyLookup {
        Arc::new(|_: &str| Some("test-key".to_string()))
    }

    /// A fake machine with its own folder for the lock and the report.
    fn machine_in(lock_dir: &Path) -> FakeMachine {
        let (mut machine, _wires) = fake_machine(fake_engine(&[]));
        machine.lock_dir = lock_dir.to_path_buf();
        machine
    }

    #[test]
    fn test_a_file_that_is_not_a_profile_is_a_problem_and_no_session_starts() {
        let dir = tempfile::tempdir().unwrap();
        let config = crate::commands_source::test_support::fixture_config(dir.path(), "");
        let profile = dir.path().join("ed").join("EDCoPilot-Profile.vap");
        std::fs::write(&profile, "this is a text file, not a profile").unwrap();
        let locks = dir.path().join("locks");
        let (sink, events) = recorder();
        let error = start(&config, machine_in(&locks), test_key(), sink)
            .err()
            .expect("a file that is not a profile refuses the start");
        assert_eq!(error.problems.len(), 1, "{:?}", error.problems);
        let prefix = format!(
            "the file {} is not an EDCoPilot profile that the program understands: ",
            profile.display()
        );
        assert!(
            error.problems[0].starts_with(&prefix),
            "{:?}",
            error.problems
        );
        let all: Vec<RuntimeEvent> = events.try_iter().collect();
        assert!(
            !all.iter().any(|e| matches!(e, RuntimeEvent::Started(_))),
            "{all:?}"
        );
        assert!(!locks.join(crate::commands_source::REPORT_FILE).exists());
        // `check` gives the same text as a problem.
        let found = crate::run::check_config_in(&config, |_| Some("k".into()), None);
        assert!(found.report.is_none());
        assert!(
            found.problems.iter().any(|p| p.starts_with(&prefix)),
            "{found:?}"
        );
    }

    #[test]
    fn test_a_session_start_writes_the_profile_report() {
        let dir = tempfile::tempdir().unwrap();
        let config = crate::commands_source::test_support::fixture_config(dir.path(), "");
        let locks = dir.path().join("locks");
        let report = locks.join(crate::commands_source::REPORT_FILE);
        let (sink, events) = recorder();
        let mut runtime = start(&config, machine_in(&locks), test_key(), sink).unwrap();
        match next_event(&events) {
            RuntimeEvent::Started(info) => {
                let profile = info.report.profile;
                assert_eq!(info.report.commands, profile.records);
            }
            other => panic!("{other:?}"),
        }
        runtime.stop();
        let saved = crate::commands_source::read_saved_report(&locks).expect("a saved report");
        assert!(report.is_file());
        assert!(!saved.report.mapped_phrases.is_empty());
    }

    /// A configuration with the keys of an older version starts as one
    /// without them: the session builds from the profile and writes the
    /// report. The start gives the warning of the old keys one time.
    #[test]
    fn test_a_session_with_the_old_map_keys_builds_from_the_profile_and_warns_one_time() {
        use crate::commands_source::test_support::fixture_config_with;
        let dir = tempfile::tempdir().unwrap();
        let keys = "command_map = 'C:\\Old\\resources\\data\\command_map_v3.json'\n\
                    profile_vars = ''\n";
        let config = fixture_config_with(dir.path(), keys, "");
        let locks = dir.path().join("locks");
        let lines = start_lines(&config, &locks);
        let expected = format!("warning: {}", crate::config::OLD_MAP_KEYS);
        assert_eq!(
            lines.iter().filter(|l| **l == expected).count(),
            1,
            "{lines:?}"
        );
        let saved = crate::commands_source::read_saved_report(&locks).expect("a saved report");
        assert!(!saved.report.mapped_phrases.is_empty());
        // Without the keys: no such line.
        let other = tempfile::tempdir().unwrap();
        let config = crate::commands_source::test_support::fixture_config(other.path(), "");
        let lines = start_lines(&config, &other.path().join("locks"));
        assert!(!lines.contains(&expected), "{lines:?}");
    }
    /// The fixture profile with more commands.
    fn profile_with(phrases: &[(&str, &str)]) -> Vec<u8> {
        let mut b = edcopilot_voicebridge_core::profile::fixture::fixture_builder();
        b.category("Logbook");
        for (phrase, token) in phrases {
            b.fixed(phrase, token);
        }
        b.build()
    }

    /// The next warning line, or `None` after `WAIT`.
    fn next_warning(events: &Receiver<RuntimeEvent>) -> Option<String> {
        let deadline = Instant::now() + WAIT;
        while Instant::now() < deadline {
            if let Ok(RuntimeEvent::Warning(line)) = events.recv_timeout(VOCAB_POLL) {
                return Some(line);
            }
        }
        None
    }

    /// No event for four polls of the profile.
    fn assert_quiet(events: &Receiver<RuntimeEvent>) {
        let quiet = Instant::now() + 4 * PROFILE_POLL;
        while Instant::now() < quiet {
            if let Ok(event) = events.recv_timeout(VOCAB_POLL) {
                panic!("an event: {event:?}");
            }
        }
    }

    /// The phrase of the fixture that the session matches, and its token.
    const LAMP_ON: (&str, &str) = ("lamp on", "Lamp:All:On");
    /// A phrase that only the changed profile has.
    const EXTRA: (&str, &str) = ("open the invented extra room", "OpenRoom:Extra");

    #[test]
    fn test_a_change_of_the_profile_while_a_session_runs_gives_one_warning() {
        let dir = tempfile::tempdir().unwrap();
        let config = crate::commands_source::test_support::fixture_config(
            dir.path(),
            "[push_to_talk]\npre_roll_ms = 0\nrelease_tail_ms = 0\n",
        );
        let profile = config.edcopilot.profile.clone();
        let (sink, events) = recorder();
        let locks = dir.path().join("locks");
        let (mut machine, wires) = fake_machine(fake_engine(&[LAMP_ON.0, EXTRA.0]));
        machine.lock_dir = locks;
        let mut runtime = start(&config, machine, test_key(), sink).unwrap();
        let commands = match next_event(&events) {
            RuntimeEvent::Started(info) => info.report.commands,
            other => panic!("{other:?}"),
        };
        assert_quiet(&events);
        let changed = format!("warning: {}", crate::commands_source::PROFILE_CHANGED);
        // The profile changes: one more command.
        std::fs::write(&profile, profile_with(&[EXTRA])).unwrap();
        assert_eq!(next_warning(&events), Some(changed.clone()));
        // No second warning for the same change.
        assert_quiet(&events);
        // The session keeps its commands (decision M7): it matches a phrase
        // of the old profile, and not the phrase of the new profile.
        wires.press();
        assert!(matches!(next_event(&events), RuntimeEvent::Talk(true)));
        assert!(matches!(next_event(&events), RuntimeEvent::Talk(false)));
        match next_event(&events) {
            RuntimeEvent::Utterance(report) => assert!(
                matches!(&report.outcome, Outcome::DryRun { token, .. } if token == LAMP_ON.1),
                "{:?}",
                report.outcome
            ),
            other => panic!("{other:?}"),
        }
        wires.press();
        assert!(matches!(next_event(&events), RuntimeEvent::Talk(true)));
        assert!(matches!(next_event(&events), RuntimeEvent::Talk(false)));
        match next_event(&events) {
            RuntimeEvent::Utterance(report) => assert!(
                !matches!(
                    &report.outcome,
                    Outcome::DryRun { .. } | Outcome::Sent { .. }
                ),
                "the new command matched: {:?}",
                report.outcome
            ),
            other => panic!("{other:?}"),
        }
        // A second change warns again.
        std::fs::write(
            &profile,
            profile_with(&[EXTRA, ("open the invented second room", "OpenRoom:Second")]),
        )
        .unwrap();
        assert_eq!(next_warning(&events), Some(changed));
        assert!(runtime.is_running());
        runtime.stop();
        // The fixture gives more than the two commands of the test.
        assert!(commands > 2, "{commands}");
    }

    /// The start lines of one session: the warnings before `Started`.
    fn start_lines(config: &Config, locks: &Path) -> Vec<String> {
        let (sink, events) = recorder();
        let mut runtime = start(config, machine_in(locks), test_key(), sink).unwrap();
        let mut lines = Vec::new();
        loop {
            match events.recv_timeout(WAIT).expect("an event") {
                RuntimeEvent::Warning(line) => lines.push(line),
                RuntimeEvent::Started(_) => break,
                other => panic!("{other:?}"),
            }
        }
        runtime.stop();
        lines
    }

    /// A change of the profile between two sessions: the next start
    /// compares with the saved report before it saves the new one.
    #[test]
    fn test_a_change_of_the_profile_between_sessions_gives_one_line_at_the_start() {
        let dir = tempfile::tempdir().unwrap();
        let config = crate::commands_source::test_support::fixture_config(dir.path(), "");
        let locks = dir.path().join("locks");
        let about_profile = |lines: Vec<String>| -> Vec<String> {
            lines
                .into_iter()
                .filter(|l| l.contains("EDCoPilot profile"))
                .collect()
        };
        assert_eq!(
            about_profile(start_lines(&config, &locks)),
            Vec::<String>::new()
        );
        std::fs::write(&config.edcopilot.profile, profile_with(&[EXTRA])).unwrap();
        assert_eq!(
            about_profile(start_lines(&config, &locks)),
            [
                "the EDCoPilot profile changed: 1 commands added, 0 removed, 0 now left out, \
              0 now in the map"
            ]
        );
        assert_eq!(
            about_profile(start_lines(&config, &locks)),
            Vec::<String>::new()
        );
    }
    /// Gives the system files 1 to `n` of the fixture a form that the
    /// program does not know.
    fn break_system_files(config: &Config, n: usize) {
        for k in 1..=n {
            let file = config
                .edcopilot
                .dynamic_vars_dir
                .join(format!("EDCoPilotSystem{k}.txt"));
            std::fs::write(file, format!("[Harbour Light {k}]")).unwrap();
        }
    }

    fn form_lines(lines: &[String]) -> Vec<&str> {
        lines
            .iter()
            .map(String::as_str)
            .filter(|l| l.contains("form that the program does not know"))
            .collect()
    }

    #[test]
    fn test_a_vocabulary_file_of_unknown_form_warns_at_the_start_and_not_at_a_reload() {
        let dir = tempfile::tempdir().unwrap();
        let config = crate::commands_source::test_support::fixture_config(dir.path(), "");
        break_system_files(&config, 1);
        let (sink, events) = recorder();
        let locks = dir.path().join("locks");
        let mut runtime = start(&config, machine_in(&locks), test_key(), sink).unwrap();
        let mut lines = Vec::new();
        loop {
            match events.recv_timeout(WAIT).expect("an event") {
                RuntimeEvent::Warning(line) => lines.push(line),
                RuntimeEvent::Started(_) => break,
                other => panic!("{other:?}"),
            }
        }
        assert_eq!(
            form_lines(&lines),
            [
                "warning: the vocabulary file EDCoPilotSystem1.txt has a form that the program \
              does not know, its names are not used"
            ]
        );
        // A reload with the same forms: no second warning.
        let group = config
            .edcopilot
            .dynamic_vars_dir
            .join("EDCoPilotBookmarkGrp1.txt");
        std::fs::write(group, "Marta Quillon").unwrap();
        match events.recv_timeout(WAIT).expect("a reload") {
            RuntimeEvent::Warning(line) => panic!("a warning at the reload: {line}"),
            RuntimeEvent::VocabularyReloaded { .. } => {}
            other => panic!("{other:?}"),
        }
        runtime.stop();
    }

    #[test]
    fn test_more_than_five_files_of_unknown_form_give_one_line_at_the_start() {
        let dir = tempfile::tempdir().unwrap();
        let config = crate::commands_source::test_support::fixture_config(dir.path(), "");
        break_system_files(&config, 6);
        let lines = start_lines(&config, &dir.path().join("locks"));
        assert_eq!(
            form_lines(&lines),
            [
                "warning: 6 vocabulary files have a form that the program does not know, their \
              names are not used: EDCoPilotSystem1.txt, EDCoPilotSystem2.txt, \
              EDCoPilotSystem3.txt, EDCoPilotSystem4.txt, EDCoPilotSystem5.txt, ..."
            ]
        );
    }
}
