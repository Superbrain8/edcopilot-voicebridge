//! The commands of the web part. The functions here take the state and
//! return `Result<T, String>`, so the tests call them without a Tauri
//! program. The module `ipc` has the Tauri commands: one line each.
//!
//! No function here gives an API key back. No error text holds a key.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use edcopilot_voicebridge_audio::PressedInput;
use voicebridge::config::{is_variable_name, url_origin, Config, API_KEY_ENV_PROBLEM};
use voicebridge::config_edit::{self, Value};
use voicebridge::list::composite_lines;
use voicebridge::run::{check_config_in, phrase_test_session, session_options};
use voicebridge::runtime::{KeyLookup, RuntimeOptions};
use voicebridge::session::Outcome;

use crate::capture::capture_press;
use crate::dto::{
    BindingDto, CheckDto, CommandListDto, CommandRowDto, CompositeGroupDto, ConfigDto,
    DiagnosticsPart, EditDto, KeyResultDto, LeftOutListDto, LeftOutRowDto, ModelEventDto,
    ModelInfoDto, OutcomeDto, PhraseTestDto, RowDto, SaveResultDto, StartupNotesDto, StatusDto,
    TableCheckDto, TableDto, TableKindDto, TableSaveDto, UpdateStatusDto, BASE_URL_SECRET,
};
use crate::events::{Emit, ModelEmit, UpdateEmit, WindowSink};
use crate::model_download::{self, DownloadEvent, Fetch, ModelSource};
use crate::secrets::{host_entry, key_lookup, remove_key, save_key, store_key_matches};
use crate::state::{lock, AppState};
use crate::updater::{self, PendingUpdate};
use voicebridge::tables::{self, Row, TableKind};

/// The text of an error with its causes, for example of a configuration
/// file that does not load.
fn load_error(error: impl std::fmt::Display) -> String {
    format!("{error:#}")
}

/// All settings of the configuration file. A file that is absent or that
/// does not load gives the error text. The file is never written here.
pub fn get_config(state: &AppState) -> Result<ConfigDto, String> {
    if let Some(problem) = state.startup_problem() {
        return Err(problem);
    }
    let path = state.config_path();
    let (config, text) = Config::load_as_written_text(path).map_err(load_error)?;
    let version = config_edit::text_version(&text);
    // The profile that the program reads: the path of `Config::load`.
    let profile = Config::parse_file(&text, path)
        .map(|c| c.edcopilot.profile)
        .unwrap_or_default();
    remember_text(state, &version, text);
    let name = &config.stt.openai_compatible.api_key_env;
    let valid = is_variable_name(name);
    let keys = state.keys();
    let (has_key, api_key_host) = {
        let _keys = lock(&state.key_lock);
        // The host is not secret.
        (
            valid && keys.has(name),
            valid.then(|| keys.get(&host_entry(name))).flatten(),
        )
    };
    let api_key_set = has_key || (valid && env_has_value(name));
    let hotwords = tables::default_hotwords_file(path);
    Ok(ConfigDto::new(
        path,
        &config,
        api_key_set,
        api_key_host,
        &hotwords,
        &profile,
        version,
    ))
}

/// The number of texts of the configuration file that the program keeps
/// for the check of a save with an older version.
const KEPT_TEXTS: usize = 32;

/// Keeps the text of the file for its version. See `file_changed_problem`.
fn remember_text(state: &AppState, version: &str, text: String) {
    let mut texts = lock(&state.config_texts);
    if texts.iter().any(|(v, _)| v == version) {
        return;
    }
    if texts.len() >= KEPT_TEXTS {
        texts.pop_front();
    }
    texts.push_back((version.to_string(), text));
}

/// The text of a save after another program changed a key of the save.
pub const CONFIG_CHANGED: &str = "the configuration file changed, read it again: \
     another program changed a setting that this save changes too";

/// The rule of a save with the version `read` of the page, for the text
/// `now` of the file. The save goes through when the file has the version
/// of the read. Else it is refused when the program does not have the text
/// of that version, or when a key of the edits has another value in the
/// file now than in the text of the read. A change of other keys does not
/// stop the save: the edits apply to the file as it is now.
fn file_changed_problem(state: &AppState, read: &str, now: &str, keys: &[&str]) -> Option<String> {
    if config_edit::text_version(now) == read {
        return None;
    }
    let texts = lock(&state.config_texts);
    let then = texts.iter().find(|(v, _)| v == read).map(|(_, t)| t);
    match then.map(|then| config_edit::changed_values(then, now, keys)) {
        Some(Ok(changed)) if changed.is_empty() => None,
        _ => Some(CONFIG_CHANGED.to_string()),
    }
}

/// The texts of the start: a problem that stops the use of a
/// configuration, the error of a failed first start, the notes and the
/// problems of the update step, and the data folder.
pub fn startup_notes(state: &AppState) -> StartupNotesDto {
    let notes = lock(&state.startup).clone();
    StartupNotesDto {
        problem: notes.problem,
        first_start_error: notes.first_start_error,
        notes: notes.notes,
        update_problems: notes.update_problems,
        data_dir: state.data_dir().map(|d| d.display().to_string()),
    }
}

/// Marks "Start..." of the tray menu for the page.
pub fn mark_tray_start(state: &AppState) {
    state.tray_start.store(true, Ordering::SeqCst);
}

/// True one time after "Start..." of the tray menu. The page calls it when
/// it loads and when the event comes, so that no click is lost and no
/// click counts two times.
pub fn take_tray_start(state: &AppState) -> bool {
    state.tray_start.swap(false, Ordering::SeqCst)
}

/// The text of "Open folder" while the utterance log is off.
pub const LOG_OFF: &str = "the utterance log is off";
/// The text of "Open folder" while the audio capture is off.
pub const AUDIO_OFF: &str = "the audio capture is off";
/// The folder of the diagnostics files that the switches write, below the
/// folder of the configuration file.
pub const DIAGNOSTICS_FOLDER: &str = "logs";
/// The file of the utterance log in `DIAGNOSTICS_FOLDER`.
pub const DEFAULT_LOG_FILE: &str = "utterances.tsv";
/// The folder of the audio capture in `DIAGNOSTICS_FOLDER`.
pub const DEFAULT_AUDIO_DIR: &str = "audio";

/// The configuration key of a part of the diagnostics.
fn diagnostics_key(part: DiagnosticsPart) -> Result<&'static str, String> {
    match part {
        DiagnosticsPart::Log => Ok("diagnostics.log_file"),
        DiagnosticsPart::Audio => Ok("diagnostics.save_audio_dir"),
        DiagnosticsPart::Data => Err("the data folder has no switch".to_string()),
    }
}

/// The text of "Open folder" for the data folder when the program has none.
pub const NO_DATA_FOLDER: &str = "the program has no data folder in this start";

/// The folder of the configuration file as an absolute path.
fn config_folder(state: &AppState) -> PathBuf {
    let path = state.config_path();
    let path = std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf());
    path.parent().map(Path::to_path_buf).unwrap_or_default()
}

/// Switches the utterance log or the audio capture on or off with the
/// save rule of `save_config`. "On" keeps a path that the file has. Else
/// it writes the default path below the folder of the configuration file
/// as an absolute path, and makes the folder. "Off" writes the empty text.
pub fn set_diagnostics(
    state: &AppState,
    part: DiagnosticsPart,
    on: bool,
) -> Result<SaveResultDto, String> {
    let key = diagnostics_key(part)?;
    let result = {
        let _file = lock(&state.config_lock);
        let config = Config::load_as_written(state.config_path()).map_err(load_error)?;
        let now = match part {
            DiagnosticsPart::Log => &config.diagnostics.log_file,
            _ => &config.diagnostics.save_audio_dir,
        };
        let value = if !on {
            String::new()
        } else if !now.as_os_str().is_empty() {
            now.display().to_string()
        } else {
            let logs = config_folder(state).join(DIAGNOSTICS_FOLDER);
            let (path, folder) = match part {
                DiagnosticsPart::Log => (logs.join(DEFAULT_LOG_FILE), logs),
                _ => {
                    let dir = logs.join(DEFAULT_AUDIO_DIR);
                    (dir.clone(), dir)
                }
            };
            std::fs::create_dir_all(&folder)
                .map_err(|e| format!("cannot make the folder {}: {e}", folder.display()))?;
            path.display().to_string()
        };
        config_edit::save_edits(
            state.config_path(),
            &[(key.to_string(), Value::Text(value))],
        )
        .map_err(load_error)?
    };
    let restart_needed = result.written && state.session_active();
    Ok(SaveResultDto {
        problems: result.problems,
        written: result.written,
        restart_needed,
    })
}

/// Opens the folder of the utterance log, of the audio capture, or the data
/// folder. The path comes from the saved configuration or from the start,
/// never from the page. `open` gets only the canonical path of a folder
/// that exists. See `checked_folder`.
pub fn open_folder(
    state: &AppState,
    part: DiagnosticsPart,
    open: &dyn Fn(&Path) -> Result<(), String>,
) -> Result<(), String> {
    let folder = match part {
        DiagnosticsPart::Data => state
            .data_dir()
            .map(Path::to_path_buf)
            .ok_or_else(|| NO_DATA_FOLDER.to_string())?,
        DiagnosticsPart::Log => {
            let config = Config::load(state.config_path()).map_err(load_error)?;
            let file = config.diagnostics.log_file;
            if file.as_os_str().is_empty() {
                return Err(LOG_OFF.to_string());
            }
            file.parent().map(Path::to_path_buf).unwrap_or_default()
        }
        DiagnosticsPart::Audio => {
            let config = Config::load(state.config_path()).map_err(load_error)?;
            let dir = config.diagnostics.save_audio_dir;
            if dir.as_os_str().is_empty() {
                return Err(AUDIO_OFF.to_string());
            }
            dir
        }
    };
    open(&checked_folder(&folder)?)
}

/// True when a part of the path has a name that the shell reads as a
/// namespace folder, for example `x.{21EC2020-3AEA-1069-A2DD-08002B30309D}`.
fn is_namespace_name(path: &Path) -> bool {
    path.components().any(|part| {
        let name = part
            .as_os_str()
            .to_string_lossy()
            .trim_end()
            .to_ascii_lowercase();
        name.ends_with('}') && name.contains(".{")
    })
}

/// The canonical path of a folder that "Open folder" may open. It refuses
/// a path that does not exist, a path that is not a folder (also a link to
/// a file), and a folder name that the shell reads as a namespace folder.
fn checked_folder(folder: &Path) -> Result<PathBuf, String> {
    let absolute = std::path::absolute(folder).unwrap_or_else(|_| folder.to_path_buf());
    let canonical = match absolute.canonicalize() {
        Ok(path) => path,
        Err(_) => {
            return Err(format!(
                "the folder does not exist yet: {}",
                absolute.display()
            ))
        }
    };
    // `canonicalize` follows links: this is the type of the target.
    if !canonical.is_dir() {
        return Err(format!("the path is not a folder: {}", absolute.display()));
    }
    if is_namespace_name(&absolute) || is_namespace_name(&canonical) {
        return Err(format!(
            "the folder name looks like a shell folder, the program does not open it: {}",
            absolute.display()
        ));
    }
    Ok(crate::paths::plain_path(&canonical))
}

/// Opens a folder that exists in the file manager of the system. No shell
/// and no command text: `ShellExecuteExW` gets the path with the class
/// "folder", so the call can only open a folder, never run a file.
#[cfg(windows)]
pub fn open_in_file_manager(dir: &Path) -> Result<(), String> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::UI::Shell::{
        ShellExecuteExW, SEE_MASK_CLASSNAME, SEE_MASK_FLAG_NO_UI, SEE_MASK_NOASYNC,
        SHELLEXECUTEINFOW,
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
    let dir = checked_folder(dir)?;
    let wide = |text: &std::ffi::OsStr| -> Vec<u16> { text.encode_wide().chain(Some(0)).collect() };
    let file = wide(dir.as_os_str());
    let verb = wide(std::ffi::OsStr::new("open"));
    let class = wide(std::ffi::OsStr::new("folder"));
    let mut info = SHELLEXECUTEINFOW {
        cbSize: std::mem::size_of::<SHELLEXECUTEINFOW>() as u32,
        fMask: SEE_MASK_CLASSNAME | SEE_MASK_NOASYNC | SEE_MASK_FLAG_NO_UI,
        lpVerb: verb.as_ptr(),
        lpFile: file.as_ptr(),
        lpClass: class.as_ptr(),
        nShow: SW_SHOWNORMAL,
        ..Default::default()
    };
    // SAFETY: `info` has its size, and the three texts end with 0 and live
    // during the call. The other fields are zero, which the function
    // permits.
    let ok = unsafe { ShellExecuteExW(&mut info) };
    if ok != 0 {
        Ok(())
    } else {
        Err(format!("cannot open the folder {}", dir.display()))
    }
}

/// Other systems: the program does not open folders.
#[cfg(not(windows))]
pub fn open_in_file_manager(dir: &Path) -> Result<(), String> {
    Err(format!("open the folder by hand: {}", dir.display()))
}

/// The key lookup of a session: the stored key only for the host of the
/// configured `base_url`, then the environment variable.
pub fn session_key_lookup(state: &AppState, config: &Config) -> KeyLookup {
    key_lookup(
        state.keys(),
        url_origin(&config.stt.openai_compatible.base_url),
        state.key_lock(),
    )
}

/// The name of the cloud engine in `stt.engine`.
const CLOUD_ENGINE: &str = "openai_compatible";

/// The problem of a stored key that was saved for another host, when the
/// environment variable does not give a key either.
fn key_host_problem(state: &AppState, config: &Config) -> Option<String> {
    let cloud = &config.stt.openai_compatible;
    let name = &cloud.api_key_env;
    if config.stt.engine != CLOUD_ENGINE || !is_variable_name(name) {
        return None;
    }
    let keys = state.keys();
    let host = url_origin(&cloud.base_url);
    let saved = {
        let _keys = lock(&state.key_lock);
        if !keys.has(name) || store_key_matches(&*keys, name, host.as_deref()) {
            return None;
        }
        keys.get(&host_entry(name))
    };
    if env_has_value(name) {
        return None;
    }
    let saved = saved.unwrap_or_else(|| "an unknown host".to_string());
    let new = host.unwrap_or_else(|| "this base_url".to_string());
    Some(format!(
        "the saved API key belongs to {saved}, enter the key again for {new}"
    ))
}

/// True when the environment variable has a value that is not empty. The
/// value is only borrowed for the check.
fn env_has_value(name: &str) -> bool {
    std::env::var_os(name).is_some_and(|value| value.to_str().is_some_and(|v| !v.trim().is_empty()))
}

/// The most edits of one save. The page sends one edit for each changed
/// key, and the configuration has fewer keys.
pub const MAX_EDITS: usize = 200;
/// The text of a save with more than `MAX_EDITS` edits.
pub const TOO_MANY_EDITS: &str = "the save has too many changes: use 200 or fewer";

/// Applies the edits and writes the file when the save rule of
/// `config_edit::save_edits` permits it, and when the rule of
/// `file_changed_problem` permits it for `version`, the version of the read
/// of the page.
pub fn save_config(
    state: &AppState,
    edits: Vec<EditDto>,
    version: String,
) -> Result<SaveResultDto, String> {
    if edits.len() > MAX_EDITS {
        return Err(TOO_MANY_EDITS.to_string());
    }
    let edits: Vec<(String, Value)> = edits
        .into_iter()
        .map(|edit| (edit.key, Value::from(edit.value)))
        .collect();
    let keys: Vec<&str> = edits.iter().map(|(key, _)| key.as_str()).collect();
    let result = {
        let _file = lock(&state.config_lock);
        config_edit::save_edits_guarded(
            state.config_path(),
            &edits,
            config_edit::save_problems,
            |now| file_changed_problem(state, &version, now, &keys),
        )
        .map_err(load_error)?
    };
    // The running session keeps the configuration of its start. The
    // session does not use the update switch.
    let restart_needed = result.written
        && state.session_active()
        && keys.iter().any(|key| *key != UPDATES_CHECK_KEY);
    Ok(SaveResultDto {
        problems: result.problems,
        written: result.written,
        restart_needed,
    })
}

/// The content of `voicebridge check`. A file that does not load gives one
/// problem with the same text as `get_config`.
pub fn check(state: &AppState) -> Result<CheckDto, String> {
    if let Some(problem) = state.startup_problem() {
        return Ok(CheckDto::not_loaded(problem));
    }
    let config = match Config::load(state.config_path()) {
        Ok(config) => config,
        Err(error) => return Ok(CheckDto::not_loaded(load_error(error))),
    };
    let lookup = session_key_lookup(state, &config);
    let report_dir = state.report_dir();
    let mut found = check_config_in(&config, &*lookup, report_dir.as_deref());
    if let Some(problem) = key_host_problem(state, &config) {
        // This text replaces the text of the absent environment variable.
        let name = &config.stt.openai_compatible.api_key_env;
        let absent = format!("environment variable {name} is absent");
        found.problems.retain(|p| !p.starts_with(&absent));
        found.problems.push(problem);
    } else if config.stt.engine == CLOUD_ENGINE {
        // The console text stays as it is. Only the window has the field.
        let name = &config.stt.openai_compatible.api_key_env;
        let absent =
            format!("environment variable {name} is absent or empty, it must hold the API key");
        for problem in found.problems.iter_mut().filter(|p| **p == absent) {
            *problem = format!("{absent}, {KEY_FIELD_HINT}");
        }
    }
    Ok(CheckDto::from_report(&found))
}

/// The end of the window text of an absent key: the window has a key
/// field. The label is the heading of the field on the Setup screen.
pub const KEY_FIELD_HINT: &str = "or save a key in the field \"API key\" on the Setup screen";

/// The configuration key of the update switch.
pub const UPDATES_CHECK_KEY: &str = "updates.check";

/// The text of a start while a session runs.
pub const ALREADY_RUNNING: &str = "a session is already running";
/// The text of a start or a capture while the other one runs.
pub const CAPTURE_RUNS: &str = "a talk button capture is in progress, finish it first";
/// The text of a capture while a session runs.
pub const STOP_FIRST: &str =
    "stop the session first: the session reads the talk button, and two readers do not work";

/// Clears the flag of a start when it is dropped, also during a panic.
struct Starting<'a>(&'a AtomicBool);

impl Drop for Starting<'_> {
    fn drop(&mut self) {
        self.0.store(false, Ordering::SeqCst);
    }
}

/// Starts a session with the configuration file. This call blocks while the
/// model loads. The problems come back as the error text, one on each line.
#[cfg(test)]
pub fn start_session(state: &AppState, dry_run: bool, emit: Arc<dyn Emit>) -> Result<(), String> {
    start_session_with(state, dry_run, emit, &|| {})
}

/// `start_session` with a call right after the start is marked, for the
/// tooltip of the tray.
pub fn start_session_with(
    state: &AppState,
    dry_run: bool,
    emit: Arc<dyn Emit>,
    on_starting: &dyn Fn(),
) -> Result<(), String> {
    let mut slot = lock(&state.session);
    if slot.as_ref().is_some_and(|session| session.is_running()) {
        return Err(ALREADY_RUNNING.to_string());
    }
    let stop = Arc::new(AtomicBool::new(false));
    {
        // The capture lock is held from the check until `starting` is set:
        // a capture checks `starting` under the same lock.
        let capture = lock(&state.capture);
        if capture.is_some() {
            return Err(CAPTURE_RUNS.to_string());
        }
        // The install of an update checks `starting` under the same lock.
        if state.updates.blocks_session() {
            return Err(updater::IN_PROGRESS.to_string());
        }
        // The stop flag is in place before `starting` is set. Thus a stop
        // that sees `starting` always finds the flag of this start, and
        // does not wait for the whole start on the session lock.
        *lock(&state.stop_flag) = Some(Arc::clone(&stop));
        // The engine is not known before the load: a key change counts.
        state.session_cloud.store(true, Ordering::SeqCst);
        state.starting.store(true, Ordering::SeqCst);
    }
    let _starting = Starting(&state.starting);
    on_starting();
    #[cfg(test)]
    if let Some(hook) = lock(&state.start_hook).as_ref() {
        hook(state);
    }
    // A session that stopped with an error: its threads end now.
    if let Some(mut old) = slot.take() {
        old.stop();
    }
    if let Some(problem) = state.startup_problem() {
        return Err(problem);
    }
    let config = Config::load(state.config_path()).map_err(load_error)?;
    state
        .session_cloud
        .store(config.stt.engine == CLOUD_ENGINE, Ordering::SeqCst);
    if let Some(problem) = key_host_problem(state, &config) {
        return Err(problem);
    }
    let generation = {
        let mut status = lock(&state.status);
        status.generation += 1;
        status.generation
    };
    let sink = Arc::new(WindowSink::new(emit, Arc::clone(&state.status), generation));
    let session = state
        .starter()
        .start(
            &config,
            RuntimeOptions { dry_run },
            session_key_lookup(state, &config),
            sink,
            stop,
        )
        .map_err(|problems| problems.join("\n"))?;
    {
        // The sink can have the `Started` event already, or an error stop.
        let mut status = lock(&state.status);
        if status.generation == generation && session.is_running() {
            status.running = true;
            status.dry_run = dry_run;
        }
    }
    *slot = Some(session);
    Ok(())
}

/// Stops the session and waits until the utterance in work is complete.
/// A start in work stops at its next step (the model load itself does not
/// stop). Without a session it does nothing.
pub fn stop_session(state: &AppState) -> Result<(), String> {
    // Set the flag first: a start holds the session lock while it loads.
    if let Some(stop) = lock(&state.stop_flag).as_ref() {
        stop.store(true, Ordering::SeqCst);
    }
    let mut slot = lock(&state.session);
    if let Some(mut session) = slot.take() {
        session.stop();
    }
    lock(&state.status).running = false;
    Ok(())
}

/// The end of the program: ends a capture and stops the session. It waits
/// until the utterance in work is complete, so that the runtime withdraws
/// a token that EDCoPilot did not read and writes its last log line. Safe
/// to call more than one time.
pub fn shutdown(state: &AppState) {
    // After the download it is too late: the installer starts.
    let _ = state.updates.cancel();
    let _ = cancel_model_download(state);
    let _ = cancel_capture(state);
    let _ = stop_session(state);
}

pub fn session_status(state: &AppState) -> Result<StatusDto, String> {
    let starting = state.starting.load(Ordering::SeqCst);
    Ok(lock(&state.status).dto(starting))
}

/// The input devices as `voicebridge devices` shows them.
pub fn list_devices() -> Result<Vec<String>, String> {
    Ok(edcopilot_voicebridge_audio::list_input_devices())
}

/// Clears the capture flag when it is dropped, also during a panic.
struct Capturing<'a>(&'a AppState);

impl Drop for Capturing<'_> {
    fn drop(&mut self) {
        *lock(&self.0.capture) = None;
    }
}

/// Waits for one press and its release of a key or a joystick button. It
/// does not write the configuration. `scan` gives the inputs that are down.
/// The tests use this form. The Tauri command uses `capture_binding_with`.
#[cfg(test)]
pub fn capture_binding(
    state: &AppState,
    scan: impl FnMut() -> Vec<PressedInput>,
    poll: Duration,
    timeout: Duration,
) -> Result<BindingDto, String> {
    capture_binding_with(state, None, || scan, poll, timeout)
}

/// `capture_binding` with a token and a function that starts the scanner.
/// `cancel_capture_token` with the same token ends this capture. The
/// function runs after the capture flag is set, so that a cancel while the
/// scanner starts ends the capture. A refused capture does not call it.
pub fn capture_binding_with<S: FnMut() -> Vec<PressedInput>>(
    state: &AppState,
    token: Option<u32>,
    start_scanner: impl FnOnce() -> S,
    poll: Duration,
    timeout: Duration,
) -> Result<BindingDto, String> {
    let cancel = {
        let mut capture = lock(&state.capture);
        if capture.is_some() {
            return Err(CAPTURE_RUNS.to_string());
        }
        if state.session_active() {
            return Err(STOP_FIRST.to_string());
        }
        let cancel = Arc::new(AtomicBool::new(false));
        *capture = Some((token, Arc::clone(&cancel)));
        cancel
    };
    let _capturing = Capturing(state);
    let scan = start_scanner();
    capture_press(scan, &cancel, poll, timeout).map(|binding| BindingDto::from(&binding))
}

/// Ends any capture that runs. Without a capture it does nothing.
pub fn cancel_capture(state: &AppState) -> Result<(), String> {
    cancel_capture_token(state, None)
}

/// Ends the capture with this token. `None` ends any capture. A token of
/// another capture does nothing.
pub fn cancel_capture_token(state: &AppState, token: Option<u32>) -> Result<(), String> {
    if let Some((running, cancel)) = lock(&state.capture).as_ref() {
        if token.is_none() || token == *running {
            cancel.store(true, Ordering::SeqCst);
        }
    }
    Ok(())
}

/// The name of the key entry (`stt.openai_compatible.api_key_env`) and the
/// origin of `base_url`, when the URL can be read.
fn api_key_name(state: &AppState) -> Result<(String, Option<String>), String> {
    let config = Config::load_as_written(state.config_path()).map_err(|error| {
        format!(
            "cannot find the name of the API key variable: {}",
            load_error(error)
        )
    })?;
    let cloud = config.stt.openai_compatible;
    if cloud.api_key_env.is_empty() {
        return Err(
            "stt.openai_compatible.api_key_env is empty, it must name an environment variable"
                .to_string(),
        );
    }
    if !is_variable_name(&cloud.api_key_env) {
        return Err(API_KEY_ENV_PROBLEM.to_string());
    }
    // The window does not show the full address, so no key goes to it.
    if edcopilot_voicebridge_stt::base_url_without_secrets(&cloud.base_url).is_some() {
        return Err(BASE_URL_SECRET.to_string());
    }
    Ok((cloud.api_key_env, url_origin(&cloud.base_url)))
}

/// The text of a key send when the server in the file is not the server
/// that the window showed.
pub const SERVER_CHANGED: &str =
    "the server address in the file changed, check it and enter the key again";
/// The text of a key send when the variable name in the file is not the
/// name that the window showed.
pub const NAME_CHANGED: &str =
    "the variable name in the file changed, check it and enter the key again";

/// The longest API key, in characters, without the spaces at its ends.
pub const MAX_KEY_CHARS: usize = 512;
/// The text of an empty key.
pub const KEY_EMPTY: &str = "the API key is empty";
/// The text of a key that is too long. It never repeats the key.
pub const KEY_TOO_LONG: &str = "the API key is too long: use 512 characters or fewer";
/// The text of a key with a control character. It never repeats the key.
pub const KEY_CONTROL: &str =
    "the API key has a line break or another control character, enter the key again";

/// The problem of a key text, or `None`. The text of the problem is fixed.
fn key_problem(key: &str) -> Option<&'static str> {
    let key = key.trim();
    if key.is_empty() {
        Some(KEY_EMPTY)
    } else if key.chars().count() > MAX_KEY_CHARS {
        Some(KEY_TOO_LONG)
    } else if key.chars().any(char::is_control) {
        Some(KEY_CONTROL)
    } else {
        None
    }
}

/// Keeps the key in the key store, with the host of the current `base_url`.
/// The key never comes back. `expected_origin` and `expected_name` are the
/// server and the variable name that the window showed. When the file now
/// has other values, the key is not saved: the file changed after the window
/// read it.
pub fn set_api_key(
    state: &AppState,
    key: &str,
    expected_origin: &str,
    expected_name: &str,
) -> Result<KeyResultDto, String> {
    if let Some(problem) = key_problem(key) {
        return Err(problem.to_string());
    }
    let _file = lock(&state.config_lock);
    let (name, host) = api_key_name(state)?;
    if name != expected_name {
        return Err(NAME_CHANGED.to_string());
    }
    let host = host.ok_or_else(|| {
        "stt.openai_compatible.base_url is not an http or https URL, \
         the key needs the host of the service"
            .to_string()
    })?;
    if host != expected_origin {
        return Err(SERVER_CHANGED.to_string());
    }
    let keys = state.keys();
    // `config_lock` is held: the order is `config_lock`, then `key_lock`.
    {
        let _keys = lock(&state.key_lock);
        save_key(&*keys, &name, key, &host)?;
    }
    Ok(key_result(state))
}

/// The result of a key change. The locks of the key are free here.
fn key_result(state: &AppState) -> KeyResultDto {
    KeyResultDto {
        restart_needed: state.session_active() && state.session_cloud.load(Ordering::SeqCst),
    }
}

/// Removes the key and its host from the key store.
pub fn clear_api_key(state: &AppState) -> Result<KeyResultDto, String> {
    {
        let _file = lock(&state.config_lock);
        let (name, _) = api_key_name(state)?;
        let keys = state.keys();
        let _keys = lock(&state.key_lock);
        remove_key(&*keys, &name)?;
    }
    Ok(key_result(state))
}

/// The text of a download while a download runs.
pub const DOWNLOAD_RUNS: &str = model_download::ALREADY_RUNNING;
/// The text of a download that stopped with a program error (a panic).
pub const DOWNLOAD_PANIC: &str = "the download stopped with a program error";
/// The text when the program cannot find the per-user data folder.
pub const NO_DATA_DIR: &str = "cannot find the per-user data folder (APPDATA) for the models";

/// The model that the download gets, and the folder for it.
pub fn model_info(state: &AppState, models_dir: Option<PathBuf>) -> Result<ModelInfoDto, String> {
    let source = &model_download::PARAKEET_V3_INT8;
    Ok(ModelInfoDto {
        name: source.name.to_string(),
        archive_bytes: source.archive_bytes,
        unpacked_bytes: source.unpacked_bytes,
        models_dir: models_dir.map(|d| d.display().to_string()),
        running: lock(&state.model_download).is_some(),
        step: lock(&state.model_step).clone(),
    })
}

/// Marks the start of a download and gives its cancel flag. A second
/// download is refused until `run_model_download` ends.
pub fn begin_model_download(state: &AppState) -> Result<Arc<AtomicBool>, String> {
    let mut slot = lock(&state.model_download);
    if slot.is_some() {
        return Err(DOWNLOAD_RUNS.to_string());
    }
    let cancel = Arc::new(AtomicBool::new(false));
    *slot = Some(Arc::clone(&cancel));
    Ok(cancel)
}

/// Clears the download mark and the last step when it is dropped, also
/// during a panic.
struct Downloading<'a>(&'a AppState);

impl Drop for Downloading<'_> {
    fn drop(&mut self) {
        *lock(&self.0.model_step) = None;
        *lock(&self.0.model_download) = None;
    }
}

/// Runs the download that `begin_model_download` marked. After the
/// download, it sets `stt.parakeet.model_dir` in the configuration file,
/// then it sends `done`. The running session keeps its model. A panic in
/// the download gives `failed`, so that the window does not wait for ever.
pub fn run_model_download<'f>(
    state: &AppState,
    source: &ModelSource,
    cancel: &Arc<AtomicBool>,
    models_dir: &Path,
    make_fetch: &dyn Fn() -> Result<Box<dyn Fetch + 'f>, String>,
    free: &dyn Fn(&Path) -> Option<u64>,
    emit: &dyn ModelEmit,
) -> Result<PathBuf, String> {
    let _downloading = Downloading(state);
    let work = std::panic::AssertUnwindSafe(|| {
        download_and_save(state, source, cancel, models_dir, make_fetch, free, emit)
    });
    match std::panic::catch_unwind(work) {
        Ok(result) => result,
        Err(_) => {
            // The text of the panic is not shown: it is not checked.
            emit.emit_model(&ModelEventDto::Failed {
                message: DOWNLOAD_PANIC.to_string(),
            });
            Err(DOWNLOAD_PANIC.to_string())
        }
    }
}

fn download_and_save<'f>(
    state: &AppState,
    source: &ModelSource,
    cancel: &Arc<AtomicBool>,
    models_dir: &Path,
    make_fetch: &dyn Fn() -> Result<Box<dyn Fetch + 'f>, String>,
    free: &dyn Fn(&Path) -> Option<u64>,
    emit: &dyn ModelEmit,
) -> Result<PathBuf, String> {
    let on_event = |event: DownloadEvent| {
        let dto = match event {
            DownloadEvent::Progress { done, total } => ModelEventDto::Progress { done, total },
            DownloadEvent::Verifying => ModelEventDto::Verifying,
            DownloadEvent::Unpacking => ModelEventDto::Unpacking,
            // `done` comes after the save of the configuration.
            DownloadEvent::Done { .. } => return,
            DownloadEvent::Failed(message) => ModelEventDto::Failed { message },
            DownloadEvent::Cancelled => ModelEventDto::Cancelled,
        };
        if matches!(
            dto,
            ModelEventDto::Progress { .. } | ModelEventDto::Verifying | ModelEventDto::Unpacking
        ) {
            // A window that loads again shows this step.
            *lock(&state.model_step) = Some(dto.clone());
        }
        emit.emit_model(&dto);
    };
    let fetch = match make_fetch() {
        Ok(fetch) => fetch,
        Err(message) => {
            on_event(DownloadEvent::Failed(message.clone()));
            return Err(message);
        }
    };
    let model =
        model_download::download_model_with(source, models_dir, &*fetch, cancel, &on_event, free)?;
    let edit = [(
        "stt.parakeet.model_dir".to_string(),
        Value::Text(model.display().to_string()),
    )];
    let saved = {
        let _file = lock(&state.config_lock);
        config_edit::save_edits(state.config_path(), &edit)
    };
    let (config_error, written) = match saved {
        Ok(result) if result.written || result.problems.is_empty() => (None, result.written),
        Ok(result) => (Some(result.problems.join("\n")), false),
        Err(error) => (Some(load_error(error)), false),
    };
    emit.emit_model(&ModelEventDto::Done {
        model_dir: model.display().to_string(),
        config_error,
        // The running session keeps the model folder of its start.
        restart_needed: written && state.session_active(),
    });
    Ok(model)
}

/// Stops the download at its next block. Without a download it does
/// nothing. The `.part` file stays for the next try.
pub fn cancel_model_download(state: &AppState) -> Result<(), String> {
    if let Some(cancel) = lock(&state.model_download).as_ref() {
        cancel.store(true, Ordering::SeqCst);
    }
    Ok(())
}

/// Marks the start of the install of the offered update. It is refused
/// while a session runs, starts, or stops. The capture lock is the gate: a
/// session start sets `starting` under it and checks the update under it.
/// Thus a start and an install never both go through.
pub fn begin_update_install(state: &AppState) -> Result<(u64, Arc<dyn PendingUpdate>), String> {
    let _gate = lock(&state.capture);
    if state.session_active() {
        return Err(updater::STOP_FIRST.to_string());
    }
    state.updates.begin_download()
}

/// `updates.check` of the configuration in use. The error text when the
/// file does not load: then no automatic check runs.
pub fn updates_check_enabled(state: &AppState) -> Result<bool, String> {
    if let Some(problem) = state.startup_problem() {
        return Err(problem);
    }
    Config::load_as_written(state.config_path())
        .map(|config| config.updates.check)
        .map_err(load_error)
}

/// The longest filter, in characters.
pub const MAX_FILTER_CHARS: usize = 200;
/// The longest phrase of the phrase test, in characters.
pub const MAX_PHRASE_CHARS: usize = 500;
/// The most rows of one page of the command list.
pub const MAX_PAGE_ROWS: usize = 1000;
/// The text of a filter that is too long.
pub const FILTER_TOO_LONG: &str = "the filter is too long: use 200 characters or fewer";
/// The text of a phrase that is too long.
pub const PHRASE_TOO_LONG: &str = "the phrase is too long: use 500 characters or fewer";

/// The commands of `voicebridge commands <filter>`, with `all` as the switch
/// `--all`. The rows and the reasons are the lines of the console. The
/// result has the rows from `offset`, at most `limit` (and at most
/// `MAX_PAGE_ROWS`), and the number of all rows of the filter. A file that
/// does not load gives the error text.
pub fn list_commands(
    state: &AppState,
    filter: &str,
    all: bool,
    offset: usize,
    limit: usize,
) -> Result<CommandListDto, String> {
    if filter.chars().count() > MAX_FILTER_CHARS {
        return Err(FILTER_TOO_LONG.to_string());
    }
    let catalog = crate::catalog::catalog(state)?;
    let list = &catalog.list;
    let shown = list.shown(filter, all);
    let total = shown.len();
    let rows: Vec<CommandRowDto> = shown
        .into_iter()
        .skip(offset)
        .take(limit.min(MAX_PAGE_ROWS))
        .map(CommandRowDto::from)
        .collect();
    let composite = list
        .views
        .iter()
        .filter_map(|view| CompositeGroupDto::from_lines(composite_lines(view, filter, all)))
        .collect();
    // All problems of the load concern the command data. The matching
    // setting does too. The model, the key and the microphone do not.
    let problems = catalog
        .config
        .matching_problem()
        .into_iter()
        .chain(catalog.problems.iter().cloned())
        .collect();
    Ok(CommandListDto {
        summary: list.summary(total),
        rows,
        offset,
        total,
        catalog_id: catalog.ticket,
        composite,
        problems,
        warnings: catalog.warnings.clone(),
    })
}

/// The commands that the build from the EDCoPilot profile left out, with
/// the reason of each, as `voicebridge check --left-out` prints them. The
/// result has the rows from `offset`, at most `limit` (and at most
/// `MAX_PAGE_ROWS`), and the number of all rows. A row shows when each
/// word of the filter is in its line. A file that does not load gives the
/// error text, as `list_commands`.
pub fn list_left_out(
    state: &AppState,
    filter: &str,
    offset: usize,
    limit: usize,
) -> Result<LeftOutListDto, String> {
    if filter.chars().count() > MAX_FILTER_CHARS {
        return Err(FILTER_TOO_LONG.to_string());
    }
    let catalog = crate::catalog::catalog(state)?;
    let report = &catalog.profile;
    let words: Vec<String> = filter.split_whitespace().map(str::to_lowercase).collect();
    let shown: Vec<LeftOutRowDto> = report
        .left_out
        .iter()
        .map(LeftOutRowDto::from)
        .filter(|row| {
            let line = row.line.to_lowercase();
            words.iter().all(|w| line.contains(w.as_str()))
        })
        .collect();
    let total = shown.len();
    Ok(LeftOutListDto {
        rows: shown
            .into_iter()
            .skip(offset)
            .take(limit.min(MAX_PAGE_ROWS))
            .collect(),
        offset,
        total,
        catalog_id: catalog.ticket,
    })
}

/// What the live session does with the phrase when the engine hears it,
/// always as a dry run. The options are the options of the live session
/// for the saved configuration. In always_listening mode the phrase needs a
/// wake word, as live. Nothing is sent and the request file is not written,
/// also while a session runs. The phrase test takes no lock of the session.
pub fn test_phrase(state: &AppState, phrase: &str) -> Result<PhraseTestDto, String> {
    if phrase.chars().count() > MAX_PHRASE_CHARS {
        return Err(PHRASE_TOO_LONG.to_string());
    }
    let catalog = crate::catalog::catalog(state)?;
    // Only the matching settings, as `voicebridge text`.
    if let Some(problem) = catalog.config.matching_problem() {
        return Err(problem);
    }
    // The pipeline is shared and read only. The session is new for each call.
    let mut session = phrase_test_session(&catalog.config, Arc::clone(&catalog.pipeline));
    let outcome = session.on_heard_text(phrase);
    debug_assert!(
        !matches!(
            outcome,
            Outcome::Sent { .. } | Outcome::SendError { .. } | Outcome::NotConsumed { .. }
        ),
        "the phrase test gave the outcome of a send"
    );
    Ok(PhraseTestDto {
        outcome: OutcomeDto::from_outcome(&outcome),
        line: outcome.log_line(),
        warnings: catalog.problems.clone(),
        wake_words: session_options(&catalog.config, true).wake_words,
    })
}

/// The table of a kind and a language from the page. Only an alias file
/// has a language. The language is checked as a file name.
pub fn table_kind(kind: TableKindDto, language: Option<String>) -> Result<TableKind, String> {
    let kind = match (kind, language) {
        (TableKindDto::Aliases, Some(language)) => {
            tables::check_language(&language).map_err(load_error)?;
            TableKind::Aliases { language }
        }
        (TableKindDto::Aliases, None) => return Err("an alias table needs a language".to_string()),
        (_, Some(_)) => return Err("only an alias table has a language".to_string()),
        (TableKindDto::Synonyms, None) => TableKind::Synonyms,
        (TableKindDto::SoundAlikes, None) => TableKind::SoundAlikes,
        (TableKindDto::Fillers, None) => TableKind::Fillers,
        (TableKindDto::Hotwords, None) => TableKind::Hotwords,
    };
    Ok(kind)
}

/// The rows of a word table. The path comes from the saved configuration.
pub fn read_table(
    state: &AppState,
    kind: TableKindDto,
    language: Option<String>,
) -> Result<TableDto, String> {
    let kind = table_kind(kind, language)?;
    let file = tables::read_table(state.config_path(), &kind).map_err(load_error)?;
    Ok(TableDto::new(&kind, &file))
}

/// The lines of `voicebridge check` for the rows, before a save.
pub fn check_table(
    state: &AppState,
    kind: TableKindDto,
    language: Option<String>,
    rows: Vec<RowDto>,
) -> Result<TableCheckDto, String> {
    let kind = table_kind(kind, language)?;
    let rows: Vec<Row> = rows.into_iter().map(Row::from).collect();
    let check = tables::check_table(state.config_path(), &kind, &rows);
    Ok(TableCheckDto::from(&check))
}

/// Writes the rows when the check of the rows has no problem, when the page
/// confirmed each warning, and when the file still has the stamp `version`
/// of the read. The save holds the lock of the configuration file, so two
/// saves do not mix. After a write, the phrase test builds its pipeline
/// again. A running session keeps its old tables.
pub fn save_table(
    state: &AppState,
    kind: TableKindDto,
    language: Option<String>,
    rows: Vec<RowDto>,
    confirmed: Vec<String>,
    version: String,
) -> Result<TableSaveDto, String> {
    let kind = table_kind(kind, language)?;
    let rows: Vec<Row> = rows.into_iter().map(Row::from).collect();
    let saved = {
        let _file = lock(&state.config_lock);
        tables::save_table(state.config_path(), &kind, &rows, &confirmed, &version)
            .map_err(load_error)?
    };
    if saved.written {
        // The stamp of the data files also sees the change. This makes it
        // certain.
        *lock(&state.catalog) = None;
    }
    let restart_needed = saved.written && state.session_active();
    Ok(TableSaveDto::new(&saved, restart_needed))
}

/// The Tauri commands. Each command that can block runs on a blocking
/// thread, not on the main thread.
pub mod ipc {
    use tauri::{AppHandle, Manager};

    use super::*;
    use crate::capture::{CAPTURE_POLL, CAPTURE_TIMEOUT};
    use crate::events::TauriEmit;

    /// The folder for the models in the data folder of the start. Also
    /// with `--config`, the models go to the data folder.
    fn models_dir(state: &AppState) -> Option<std::path::PathBuf> {
        state.data_dir().map(crate::paths::models_dir)
    }

    /// Runs `f` with the state on a blocking thread.
    async fn blocking<T: Send + 'static>(
        app: AppHandle,
        f: impl FnOnce(&AppHandle, &AppState) -> Result<T, String> + Send + 'static,
    ) -> Result<T, String> {
        tauri::async_runtime::spawn_blocking(move || f(&app, &app.state::<AppState>()))
            .await
            // The text of a panic is not shown: it is not checked for a key.
            .map_err(|_| "the command failed".to_string())?
    }

    #[tauri::command]
    pub async fn get_config(app: AppHandle) -> Result<ConfigDto, String> {
        blocking(app, |_, state| super::get_config(state)).await
    }

    #[tauri::command]
    pub async fn save_config(
        app: AppHandle,
        edits: Vec<EditDto>,
        version: String,
    ) -> Result<SaveResultDto, String> {
        blocking(app, |_, state| super::save_config(state, edits, version)).await
    }

    #[tauri::command]
    pub async fn check(app: AppHandle) -> Result<CheckDto, String> {
        blocking(app, |_, state| super::check(state)).await
    }

    /// The tray shows "session starting" during the model load, and the
    /// state after a start that failed.
    #[tauri::command]
    pub async fn start_session(app: AppHandle, dry_run: bool) -> Result<(), String> {
        let handle = app.clone();
        let result = blocking(app, move |app, state| {
            let tray = app.clone();
            let emit = Arc::new(TauriEmit(app.clone()));
            super::start_session_with(state, dry_run, emit, &move || {
                crate::tray::refresh_later(&tray)
            })
        })
        .await;
        crate::tray::refresh_later(&handle);
        result
    }

    #[tauri::command]
    pub async fn stop_session(app: AppHandle) -> Result<(), String> {
        let handle = app.clone();
        let result = blocking(app, |_, state| super::stop_session(state)).await;
        crate::tray::refresh_later(&handle);
        result
    }

    #[tauri::command]
    pub fn session_status(state: tauri::State<'_, AppState>) -> Result<StatusDto, String> {
        super::session_status(&state)
    }

    #[tauri::command]
    pub async fn list_devices() -> Result<Vec<String>, String> {
        tauri::async_runtime::spawn_blocking(super::list_devices)
            .await
            .map_err(|_| "the command failed".to_string())?
    }

    #[tauri::command]
    pub async fn capture_binding(app: AppHandle, token: Option<u32>) -> Result<BindingDto, String> {
        blocking(app, move |_, state| {
            // The scanner starts after the capture flag is set: a cancel
            // while it starts ends the capture.
            let start_scanner = || {
                let mut scanner = edcopilot_voicebridge_audio::InputScanner::new();
                move || scanner.scan()
            };
            super::capture_binding_with(state, token, start_scanner, CAPTURE_POLL, CAPTURE_TIMEOUT)
        })
        .await
    }

    /// `token` null ends any capture.
    #[tauri::command]
    pub fn cancel_capture(
        state: tauri::State<'_, AppState>,
        token: Option<u32>,
    ) -> Result<(), String> {
        super::cancel_capture_token(&state, token)
    }

    #[tauri::command]
    pub async fn set_api_key(
        app: AppHandle,
        key: String,
        expected_origin: String,
        expected_name: String,
    ) -> Result<KeyResultDto, String> {
        blocking(app, move |_, state| {
            super::set_api_key(state, &key, &expected_origin, &expected_name)
        })
        .await
    }

    #[tauri::command]
    pub async fn clear_api_key(app: AppHandle) -> Result<KeyResultDto, String> {
        blocking(app, |_, state| super::clear_api_key(state)).await
    }

    #[tauri::command]
    pub fn model_info(state: tauri::State<'_, AppState>) -> Result<ModelInfoDto, String> {
        super::model_info(&state, models_dir(&state))
    }

    /// Starts the download on its own thread and returns at once. The
    /// address and the folder come from the program, never from the web
    /// part.
    #[tauri::command]
    pub fn download_model(app: AppHandle, state: tauri::State<'_, AppState>) -> Result<(), String> {
        let models = models_dir(&state).ok_or_else(|| NO_DATA_DIR.to_string())?;
        let cancel = super::begin_model_download(&state)?;
        std::thread::spawn(move || {
            let state = app.state::<AppState>();
            let emit = TauriEmit(app.clone());
            let make_fetch = || {
                model_download::HttpFetch::new(Arc::clone(&cancel))
                    .map(|fetch| Box::new(fetch) as Box<dyn Fetch>)
            };
            let _ = super::run_model_download(
                &state,
                &model_download::PARAKEET_V3_INT8,
                &cancel,
                &models,
                &make_fetch,
                &model_download::free_space,
                &emit,
            );
        });
        Ok(())
    }

    #[tauri::command]
    pub fn cancel_model_download(state: tauri::State<'_, AppState>) -> Result<(), String> {
        super::cancel_model_download(&state)
    }

    /// The update server of the plugin, with the address and the key of
    /// `tauri.conf.json`. Just before the installer starts, the program
    /// stops the session and ends the tray icon.
    ///
    /// The plugin runs `before_exit` before it starts the installer, and
    /// ends the program itself after a start. Thus `before_exit` does only
    /// steps that the program can undo: it stops the session and hides the
    /// tray icon (a hidden icon leaves no ghost icon after the end). It does
    /// not call `cleanup_before_exit`. When the installer does not start,
    /// `restore_ui` shows the icon and the window again.
    fn update_source(app: &AppHandle) -> Result<updater::PluginSource, updater::Cause> {
        let exit = app.clone();
        updater::PluginSource::new(app, move || {
            super::shutdown(&exit.state::<AppState>());
            crate::tray::set_icon_visible(&exit, false);
        })
    }

    /// Puts the tray icon and the main window back after an installer that
    /// did not start.
    fn restore_ui(app: &AppHandle) -> updater::RestoreUi {
        let app = app.clone();
        Arc::new(move || {
            crate::tray::set_icon_visible(&app, true);
            crate::tray::show_main(&app);
        })
    }

    /// The automatic checks. The first look comes about 10 seconds after
    /// the start, then one each 30 minutes. A check runs when 24 hours of
    /// the computer clock passed since the last check (also over a sleep of
    /// the computer). Each look reads `updates.check` again, so a switch
    /// from off to on applies at the next look. A developer build never
    /// checks.
    pub fn start_update_timer(app: AppHandle) {
        if app.state::<AppState>().updates.developer_build() {
            return;
        }
        std::thread::spawn(move || {
            std::thread::sleep(updater::FIRST_CHECK_DELAY);
            loop {
                let state = app.state::<AppState>();
                let enabled = super::updates_check_enabled(&state);
                let emit = TauriEmit(app.clone());
                let source =
                    || update_source(&app).map(|s| Box::new(s) as Box<dyn updater::UpdateSource>);
                tauri::async_runtime::block_on(updater::auto_check(
                    &state.updates,
                    &source,
                    enabled,
                    &emit,
                    updater::now_ms(),
                ));
                std::thread::sleep(updater::TICK);
            }
        });
    }

    #[tauri::command]
    pub fn update_status(state: tauri::State<'_, AppState>) -> UpdateStatusDto {
        state.updates.status()
    }

    /// "Check for updates". It also works when `updates.check` is false.
    #[tauri::command]
    pub async fn check_for_updates(app: AppHandle) -> Result<UpdateStatusDto, String> {
        let updates = Arc::clone(&app.state::<AppState>().updates);
        if updates.developer_build() {
            return Err(updater::DEVELOPER_BUILD.to_string());
        }
        let source = update_source(&app).map_err(updater::check_failed_text)?;
        let emit = TauriEmit(app.clone());
        updater::run_check(&updates, &source, true, &emit).await
    }

    /// "Install update": refused while a session runs. The download runs
    /// on its own task. The steps come as `update://event`.
    #[tauri::command]
    pub fn install_update(app: AppHandle, state: tauri::State<'_, AppState>) -> Result<(), String> {
        let (id, pending) = super::begin_update_install(&state)?;
        let emit: Arc<dyn UpdateEmit> = Arc::new(TauriEmit(app.clone()));
        emit.emit_update(&state.updates.status());
        updater::start_download(
            Arc::clone(&state.updates),
            id,
            pending,
            emit,
            updater::READY_PAUSE,
            restore_ui(&app),
        );
        Ok(())
    }

    #[tauri::command]
    pub fn cancel_update(app: AppHandle, state: tauri::State<'_, AppState>) -> Result<(), String> {
        state.updates.cancel()?;
        TauriEmit(app.clone()).emit_update(&state.updates.status());
        Ok(())
    }

    /// "Later": hides the note until the next start or manual check.
    #[tauri::command]
    pub fn hide_update(app: AppHandle, state: tauri::State<'_, AppState>) {
        state.updates.hide();
        TauriEmit(app.clone()).emit_update(&state.updates.status());
    }

    #[tauri::command]
    pub async fn list_commands(
        app: AppHandle,
        filter: String,
        all: bool,
        offset: usize,
        limit: usize,
    ) -> Result<CommandListDto, String> {
        blocking(app, move |_, state| {
            super::list_commands(state, &filter, all, offset, limit)
        })
        .await
    }

    #[tauri::command]
    pub async fn list_left_out(
        app: AppHandle,
        filter: String,
        offset: usize,
        limit: usize,
    ) -> Result<LeftOutListDto, String> {
        blocking(app, move |_, state| {
            super::list_left_out(state, &filter, offset, limit)
        })
        .await
    }

    #[tauri::command]
    pub async fn test_phrase(app: AppHandle, phrase: String) -> Result<PhraseTestDto, String> {
        blocking(app, move |_, state| super::test_phrase(state, &phrase)).await
    }

    #[tauri::command]
    pub fn startup_notes(state: tauri::State<'_, AppState>) -> StartupNotesDto {
        super::startup_notes(&state)
    }

    #[tauri::command]
    pub fn take_tray_start(state: tauri::State<'_, AppState>) -> bool {
        super::take_tray_start(&state)
    }

    #[tauri::command]
    pub async fn set_diagnostics(
        app: AppHandle,
        part: DiagnosticsPart,
        on: bool,
    ) -> Result<SaveResultDto, String> {
        blocking(app, move |_, state| super::set_diagnostics(state, part, on)).await
    }

    /// The page names the part. The path comes from the saved file.
    #[tauri::command]
    pub async fn open_folder(app: AppHandle, part: DiagnosticsPart) -> Result<(), String> {
        blocking(app, move |_, state| {
            super::open_folder(state, part, &super::open_in_file_manager)
        })
        .await
    }

    #[tauri::command]
    pub async fn read_table(
        app: AppHandle,
        kind: TableKindDto,
        language: Option<String>,
    ) -> Result<TableDto, String> {
        blocking(app, move |_, state| {
            super::read_table(state, kind, language)
        })
        .await
    }

    #[tauri::command]
    pub async fn check_table(
        app: AppHandle,
        kind: TableKindDto,
        language: Option<String>,
        rows: Vec<RowDto>,
    ) -> Result<TableCheckDto, String> {
        blocking(app, move |_, state| {
            super::check_table(state, kind, language, rows)
        })
        .await
    }

    #[tauri::command]
    pub async fn save_table(
        app: AppHandle,
        kind: TableKindDto,
        language: Option<String>,
        rows: Vec<RowDto>,
        confirmed: Vec<String>,
        version: String,
    ) -> Result<TableSaveDto, String> {
        blocking(app, move |_, state| {
            super::save_table(state, kind, language, rows, confirmed, version)
        })
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::{Path, PathBuf};
    use std::sync::Mutex;

    use voicebridge::runtime::{EventSink, KeyLookup, RuntimeEvent, StopReason};

    use crate::events::tests::{start_info, FakeEmit};
    use crate::secrets::{KeyStore, MemoryStore};
    use crate::state::{SessionControl, Starter};

    const SECRET: &str = "sk-test-SECRET";

    /// A session that runs until `stop`.
    struct FakeSession {
        running: Arc<AtomicBool>,
        sink: Arc<dyn EventSink>,
    }

    impl SessionControl for FakeSession {
        fn stop(&mut self) {
            if self.running.swap(false, Ordering::SeqCst) {
                self.sink.event(RuntimeEvent::Stopped {
                    reason: StopReason::Requested,
                    glitches: 0,
                });
            }
        }

        fn is_running(&self) -> bool {
            self.running.load(Ordering::SeqCst)
        }
    }

    /// The running flag and the sink of one fake session.
    type FakeParts = (Arc<AtomicBool>, Arc<dyn EventSink>);

    /// Checks the configuration as the first step of the runtime does, and
    /// keeps the configuration and the sink of each start.
    #[derive(Default)]
    struct FakeStarter {
        configs: Mutex<Vec<Config>>,
        sessions: Mutex<Vec<FakeParts>>,
        /// The start waits like a model load, until the stop flag is set
        /// or 3 seconds pass, then it is cancelled.
        slow: AtomicBool,
    }

    impl Starter for FakeStarter {
        fn start(
            &self,
            config: &Config,
            options: RuntimeOptions,
            key_lookup: KeyLookup,
            sink: Arc<dyn EventSink>,
            stop: Arc<AtomicBool>,
        ) -> Result<Box<dyn SessionControl>, Vec<String>> {
            if self.slow.load(Ordering::SeqCst) {
                let end = std::time::Instant::now() + Duration::from_secs(3);
                while !stop.load(Ordering::SeqCst) && std::time::Instant::now() < end {
                    std::thread::sleep(Duration::from_millis(2));
                }
                return Err(vec!["the start was cancelled".to_string()]);
            }
            let mut problems = config.validate();
            problems.extend(voicebridge::run::engine_problems(config, &*key_lookup));
            if !problems.is_empty() {
                return Err(problems);
            }
            lock(&self.configs).push(config.clone());
            let running = Arc::new(AtomicBool::new(true));
            lock(&self.sessions).push((running.clone(), sink.clone()));
            sink.event(RuntimeEvent::Started(Box::new(start_info(options.dry_run))));
            Ok(Box::new(FakeSession { running, sink }))
        }
    }

    struct Setup {
        _dir: tempfile::TempDir,
        file: PathBuf,
        state: AppState,
        starter: Arc<FakeStarter>,
        keys: Arc<MemoryStore>,
        emit: Arc<FakeEmit>,
    }

    /// A copy of the example configuration that is valid, in a temporary
    /// folder.
    fn setup(extra: &[(&str, Value)]) -> Setup {
        let dir = tempfile::tempdir().unwrap();
        let model = dir.path().join("model");
        std::fs::create_dir(&model).unwrap();
        let mut edits = vec![
            (
                "edcopilot.dir".to_string(),
                Value::Text(dir.path().join("ed").display().to_string()),
            ),
            (
                "stt.parakeet.model_dir".to_string(),
                Value::Text(model.display().to_string()),
            ),
            ("stt.parakeet.threads".to_string(), Value::Integer(1)),
        ];
        edits.extend(extra.iter().map(|(k, v)| (k.to_string(), v.clone())));
        let example = include_str!("../../app/voicebridge.example.toml");
        let text = config_edit::apply_edits(example, &edits).unwrap();
        let file = dir.path().join("voicebridge.toml");
        std::fs::write(&file, text).unwrap();
        let starter = Arc::new(FakeStarter::default());
        let keys = Arc::new(MemoryStore::new());
        let state = AppState::new(file.clone(), keys.clone(), starter.clone());
        Setup {
            _dir: dir,
            file,
            state,
            starter,
            keys,
            emit: Arc::new(FakeEmit::default()),
        }
    }

    impl Setup {
        fn start(&self, dry_run: bool) -> Result<(), String> {
            start_session(&self.state, dry_run, self.emit.clone())
        }
    }

    fn edit(key: &str, value: crate::dto::ValueDto) -> EditDto {
        EditDto {
            key: key.to_string(),
            value,
        }
    }

    fn json<T: serde::Serialize>(value: &T) -> String {
        serde_json::to_string(value).unwrap()
    }

    /// `save_config` as a window that read the file just now calls it.
    fn save_config(state: &AppState, edits: Vec<EditDto>) -> Result<SaveResultDto, String> {
        let _ = get_config(state);
        let version = std::fs::read_to_string(state.config_path())
            .map(|text| config_edit::text_version(&text))
            .unwrap_or_default();
        super::save_config(state, edits, version)
    }

    /// `set_api_key` as a window that read the file just now calls it.
    fn set_key(state: &AppState, key: &str) -> Result<KeyResultDto, String> {
        let (origin, name) = Config::load_as_written(state.config_path())
            .map(|c| {
                let cloud = c.stt.openai_compatible;
                (
                    url_origin(&cloud.base_url).unwrap_or_default(),
                    cloud.api_key_env,
                )
            })
            .unwrap_or_default();
        set_api_key(state, key, &origin, &name)
    }

    fn assert_no_secret(text: &str) {
        assert!(!text.contains("SECRET"), "{text}");
    }

    #[test]
    fn test_get_config_never_holds_the_key() {
        let s = setup(&[]);
        s.keys.set("OPENAI_API_KEY", SECRET).unwrap();
        let config = get_config(&s.state).unwrap();
        assert!(config.api_key_set);
        assert_eq!(config.stt.openai_compatible.api_key_env, "OPENAI_API_KEY");
        assert_no_secret(&json(&config));
        assert_no_secret(&format!("{config:?}"));
        // The values are as the file has them.
        assert_eq!(config.matching.threshold, 0.8);
        assert_eq!(config.edcopilot.request_file, "");
        assert_eq!(config.push_to_talk.key.as_deref(), Some("F13"));
        assert_eq!(config.path, s.file.display().to_string());
    }

    /// The Microphone screen sets the talk button with these edits. The file
    /// must be the same as the file of `voicebridge bind`.
    #[test]
    fn test_the_binding_edits_of_the_page_give_the_file_of_bind() {
        use crate::dto::ValueDto;
        use voicebridge::bind::{apply_binding, Binding};
        let key_edits = |key: &str| {
            vec![
                edit("push_to_talk.joystick", ValueDto::Absent),
                edit("push_to_talk.key", ValueDto::Text(key.into())),
            ]
        };
        let joystick_edits = |device: &str, button: i64| {
            vec![
                edit("push_to_talk.key", ValueDto::Absent),
                edit(
                    "push_to_talk.joystick.device",
                    ValueDto::Text(device.into()),
                ),
                edit("push_to_talk.joystick.button", ValueDto::Integer(button)),
            ]
        };
        let s = setup(&[]);
        let steps = [
            (
                joystick_edits("MOZA \"AB9\"", 5),
                Binding::Joystick {
                    device: "MOZA \"AB9\"".into(),
                    button: 5,
                },
            ),
            (key_edits("RControl"), Binding::Key("RControl".into())),
            (
                joystick_edits("Pad \\ 2", 7),
                Binding::Joystick {
                    device: "Pad \\ 2".into(),
                    button: 7,
                },
            ),
            (
                joystick_edits("Steuerknüppel Ä – 飛行", 2),
                Binding::Joystick {
                    device: "Steuerknüppel Ä – 飛行".into(),
                    button: 2,
                },
            ),
            (
                joystick_edits("T.16000M", 3),
                Binding::Joystick {
                    device: "T.16000M".into(),
                    button: 3,
                },
            ),
        ];
        for (edits, binding) in steps {
            let before = std::fs::read_to_string(&s.file).unwrap();
            let expected = apply_binding(&before, &binding).unwrap();
            let result = save_config(&s.state, edits).unwrap();
            assert!(result.written, "{binding}: {:?}", result.problems);
            assert_eq!(
                std::fs::read_to_string(&s.file).unwrap(),
                expected,
                "{binding}"
            );
            // The name comes back to the page without a change.
            if let Binding::Joystick { device, button } = &binding {
                let joystick = get_config(&s.state).unwrap().push_to_talk.joystick.unwrap();
                assert_eq!((&joystick.device, joystick.button), (device, *button));
            }
        }
        // The key of a German keyboard keeps its text from the file to the page.
        let text = std::fs::read_to_string(&s.file).unwrap();
        std::fs::write(
            &s.file,
            apply_binding(&text, &Binding::Key("<".into())).unwrap(),
        )
        .unwrap();
        let config = get_config(&s.state).unwrap();
        assert_eq!(config.push_to_talk.key.as_deref(), Some("<"));
        assert!(config.push_to_talk.joystick.is_none());
    }

    #[test]
    fn test_save_config_writes_and_refuses() {
        let s = setup(&[]);
        let threshold = |n: f64| vec![edit("matching.threshold", crate::dto::ValueDto::Number(n))];
        let result = save_config(&s.state, threshold(0.85)).unwrap();
        assert_eq!(
            result,
            SaveResultDto {
                problems: vec![],
                written: true,
                restart_needed: false
            }
        );
        // The same value again: nothing to write.
        let result = save_config(&s.state, threshold(0.85)).unwrap();
        assert!(!result.written);
        let before = std::fs::read(&s.file).unwrap();
        let result = save_config(
            &s.state,
            vec![edit(
                "matching.threshold",
                crate::dto::ValueDto::Number(7.0),
            )],
        )
        .unwrap();
        assert_eq!(result.problems.len(), 1, "{result:?}");
        assert!(!result.restart_needed);
        assert_eq!(std::fs::read(&s.file).unwrap(), before);
        // A key that is not in the configuration.
        let error = save_config(
            &s.state,
            vec![edit(
                "stt.openai_compatible.api_key",
                crate::dto::ValueDto::Text("x".into()),
            )],
        )
        .unwrap_err();
        assert!(error.contains("not a key"), "{error}");
        assert_eq!(std::fs::read(&s.file).unwrap(), before);
    }

    #[test]
    fn test_save_while_running_reports_restart_needed_and_the_session_keeps_its_settings() {
        let s = setup(&[]);
        s.start(true).unwrap();
        let result = save_config(
            &s.state,
            vec![edit(
                "matching.threshold",
                crate::dto::ValueDto::Number(0.9),
            )],
        )
        .unwrap();
        assert!(result.problems.is_empty(), "{result:?}");
        assert!(result.written);
        assert!(result.restart_needed);
        // The same value again: no write, so no restart.
        let result = save_config(
            &s.state,
            vec![edit(
                "matching.threshold",
                crate::dto::ValueDto::Number(0.9),
            )],
        )
        .unwrap();
        assert!(!result.written && !result.restart_needed, "{result:?}");
        // The file has the new value, the running session the old one.
        assert_eq!(Config::load(&s.file).unwrap().matching.threshold, 0.9);
        assert_eq!(lock(&s.starter.configs)[0].matching.threshold, 0.8);
        assert!(session_status(&s.state).unwrap().running);
        // After a restart the session has the new value.
        stop_session(&s.state).unwrap();
        s.start(true).unwrap();
        assert_eq!(lock(&s.starter.configs)[1].matching.threshold, 0.9);
        stop_session(&s.state).unwrap();
        let result = save_config(
            &s.state,
            vec![edit(
                "matching.threshold",
                crate::dto::ValueDto::Number(0.8),
            )],
        )
        .unwrap();
        assert!(result.written);
        assert!(!result.restart_needed);
    }

    #[test]
    fn test_save_of_a_fresh_example_keeps_old_problems_as_information() {
        // The model folder is empty, as in a fresh copy of the example.
        let s = setup(&[("stt.parakeet.model_dir", Value::Text(String::new()))]);
        let result = save_config(
            &s.state,
            vec![edit(
                "matching.threshold",
                crate::dto::ValueDto::Number(0.85),
            )],
        )
        .unwrap();
        assert!(result.written, "{result:?}");
        assert_eq!(result.problems.len(), 1, "{result:?}");
        assert!(result.problems[0].starts_with("stt.parakeet.model_dir is empty"));
    }

    #[test]
    fn test_a_configuration_error_never_shows_the_line_or_a_value() {
        const SECRET: &str = "sk-test-1234567890abcdef";
        let s = setup(&[]);
        let head = "[edcopilot]\ndir = 'C:/E'\n";
        for body in [
            format!("[stt.openai_compatible]\napi_key_env = {SECRET}\n"),
            format!("[stt.parakeet]\nthreads = \"{SECRET}\"\n"),
            format!("[stt.openai_compatible]\napi_key = \"{SECRET}\"\n"),
        ] {
            std::fs::write(&s.file, format!("{head}{body}")).unwrap();
            let mut texts = vec![
                get_config(&s.state).unwrap_err(),
                s.start(true).unwrap_err(),
            ];
            texts.extend(check(&s.state).unwrap().problems);
            for text in texts {
                assert!(!text.contains(SECRET), "{text}");
                assert!(text.contains("line 4, column "), "{text}");
            }
        }
    }

    #[test]
    fn test_start_with_a_broken_configuration_file_gives_problems_and_keeps_the_file() {
        let s = setup(&[]);
        let broken = "[edcopilot\ndir = 'x'\n";
        std::fs::write(&s.file, broken).unwrap();
        let error = s.start(true).unwrap_err();
        assert!(error.contains("voicebridge.toml"), "{error}");
        // The window shows the same text in each place.
        let config_error = get_config(&s.state).unwrap_err();
        let found = check(&s.state).unwrap();
        assert_eq!(found.problems.len(), 1, "{found:?}");
        assert!(found.report.is_none());
        assert!(found.problems[0].contains("voicebridge.toml"), "{found:?}");
        assert!(config_error.contains("voicebridge.toml"), "{config_error}");
        assert!(lock(&s.starter.configs).is_empty());
        assert!(!session_status(&s.state).unwrap().running);
        assert_eq!(std::fs::read_to_string(&s.file).unwrap(), broken);

        // An absent file: also no crash, and no file is made.
        std::fs::remove_file(&s.file).unwrap();
        assert!(get_config(&s.state).is_err());
        assert_eq!(check(&s.state).unwrap().problems.len(), 1);
        assert!(s.start(false).is_err());
        assert!(!s.file.exists());

        // A file that loads but has problems: one problem on each line.
        std::fs::write(
            &s.file,
            "[edcopilot]\ndir = 'ed'\n[matching]\nthreshold = 7.0\n[stt.parakeet]\nthreads = 0\n",
        )
        .unwrap();
        let error = s.start(false).unwrap_err();
        let lines: Vec<&str> = error.lines().collect();
        assert!(lines.len() >= 3, "{error}");
        assert!(lines[0].contains("matching.threshold"), "{error}");
    }

    #[test]
    fn test_start_twice_gives_an_error() {
        let s = setup(&[]);
        s.start(true).unwrap();
        assert_eq!(s.start(false).unwrap_err(), ALREADY_RUNNING);
        let status = session_status(&s.state).unwrap();
        assert!(status.running);
        assert!(status.dry_run);
        assert_eq!(lock(&s.starter.configs).len(), 1);
        stop_session(&s.state).unwrap();
        let status = session_status(&s.state).unwrap();
        assert!(!status.running);
        assert_eq!(status.stop_reason.unwrap().kind, "requested");
        assert_eq!(s.emit.types(), ["started", "stopped"]);
    }

    #[test]
    fn test_stop_without_a_session_is_ok() {
        let s = setup(&[]);
        assert_eq!(stop_session(&s.state), Ok(()));
        assert_eq!(stop_session(&s.state), Ok(()));
        let status = session_status(&s.state).unwrap();
        assert_eq!(
            status,
            StatusDto {
                running: false,
                starting: false,
                dry_run: false,
                start_info: None,
                stop_reason: None
            }
        );
    }

    #[test]
    fn test_status_is_not_running_after_an_error_stop() {
        let s = setup(&[]);
        s.start(false).unwrap();
        assert!(session_status(&s.state).unwrap().running);
        // The microphone fails: the runtime sends `Stopped` by itself.
        let (running, sink) = lock(&s.starter.sessions)[0].clone();
        running.store(false, Ordering::SeqCst);
        sink.event(RuntimeEvent::Stopped {
            reason: StopReason::DeviceError("the device was removed".into()),
            glitches: 1,
        });
        let status = session_status(&s.state).unwrap();
        assert!(!status.running);
        let reason = status.stop_reason.unwrap();
        assert_eq!(reason.kind, "device_error");
        assert_eq!(reason.message.as_deref(), Some("the device was removed"));
        assert!(status.start_info.is_some());
        // A new start works without a stop from the web part.
        s.start(false).unwrap();
        assert!(session_status(&s.state).unwrap().running);
        stop_session(&s.state).unwrap();
    }

    #[test]
    fn test_capture_is_refused_while_a_session_runs() {
        let s = setup(&[]);
        s.start(true).unwrap();
        let fast = Duration::from_millis(1);
        let mut scans = 0;
        let error = capture_binding(
            &s.state,
            || {
                scans += 1;
                Vec::new()
            },
            fast,
            fast,
        )
        .unwrap_err();
        assert_eq!(error, STOP_FIRST);
        assert_eq!(scans, 0, "the capture read the inputs");
        stop_session(&s.state).unwrap();
        // After the stop the capture works.
        let mut script = vec![
            vec![],
            vec![PressedInput::Key("F14".into())],
            vec![PressedInput::Key("F14".into())],
        ]
        .into_iter();
        let binding = capture_binding(
            &s.state,
            move || script.next().unwrap_or_default(),
            fast,
            Duration::from_secs(5),
        )
        .unwrap();
        assert_eq!(binding.key.as_deref(), Some("F14"));
        assert_eq!(binding.kind, "key");
        // The capture did not write the configuration.
        assert_eq!(
            Config::load(&s.file).unwrap().push_to_talk.key.as_deref(),
            Some("F13")
        );
        assert!(!s.state.capture_active());
    }

    /// A capture that comes between the check of the start and the start
    /// itself must be refused: two readers of the talk button do not work.
    #[test]
    fn test_capture_and_start_cannot_both_run() {
        let s = setup(&[]);
        let result: Arc<Mutex<Option<Result<BindingDto, String>>>> = Arc::default();
        let out = Arc::clone(&result);
        *lock(&s.state.start_hook) = Some(Box::new(move |state: &AppState| {
            let mut script =
                vec![vec![], vec![PressedInput::Key("F14".into())], vec![]].into_iter();
            let fast = Duration::from_millis(1);
            *lock(&out) = Some(capture_binding(
                state,
                move || script.next().unwrap_or_default(),
                fast,
                Duration::from_secs(5),
            ));
        }));
        s.start(true).unwrap();
        let capture = lock(&result).take().expect("the hook ran");
        assert_eq!(capture, Err(STOP_FIRST.to_string()));
        stop_session(&s.state).unwrap();
    }

    /// The input scanner needs time to start (50 ms for the joystick
    /// backend). A cancel in that time must end the capture, so the flag
    /// must be in place before the scanner starts. A refused capture does
    /// not start the scanner.
    #[test]
    fn test_a_cancel_while_the_scanner_starts_ends_the_capture() {
        let s = setup(&[]);
        let fast = Duration::from_millis(1);
        let mut started = 0;
        let error = capture_binding_with(
            &s.state,
            None,
            || {
                started += 1;
                assert!(s.state.capture_active(), "the flag is not set yet");
                cancel_capture(&s.state).unwrap();
                Vec::new
            },
            fast,
            Duration::from_secs(10),
        )
        .unwrap_err();
        assert_eq!(error, "the capture was cancelled");
        assert_eq!(started, 1);
        assert!(!s.state.capture_active());

        s.start(true).unwrap();
        let mut started = 0;
        let error = capture_binding_with(
            &s.state,
            None,
            || {
                started += 1;
                Vec::new
            },
            fast,
            fast,
        )
        .unwrap_err();
        assert_eq!(error, STOP_FIRST);
        assert_eq!(started, 0, "a refused capture started the scanner");
        stop_session(&s.state).unwrap();
    }

    /// A cancel of the page names its capture. A late cancel of an old
    /// capture must not end the next capture. A cancel without a token
    /// (a new page load, the end of the program) ends any capture.
    #[test]
    fn test_a_cancel_with_an_old_token_does_not_cancel_a_new_capture() {
        let s = Arc::new(setup(&[]));
        let run = |token: Option<u32>| {
            let s = Arc::clone(&s);
            std::thread::spawn(move || {
                capture_binding_with(
                    &s.state,
                    token,
                    || Vec::new,
                    Duration::from_millis(1),
                    Duration::from_secs(10),
                )
            })
        };
        let wait_active = || {
            while !s.state.capture_active() {
                std::thread::sleep(Duration::from_millis(1));
            }
        };
        let worker = run(Some(2));
        wait_active();
        cancel_capture_token(&s.state, Some(1)).unwrap();
        std::thread::sleep(Duration::from_millis(30));
        assert!(s.state.capture_active(), "an old token ended the capture");
        cancel_capture_token(&s.state, Some(2)).unwrap();
        assert!(worker.join().unwrap().unwrap_err().contains("cancelled"));

        let worker = run(Some(3));
        wait_active();
        cancel_capture_token(&s.state, None).unwrap();
        assert!(worker.join().unwrap().unwrap_err().contains("cancelled"));
        assert!(!s.state.capture_active());
    }

    #[test]
    fn test_capture_can_be_cancelled_and_blocks_a_start() {
        let s = Arc::new(setup(&[]));
        let worker = {
            let s = Arc::clone(&s);
            std::thread::spawn(move || {
                capture_binding(
                    &s.state,
                    Vec::new,
                    Duration::from_millis(1),
                    Duration::from_secs(10),
                )
            })
        };
        while !s.state.capture_active() {
            std::thread::sleep(Duration::from_millis(1));
        }
        assert_eq!(s.start(true).unwrap_err(), CAPTURE_RUNS);
        cancel_capture(&s.state).unwrap();
        let error = worker.join().unwrap().unwrap_err();
        assert!(error.contains("cancelled"), "{error}");
        assert!(!s.state.capture_active());
        assert_eq!(cancel_capture(&s.state), Ok(()));
        s.start(true).unwrap();
        stop_session(&s.state).unwrap();
    }

    #[test]
    fn test_set_api_key_then_config_reports_key_set_and_no_text_holds_it() {
        // A variable name that the environment does not have.
        let name = "VOICEBRIDGE_UI_TEST_NO_SUCH_KEY";
        let s = setup(&[
            ("stt.engine", Value::Text("openai_compatible".into())),
            (
                "stt.openai_compatible.api_key_env",
                Value::Text(name.into()),
            ),
        ]);
        assert!(!get_config(&s.state).unwrap().api_key_set);
        // Without a key the start is refused, and the problem names the
        // variable.
        let error = s.start(true).unwrap_err();
        assert!(error.contains(name), "{error}");

        set_key(&s.state, SECRET).unwrap();
        assert_eq!(s.keys.get(name).as_deref(), Some(SECRET));
        assert_eq!(set_key(&s.state, "  ").unwrap_err(), "the API key is empty");

        let mut texts = Vec::new();
        let config = get_config(&s.state).unwrap();
        assert!(config.api_key_set);
        texts.push(json(&config));
        let found = check(&s.state).unwrap();
        assert!(
            !found.problems.iter().any(|p| p.contains(name)),
            "{found:?}"
        );
        texts.push(json(&found));
        texts.push(json(
            &save_config(
                &s.state,
                vec![edit(
                    "stt.openai_compatible.model",
                    crate::dto::ValueDto::Text("m".into()),
                )],
            )
            .unwrap(),
        ));
        texts.push(json(&session_status(&s.state).unwrap()));

        // A start with the key, then a stop.
        s.start(true).unwrap();
        texts.push(json(&session_status(&s.state).unwrap()));
        texts.push(s.start(true).unwrap_err());
        stop_session(&s.state).unwrap();
        texts.push(json(&session_status(&s.state).unwrap()));

        // A failed start with the key: a bad threshold.
        let text = std::fs::read_to_string(&s.file).unwrap();
        let bad = config_edit::set_value(&text, "matching.threshold", &Value::Number(7.0)).unwrap();
        std::fs::write(&s.file, bad).unwrap();
        let error = s.start(false).unwrap_err();
        assert!(error.contains("matching.threshold"), "{error}");
        texts.push(error);
        texts.push(json(&check(&s.state).unwrap()));
        std::fs::write(&s.file, &text).unwrap();

        // A capture that times out, and the events of the window.
        let fast = Duration::from_millis(1);
        texts.push(capture_binding(&s.state, Vec::new, fast, fast).unwrap_err());
        texts.extend(lock(&s.emit.0).iter().map(|event| event.to_string()));
        for text in &texts {
            assert_no_secret(text);
        }
        assert!(texts.len() >= 10);

        clear_api_key(&s.state).unwrap();
        assert!(!get_config(&s.state).unwrap().api_key_set);
        assert_eq!(s.keys.get(name), None);
    }

    #[test]
    fn test_a_key_in_the_variable_name_field_is_refused_and_never_shown() {
        let s = setup(&[]);
        let before = std::fs::read(&s.file).unwrap();
        let error = save_config(
            &s.state,
            vec![edit(
                "stt.openai_compatible.api_key_env",
                crate::dto::ValueDto::Text(SECRET.into()),
            )],
        )
        .unwrap_err();
        assert_no_secret(&error);
        assert_eq!(std::fs::read(&s.file).unwrap(), before);

        // A file that already holds a key in that field.
        let text = String::from_utf8(before.clone()).unwrap();
        let bad = text.replace("\"OPENAI_API_KEY\"", &format!("\"{SECRET}\""));
        assert_ne!(bad, text);
        std::fs::write(&s.file, &bad).unwrap();
        s.keys.set(SECRET, "stored").unwrap();
        let config = get_config(&s.state).unwrap();
        assert_eq!(config.stt.openai_compatible.api_key_env, "");
        assert!(!config.api_key_set);
        let mut texts = vec![json(&config)];
        let found = check(&s.state).unwrap();
        assert!(
            found
                .problems
                .iter()
                .any(|p| p == voicebridge::config::API_KEY_ENV_PROBLEM),
            "{found:?}"
        );
        texts.push(json(&found));
        texts.push(set_key(&s.state, "sk-other").unwrap_err());
        texts.push(clear_api_key(&s.state).unwrap_err());
        texts.push(s.start(true).unwrap_err());
        texts.push(json(&save_config(
            &s.state,
            vec![edit(
                "matching.threshold",
                crate::dto::ValueDto::Number(0.85),
            )],
        )));
        for text in &texts {
            assert_no_secret(text);
        }
        // The key store is not touched with that value as a name.
        assert_eq!(s.keys.get(SECRET).as_deref(), Some("stored"));
    }

    #[test]
    fn test_saved_key_is_bound_to_its_host() {
        let name = "VOICEBRIDGE_UI_TEST_NO_SUCH_KEY";
        let s = setup(&[
            ("stt.engine", Value::Text("openai_compatible".into())),
            (
                "stt.openai_compatible.api_key_env",
                Value::Text(name.into()),
            ),
        ]);
        assert_eq!(get_config(&s.state).unwrap().api_key_host, None);
        set_key(&s.state, SECRET).unwrap();
        let config = get_config(&s.state).unwrap();
        assert_eq!(
            config.api_key_host.as_deref(),
            Some("https://api.openai.com")
        );
        s.start(true).unwrap();
        stop_session(&s.state).unwrap();

        // Another server: the stored key is not given out.
        let url = |u: &str| {
            vec![edit(
                "stt.openai_compatible.base_url",
                crate::dto::ValueDto::Text(u.into()),
            )]
        };
        assert!(
            save_config(&s.state, url("https://evil.example.org/v1"))
                .unwrap()
                .written
        );
        let expected = "the saved API key belongs to https://api.openai.com, \
                        enter the key again for https://evil.example.org";
        assert_eq!(s.start(true).unwrap_err(), expected);
        assert_eq!(lock(&s.starter.configs).len(), 1, "the second start ran");
        let found = check(&s.state).unwrap();
        assert!(found.problems.iter().any(|p| p == expected), "{found:?}");
        let config = Config::load(&s.file).unwrap();
        assert_eq!(session_key_lookup(&s.state, &config)(name), None);
        // A port is part of the host.
        save_config(&s.state, url("https://api.openai.com:8443/v1")).unwrap();
        assert!(s.start(true).unwrap_err().contains(":8443"));

        // The key again for the new server.
        save_config(&s.state, url("https://evil.example.org/v1")).unwrap();
        set_key(&s.state, "sk-other").unwrap();
        assert_eq!(
            get_config(&s.state).unwrap().api_key_host.as_deref(),
            Some("https://evil.example.org")
        );
        s.start(true).unwrap();
        stop_session(&s.state).unwrap();

        // The removal removes both entries.
        clear_api_key(&s.state).unwrap();
        assert_eq!(s.keys.get(name), None);
        assert_eq!(s.keys.get(&crate::secrets::host_entry(name)), None);
        assert_eq!(get_config(&s.state).unwrap().api_key_host, None);

        // A base URL that cannot be read: no key is saved.
        let text = std::fs::read_to_string(&s.file).unwrap();
        let bad = config_edit::set_value(
            &text,
            "stt.openai_compatible.base_url",
            &Value::Text("not a url".into()),
        )
        .unwrap();
        std::fs::write(&s.file, bad).unwrap();
        let error = set_key(&s.state, SECRET).unwrap_err();
        assert!(error.contains("base_url"), "{error}");
        assert_no_secret(&error);
        assert_eq!(s.keys.get(name), None);
    }

    /// A key store that runs a hook one time, right after the write of a
    /// key and before the write of its host entry.
    struct PausingStore {
        inner: Arc<MemoryStore>,
        after_key: Mutex<Option<Box<dyn FnOnce() + Send>>>,
    }

    impl KeyStore for PausingStore {
        fn set(&self, name: &str, key: &str) -> Result<(), String> {
            self.inner.set(name, key)?;
            if !name.ends_with(".host") {
                let hook = lock(&self.after_key).take();
                if let Some(hook) = hook {
                    hook();
                }
            }
            Ok(())
        }
        fn has(&self, name: &str) -> bool {
            self.inner.has(name)
        }
        fn get(&self, name: &str) -> Option<String> {
            self.inner.get(name)
        }
        fn delete(&self, name: &str) -> Result<(), String> {
            self.inner.delete(name)
        }
    }

    /// A hand-written `base_url` with a user name, a password, a query or a
    /// fragment. No text that goes to the page holds these parts.
    #[test]
    fn test_a_secret_in_the_base_url_never_reaches_the_page() {
        use crate::dto::ValueDto;
        let name = "VOICEBRIDGE_UI_TEST_NO_SUCH_KEY";
        let s = setup(&[
            ("stt.engine", Value::Text("openai_compatible".into())),
            (
                "stt.openai_compatible.api_key_env",
                Value::Text(name.into()),
            ),
            (
                "stt.openai_compatible.base_url",
                Value::Text("https://user:pw@host.example/v1?api_key=abc#frag".into()),
            ),
        ]);
        let mut texts = Vec::new();
        let config = get_config(&s.state).unwrap();
        let cloud = &config.stt.openai_compatible;
        assert_eq!(cloud.base_url, "https://host.example/v1");
        assert_eq!(cloud.base_url_problem.as_deref(), Some(BASE_URL_SECRET));
        texts.push(json(&config));
        texts.push(format!("{config:?}"));
        let found = check(&s.state).unwrap();
        assert!(
            found.problems.iter().any(|p| p.contains("base_url")),
            "{found:?}"
        );
        texts.push(json(&found));
        let saved = save_config(
            &s.state,
            vec![edit("matching.threshold", ValueDto::Number(0.85))],
        )
        .unwrap();
        texts.push(json(&saved));
        let error = set_key(&s.state, SECRET).unwrap_err();
        assert_eq!(error, BASE_URL_SECRET);
        assert!(!s.keys.has(name));
        texts.push(error);
        texts.push(s.start(true).unwrap_err());

        // Only a query and a fragment: the start works.
        let text = std::fs::read_to_string(&s.file).unwrap();
        let query = config_edit::set_value(
            &text,
            "stt.openai_compatible.base_url",
            &Value::Text("https://host.example/v1?api_key=abc#frag".into()),
        )
        .unwrap();
        std::fs::write(&s.file, query).unwrap();
        s.keys.set(name, SECRET).unwrap();
        s.keys
            .set(&crate::secrets::host_entry(name), "https://host.example")
            .unwrap();
        let config = get_config(&s.state).unwrap();
        assert_eq!(
            config.stt.openai_compatible.base_url,
            "https://host.example/v1"
        );
        texts.push(json(&config));
        texts.push(json(&check(&s.state).unwrap()));
        texts.push(set_key(&s.state, "sk-other").unwrap_err());
        s.start(true).unwrap();
        texts.push(json(&session_status(&s.state).unwrap()));
        stop_session(&s.state).unwrap();
        texts.extend(lock(&s.emit.0).iter().map(|event| event.to_string()));

        // The temporary folder has a random name: it is not checked.
        let dir = s.file.parent().unwrap().display().to_string();
        for text in &texts {
            let text = text
                .replace(&dir.replace('\\', "\\\\"), "<dir>")
                .replace(&dir, "<dir>");
            for secret in ["pw", "abc", "frag", "user:"] {
                assert!(!text.contains(secret), "{secret} in {text}");
            }
            assert_no_secret(&text);
        }
    }

    /// The window has a key field. Its check names it where the console
    /// names only the environment variable.
    #[test]
    fn test_check_of_the_window_points_to_the_key_field() {
        let name = "VOICEBRIDGE_UI_TEST_NO_SUCH_KEY";
        let s = setup(&[
            ("stt.engine", Value::Text("openai_compatible".into())),
            (
                "stt.openai_compatible.api_key_env",
                Value::Text(name.into()),
            ),
        ]);
        let console =
            format!("environment variable {name} is absent or empty, it must hold the API key");
        let config = Config::load(&s.file).unwrap();
        let report_dir = tempfile::tempdir().unwrap();
        assert!(
            voicebridge::run::check_config_in(&config, |_| None, Some(report_dir.path()))
                .problems
                .contains(&console)
        );
        let found = check(&s.state).unwrap();
        assert!(
            found
                .problems
                .contains(&format!("{console}, {KEY_FIELD_HINT}")),
            "{found:?}"
        );
        assert!(!found.problems.contains(&console), "{found:?}");
    }

    /// A running session read the key at its start. A key change needs a
    /// restart only when that session uses the cloud engine.
    #[test]
    fn test_a_key_change_needs_a_restart_only_for_a_cloud_session() {
        let name = "VOICEBRIDGE_UI_TEST_NO_SUCH_KEY";
        let cloud = setup(&[
            ("stt.engine", Value::Text("openai_compatible".into())),
            (
                "stt.openai_compatible.api_key_env",
                Value::Text(name.into()),
            ),
        ]);
        assert!(!set_key(&cloud.state, "sk-one").unwrap().restart_needed);
        cloud.start(true).unwrap();
        assert!(set_key(&cloud.state, "sk-two").unwrap().restart_needed);
        assert!(clear_api_key(&cloud.state).unwrap().restart_needed);
        stop_session(&cloud.state).unwrap();
        assert!(!set_key(&cloud.state, "sk-three").unwrap().restart_needed);
        assert!(!clear_api_key(&cloud.state).unwrap().restart_needed);

        // The local engine does not read the key.
        let local = setup(&[(
            "stt.openai_compatible.api_key_env",
            Value::Text(name.into()),
        )]);
        local.start(true).unwrap();
        assert!(!set_key(&local.state, "sk-one").unwrap().restart_needed);
        assert!(!clear_api_key(&local.state).unwrap().restart_needed);
        stop_session(&local.state).unwrap();
    }

    /// A key change during a start needs a restart: the start can have read
    /// the old key.
    #[test]
    fn test_a_key_change_during_a_start_needs_a_restart() {
        let name = "VOICEBRIDGE_UI_TEST_NO_SUCH_KEY";
        let s = Arc::new(setup(&[
            ("stt.engine", Value::Text("openai_compatible".into())),
            (
                "stt.openai_compatible.api_key_env",
                Value::Text(name.into()),
            ),
        ]));
        s.starter.slow.store(true, Ordering::SeqCst);
        let starting = {
            let s = Arc::clone(&s);
            std::thread::spawn(move || s.start(true))
        };
        while !s.state.session_active() {
            std::thread::sleep(Duration::from_millis(1));
        }
        assert!(set_key(&s.state, "sk-one").unwrap().restart_needed);
        stop_session(&s.state).unwrap();
        starting.join().unwrap().unwrap_err();
    }

    /// A bad key text is refused with a fixed text before the key store
    /// changes. The saved key stays.
    #[test]
    fn test_set_api_key_refuses_an_empty_long_or_control_key_and_keeps_the_saved_key() {
        let name = "VOICEBRIDGE_UI_TEST_NO_SUCH_KEY";
        let s = setup(&[
            ("stt.engine", Value::Text("openai_compatible".into())),
            (
                "stt.openai_compatible.api_key_env",
                Value::Text(name.into()),
            ),
        ]);
        set_key(&s.state, SECRET).unwrap();
        let long = format!("SECRET{}", "x".repeat(MAX_KEY_CHARS));
        let cases = [
            (String::new(), KEY_EMPTY),
            (" \t ".to_string(), KEY_EMPTY),
            (long, KEY_TOO_LONG),
            ("sk-SECRET\nnext".to_string(), KEY_CONTROL),
            ("sk-SECRET\u{7f}".to_string(), KEY_CONTROL),
        ];
        for (key, text) in cases {
            assert_eq!(set_key(&s.state, &key).unwrap_err(), text);
            assert_eq!(s.keys.get(name).as_deref(), Some(SECRET));
            assert_eq!(
                s.keys.get(&crate::secrets::host_entry(name)).as_deref(),
                Some("https://api.openai.com")
            );
        }
        // A key of the longest length is saved. Spaces at the ends do not count.
        let longest = "k".repeat(MAX_KEY_CHARS);
        set_key(&s.state, &format!(" {longest} ")).unwrap();
        assert_eq!(s.keys.get(name), Some(longest));
    }

    /// A session that read the file before a change of the server starts
    /// while the window saves a key for the new server. The lookup of that
    /// session must never give the new key: it waits for the save, and then
    /// the host entry names the new server.
    #[test]
    fn test_a_key_save_never_gives_the_new_key_to_the_old_host() {
        let name = "VOICEBRIDGE_UI_TEST_NO_SUCH_KEY";
        let s = setup(&[
            ("stt.engine", Value::Text("openai_compatible".into())),
            (
                "stt.openai_compatible.api_key_env",
                Value::Text(name.into()),
            ),
        ]);
        set_key(&s.state, "sk-old-for-a").unwrap();
        let old_config = Config::load(&s.file).unwrap();
        let old_host = url_origin(&old_config.stt.openai_compatible.base_url);
        let moved = save_config(
            &s.state,
            vec![edit(
                "stt.openai_compatible.base_url",
                crate::dto::ValueDto::Text("https://new.example.org/v1".into()),
            )],
        )
        .unwrap();
        assert!(moved.written);
        let new_host = Some("https://new.example.org".to_string());

        let store = Arc::new(PausingStore {
            inner: s.keys.clone(),
            after_key: Mutex::new(None),
        });
        let state = Arc::new(AppState::new(
            s.file.clone(),
            store.clone(),
            s.starter.clone(),
        ));
        let old_lookup = session_key_lookup(&state, &old_config);
        let gap: Arc<Mutex<Vec<String>>> = Arc::default();
        let reader: Arc<Mutex<Option<std::thread::JoinHandle<Option<String>>>>> = Arc::default();
        {
            let gap = Arc::clone(&gap);
            let reader = Arc::clone(&reader);
            let inner = s.keys.clone();
            let (old_host, new_host) = (old_host.clone(), new_host.clone());
            *lock(&store.after_key) = Some(Box::new(move || {
                // A reader without the key lock sees the store as it is
                // now: a key without its host entry. It must refuse.
                for host in [old_host, new_host] {
                    let free = crate::secrets::key_lookup(inner.clone(), host, Arc::default());
                    if let Some(key) = free(name) {
                        lock(&gap).push(format!("unlocked lookup gave {key}"));
                    }
                }
                // The lookup of a session starts in the gap. It must wait.
                let (done, waiting) = std::sync::mpsc::channel();
                let lookup = Arc::clone(&old_lookup);
                *lock(&reader) = Some(std::thread::spawn(move || {
                    let found = lookup(name);
                    let _ = done.send(());
                    found
                }));
                if waiting.recv_timeout(Duration::from_millis(200)).is_ok() {
                    lock(&gap).push("the lookup did not wait for the save".into());
                }
            }));
        }
        set_api_key(&state, "sk-new-for-b", "https://new.example.org", name).unwrap();
        let found = lock(&reader).take().unwrap().join().unwrap();
        assert_eq!(*lock(&gap), Vec::<String>::new());
        assert_eq!(found, None, "the old server got a key");
        // The new server gets the new key.
        let config = Config::load(&s.file).unwrap();
        assert_eq!(
            session_key_lookup(&state, &config)(name).as_deref(),
            Some("sk-new-for-b")
        );
    }

    /// The file changes between the load of the page and the key send (the
    /// console program, an editor, a second window). The key must not go to
    /// a server or a name that the page did not show.
    #[test]
    fn test_set_api_key_refuses_when_the_file_changed_after_the_page_load() {
        let name = "VOICEBRIDGE_UI_TEST_NO_SUCH_KEY";
        let s = setup(&[
            ("stt.engine", Value::Text("openai_compatible".into())),
            (
                "stt.openai_compatible.api_key_env",
                Value::Text(name.into()),
            ),
        ]);
        let shown = get_config(&s.state).unwrap();
        let origin = "https://api.openai.com";
        assert_eq!(
            url_origin(&shown.stt.openai_compatible.base_url).as_deref(),
            Some(origin)
        );
        // Another program changes the server in the file.
        let text = std::fs::read_to_string(&s.file).unwrap();
        let moved = config_edit::set_value(
            &text,
            "stt.openai_compatible.base_url",
            &Value::Text("https://evil.example.org/v1".into()),
        )
        .unwrap();
        std::fs::write(&s.file, &moved).unwrap();
        let error = set_api_key(&s.state, SECRET, origin, name).unwrap_err();
        assert_eq!(error, SERVER_CHANGED);
        assert_no_secret(&error);
        assert_eq!(s.keys.get(name), None);
        assert_eq!(s.keys.get(&crate::secrets::host_entry(name)), None);

        // Another program changes the variable name in the file.
        let renamed = config_edit::set_value(
            &text,
            "stt.openai_compatible.api_key_env",
            &Value::Text("OTHER_NAME".into()),
        )
        .unwrap();
        std::fs::write(&s.file, &renamed).unwrap();
        let error = set_api_key(&s.state, SECRET, origin, name).unwrap_err();
        assert_eq!(error, NAME_CHANGED);
        assert!(!s.keys.has(name) && !s.keys.has("OTHER_NAME"));

        // The values that the page showed: the key is saved.
        std::fs::write(&s.file, &text).unwrap();
        set_api_key(&s.state, SECRET, origin, name).unwrap();
        assert_eq!(s.keys.get(name).as_deref(), Some(SECRET));
    }

    #[test]
    fn test_check_warns_about_a_key_without_encryption() {
        let s = setup(&[
            ("stt.engine", Value::Text("openai_compatible".into())),
            (
                "stt.openai_compatible.base_url",
                Value::Text("http://stt.example.org/v1".into()),
            ),
        ]);
        let found = check(&s.state).unwrap();
        assert!(
            found
                .warnings
                .iter()
                .any(|w| w == "the cloud engine sends the API key without encryption"),
            "{found:?}"
        );
    }

    /// A stop during the model load ends the start without a wait for the
    /// whole load. Then the status is "not running" and a new start works.
    #[test]
    fn test_stop_during_a_start_cancels_the_start() {
        let s = Arc::new(setup(&[]));
        s.starter.slow.store(true, Ordering::SeqCst);
        let starting = {
            let s = Arc::clone(&s);
            std::thread::spawn(move || s.start(true))
        };
        while !s.state.session_active() {
            std::thread::sleep(Duration::from_millis(1));
        }
        let begin = std::time::Instant::now();
        stop_session(&s.state).unwrap();
        assert!(
            begin.elapsed() < Duration::from_secs(1),
            "the stop waited {:?}",
            begin.elapsed()
        );
        let error = starting.join().unwrap().unwrap_err();
        assert!(error.contains("cancelled"), "{error}");
        assert!(!session_status(&s.state).unwrap().running);
        assert!(!session_status(&s.state).unwrap().starting);
        assert!(!s.state.session_active());
        s.starter.slow.store(false, Ordering::SeqCst);
        s.start(true).unwrap();
        assert!(session_status(&s.state).unwrap().running);
        stop_session(&s.state).unwrap();
    }

    /// A stop that comes right after the start set `starting` must find the
    /// stop flag of that start. Else the stop waits for the whole start.
    #[test]
    fn test_stop_right_after_the_start_marks_starting_finds_the_stop_flag() {
        let s = setup(&[]);
        s.starter.slow.store(true, Ordering::SeqCst);
        let found = Arc::new(AtomicBool::new(false));
        let seen = Arc::clone(&found);
        // The hook runs after `starting` is set: a stop at this point must
        // reach the flag. This is the first part of `stop_session`.
        *lock(&s.state.start_hook) = Some(Box::new(move |state: &AppState| {
            assert!(state.session_active());
            if let Some(stop) = lock(&state.stop_flag).as_ref() {
                stop.store(true, Ordering::SeqCst);
                seen.store(true, Ordering::SeqCst);
            }
        }));
        let begin = std::time::Instant::now();
        let error = s.start(true).unwrap_err();
        assert!(found.load(Ordering::SeqCst), "no stop flag");
        assert!(error.contains("cancelled"), "{error}");
        // The slow fake start ends at once when its flag is set.
        assert!(
            begin.elapsed() < Duration::from_secs(2),
            "{:?}",
            begin.elapsed()
        );
        assert!(!s.state.session_active());
    }

    /// A window that loads again during a start must know that a start is
    /// in work, so that it does not offer a second start.
    #[test]
    fn test_status_reports_a_start_in_work() {
        let s = Arc::new(setup(&[]));
        assert!(!session_status(&s.state).unwrap().starting);
        s.starter.slow.store(true, Ordering::SeqCst);
        let starting = {
            let s = Arc::clone(&s);
            std::thread::spawn(move || s.start(true))
        };
        while !s.state.session_active() {
            std::thread::sleep(Duration::from_millis(1));
        }
        let status = session_status(&s.state).unwrap();
        assert!(status.starting, "{status:?}");
        assert!(!status.running, "{status:?}");
        stop_session(&s.state).unwrap();
        starting.join().unwrap().unwrap_err();
        assert!(!session_status(&s.state).unwrap().starting);
        // A start that works: the status says running, not starting.
        s.starter.slow.store(false, Ordering::SeqCst);
        s.start(true).unwrap();
        let status = session_status(&s.state).unwrap();
        assert!(status.running && !status.starting, "{status:?}");
        stop_session(&s.state).unwrap();
    }

    /// The end of the program stops the session, so that the runtime
    /// withdraws a token that EDCoPilot did not read and writes its last
    /// log line. It also ends a capture.
    #[test]
    fn test_shutdown_stops_the_session_and_the_capture() {
        let s = Arc::new(setup(&[]));
        s.start(false).unwrap();
        shutdown(&s.state);
        assert!(!session_status(&s.state).unwrap().running);
        assert_eq!(s.emit.types(), ["started", "stopped"]);
        assert!(!lock(&s.starter.sessions)[0].0.load(Ordering::SeqCst));
        // A capture ends too.
        let worker = {
            let s = Arc::clone(&s);
            std::thread::spawn(move || {
                capture_binding(
                    &s.state,
                    Vec::new,
                    Duration::from_millis(1),
                    Duration::from_secs(10),
                )
            })
        };
        while !s.state.capture_active() {
            std::thread::sleep(Duration::from_millis(1));
        }
        shutdown(&s.state);
        assert!(worker.join().unwrap().is_err());
        // A second call does nothing.
        shutdown(&s.state);
    }

    /// The file changes by hand while the window is open. A save with the
    /// version of the read is refused when the hand edit changed a key that
    /// the save changes too. A hand edit of another key does not stop the
    /// save, and the save keeps it.
    #[test]
    fn test_a_hand_edit_of_an_edited_key_refuses_the_save_and_another_key_does_not() {
        use crate::dto::ValueDto;
        let s = setup(&[]);
        let read = get_config(&s.state).unwrap();
        let hand = |key: &str, value: Value| {
            let text = std::fs::read_to_string(&s.file).unwrap();
            std::fs::write(&s.file, config_edit::set_value(&text, key, &value).unwrap()).unwrap();
        };
        let threshold = || vec![edit("matching.threshold", ValueDto::Number(0.85))];

        // Another key: the save goes through and keeps the hand edit.
        hand("stt.parakeet.threads", Value::Integer(2));
        let saved = super::save_config(&s.state, threshold(), read.version.clone()).unwrap();
        assert!(saved.written, "{saved:?}");
        let config = Config::load(&s.file).unwrap();
        assert_eq!(config.stt.parakeet.threads, 2);
        assert_eq!(config.matching.threshold, 0.85);

        // The same key: the save is refused, and the hand edit stays.
        let read = get_config(&s.state).unwrap();
        hand("matching.threshold", Value::Number(0.7));
        let before = std::fs::read(&s.file).unwrap();
        let refused = super::save_config(
            &s.state,
            vec![edit("matching.threshold", ValueDto::Number(0.9))],
            read.version.clone(),
        )
        .unwrap();
        assert!(!refused.written);
        assert_eq!(refused.problems, [CONFIG_CHANGED]);
        assert_eq!(std::fs::read(&s.file).unwrap(), before);

        // A read again gives the new version, and the save goes through.
        let read = get_config(&s.state).unwrap();
        assert_eq!(read.matching.threshold, 0.7);
        let saved = super::save_config(
            &s.state,
            vec![edit("matching.threshold", ValueDto::Number(0.9))],
            read.version,
        )
        .unwrap();
        assert!(saved.written, "{saved:?}");

        // A version that the program never gave is refused.
        let refused = super::save_config(&s.state, threshold(), "not a version".into()).unwrap();
        assert_eq!(refused.problems, [CONFIG_CHANGED]);
    }

    /// The page sends at most `MAX_EDITS` edits in one save.
    #[test]
    fn test_save_config_refuses_too_many_edits() {
        use crate::dto::ValueDto;
        let s = setup(&[]);
        let before = std::fs::read(&s.file).unwrap();
        let many = |n: usize| -> Vec<EditDto> {
            (0..n)
                .map(|_| edit("matching.threshold", ValueDto::Number(0.85)))
                .collect()
        };
        assert_eq!(
            save_config(&s.state, many(MAX_EDITS + 1)).unwrap_err(),
            TOO_MANY_EDITS
        );
        assert_eq!(std::fs::read(&s.file).unwrap(), before);
        assert!(save_config(&s.state, many(MAX_EDITS)).unwrap().written);
    }

    /// The talk button edits: a hand edit of the joystick table changes a
    /// key of the edits.
    #[test]
    fn test_a_hand_edit_of_the_joystick_refuses_a_save_of_the_binding() {
        use crate::dto::ValueDto;
        let s = setup(&[]);
        let read = get_config(&s.state).unwrap();
        let text = std::fs::read_to_string(&s.file).unwrap();
        let text = config_edit::apply_edits(
            &text,
            &[
                ("push_to_talk.key".to_string(), Value::Absent),
                (
                    "push_to_talk.joystick.device".to_string(),
                    Value::Text("Pad".into()),
                ),
                (
                    "push_to_talk.joystick.button".to_string(),
                    Value::Integer(3),
                ),
            ],
        )
        .unwrap();
        std::fs::write(&s.file, text).unwrap();
        let refused = super::save_config(
            &s.state,
            vec![
                edit("push_to_talk.joystick", ValueDto::Absent),
                edit("push_to_talk.key", ValueDto::Text("RControl".into())),
            ],
            read.version,
        )
        .unwrap();
        assert_eq!(refused.problems, [CONFIG_CHANGED]);
    }

    /// The other writers of the file (the diagnostics switch, the model
    /// download) do not stop the next save of the page, also before the
    /// page reads the file again.
    #[test]
    fn test_a_write_of_the_program_does_not_stop_the_next_save_of_the_page() {
        use crate::dto::ValueDto;
        let s = setup(&[]);
        let read = get_config(&s.state).unwrap();
        assert!(
            set_diagnostics(&s.state, DiagnosticsPart::Log, true)
                .unwrap()
                .written
        );
        let saved = super::save_config(
            &s.state,
            vec![edit("matching.threshold", ValueDto::Number(0.85))],
            read.version,
        )
        .unwrap();
        assert!(saved.written, "{saved:?}");
        let config = Config::load(&s.file).unwrap();
        assert!(!config.diagnostics.log_file.as_os_str().is_empty());
    }

    /// Two windows (or two quick clicks) save different keys at the same
    /// time. No change is lost.
    #[test]
    fn test_parallel_saves_lose_no_change() {
        let s = Arc::new(setup(&[]));
        let rounds = 40;
        let worker = |key: &'static str, value: fn(i64) -> crate::dto::ValueDto| {
            let s = Arc::clone(&s);
            std::thread::spawn(move || {
                for i in 1..=rounds {
                    let result = save_config(&s.state, vec![edit(key, value(i))]).unwrap();
                    assert!(result.written, "{result:?}");
                }
            })
        };
        let a = worker("push_to_talk.pre_roll_ms", crate::dto::ValueDto::Integer);
        let b = worker(
            "push_to_talk.release_tail_ms",
            crate::dto::ValueDto::Integer,
        );
        a.join().unwrap();
        b.join().unwrap();
        let config = Config::load(&s.file).unwrap();
        assert_eq!(config.push_to_talk.pre_roll_ms, rounds as u32);
        assert_eq!(config.push_to_talk.release_tail_ms, rounds as u32);
    }

    /// A local configuration with an empty variable name works as on
    /// master. The key commands say that the name is empty.
    #[test]
    fn test_empty_variable_name_with_the_local_engine() {
        let s = setup(&[(
            "stt.openai_compatible.api_key_env",
            Value::Text(String::new()),
        )]);
        let found = check(&s.state).unwrap();
        assert!(
            !found.problems.iter().any(|p| p.contains("api_key_env")),
            "{found:?}"
        );
        let config = get_config(&s.state).unwrap();
        assert!(!config.api_key_set);
        assert_eq!(config.stt.openai_compatible.api_key_env, "");
        s.start(true).unwrap();
        stop_session(&s.state).unwrap();
        let error = set_key(&s.state, SECRET).unwrap_err();
        assert!(error.contains("is empty"), "{error}");
        assert!(clear_api_key(&s.state).unwrap_err().contains("is empty"));
    }

    #[test]
    fn test_set_api_key_needs_a_configuration() {
        let s = setup(&[]);
        std::fs::remove_file(&s.file).unwrap();
        let error = set_key(&s.state, SECRET).unwrap_err();
        assert!(error.contains("API key variable"), "{error}");
        assert_no_secret(&error);
        assert!(!s.keys.has("OPENAI_API_KEY"));
    }

    mod phrases {
        use super::*;
        use crate::dto::ValueDto;
        use voicebridge::list::{line, ListData};

        /// An invented synonym table for the fixture commands.
        pub(super) const SYNONYMS: &str = "# Invented synonyms of the harbour test table.\n\
            [synonyms]\n\
            \"make for\" = [\"sail to\", \"sail for\"]\n\
            \"make\" = [\"sail for\", \"sail to\"]\n";

        /// A setup with the invented EDCoPilot folder of the fixture (the
        /// program reads its profile), the shipped
        /// sound-alike and filler tables, an invented synonym table, and an
        /// empty alias folder. The EDCoPilot folder exists, so that a write
        /// of the request file would work.
        pub(super) fn data_setup() -> Setup {
            let core = Path::new(env!("CARGO_MANIFEST_DIR")).join("../core");
            let core = core.canonicalize().unwrap();
            let path = |p: &str| Value::Text(core.join(p).display().to_string());
            let s = setup(&[
                ("data.sound_alikes", path("data/sound_alikes.toml")),
                ("data.fillers", path("data/fillers.toml")),
            ]);
            let own = s._dir.path().join("data");
            std::fs::create_dir_all(own.join("aliases")).unwrap();
            std::fs::write(own.join("synonyms.toml"), SYNONYMS).unwrap();
            let edits = vec![
                (
                    "data.aliases_dir".to_string(),
                    Value::Text(own.join("aliases").display().to_string()),
                ),
                (
                    "data.synonyms".to_string(),
                    Value::Text(own.join("synonyms.toml").display().to_string()),
                ),
            ];
            let text = std::fs::read_to_string(&s.file).unwrap();
            std::fs::write(&s.file, config_edit::apply_edits(&text, &edits).unwrap()).unwrap();
            edcopilot_voicebridge_core::profile::fixture::write_fixture_edcopilot(
                &s._dir.path().join("ed"),
            );
            s
        }

        fn request_file(s: &Setup) -> PathBuf {
            Config::load(&s.file).unwrap().edcopilot.request_file
        }

        /// The whole list for one filter: all pages together.
        fn full(state: &AppState, filter: &str, all: bool) -> Result<CommandListDto, String> {
            let mut list = list_commands(state, filter, all, 0, MAX_PAGE_ROWS)?;
            while list.rows.len() < list.total {
                let next = list_commands(state, filter, all, list.rows.len(), MAX_PAGE_ROWS)?;
                assert!(!next.rows.is_empty());
                list.rows.extend(next.rows);
            }
            Ok(list)
        }

        /// A setup in always_listening mode with the wake words of the
        /// example: "copilot" and "co pilot".
        fn listening_setup() -> Setup {
            let s = data_setup();
            let result = save_config(
                &s.state,
                vec![edit(
                    "microphone.mode",
                    ValueDto::Text("always_listening".into()),
                )],
            )
            .unwrap();
            assert!(result.written, "{result:?}");
            s
        }

        fn kind(s: &Setup, phrase: &str) -> String {
            test_phrase(&s.state, phrase).unwrap().outcome.kind
        }

        /// (a) No wake word: the outcome of the live session, never a match.
        #[test]
        fn test_phrase_in_always_listening_without_a_wake_word_gives_no_wake_word() {
            let s = listening_setup();
            for phrase in ["help tide log", "sail for home port", "belays"] {
                let result = test_phrase(&s.state, phrase).unwrap();
                assert_eq!(result.outcome.kind, "no_wake_word", "{phrase}");
                assert_eq!(result.line, format!("no wake word in \"{phrase}\""));
                assert_eq!(result.wake_words, ["copilot", "co pilot"]);
            }
        }

        /// (b) The wake word is removed before the one-word rule.
        #[test]
        fn test_phrase_wake_word_and_one_misspelled_word_is_rejected_as_in_push_to_talk() {
            let s = listening_setup();
            let heard = test_phrase(&s.state, "copilot belays").unwrap();
            assert_eq!(heard.outcome.kind, "rejected");
            assert_eq!(
                heard.line,
                "rejected \"belays\" -> Belay (0.83): \
                 a phrase of one word needs an exact match"
            );
            let push = data_setup();
            assert_eq!(test_phrase(&push.state, "belays").unwrap().line, heard.line);
        }

        /// (c) The wake word and a command give the dry-run match.
        #[test]
        fn test_phrase_wake_word_and_a_command_gives_the_dry_run_match() {
            let s = listening_setup();
            let result = test_phrase(&s.state, "Copilot, sail for home port").unwrap();
            assert_eq!(result.outcome.kind, "dry_run");
            assert_eq!(
                result.line,
                "dry run \"sail for home port\" -> SailHomePort (1.00)"
            );
            // An empty wake word list: the text is matched as it is, as live.
            save_config(
                &s.state,
                vec![edit(
                    "always_listening.wake_words",
                    ValueDto::TextList(vec![]),
                )],
            )
            .unwrap();
            let result = test_phrase(&s.state, "sail for home port").unwrap();
            assert_eq!(result.outcome.kind, "dry_run");
            assert!(result.wake_words.is_empty());
        }

        /// A phrase of a command that the program cannot send: no match,
        /// and the result says why.
        #[test]
        fn test_phrase_of_a_command_without_a_token_says_why() {
            let s = data_setup();
            let result = test_phrase(&s.state, "lock lantern").unwrap();
            assert_eq!(result.outcome.kind, "no_match");
            assert_eq!(result.outcome.blocked.as_deref(), Some("no_token"));
            assert_eq!(
                result.outcome.reason.as_deref(),
                Some("blocked: the text is a phrase of a command that the program cannot send")
            );
            assert_eq!(
                result.line,
                "no match for \"lock lantern\": blocked: the text is a phrase of a command \
                 that the program cannot send"
            );
            // A text without a candidate keeps the plain line.
            let plain = test_phrase(&s.state, "mmm gibberish").unwrap();
            assert_eq!(plain.outcome.blocked, None);
            assert_eq!(plain.outcome.reason, None);
            assert_eq!(plain.line, "no match for \"mmm gibberish\"");
        }

        /// (d) Push-to-talk ignores the wake words, as live.
        #[test]
        fn test_phrase_in_push_to_talk_needs_no_wake_word() {
            let s = data_setup();
            let result = test_phrase(&s.state, "sail for home port").unwrap();
            assert_eq!(result.outcome.kind, "dry_run");
            assert!(result.wake_words.is_empty());
            assert_eq!(kind(&s, "copilot sail for home port"), "no_match");
        }

        /// The phrase test never gives an outcome of a send.
        #[test]
        fn test_phrase_never_gives_the_outcome_of_a_send() {
            let push = data_setup();
            let listening = listening_setup();
            let phrases = [
                "help tide log",
                "sail for home port",
                "belays",
                "mmm gibberish",
                "copilot sail for home port",
                "copilot belays",
                "copilot",
                "",
                "   ",
            ];
            for s in [&push, &listening] {
                for phrase in phrases {
                    let kind = kind(s, phrase);
                    assert!(
                        ["dry_run", "rejected", "no_match", "no_wake_word"]
                            .contains(&kind.as_str()),
                        "{phrase:?}: {kind}"
                    );
                }
            }
            assert!(!request_file(&push).exists());
            assert!(!request_file(&listening).exists());
        }

        #[test]
        fn test_long_phrase_and_long_filter_are_refused() {
            let s = data_setup();
            // Characters, not bytes: "ä" has two bytes.
            let phrase = "ä".repeat(MAX_PHRASE_CHARS);
            assert_eq!(kind(&s, &phrase), "no_match");
            let error = test_phrase(&s.state, &format!("{phrase}x")).unwrap_err();
            assert_eq!(error, PHRASE_TOO_LONG);
            let filter = "ä".repeat(MAX_FILTER_CHARS);
            assert!(list_commands(&s.state, &filter, false, 0, 10).is_ok());
            let error = list_commands(&s.state, &format!("{filter}x"), false, 0, 10).unwrap_err();
            assert_eq!(error, FILTER_TOO_LONG);
        }

        #[test]
        fn test_list_commands_gives_the_id_of_the_catalog() {
            let s = data_setup();
            let first = list_commands(&s.state, "", true, 0, 200).unwrap();
            let second = list_commands(&s.state, "", true, 200, 200).unwrap();
            assert_eq!(first.catalog_id, second.catalog_id, "no rebuild, same id");
            let vocab = Config::load(&s.file).unwrap().edcopilot.dynamic_vars_dir;
            std::fs::write(vocab.join("EDCoPilotSystem1.txt"), "Sol\n").unwrap();
            let third = list_commands(&s.state, "", true, 400, 200).unwrap();
            assert_ne!(
                first.catalog_id, third.catalog_id,
                "a rebuild gives a new id"
            );
        }

        #[test]
        fn test_list_commands_gives_one_page_and_the_total() {
            let s = data_setup();
            // Invented alias phrases of a working command make the list of
            // the fixture longer than the page limit.
            let aliases = Config::load(&s.file).unwrap().data.aliases_dir;
            let phrases: Vec<String> = (0..MAX_PAGE_ROWS)
                .map(|i| format!("\"belay alias {i}\""))
                .collect();
            std::fs::write(
                aliases.join("xx.toml"),
                format!("'Belay' = [{}]\n", phrases.join(", ")),
            )
            .unwrap();
            let whole = full(&s.state, "", true).unwrap();
            assert!(whole.total > MAX_PAGE_ROWS, "{}", whole.total);
            let first = list_commands(&s.state, "", true, 0, 200).unwrap();
            assert_eq!(first.rows.len(), 200);
            assert_eq!(first.offset, 0);
            assert_eq!(first.total, whole.total);
            assert_eq!(first.summary, whole.summary);
            let second = list_commands(&s.state, "", true, 200, 200).unwrap();
            assert_eq!(second.offset, 200);
            assert_eq!(second.rows[..], whole.rows[200..400]);
            // A page after the end is empty. A limit above the maximum is cut.
            let after = list_commands(&s.state, "", true, whole.total + 5, 200).unwrap();
            assert!(after.rows.is_empty());
            let big = list_commands(&s.state, "", true, 0, usize::MAX).unwrap();
            assert_eq!(big.rows.len(), MAX_PAGE_ROWS.min(whole.total));
        }

        /// The problems of the command data come with the list. A problem
        /// of the model folder does not.
        #[test]
        fn test_list_commands_gives_the_problems_of_the_command_data() {
            let s = data_setup();
            let model = Config::load(&s.file).unwrap().stt.parakeet.model_dir;
            std::fs::remove_dir(&model).unwrap();
            let vocab = Config::load(&s.file).unwrap().edcopilot.dynamic_vars_dir;
            std::fs::remove_dir_all(&vocab).unwrap();
            let list = full(&s.state, "", false).unwrap();
            assert!(
                list.problems
                    .iter()
                    .any(|p| p.starts_with("vocabulary directory not found")),
                "{:?}",
                list.problems
            );
            assert!(
                !list.problems.iter().any(|p| p.contains("model_dir")),
                "{:?}",
                list.problems
            );
            // The check of the window has the model problem.
            assert!(check(&s.state)
                .unwrap()
                .problems
                .iter()
                .any(|p| p.contains("model_dir")));
            // A bad threshold is a problem of the command data.
            let text = std::fs::read_to_string(&s.file).unwrap();
            let bad =
                config_edit::set_value(&text, "matching.threshold", &Value::Number(7.0)).unwrap();
            std::fs::write(&s.file, bad).unwrap();
            let list = full(&s.state, "", false).unwrap();
            assert!(
                list.problems[0].starts_with("matching.threshold"),
                "{:?}",
                list.problems
            );
        }

        /// A save right after the load of the configuration: the next call
        /// must see the new file. No catalog of the old content stays.
        #[test]
        fn test_a_save_during_a_catalog_call_never_leaves_an_old_catalog() {
            let s = data_setup();
            let file = s.file.clone();
            *lock(&s.state.catalog_hook) = Some(Box::new(move |_: &AppState| {
                let text = std::fs::read_to_string(&file).unwrap();
                let new = config_edit::set_value(
                    &text,
                    "matching.exact_single_word",
                    &Value::Flag(false),
                )
                .unwrap();
                std::fs::write(&file, new).unwrap();
            }));
            // This call loaded the old file: the one-word rule is on.
            assert_eq!(kind(&s, "belays"), "rejected");
            assert!(lock(&s.state.catalog).is_none(), "an old catalog was kept");
            // The next call sees the saved file.
            assert_eq!(kind(&s, "belays"), "dry_run");
            assert_eq!(kind(&s, "belays"), "dry_run");
        }

        /// A build of an older call does not replace the catalog of a newer
        /// call.
        #[test]
        fn test_an_older_build_does_not_replace_a_newer_catalog() {
            let s = data_setup();
            let vocab = Config::load(&s.file).unwrap().edcopilot.dynamic_vars_dir;
            let newer: Arc<Mutex<Option<Arc<crate::catalog::Catalog>>>> = Arc::default();
            let out = Arc::clone(&newer);
            *lock(&s.state.catalog_hook) = Some(Box::new(move |state: &AppState| {
                // A data file changes, and a newer call builds and keeps its catalog.
                std::fs::write(vocab.join("EDCoPilotSystem1.txt"), "Sol\n").unwrap();
                *lock(&out) = Some(crate::catalog::catalog(state).unwrap());
            }));
            let older = crate::catalog::catalog(&s.state).unwrap();
            let newer = lock(&newer).take().expect("the hook ran");
            assert!(newer.ticket > older.ticket);
            let kept = lock(&s.state.catalog).clone().unwrap();
            assert!(
                Arc::ptr_eq(&kept, &newer),
                "the older build replaced the newer one"
            );
        }

        /// The console line of one row: `voicebridge commands` prints it.
        fn console_line(row: &crate::dto::CommandRowDto) -> String {
            let mut text = format!("{}  ->  {}", row.phrase, row.token);
            if row.alias {
                text.push_str("  (alias)");
            }
            if let Some(reason) = &row.reason {
                text.push_str(&format!("  [{reason}]"));
            }
            text
        }

        #[test]
        fn test_list_commands_filter_gives_the_rows_of_the_console_list() {
            let s = data_setup();
            let filter = "sail for home port";
            let list = full(&s.state, filter, false).unwrap();
            // The line that `voicebridge commands "sail for home port"` prints.
            let lines: Vec<String> = list.rows.iter().map(console_line).collect();
            assert_eq!(
                lines,
                ["[row home; sail for home port; take [me;us] back]  ->  SailHomePort"]
            );
            assert_eq!(list.rows[0].category, "Harbour traffic");
            assert!(list.rows[0].works);
            // Each filter and the switch give the lines of the console.
            let data = ListData::load(&Config::load(&s.file).unwrap()).unwrap();
            for (filter, all) in [("", false), ("", true), ("tide log", true), ("cork", false)] {
                let list = full(&s.state, filter, all).unwrap();
                let expected: Vec<String> = data.shown(filter, all).into_iter().map(line).collect();
                let lines: Vec<String> = list.rows.iter().map(console_line).collect();
                assert_eq!(lines, expected, "{filter:?} {all}");
                assert_eq!(list.summary, data.summary(expected.len()));
            }
            let all = full(&s.state, "", true).unwrap();
            let broken = all
                .rows
                .iter()
                .find(|r| !r.works)
                .expect("a row that does not work");
            assert!(broken
                .reason
                .as_deref()
                .unwrap()
                .starts_with("does not work: "));
            assert!(all
                .rows
                .iter()
                .filter(|r| r.works)
                .all(|r| r.reason.is_none()));
            // The composite group of the console.
            let cork = full(&s.state, "cork", false).unwrap();
            assert!(!cork.composite.is_empty());
            assert!(cork.composite[0].heading.starts_with("Composite group "));
            assert!(cork.composite[0].lines.iter().any(|l| l.contains("cork")));
        }

        #[test]
        fn test_test_phrase_never_writes_the_request_file() {
            let s = data_setup();
            let request = request_file(&s);
            assert!(request.parent().unwrap().is_dir());
            let cases = [
                (
                    "help tide log",
                    "dry_run",
                    "dry run \"help tide log\" -> Guide:Panel:TideLog (1.00)",
                ),
                (
                    "belays",
                    "rejected",
                    "rejected \"belays\" -> Belay (0.83): \
                     a phrase of one word needs an exact match",
                ),
                (
                    "mmm gibberish",
                    "no_match",
                    "no match for \"mmm gibberish\"",
                ),
            ];
            for (phrase, kind, console) in cases {
                let result = test_phrase(&s.state, phrase).unwrap();
                assert_eq!(result.outcome.kind, kind, "{phrase}");
                assert_eq!(result.line, console, "{phrase}");
                assert!(!request.exists(), "{phrase}: the request file was written");
            }
            // No new file in the EDCoPilot folder.
            let mut left: Vec<_> = std::fs::read_dir(request.parent().unwrap())
                .unwrap()
                .map(|e| e.unwrap().file_name())
                .collect();
            left.sort();
            assert_eq!(
                left,
                ["Dynamic var files", "EDCoPilot-Profile.vap", "working"]
            );
        }

        #[test]
        fn test_test_phrase_works_while_a_session_runs() {
            let s = Arc::new(data_setup());
            s.start(false).unwrap();
            let events = s.emit.types().len();
            let result = test_phrase(&s.state, "help tide log").unwrap();
            assert_eq!(result.outcome.kind, "dry_run");
            assert!(session_status(&s.state).unwrap().running);
            assert_eq!(
                s.emit.types().len(),
                events,
                "the phrase test sent an event"
            );
            // A start or a stop holds the session lock for a long time (the
            // model load). The phrase test does not wait for it.
            let held = lock(&s.state.session);
            let (tx, rx) = std::sync::mpsc::channel();
            let worker = {
                let s = Arc::clone(&s);
                std::thread::spawn(move || {
                    let _ = tx.send(test_phrase(&s.state, "help tide log"));
                })
            };
            let answer = rx.recv_timeout(Duration::from_secs(20));
            drop(held);
            worker.join().unwrap();
            assert_eq!(
                answer
                    .expect("the phrase test waited")
                    .unwrap()
                    .outcome
                    .kind,
                "dry_run"
            );
            assert!(!request_file(&s).exists());
            stop_session(&s.state).unwrap();
        }

        #[test]
        fn test_the_pipeline_is_kept_until_the_configuration_or_the_data_changes() {
            let s = data_setup();
            let first = crate::catalog::catalog(&s.state).unwrap();
            let again = crate::catalog::catalog(&s.state).unwrap();
            assert!(Arc::ptr_eq(&first, &again), "built again without a change");
            assert_eq!(
                test_phrase(&s.state, "belays").unwrap().outcome.kind,
                "rejected"
            );
            // A changed setting: the phrase test uses it at once.
            let result = save_config(
                &s.state,
                vec![edit("matching.exact_single_word", ValueDto::Flag(false))],
            )
            .unwrap();
            assert!(result.written, "{result:?}");
            assert_eq!(
                test_phrase(&s.state, "belays").unwrap().outcome.kind,
                "dry_run"
            );
            let second = crate::catalog::catalog(&s.state).unwrap();
            assert!(!Arc::ptr_eq(&first, &second));
            // A new vocabulary file.
            let vocab = Config::load(&s.file).unwrap().edcopilot.dynamic_vars_dir;
            std::fs::write(vocab.join("EDCoPilotSystem1.txt"), "Sol\n").unwrap();
            let third = crate::catalog::catalog(&s.state).unwrap();
            assert!(
                !Arc::ptr_eq(&second, &third),
                "a vocabulary change was not seen"
            );
            assert!(Arc::ptr_eq(
                &third,
                &crate::catalog::catalog(&s.state).unwrap()
            ));
        }

        /// Measures the build of the catalog and the calls after it with a
        /// real data set. Set VOICEBRIDGE_UI_CONFIG to a configuration file.
        /// It only reads the files.
        #[test]
        #[ignore]
        fn the_catalog_times_with_a_real_data_set() {
            let file = std::env::var("VOICEBRIDGE_UI_CONFIG").expect("VOICEBRIDGE_UI_CONFIG");
            let state = AppState::new(
                PathBuf::from(file),
                Arc::new(MemoryStore::new()),
                Arc::new(FakeStarter::default()),
            );
            let time = |name: &str, f: &dyn Fn() -> usize| {
                let begin = std::time::Instant::now();
                let n = f();
                eprintln!("{name}: {} ms ({n})", begin.elapsed().as_millis());
            };
            time("first list_commands (build)", &|| {
                full(&state, "", false).unwrap().rows.len()
            });
            time("list_commands all", &|| {
                full(&state, "", true).unwrap().rows.len()
            });
            time("list_commands 'tide'", &|| {
                full(&state, "tide", false).unwrap().rows.len()
            });
            time("test_phrase", &|| {
                test_phrase(&state, "sail for home port")
                    .unwrap()
                    .line
                    .len()
            });
            time("stamp only", &|| {
                let config = Config::load(state.config_path()).unwrap();
                usize::from(
                    crate::catalog::stamp(state.config_path(), &config)
                        == crate::catalog::stamp(state.config_path(), &config),
                )
            });
            time("json of all rows", &|| {
                serde_json::to_string(&full(&state, "", true).unwrap())
                    .unwrap()
                    .len()
            });
        }

        #[test]
        fn test_a_configuration_problem_gives_the_text_of_the_console() {
            let s = data_setup();
            let text = std::fs::read_to_string(&s.file).unwrap();
            let bad =
                config_edit::set_value(&text, "matching.threshold", &Value::Number(7.0)).unwrap();
            std::fs::write(&s.file, bad).unwrap();
            let error = test_phrase(&s.state, "help tide log").unwrap_err();
            let console = Config::load(&s.file).unwrap().matching_problem().unwrap();
            assert_eq!(error, console);
            // The list does not need the threshold, as on the console.
            assert!(full(&s.state, "", false).is_ok());
            // No profile: both give the load error.
            std::fs::write(&s.file, &text).unwrap();
            let profile = Config::load(&s.file).unwrap().edcopilot.profile;
            std::fs::remove_file(&profile).unwrap();
            let name = "EDCoPilot-Profile.vap";
            assert!(full(&s.state, "", false).unwrap_err().contains(name));
            assert!(test_phrase(&s.state, "x").unwrap_err().contains(name));
            // A file that does not load.
            std::fs::write(&s.file, "[edcopilot\n").unwrap();
            assert!(full(&s.state, "", false)
                .unwrap_err()
                .contains("voicebridge.toml"));
        }
    }

    mod profile {
        use super::phrases::data_setup;
        use super::*;
        use edcopilot_voicebridge_core::profile::fixture::fixture_builder;
        use voicebridge::commands_source::{left_out_line, profile_lines};
        use voicebridge::config::{load_commands, EDCOPILOT_DIR_EMPTY, OLD_MAP_KEYS};

        fn profile_path(s: &Setup) -> PathBuf {
            Config::load(&s.file).unwrap().edcopilot.profile
        }

        /// An EDCoPilot update writes a new profile: the next call of the
        /// Commands screen and of the phrase test builds a new catalog.
        #[test]
        fn test_a_change_of_the_profile_makes_a_new_catalog() {
            let s = data_setup();
            let first = crate::catalog::catalog(&s.state).unwrap();
            assert!(Arc::ptr_eq(
                &first,
                &crate::catalog::catalog(&s.state).unwrap()
            ));
            assert_eq!(kind_of(&s, "ring the quay bell"), "no_match");
            let mut builder = fixture_builder();
            builder.fixed("ring the quay bell", "QuayBell");
            std::fs::write(profile_path(&s), builder.build()).unwrap();
            let second = crate::catalog::catalog(&s.state).unwrap();
            assert!(
                !Arc::ptr_eq(&first, &second),
                "the profile change was not seen"
            );
            assert_eq!(kind_of(&s, "ring the quay bell"), "dry_run");
            let list = list_commands(&s.state, "quay bell", false, 0, 10).unwrap();
            assert_eq!(list.total, 1, "{list:?}");
            assert_eq!(list.rows[0].token, "QuayBell");
        }

        fn kind_of(s: &Setup, phrase: &str) -> String {
            test_phrase(&s.state, phrase).unwrap().outcome.kind
        }

        #[test]
        fn test_check_gives_the_profile_report_to_the_page() {
            let s = data_setup();
            let config = Config::load(&s.file).unwrap();
            let built = load_commands(&config).unwrap().profile;
            let dto = check(&s.state).unwrap();
            let profile = dto.profile.clone().expect("a profile report");
            assert_eq!(profile.records, built.records);
            assert_eq!(profile.left_out, built.left_out.len());
            assert!(profile.left_out > 0);
            // The lines of `check`, in its order. The saved report of the
            // last session of this computer can add the change line.
            let expected = profile_lines(&built, None, false);
            assert_eq!(profile.lines[..expected.len()], expected[..]);
            let changed = profile.lines.len() - expected.len();
            assert!(changed <= 1, "{profile:?}");
            if changed == 1 {
                assert!(profile.lines[expected.len()].starts_with("the EDCoPilot profile changed"));
            }
            assert!(profile.lines[0].starts_with("commands from the EDCoPilot profile: "));
            assert!(expected.len() > 1);
            assert!(expected[1..].iter().all(|l| l.starts_with("left out, ")));
            assert!(dto.edcopilot_problems.is_empty(), "{dto:?}");
        }

        #[test]
        fn test_a_profile_that_does_not_load_is_a_problem_at_the_edcopilot_folder() {
            let s = data_setup();
            let profile = profile_path(&s);
            std::fs::remove_file(&profile).unwrap();
            let dto = check(&s.state).unwrap();
            assert!(dto.profile.is_none());
            assert!(dto.report.is_none());
            let expected = format!(
                "cannot read the EDCoPilot profile {}: the file does not exist, set the \
                 EDCoPilot folder, or start EDCoPilot one time to make the file",
                profile.display()
            );
            assert_eq!(dto.edcopilot_problems, std::slice::from_ref(&expected));
            assert!(dto.problems.contains(&expected), "{dto:?}");
            // Not a profile.
            std::fs::write(&profile, "not a profile").unwrap();
            let dto = check(&s.state).unwrap();
            assert_eq!(dto.edcopilot_problems.len(), 1, "{dto:?}");
            assert!(dto.edcopilot_problems[0].starts_with(&format!(
                "the file {} is not an EDCoPilot profile that the program understands: ",
                profile.display()
            )));
            // No folder: the text of the folder, one time.
            let text = std::fs::read_to_string(&s.file).unwrap();
            let none = config_edit::set_value(&text, "edcopilot.dir", &Value::Text(String::new()))
                .unwrap();
            std::fs::write(&s.file, none).unwrap();
            let dto = check(&s.state).unwrap();
            assert_eq!(dto.edcopilot_problems, [EDCOPILOT_DIR_EMPTY]);
            // A file that does not load: no clean state anywhere.
            std::fs::write(&s.file, "[edcopilot\n").unwrap();
            let dto = check(&s.state).unwrap();
            assert_eq!(dto.problems.len(), 1);
            assert!(dto.profile.is_none());
        }

        #[test]
        fn test_get_config_gives_the_profile_in_use() {
            let s = data_setup();
            let config = get_config(&s.state).unwrap();
            assert_eq!(config.edcopilot.profile, "");
            assert_eq!(
                config.edcopilot.profile_in_use,
                profile_path(&s).display().to_string()
            );
            let text = std::fs::read_to_string(&s.file).unwrap();
            let own = s._dir.path().join("own.vap");
            let set = config_edit::set_value(
                &text,
                "edcopilot.profile",
                &Value::Text(own.display().to_string()),
            )
            .unwrap();
            std::fs::write(&s.file, set).unwrap();
            let config = get_config(&s.state).unwrap();
            assert_eq!(config.edcopilot.profile, own.display().to_string());
            assert_eq!(config.edcopilot.profile_in_use, own.display().to_string());
        }

        /// The full list of the left-out commands, in pages, with the line
        /// of `check --left-out` for each.
        #[test]
        fn test_list_left_out_gives_every_left_out_command_in_pages() {
            let s = data_setup();
            let config = Config::load(&s.file).unwrap();
            let built = load_commands(&config).unwrap().profile;
            let first = list_left_out(&s.state, "", 0, 3).unwrap();
            assert_eq!(first.total, built.left_out.len());
            assert_eq!(first.rows.len(), 3.min(first.total));
            let mut rows = first.rows.clone();
            while rows.len() < first.total {
                let page = list_left_out(&s.state, "", rows.len(), 3).unwrap();
                assert_eq!(page.catalog_id, first.catalog_id);
                assert!(!page.rows.is_empty());
                rows.extend(page.rows);
            }
            let lines: Vec<String> = rows.iter().map(|r| r.line.clone()).collect();
            let expected: Vec<String> = built.left_out.iter().map(left_out_line).collect();
            assert_eq!(lines, expected);
            assert_eq!(rows[0].class, built.left_out[0].class.as_str());
            // A filter: each word in the line.
            let word = built.left_out[0].class.as_str();
            let filtered = list_left_out(&s.state, word, 0, MAX_PAGE_ROWS).unwrap();
            assert!(filtered.total > 0);
            assert!(filtered.rows.iter().all(|r| r.line.contains(word)));
            assert!(list_left_out(&s.state, &"x".repeat(MAX_FILTER_CHARS + 1), 0, 1).is_err());
            // No profile: the load error, as the Commands screen.
            std::fs::remove_file(profile_path(&s)).unwrap();
            let error = list_left_out(&s.state, "", 0, 3).unwrap_err();
            assert!(error.contains("EDCoPilot-Profile.vap"), "{error}");
        }

        /// The warnings of the load go to the Commands screen with the
        /// other warnings of the command data. No warning is dropped.
        #[test]
        fn test_the_command_list_shows_the_warnings_of_the_load() {
            let s = data_setup();
            let config = Config::load(&s.file).unwrap();
            let (_, report) = voicebridge::config::build_pipeline(&config).unwrap();
            let slot = report
                .checked_slots
                .iter()
                .next()
                .cloned()
                .expect("the fixture has a slot");
            let name = format!("EDCoPilot{slot}.txt");
            let bad = if slot.starts_with("System") {
                "[a]"
            } else {
                "a;b"
            };
            std::fs::write(config.edcopilot.dynamic_vars_dir.join(&name), bad).unwrap();
            let list = list_commands(&s.state, "", false, 0, 1).unwrap();
            assert!(
                list.warnings.iter().any(|w| w.contains(&name)),
                "{:?}",
                list.warnings
            );
            // The working phrase of the fixture that a value command also
            // has: one warning on the Commands screen and in the check.
            let shadow = "the phrase \"set lamp colour\" of the command \
                          \"[paint the lamp; set lamp colour]\" is also a phrase of a command \
                          that the program cannot send, the program does not send a command \
                          for it";
            assert_eq!(
                list.warnings.iter().filter(|w| *w == shadow).count(),
                1,
                "{:?}",
                list.warnings
            );
            let dto = check(&s.state).unwrap();
            assert_eq!(dto.warnings.iter().filter(|w| *w == shadow).count(), 1);
            let left_before = list_left_out(&s.state, "", 0, MAX_PAGE_ROWS).unwrap();
            // The keys of an older version: one warning, and the same
            // commands and left-out list as before.
            let text = std::fs::read_to_string(&s.file).unwrap();
            let at = text.find("[data]").expect("the setup has a [data] table") + "[data]".len();
            let nl = if text[at..].starts_with("\r\n") {
                "\r\n"
            } else {
                "\n"
            };
            let keys = format!(
                "command_map = 'C:\\Old\\resources\\data\\command_map_v3.json'{nl}\
                 profile_vars = ''{nl}"
            );
            let mut old = text.clone();
            old.insert_str(at + nl.len(), &keys);
            std::fs::write(&s.file, old).unwrap();
            let with_keys = list_commands(&s.state, "", false, 0, 1).unwrap();
            assert_eq!(
                with_keys
                    .warnings
                    .iter()
                    .filter(|w| *w == OLD_MAP_KEYS)
                    .count(),
                1,
                "{:?}",
                with_keys.warnings
            );
            assert_eq!(with_keys.total, list.total);
            let left = list_left_out(&s.state, "", 0, MAX_PAGE_ROWS).unwrap();
            assert_eq!(left.rows, left_before.rows);
            assert_eq!(left.total, left_before.total);
            // The check of the window shows it one time too.
            let dto = check(&s.state).unwrap();
            assert_eq!(
                dto.warnings.iter().filter(|w| *w == OLD_MAP_KEYS).count(),
                1,
                "{:?}",
                dto.warnings
            );
        }
    }

    mod words {
        use super::phrases::data_setup;
        use super::*;
        use crate::dto::{RowDto, TableKindDto, ValueDto};

        /// The data setup with COPIES of the shipped sound-alike, filler and
        /// hotword tables, the invented synonym table, and an invented
        /// German alias file in its temporary folder. The shipped tables
        /// are never written.
        fn words_setup() -> Setup {
            let s = data_setup();
            let core = Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../core")
                .canonicalize()
                .unwrap();
            let tables = s._dir.path().join("tables");
            std::fs::create_dir_all(tables.join("aliases")).unwrap();
            for name in ["sound_alikes.toml", "fillers.toml", "hotwords.txt"] {
                std::fs::copy(core.join("data").join(name), tables.join(name)).unwrap();
            }
            std::fs::write(tables.join("synonyms.toml"), super::phrases::SYNONYMS).unwrap();
            std::fs::write(
                tables.join("aliases/de.toml"),
                "'OpenTideLogPanel' = [\"gezeitenbuch [öffnen; zeigen;]\"]\n",
            )
            .unwrap();
            let path = |p: &str| Value::Text(tables.join(p).display().to_string());
            let edits = [
                ("data.aliases_dir", path("aliases")),
                ("data.sound_alikes", path("sound_alikes.toml")),
                ("data.synonyms", path("synonyms.toml")),
                ("data.fillers", path("fillers.toml")),
                ("stt.parakeet.hotwords_file", path("hotwords.txt")),
            ];
            let text = std::fs::read_to_string(&s.file).unwrap();
            let edits: Vec<(String, Value)> =
                edits.into_iter().map(|(k, v)| (k.to_string(), v)).collect();
            std::fs::write(&s.file, config_edit::apply_edits(&text, &edits).unwrap()).unwrap();
            let config = Config::load(&s.file).unwrap();
            assert!(config.data.synonyms.starts_with(s._dir.path()));
            s
        }

        fn row(spoken: &str, meant: &[&str]) -> RowDto {
            RowDto {
                spoken: spoken.to_string(),
                meant: meant.iter().map(|m| m.to_string()).collect(),
            }
        }

        /// Reads the synonyms and adds one row.
        fn synonyms_with(s: &Setup, new: RowDto) -> (String, Vec<RowDto>) {
            let table = read_table(&s.state, TableKindDto::Synonyms, None).unwrap();
            let mut rows = table.rows;
            rows.push(new);
            (table.version, rows)
        }

        #[test]
        fn test_save_table_lets_the_phrase_test_see_the_change() {
            let s = words_setup();
            let phrase = "row me for home port";
            // The catalog of the old tables is in the cache now.
            assert_ne!(
                test_phrase(&s.state, phrase).unwrap().outcome.kind,
                "dry_run"
            );
            let (version, rows) = synonyms_with(&s, row("row me for", &["sail for"]));
            let check = check_table(&s.state, TableKindDto::Synonyms, None, rows.clone()).unwrap();
            assert!(check.problems.is_empty(), "{check:?}");
            let saved = save_table(
                &s.state,
                TableKindDto::Synonyms,
                None,
                rows,
                check.warnings,
                version,
            )
            .unwrap();
            assert!(saved.written, "{saved:?}");
            assert!(!saved.restart_needed);
            let result = test_phrase(&s.state, phrase).unwrap();
            assert_eq!(
                result.line,
                "dry run \"row me for home port\" -> SailHomePort (1.00) (synonym)"
            );
        }

        #[test]
        fn test_save_table_while_a_session_runs_reports_restart_needed() {
            let s = words_setup();
            s.start(true).unwrap();
            let (version, rows) = synonyms_with(&s, row("row me for", &["sail for"]));
            let saved = save_table(
                &s.state,
                TableKindDto::Synonyms,
                None,
                rows,
                vec![],
                version,
            )
            .unwrap();
            assert!(saved.written && saved.restart_needed, "{saved:?}");
            // A refused save needs no restart.
            let (version, rows) = synonyms_with(&s, row("bad.", &["sail for"]));
            let saved = save_table(
                &s.state,
                TableKindDto::Synonyms,
                None,
                rows,
                vec![],
                version,
            )
            .unwrap();
            assert!(!saved.written && !saved.restart_needed, "{saved:?}");
            stop_session(&s.state).unwrap();
        }

        /// Two saves of the same read: the second save does not remove the
        /// row of the first save.
        #[test]
        fn test_two_quick_saves_of_one_read_write_one_time() {
            let s = words_setup();
            let (version, first) = synonyms_with(&s, row("row me for", &["sail for"]));
            let (_, second) = synonyms_with(&s, row("row me to", &["sail to"]));
            let results: Vec<TableSaveDto> = std::thread::scope(|scope| {
                let a = scope.spawn(|| {
                    save_table(
                        &s.state,
                        TableKindDto::Synonyms,
                        None,
                        first,
                        vec![],
                        version.clone(),
                    )
                });
                let b = scope.spawn(|| {
                    save_table(
                        &s.state,
                        TableKindDto::Synonyms,
                        None,
                        second,
                        vec![],
                        version.clone(),
                    )
                });
                vec![a.join().unwrap().unwrap(), b.join().unwrap().unwrap()]
            });
            let written = results.iter().filter(|r| r.written).count();
            assert_eq!(written, 1, "{results:?}");
            let refused = results.iter().find(|r| !r.written).unwrap();
            assert_eq!(refused.problems, [voicebridge::tables::FILE_CHANGED]);
        }

        /// The page sends the paths of the word tables. A path with a
        /// wrong file name end is refused, so that the Words screen never
        /// writes rows to another kind of file.
        #[test]
        fn test_a_word_table_path_with_a_wrong_end_is_refused_at_the_save() {
            let s = words_setup();
            let tables = s._dir.path().join("tables");
            let text = |p: &str| ValueDto::Text(tables.join(p).display().to_string());
            let cases = [
                ("data.synonyms", "synonyms.exe", ".toml"),
                ("data.sound_alikes", "sound_alikes.txt", ".toml"),
                ("data.fillers", "fillers", ".toml"),
                ("stt.parakeet.hotwords_file", "hotwords.toml", ".txt"),
            ];
            for (key, file, end) in cases {
                let before = std::fs::read(&s.file).unwrap();
                let result = save_config(&s.state, vec![edit(key, text(file))]).unwrap();
                assert!(!result.written, "{key}: {result:?}");
                let expected = format!(
                    "{key} must name a file that ends with {end}: {}",
                    tables.join(file).display()
                );
                assert_eq!(result.problems[0], expected, "{result:?}");
                assert_eq!(std::fs::read(&s.file).unwrap(), before);
            }
            // The end is compared without case.
            std::fs::copy(tables.join("synonyms.toml"), tables.join("Syn.TOML")).unwrap();
            std::fs::copy(tables.join("hotwords.txt"), tables.join("Hot.TXT")).unwrap();
            let result = save_config(
                &s.state,
                vec![
                    edit("data.synonyms", text("Syn.TOML")),
                    edit("stt.parakeet.hotwords_file", text("Hot.TXT")),
                ],
            )
            .unwrap();
            assert!(result.written, "{result:?}");
            // The alias folder must be a folder.
            let result =
                save_config(&s.state, vec![edit("data.aliases_dir", text("Syn.TOML"))]).unwrap();
            assert!(!result.written, "{result:?}");
            assert!(
                result
                    .problems
                    .iter()
                    .any(|p| p.starts_with("data.aliases_dir must name a folder")),
                "{result:?}"
            );
        }

        /// A hand-edited configuration can name any file. The Words screen
        /// reads it, but never writes rows to it.
        #[test]
        fn test_save_table_refuses_to_write_a_table_file_with_a_wrong_end() {
            let s = words_setup();
            let tables = s._dir.path().join("tables");
            let other = tables.join("notes.ini");
            std::fs::copy(tables.join("synonyms.toml"), &other).unwrap();
            let content = std::fs::read(&other).unwrap();
            let text = std::fs::read_to_string(&s.file).unwrap();
            let moved = config_edit::set_value(
                &text,
                "data.synonyms",
                &Value::Text(other.display().to_string()),
            )
            .unwrap();
            std::fs::write(&s.file, moved).unwrap();
            let table = read_table(&s.state, TableKindDto::Synonyms, None).unwrap();
            let saved = save_table(
                &s.state,
                TableKindDto::Synonyms,
                None,
                vec![row("row me for", &["sail for"])],
                Vec::new(),
                table.version,
            )
            .unwrap();
            assert!(!saved.written, "{saved:?}");
            assert_eq!(
                saved.problems,
                [format!(
                    "the Words screen writes this table only to a file that ends with .toml, \
                     correct the path in the configuration file: {}",
                    other.display()
                )]
            );
            assert_eq!(std::fs::read(&other).unwrap(), content);
        }

        #[test]
        fn test_bad_kind_and_language_are_refused() {
            let s = words_setup();
            // The wire names of the kinds. Another name does not reach a command.
            for (name, kind) in [
                ("aliases", TableKindDto::Aliases),
                ("synonyms", TableKindDto::Synonyms),
                ("sound_alikes", TableKindDto::SoundAlikes),
                ("fillers", TableKindDto::Fillers),
                ("hotwords", TableKindDto::Hotwords),
            ] {
                let wire: TableKindDto = serde_json::from_str(&format!("\"{name}\"")).unwrap();
                assert_eq!(wire, kind);
            }
            for bad in ["commands", "Synonyms", "../synonyms", ""] {
                assert!(serde_json::from_str::<TableKindDto>(&format!("\"{bad}\"")).is_err());
            }
            assert!(read_table(&s.state, TableKindDto::Aliases, None).is_err());
            assert!(read_table(&s.state, TableKindDto::Synonyms, Some("de".into())).is_err());
            for bad in ["..\\x", "a/b", "CON", ""] {
                let error =
                    read_table(&s.state, TableKindDto::Aliases, Some(bad.into())).unwrap_err();
                assert!(error.contains("language name"), "{error}");
            }
            let table = read_table(&s.state, TableKindDto::Aliases, Some("de".into())).unwrap();
            assert_eq!(table.language.as_deref(), Some("de"));
            assert_eq!(table.languages, ["de"]);
            assert_eq!(table.rows.len(), 1);
        }

        #[test]
        fn test_hotword_table_with_the_hotwords_off() {
            let s = words_setup();
            let text = std::fs::read_to_string(&s.file).unwrap();
            let off = config_edit::set_value(
                &text,
                "stt.parakeet.hotwords_file",
                &Value::Text(String::new()),
            )
            .unwrap();
            std::fs::write(&s.file, off).unwrap();
            let table = read_table(&s.state, TableKindDto::Hotwords, None).unwrap();
            assert!(table.hotwords_off);
            // The shipped file below the configuration folder: absent here.
            assert!(!table.exists);
            assert!(table.path.ends_with("hotwords.txt"), "{}", table.path);
            assert!(table.path.starts_with(&s._dir.path().display().to_string()));
        }
    }

    mod model {
        use super::*;
        use crate::model_download::tests::{good_archive, source_for, FakeFetch};
        use crate::model_download::{is_complete_model, Fetch, CANCELLED};

        fn plenty(_: &Path) -> Option<u64> {
            Some(u64::MAX)
        }

        fn run(s: &Setup, models: &Path, fetch: &FakeFetch) -> Result<PathBuf, String> {
            let cancel = begin_model_download(&s.state)?;
            let source = source_for(&fetch.data);
            run_model_download(
                &s.state,
                &source,
                &cancel,
                models,
                &|| Ok(Box::new(fetch) as Box<dyn Fetch + '_>),
                &plenty,
                &*s.emit,
            )
        }

        #[test]
        fn test_download_sets_the_model_folder_in_the_configuration() {
            let s = setup(&[]);
            // A session runs: the download does not wait for it.
            s.start(true).unwrap();
            let models = s._dir.path().join("models");
            let fetch = FakeFetch::new(good_archive());
            let model = run(&s, &models, &fetch).unwrap();
            assert!(is_complete_model(&model));
            let config = Config::load(&s.file).unwrap();
            assert_eq!(config.stt.parakeet.model_dir, model);
            let events = lock(&s.emit.0).clone();
            let last = events.last().unwrap();
            assert_eq!(last["type"], "done");
            assert_eq!(last["model_dir"], model.display().to_string());
            assert_eq!(last["config_error"], serde_json::Value::Null);
            // The session keeps the model of its start.
            assert_eq!(last["restart_needed"], true);
            let types = s.emit.types();
            assert!(types.contains(&"progress".to_string()), "{types:?}");
            assert!(types.contains(&"verifying".to_string()), "{types:?}");
            // `done` comes one time, after the save.
            assert_eq!(types.iter().filter(|t| *t == "done").count(), 1);
            // The session still runs with its old settings.
            assert!(session_status(&s.state).unwrap().running);
            stop_session(&s.state).unwrap();
            assert!(lock(&s.state.model_download).is_none());
        }

        #[test]
        fn test_download_with_a_broken_configuration_file_reports_it_and_keeps_the_file() {
            let s = setup(&[]);
            let broken = "[edcopilot\ndir = 'x'\n";
            std::fs::write(&s.file, broken).unwrap();
            let models = s._dir.path().join("models");
            let fetch = FakeFetch::new(good_archive());
            let model = run(&s, &models, &fetch).unwrap();
            assert!(is_complete_model(&model));
            assert_eq!(std::fs::read_to_string(&s.file).unwrap(), broken);
            let events = lock(&s.emit.0).clone();
            let last = events.last().unwrap();
            assert_eq!(last["type"], "done");
            assert_eq!(last["restart_needed"], false);
            assert!(
                last["config_error"]
                    .as_str()
                    .unwrap()
                    .contains("voicebridge.toml"),
                "{last}"
            );
        }

        #[test]
        fn test_failed_download_does_not_change_the_configuration() {
            let s = setup(&[]);
            let before = std::fs::read(&s.file).unwrap();
            let models = s._dir.path().join("models");
            let mut fetch = FakeFetch::new(good_archive());
            fetch.error = Some("cannot connect to the server, check the network".into());
            let error = run(&s, &models, &fetch).unwrap_err();
            assert_eq!(error, "cannot connect to the server, check the network");
            assert_eq!(std::fs::read(&s.file).unwrap(), before);
            let events = lock(&s.emit.0).clone();
            assert_eq!(
                events.last().unwrap(),
                &serde_json::json!({"type": "failed", "message": error})
            );
            // A new download can start.
            assert!(begin_model_download(&s.state).is_ok());
        }

        #[test]
        fn test_second_download_is_refused_while_one_runs_and_cancel_stops_it() {
            let s = Arc::new(setup(&[]));
            let models = s._dir.path().join("models");
            let fetch = Arc::new(FakeFetch::new(good_archive()));
            fetch.hold.store(true, Ordering::SeqCst);
            let worker = {
                let (s, fetch, models) = (Arc::clone(&s), Arc::clone(&fetch), models.clone());
                std::thread::spawn(move || run(&s, &models, &fetch))
            };
            // The first progress event comes before the body is read.
            while model_info(&s.state, None).unwrap().step.is_none() {
                std::thread::sleep(Duration::from_millis(1));
            }
            let step = model_info(&s.state, None).unwrap().step.unwrap();
            assert!(
                matches!(step, ModelEventDto::Progress { done: 0, .. }),
                "{step:?}"
            );
            assert_eq!(begin_model_download(&s.state).unwrap_err(), DOWNLOAD_RUNS);
            assert!(model_info(&s.state, Some(models.clone())).unwrap().running);
            // The window stays usable: other commands answer.
            assert!(get_config(&s.state).is_ok());
            cancel_model_download(&s.state).unwrap();
            fetch.hold.store(false, Ordering::SeqCst);
            assert_eq!(worker.join().unwrap().unwrap_err(), CANCELLED);
            assert_eq!(s.emit.types().last().unwrap(), "cancelled");
            assert!(!models.join(fetch_name()).exists());
            let info = model_info(&s.state, None).unwrap();
            assert!(!info.running && info.step.is_none(), "{info:?}");
            // Without a download the cancel does nothing.
            assert_eq!(cancel_model_download(&s.state), Ok(()));
            // A new try continues and works.
            run(&s, &models, &fetch).unwrap();
        }

        fn fetch_name() -> &'static str {
            source_for(b"").name
        }

        #[test]
        fn test_model_info_gives_the_pinned_model() {
            let s = setup(&[]);
            let info = model_info(&s.state, Some(PathBuf::from("D:\\m"))).unwrap();
            assert_eq!(info.name, crate::model_download::PARAKEET_V3_INT8.name);
            assert_eq!(
                info.archive_bytes,
                crate::model_download::PARAKEET_V3_INT8.archive_bytes
            );
            assert_eq!(info.models_dir.as_deref(), Some("D:\\m"));
            assert!(!info.running);
        }

        #[test]
        fn test_download_without_a_session_needs_no_restart() {
            let s = setup(&[]);
            let models = s._dir.path().join("models");
            let fetch = FakeFetch::new(good_archive());
            run(&s, &models, &fetch).unwrap();
            let events = lock(&s.emit.0).clone();
            assert_eq!(events.last().unwrap()["restart_needed"], false);
            // The same folder again while a session runs: no change, no restart.
            s.start(true).unwrap();
            run(&s, &models, &fetch).unwrap();
            let events = lock(&s.emit.0).clone();
            let last = events.last().unwrap();
            assert_eq!(last["type"], "done");
            assert_eq!(last["restart_needed"], false);
            stop_session(&s.state).unwrap();
        }

        #[test]
        fn test_panic_in_the_download_gives_failed_and_frees_the_slot() {
            let s = setup(&[]);
            let models = s._dir.path().join("models");
            let mut fetch = FakeFetch::new(good_archive());
            fetch.panic = true;
            let error = run(&s, &models, &fetch).unwrap_err();
            assert_eq!(error, DOWNLOAD_PANIC);
            let events = lock(&s.emit.0).clone();
            assert_eq!(
                events.last().unwrap(),
                &serde_json::json!({"type": "failed", "message": DOWNLOAD_PANIC})
            );
            let info = model_info(&s.state, None).unwrap();
            assert!(!info.running && info.step.is_none());
            // The lock of the models folder is free again.
            fetch.panic = false;
            run(&s, &models, &fetch).unwrap();
        }

        #[test]
        fn test_shutdown_cancels_the_download() {
            let s = setup(&[]);
            let cancel = begin_model_download(&s.state).unwrap();
            shutdown(&s.state);
            assert!(cancel.load(Ordering::SeqCst));
        }
    }

    mod startup {
        use super::*;
        use crate::paths::{StartupNotes, DEV_BUILD};

        #[test]
        fn test_a_startup_problem_stops_the_configuration_and_shows_the_text() {
            let s = setup(&[]);
            lock(&s.state.startup).problem = Some(DEV_BUILD.to_string());
            assert_eq!(get_config(&s.state).unwrap_err(), DEV_BUILD);
            let check = check(&s.state).unwrap();
            assert_eq!(check.problems, [DEV_BUILD]);
            assert!(check.report.is_none());
            assert!(s.start(true).unwrap_err().contains(DEV_BUILD));
            assert_eq!(startup_notes(&s.state).problem.as_deref(), Some(DEV_BUILD));
        }

        #[test]
        fn test_startup_notes_give_the_first_start_error_the_update_notes_and_the_data_folder() {
            let data = PathBuf::from("D:\\scratch\\VoiceBridge");
            let s = setup(&[]);
            let state = AppState::new(
                s.file.clone(),
                Arc::new(MemoryStore::new()),
                Arc::new(FakeStarter::default()),
            )
            .with_data_dir(Some(data.clone()));
            *lock(&state.startup) = StartupNotes {
                problem: None,
                first_start_error: Some("the program could not make its data folder".into()),
                notes: vec![crate::paths::MAP_KEYS_REMOVED.into()],
                update_problems: vec!["cannot replace configuration file".into()],
            };
            let notes = startup_notes(&state);
            assert_eq!(
                notes.first_start_error.as_deref(),
                Some("the program could not make its data folder")
            );
            assert_eq!(notes.notes, [crate::paths::MAP_KEYS_REMOVED]);
            assert_eq!(notes.update_problems, ["cannot replace configuration file"]);
            assert_eq!(notes.data_dir.as_deref(), Some("D:\\scratch\\VoiceBridge"));
            assert_eq!(startup_notes(&s.state).data_dir, None);
        }

        #[test]
        fn test_a_tray_start_waits_until_the_page_takes_it_one_time() {
            let s = setup(&[]);
            assert!(!take_tray_start(&s.state));
            mark_tray_start(&s.state);
            assert!(take_tray_start(&s.state));
            assert!(!take_tray_start(&s.state));
        }
    }

    mod diagnostics {
        use super::*;
        use crate::dto::DiagnosticsPart;

        /// Keeps each folder that the program opens.
        #[derive(Default)]
        struct Opened(Mutex<Vec<PathBuf>>);

        impl Opened {
            fn open(&self, dir: &Path) -> Result<(), String> {
                lock(&self.0).push(dir.to_path_buf());
                Ok(())
            }
        }

        fn base(s: &Setup) -> PathBuf {
            s.file.parent().unwrap().to_path_buf()
        }

        #[test]
        fn test_switching_the_log_on_writes_the_default_file_and_makes_its_folder() {
            let s = setup(&[]);
            let result = set_diagnostics(&s.state, DiagnosticsPart::Log, true).unwrap();
            assert!(result.written, "{:?}", result.problems);
            let config = Config::load(&s.file).unwrap();
            assert_eq!(
                config.diagnostics.log_file,
                base(&s).join("logs").join("utterances.tsv")
            );
            assert!(base(&s).join("logs").is_dir());
            assert!(config.diagnostics_problems().is_empty());
            // One kind of separator in the file, as the system writes paths.
            let written = Config::load_as_written(&s.file).unwrap();
            let text = written.diagnostics.log_file.display().to_string();
            assert!(text.ends_with(&format!("logs{}utterances.tsv", std::path::MAIN_SEPARATOR)));
            // The audio capture stays off.
            assert!(config.diagnostics.save_audio_dir.as_os_str().is_empty());

            let result = set_diagnostics(&s.state, DiagnosticsPart::Log, false).unwrap();
            assert!(result.written);
            let config = Config::load(&s.file).unwrap();
            assert!(config.diagnostics.log_file.as_os_str().is_empty());
        }

        #[test]
        fn test_switching_the_audio_capture_on_and_off() {
            let s = setup(&[]);
            let result = set_diagnostics(&s.state, DiagnosticsPart::Audio, true).unwrap();
            assert!(result.written);
            let audio = base(&s).join("logs").join("audio");
            let config = Config::load(&s.file).unwrap();
            assert_eq!(config.diagnostics.save_audio_dir, audio);
            assert!(audio.is_dir());
            assert!(config.diagnostics.log_file.as_os_str().is_empty());
            set_diagnostics(&s.state, DiagnosticsPart::Audio, false).unwrap();
            let config = Config::load(&s.file).unwrap();
            assert!(config.diagnostics.save_audio_dir.as_os_str().is_empty());
        }

        #[test]
        fn test_switching_on_keeps_the_path_of_the_user_and_reports_a_running_session() {
            let own = tempfile::tempdir().unwrap();
            let file = own.path().join("mine.tsv");
            let s = setup(&[(
                "diagnostics.log_file",
                Value::Text(file.display().to_string()),
            )]);
            let before = std::fs::read(&s.file).unwrap();
            let result = set_diagnostics(&s.state, DiagnosticsPart::Log, true).unwrap();
            assert!(!result.written);
            assert_eq!(std::fs::read(&s.file).unwrap(), before);

            s.start(true).unwrap();
            let result = set_diagnostics(&s.state, DiagnosticsPart::Audio, true).unwrap();
            assert!(result.written && result.restart_needed);
            stop_session(&s.state).unwrap();
        }

        #[test]
        fn test_open_folder_opens_only_the_configured_folder() {
            let s = setup(&[]);
            set_diagnostics(&s.state, DiagnosticsPart::Log, true).unwrap();
            set_diagnostics(&s.state, DiagnosticsPart::Audio, true).unwrap();
            let opened = Opened::default();
            open_folder(&s.state, DiagnosticsPart::Log, &|d| opened.open(d)).unwrap();
            open_folder(&s.state, DiagnosticsPart::Audio, &|d| opened.open(d)).unwrap();
            // The program opens the canonical path of each folder.
            let canonical = |p: PathBuf| crate::paths::plain_path(&p.canonicalize().unwrap());
            assert_eq!(
                *lock(&opened.0),
                [
                    canonical(base(&s).join("logs")),
                    canonical(base(&s).join("logs").join("audio"))
                ]
            );
        }

        #[test]
        fn test_open_folder_of_the_data_folder_needs_the_data_folder() {
            let s = setup(&[]);
            let opened = Opened::default();
            let error =
                open_folder(&s.state, DiagnosticsPart::Data, &|d| opened.open(d)).unwrap_err();
            assert_eq!(error, NO_DATA_FOLDER);
            let state = AppState::new(
                s.file.clone(),
                Arc::new(MemoryStore::new()),
                Arc::new(FakeStarter::default()),
            )
            .with_data_dir(Some(base(&s)));
            open_folder(&state, DiagnosticsPart::Data, &|d| opened.open(d)).unwrap();
            let expected = crate::paths::plain_path(&base(&s).canonicalize().unwrap());
            assert_eq!(*lock(&opened.0), [expected]);
            // The data folder has no switch.
            assert!(set_diagnostics(&state, DiagnosticsPart::Data, true).is_err());
        }

        #[test]
        fn test_open_folder_refuses_a_file_and_a_shell_namespace_folder() {
            let s = setup(&[]);
            let opened = Opened::default();
            let set_audio = |path: &Path| {
                let text = std::fs::read_to_string(&s.file).unwrap();
                let text = config_edit::set_value(
                    &text,
                    "diagnostics.save_audio_dir",
                    &Value::Text(path.display().to_string()),
                )
                .unwrap();
                std::fs::write(&s.file, text).unwrap();
            };
            // A file, for example a program, is never opened.
            let program = base(&s).join("tool.exe");
            std::fs::write(&program, "MZ").unwrap();
            set_audio(&program);
            let error =
                open_folder(&s.state, DiagnosticsPart::Audio, &|d| opened.open(d)).unwrap_err();
            assert!(error.contains("not a folder"), "{error}");
            // A folder name that the shell reads as a namespace folder.
            let namespace = base(&s).join("x.{21EC2020-3AEA-1069-A2DD-08002B30309D}");
            std::fs::create_dir(&namespace).unwrap();
            set_audio(&namespace);
            let error =
                open_folder(&s.state, DiagnosticsPart::Audio, &|d| opened.open(d)).unwrap_err();
            assert!(error.contains("shell folder"), "{error}");
            // Also a namespace folder in the middle of the path.
            let inner = namespace.join("logs");
            std::fs::create_dir(&inner).unwrap();
            set_audio(&inner);
            let error =
                open_folder(&s.state, DiagnosticsPart::Audio, &|d| opened.open(d)).unwrap_err();
            assert!(error.contains("shell folder"), "{error}");
            assert!(lock(&opened.0).is_empty());
        }

        #[test]
        fn test_open_folder_refuses_a_part_that_is_off_or_a_folder_that_does_not_exist() {
            let s = setup(&[]);
            let opened = Opened::default();
            let error =
                open_folder(&s.state, DiagnosticsPart::Log, &|d| opened.open(d)).unwrap_err();
            assert_eq!(error, LOG_OFF);
            let error =
                open_folder(&s.state, DiagnosticsPart::Audio, &|d| opened.open(d)).unwrap_err();
            assert_eq!(error, AUDIO_OFF);
            let absent = base(&s).join("absent");
            let text = std::fs::read_to_string(&s.file).unwrap();
            let text = config_edit::set_value(
                &text,
                "diagnostics.save_audio_dir",
                &Value::Text(absent.display().to_string()),
            )
            .unwrap();
            std::fs::write(&s.file, text).unwrap();
            let error =
                open_folder(&s.state, DiagnosticsPart::Audio, &|d| opened.open(d)).unwrap_err();
            assert!(error.contains("does not exist"), "{error}");
            assert!(lock(&opened.0).is_empty());
        }

        #[test]
        fn test_the_page_names_a_part_and_never_a_path() {
            let part: DiagnosticsPart = serde_json::from_str("\"log\"").unwrap();
            assert_eq!(part, DiagnosticsPart::Log);
            let part: DiagnosticsPart = serde_json::from_str("\"audio\"").unwrap();
            assert_eq!(part, DiagnosticsPart::Audio);
            let part: DiagnosticsPart = serde_json::from_str("\"data\"").unwrap();
            assert_eq!(part, DiagnosticsPart::Data);
            assert!(serde_json::from_str::<DiagnosticsPart>("\"C:\\\\Windows\"").is_err());
        }

        #[test]
        fn test_hotwords_on_writes_a_path_that_exists_in_the_data_folder() {
            let res = crate::paths::tests::resource_folder();
            let home = tempfile::tempdir().unwrap();
            let data = home.path().join(crate::paths::DATA_FOLDER);
            crate::paths::first_start_with(&data, res.path(), None).unwrap();
            let file = data.join(crate::paths::CONFIG_FILE);
            let state = AppState::new(
                file.clone(),
                Arc::new(MemoryStore::new()),
                Arc::new(FakeStarter::default()),
            );
            let config = get_config(&state).unwrap();
            assert!(config.stt.parakeet.hotwords_file.is_empty());
            let on = config.default_hotwords_file.clone();
            // The switch of the Setup screen sends this value.
            let result = save_config(
                &state,
                vec![edit(
                    "stt.parakeet.hotwords_file",
                    crate::dto::ValueDto::Text(on.clone()),
                )],
            )
            .unwrap();
            assert!(result.written, "{:?}", result.problems);
            let loaded = Config::load(&file).unwrap();
            let hotwords = &loaded.stt.parakeet.hotwords_file;
            assert!(hotwords.is_file(), "{}", hotwords.display());
            assert!(hotwords.starts_with(&data), "{}", hotwords.display());
            assert_eq!(hotwords, &data.join("hotwords.txt"));
            // The hotword tab edits the same file, before and after the switch.
            assert_eq!(
                read_table(&state, TableKindDto::Hotwords, None)
                    .unwrap()
                    .path,
                on
            );
            assert_eq!(get_config(&state).unwrap().default_hotwords_file, on);
        }
    }

    /// Starts the real runtime as a dry run with a scratch configuration.
    /// It opens the default microphone and reads the talk button, and it
    /// makes a session lock file in the per-user folder of the runtime.
    /// Set VOICEBRIDGE_PARAKEET_DIR to the model directory.
    #[test]
    #[ignore]
    fn the_real_runtime_starts_and_stops_as_a_dry_run() {
        let model = std::env::var("VOICEBRIDGE_PARAKEET_DIR").expect("VOICEBRIDGE_PARAKEET_DIR");
        let core = Path::new(env!("CARGO_MANIFEST_DIR")).join("../core");
        let core = core.canonicalize().unwrap();
        let path = |p: &str| Value::Text(core.join(p).display().to_string());
        let s = setup(&[
            ("stt.parakeet.model_dir", Value::Text(model)),
            ("stt.parakeet.threads", Value::Integer(2)),
            ("data.aliases_dir", path("aliases")),
            ("data.sound_alikes", path("data/sound_alikes.toml")),
            ("data.synonyms", path("data/synonyms.toml")),
            ("data.fillers", path("data/fillers.toml")),
        ]);
        // The invented EDCoPilot folder of the fixture: the program reads
        // its profile.
        edcopilot_voicebridge_core::profile::fixture::write_fixture_edcopilot(
            &s._dir.path().join("ed"),
        );
        let state = AppState::new(
            s.file.clone(),
            s.keys.clone(),
            Arc::new(crate::state::RuntimeStarter),
        );
        start_session(&state, true, s.emit.clone()).unwrap();
        let status = session_status(&state).unwrap();
        assert!(status.running);
        assert!(status.dry_run);
        let result = save_config(
            &state,
            vec![edit(
                "matching.threshold",
                crate::dto::ValueDto::Number(0.9),
            )],
        )
        .unwrap();
        assert!(result.restart_needed, "{result:?}");
        stop_session(&state).unwrap();
        let status = session_status(&state).unwrap();
        assert!(!status.running);
        assert_eq!(status.stop_reason.unwrap().kind, "requested");
        let types = s.emit.types();
        assert!(types.contains(&"started".to_string()), "{types:?}");
        assert_eq!(types.last().unwrap(), "stopped");
    }

    // The update install and the session: the gate of the capture lock.

    use crate::updater::tests::{refusal, with_offer, Download, FakePending};

    /// `setup` with a release-build update state that offers version 0.3.0.
    fn setup_with_offer() -> (Setup, Arc<FakePending>) {
        let Setup {
            _dir,
            file,
            state,
            starter,
            keys,
            emit,
        } = setup(&[]);
        let pending = FakePending::new(Download::Forever);
        let state = state.with_updates(with_offer(pending.clone()));
        (
            Setup {
                _dir,
                file,
                state,
                starter,
                keys,
                emit,
            },
            pending,
        )
    }

    fn install_refusal(state: &AppState) -> String {
        match begin_update_install(state) {
            Err(text) => text,
            Ok(_) => panic!("the install was not refused"),
        }
    }

    #[test]
    fn test_install_is_refused_while_a_session_runs() {
        let (s, _) = setup_with_offer();
        s.start(false).unwrap();
        assert_eq!(install_refusal(&s.state), updater::STOP_FIRST);
        assert_eq!(s.state.updates.status().phase, "available");
        stop_session(&s.state).unwrap();
        begin_update_install(&s.state).unwrap();
        assert_eq!(s.state.updates.status().phase, "downloading");
    }

    #[test]
    fn test_install_is_refused_while_a_session_starts() {
        let (s, _) = setup_with_offer();
        let result = Arc::new(Mutex::new(None));
        let out = result.clone();
        *lock(&s.state.start_hook) = Some(Box::new(move |state: &AppState| {
            *lock(&out) = Some(install_refusal(state));
        }));
        s.start(true).unwrap();
        assert_eq!(
            lock(&result).take().as_deref(),
            Some(updater::STOP_FIRST),
            "the hook ran"
        );
        stop_session(&s.state).unwrap();
    }

    #[test]
    fn test_a_session_does_not_start_during_the_update() {
        let (s, _) = setup_with_offer();
        begin_update_install(&s.state).unwrap();
        assert_eq!(s.start(false).unwrap_err(), updater::IN_PROGRESS);
        assert!(lock(&s.starter.sessions).is_empty());
        assert!(!s.state.session_active());
        // After a cancel the session starts.
        s.state.updates.cancel().unwrap();
        s.start(false).unwrap();
        stop_session(&s.state).unwrap();
    }

    /// A start and an install at the same time: never both.
    #[test]
    fn test_a_start_and_an_install_never_both_go_through() {
        for _ in 0..40 {
            let (s, _) = setup_with_offer();
            let s = Arc::new(s);
            let barrier = Arc::new(std::sync::Barrier::new(2));
            let (a, b) = (s.clone(), barrier.clone());
            let starter = std::thread::spawn(move || {
                b.wait();
                a.start(true)
            });
            barrier.wait();
            let install = begin_update_install(&s.state).map(|_| ());
            let start = starter.join().unwrap();
            assert!(
                start.is_err() || install.is_err(),
                "both went through: {start:?} {install:?}"
            );
            assert!(start.is_ok() || install.is_ok(), "{start:?} {install:?}");
            stop_session(&s.state).unwrap();
        }
    }

    #[test]
    fn test_shutdown_stops_the_download() {
        let (s, _) = setup_with_offer();
        begin_update_install(&s.state).unwrap();
        shutdown(&s.state);
        assert_eq!(s.state.updates.status().phase, "available");
        // A new try works.
        begin_update_install(&s.state).unwrap();
        assert_eq!(refusal(&s.state.updates), updater::IN_PROGRESS);
    }

    /// An installer that does not start: the hook before the start stopped
    /// the session (as the real hook does), the window comes back, the
    /// offer shows the text, and a session can start again.
    #[test]
    fn test_an_installer_that_does_not_start_gives_the_program_back() {
        use crate::updater::tests::counting_restore;
        use crate::updater::{Cause, PendingUpdate};
        struct FailingStart;
        impl PendingUpdate for FailingStart {
            fn download<'a>(
                &'a self,
                _progress: updater::Progress<'a>,
            ) -> updater::BoxFuture<'a, Result<Vec<u8>, Cause>> {
                Box::pin(async { Ok(vec![1, 2, 3]) })
            }
            fn install(&self, _bytes: Vec<u8>) -> Result<(), Cause> {
                Err(Cause::InstallerStart)
            }
        }
        let Setup {
            _dir,
            file,
            state,
            starter,
            keys,
            emit,
        } = setup(&[]);
        let state = state.with_updates(crate::updater::tests::with_offer(Arc::new(FailingStart)));
        let s = Setup {
            _dir,
            file,
            state,
            starter,
            keys,
            emit,
        };
        let (id, pending) = begin_update_install(&s.state).unwrap();
        let (restore, restored) = counting_restore();
        crate::updater::tests::block_on(updater::run_download(
            Arc::clone(&s.state.updates),
            id,
            pending,
            Arc::new(crate::events::tests::FakeEmit::default()),
            Duration::ZERO,
            restore,
        ));
        assert_eq!(restored.load(Ordering::SeqCst), 1);
        let status = s.state.updates.status();
        assert_eq!(status.phase, "available");
        assert_eq!(
            status.error.as_deref(),
            Some(updater::install_failed_text(Cause::InstallerStart).as_str())
        );
        s.start(true).unwrap();
        stop_session(&s.state).unwrap();
    }

    #[test]
    fn test_the_update_switch_needs_no_session_restart() {
        let s = setup(&[]);
        assert_eq!(updates_check_enabled(&s.state), Ok(true));
        s.start(false).unwrap();
        let flag = |on| edit(UPDATES_CHECK_KEY, crate::dto::ValueDto::Flag(on));
        let result = save_config(&s.state, vec![flag(false)]).unwrap();
        assert!(result.written);
        assert!(!result.restart_needed);
        assert_eq!(updates_check_enabled(&s.state), Ok(false));
        // A key that the session uses still needs a restart.
        let result = save_config(
            &s.state,
            vec![
                flag(true),
                edit("matching.threshold", crate::dto::ValueDto::Number(0.85)),
            ],
        )
        .unwrap();
        assert!(result.restart_needed);
        stop_session(&s.state).unwrap();
        // A file that does not load: no automatic check.
        std::fs::write(&s.file, "not = [toml").unwrap();
        assert!(updates_check_enabled(&s.state).is_err());
    }
}
