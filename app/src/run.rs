//! The console commands, the engine selection, and the start lines.
//!
//! `run` starts a `Runtime` (see `crate::runtime`) with a `ConsoleSink`.

use std::fmt;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc};
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{bail, Context};
use edcopilot_voicebridge_audio::{list_input_devices, InputScanner, KeyboardKey};
use edcopilot_voicebridge_core::{command_word_conflicts, confusable_hotwords, Pipeline};
use edcopilot_voicebridge_stt::{
    hotwords, wav, FakeEngine, Hotwords, OpenAiCompatConfig, OpenAiCompatEngine, ParakeetEngine,
    SttEngine,
};

use crate::commands_source::{profile_change, profile_lines, ProfileChange};
use crate::config::{
    build_pipeline, is_loopback_url, is_variable_name, url_origin, Config, Joystick, LoadReport,
    API_KEY_ENV_PROBLEM,
};
use crate::console::ConsoleSink;
use crate::diagnostics::{levels, Levels, Marks};
pub use crate::runtime::Mode;
use crate::runtime::{report_dir, KeyLookup, Runtime, RuntimeOptions, StartError, StopReason};
use crate::session::{Outcome, Session, SessionOptions};

/// Exit code after a second Ctrl+C (128 + SIGINT, as in shells).
const EXIT_FORCED_STOP: i32 = 130;
/// Poll interval of `inputs`.
const INPUTS_POLL: Duration = Duration::from_millis(50);

/// Reads an environment variable. A value that is not Unicode counts as absent.
pub fn env_var(name: &str) -> Option<String> {
    std::env::var(name).ok()
}

/// The API key from the variable `name`, without leading and trailing white
/// space. The error names the variable only when the name is a valid
/// variable name, never the value. A value that is not a name can be a key.
fn api_key(name: &str, env: impl Fn(&str) -> Option<String>) -> Result<String, String> {
    if name.trim().is_empty() {
        return Err(
            "stt.openai_compatible.api_key_env is empty, it must name an environment variable"
                .to_string(),
        );
    }
    if !is_variable_name(name) {
        return Err(API_KEY_ENV_PROBLEM.to_string());
    }
    match env(name) {
        Some(key) if !key.trim().is_empty() => Ok(key.trim().to_string()),
        _ => Err(format!(
            "environment variable {name} is absent or empty, it must hold the API key"
        )),
    }
}

/// Engine problems that `Config::validate` does not find. Does not load a model.
pub fn engine_problems(config: &Config, env: impl Fn(&str) -> Option<String>) -> Vec<String> {
    engine_problems_keyed(config, env)
        .into_iter()
        .map(|(_, text)| text)
        .collect()
}

/// `engine_problems` with the configuration key of each problem.
pub fn engine_problems_keyed(
    config: &Config,
    env: impl Fn(&str) -> Option<String>,
) -> Vec<(&'static str, String)> {
    match config.stt.engine.as_str() {
        "parakeet" => {
            let dir = &config.stt.parakeet.model_dir;
            let mut problems = Vec::new();
            if !dir.as_os_str().is_empty() && !dir.is_dir() {
                problems.push((
                    "stt.parakeet.model_dir",
                    format!(
                        "stt.parakeet.model_dir is not a directory: {}",
                        dir.display()
                    ),
                ));
            }
            problems
                .extend(hotword_problem(config).map(|text| ("stt.parakeet.hotwords_file", text)));
            problems
        }
        "openai_compatible" => api_key(&config.stt.openai_compatible.api_key_env, env)
            .err()
            .map(|text| ("stt.openai_compatible.api_key_env", text))
            .into_iter()
            .collect(),
        _ => Vec::new(),
    }
}

/// The hotword list of the configuration. `None` when the hotwords are off.
fn load_hotwords(config: &Config) -> anyhow::Result<Option<Hotwords>> {
    let p = &config.stt.parakeet;
    if p.hotwords_file.as_os_str().is_empty() {
        return Ok(None);
    }
    Hotwords::load(&p.hotwords_file, p.hotwords_score)
        .map(Some)
        .context("stt.parakeet.hotwords_file")
}

/// The problem of the hotword list: it does not load, or the pieces of the
/// model cannot spell a phrase. The second check runs only when the model
/// directory has `tokens.txt`. A score out of range is not a problem here,
/// because `Config::validate` reports it.
fn hotword_problem(config: &Config) -> Option<String> {
    let score = config.stt.parakeet.hotwords_score;
    if !(hotwords::MIN_SCORE..=hotwords::MAX_SCORE).contains(&score) {
        return None;
    }
    let hotwords = match load_hotwords(config) {
        Ok(Some(h)) => h,
        Ok(None) => return None,
        Err(e) => return Some(format!("{e:#}")),
    };
    let tokens = config.stt.parakeet.model_dir.join("tokens.txt");
    let tokens = std::fs::read_to_string(tokens).ok()?;
    let bad = hotwords::unencodable(&tokens, hotwords.phrases());
    (!bad.is_empty()).then(|| {
        format!(
            "stt.parakeet.hotwords_file: the model pieces cannot spell these hotwords: {}",
            bad.join(", ")
        )
    })
}

/// The hotword phrases that the engine of the configuration uses. Empty
/// for the cloud engine, for an empty `hotwords_file`, and for a file that
/// does not load (then the engine start reports the error).
fn active_hotwords(config: &Config) -> Vec<String> {
    if config.stt.engine != "parakeet" {
        return Vec::new();
    }
    match load_hotwords(config) {
        Ok(Some(h)) => h.phrases().to_vec(),
        _ => Vec::new(),
    }
}

/// One problem for each hotword that is near another hotword or a name
/// that the matcher can send, and for each hotword that is equal to or near
/// a word of a command phrase. See `edcopilot_voicebridge_core::confusables`.
pub(crate) fn hotword_confusions(config: &Config, pipeline: &Pipeline) -> Vec<String> {
    let hotwords = active_hotwords(config);
    let mut found = confusable_hotwords(&hotwords, &pipeline.names());
    found.extend(command_word_conflicts(&hotwords, &pipeline.command_words()));
    found
        .iter()
        .map(|c| format!("stt.parakeet.hotwords_file: {}", c.describe()))
        .collect()
}

/// Prints the confusions of the hotwords and returns true when there are
/// any. `run` and `replay` then stop.
fn refuse_confused_hotwords(config: &Config, pipeline: &Pipeline) -> bool {
    let problems = hotword_confusions(config, pipeline);
    for problem in &problems {
        eprintln!("problem: {problem}");
    }
    !problems.is_empty()
}

/// A warning when the hotword file is set but has no phrase.
fn empty_hotwords_warning(config: &Config) -> Option<String> {
    if config.stt.engine != "parakeet" {
        return None;
    }
    match load_hotwords(config) {
        Ok(Some(h)) if h.is_empty() => Some(format!(
            "stt.parakeet.hotwords_file has no hotword, the second pass is off: {}",
            config.shown(&config.stt.parakeet.hotwords_file).display()
        )),
        _ => None,
    }
}

/// The start line of the hotwords of the local engine.
fn hotwords_line(hotwords: Option<&Hotwords>, file: &Path) -> String {
    match hotwords {
        Some(h) if !h.is_empty() => format!(
            "hotwords: {} from {}, score {:.1}",
            h.phrases().len(),
            file.display(),
            h.score()
        ),
        _ => "hotwords: off".to_string(),
    }
}

/// Makes the engine that the configuration names and prints its hotword
/// line.
pub fn make_engine(
    config: &Config,
    env: impl Fn(&str) -> Option<String>,
) -> anyhow::Result<Box<dyn SttEngine>> {
    let (engine, line) = make_engine_with_line(config, env)?;
    if let Some(line) = line {
        println!("{line}");
    }
    Ok(engine)
}

/// Makes the engine that the configuration names. Gives the hotword line of
/// the local engine, and `None` for the cloud engine.
pub(crate) fn make_engine_with_line(
    config: &Config,
    env: impl Fn(&str) -> Option<String>,
) -> anyhow::Result<(Box<dyn SttEngine>, Option<String>)> {
    match config.stt.engine.as_str() {
        "parakeet" => {
            let p = &config.stt.parakeet;
            let hotwords = load_hotwords(config)?;
            let engine = ParakeetEngine::new(&p.model_dir, p.threads, hotwords.as_ref())
                .context("cannot start the parakeet engine")?;
            let line = hotwords_line(hotwords.as_ref(), config.shown(&p.hotwords_file));
            Ok((Box::new(engine), Some(line)))
        }
        "openai_compatible" => {
            let o = &config.stt.openai_compatible;
            let api_key = api_key(&o.api_key_env, env).map_err(anyhow::Error::msg)?;
            let language = (!o.language.trim().is_empty()).then(|| o.language.clone());
            let engine = OpenAiCompatEngine::new(OpenAiCompatConfig {
                base_url: o.base_url.clone(),
                api_key,
                model: o.model.clone(),
                language,
                timeout: Duration::from_secs(o.timeout_seconds),
            })
            .context("cannot start the openai_compatible engine")?;
            Ok((Box::new(engine), None))
        }
        other => bail!("unknown speech engine \"{other}\""),
    }
}

/// One line with the counts of the load report.
pub fn report_line(report: &LoadReport) -> String {
    format!(
        "loaded {} commands ({} matchable, {} composite parts, {} with unresolved grammar), \
         {} vocabulary files with {} names, {} alias phrases, {} sound-alike entries, \
         {} synonym entries, {} filler words{}",
        report.commands,
        report.matchable,
        report.composite_parts,
        report.unresolved_grammar,
        report.vocabulary_files,
        report.vocabulary_names,
        report.alias_phrases,
        report.sound_alikes,
        report.synonyms,
        report.fillers,
        if report.fillers_active || report.fillers == 0 {
            ""
        } else {
            " (off in always_listening mode)"
        }
    )
}

/// The log line of an outcome with the times and the clip length in
/// milliseconds, the levels in whole dBFS, a level warning, the word
/// `hotwords` when the outcome came from the second pass of the engine, and
/// the word `glitch` when the audio stream lost samples during the clip.
pub fn outcome_line(
    outcome: &Outcome,
    transcribe: Duration,
    total: Duration,
    clip: Duration,
    levels: Levels,
    marks: Marks,
) -> String {
    let level_warning = if levels.clipping() {
        ", clipping"
    } else if levels.very_quiet() {
        ", very quiet"
    } else {
        ""
    };
    let hotwords = if marks.hotwords { ", hotwords" } else { "" };
    let glitch = if marks.glitch { ", glitch" } else { "" };
    format!(
        "{} (transcribe {} ms, total {} ms, clip {} ms, peak {} dBFS, rms {} dBFS{level_warning}{hotwords}{glitch})",
        outcome.log_line(),
        transcribe.as_millis(),
        total.as_millis(),
        clip.as_millis(),
        whole_db(levels.peak_dbfs),
        whole_db(levels.rms_dbfs),
    )
}

/// A level rounded to whole dB. An integer has no "-0".
fn whole_db(db: f32) -> i32 {
    db.round() as i32
}

pub(crate) fn mode_of(config: &Config) -> Mode {
    if config.microphone.mode == "always_listening" {
        Mode::AlwaysListening
    } else {
        Mode::PushToTalk
    }
}

/// What `voicebridge check` finds, as data.
#[derive(Debug, Clone, Default)]
pub struct CheckReport {
    /// `None` when the data did not load. Then `problems` has the reason.
    pub report: Option<LoadReport>,
    /// The warnings without the prefix `warning: `.
    pub warnings: Vec<String>,
    /// The lines of the talk button, the utterance log and the audio capture.
    pub info_lines: Vec<String>,
    /// Each problem refuses the start of a session.
    pub problems: Vec<String>,
    /// What changed in the EDCoPilot profile since the last session start.
    /// `None` without a change, without a saved report, or when the
    /// commands did not load.
    pub profile_change: Option<ProfileChange>,
    /// The error of the load of the commands, for example a profile that
    /// the program cannot read. `None` when the commands loaded. The text
    /// is also in `problems`.
    pub load_problem: Option<String>,
}

/// The checks of `voicebridge check`. `env` gives the API key for a name.
/// It prints nothing. The saved report of the last session start comes
/// from the folder of the session lock.
pub fn check_config(config: &Config, env: impl Fn(&str) -> Option<String>) -> CheckReport {
    let dir = report_dir().ok();
    check_config_in(config, env, dir.as_deref())
}

/// `check_config` with the folder of the saved report. `None`: no saved
/// report, thus no difference.
pub fn check_config_in(
    config: &Config,
    env: impl Fn(&str) -> Option<String>,
    report_dir: Option<&Path>,
) -> CheckReport {
    let mut out = CheckReport {
        problems: config.validate(),
        ..CheckReport::default()
    };
    // `validate` and the engine check can find the same problem, for
    // example a value of `api_key_env` that is not a variable name.
    for problem in engine_problems(config, env) {
        if !out.problems.contains(&problem) {
            out.problems.push(problem);
        }
    }
    match build_pipeline(config) {
        Ok((pipeline, report)) => {
            out.warnings.extend(report.warnings().cloned());
            out.warnings.extend(empty_hotwords_warning(config));
            out.problems.extend(report.problems.iter().cloned());
            out.problems.extend(hotword_confusions(config, &pipeline));
            out.profile_change =
                profile_change(&report.profile, report_dir, &config.edcopilot.profile);
            out.report = Some(report);
        }
        Err(e) => {
            // An empty EDCoPilot folder is also a problem of `validate`.
            let problem = format!("{e:#}");
            if !out.problems.contains(&problem) {
                out.problems.push(problem.clone());
            }
            out.load_problem = Some(problem);
            // The load did not give its warnings: the warning of the old
            // keys still shows one time.
            out.warnings.extend(config.old_map_keys_warning());
        }
    }
    out.warnings.extend(plain_http_warning(config));
    out.info_lines.extend(push_to_talk_line(config));
    out.info_lines.extend(diagnostics_lines(config));
    out.problems.extend(config.diagnostics_problems());
    out
}

/// A warning when the cloud engine sends its key over `http://` to another
/// computer.
fn plain_http_warning(config: &Config) -> Option<String> {
    let url = &config.stt.openai_compatible.base_url;
    // The origin comes from the URL parser of the HTTP client.
    let plain = url_origin(url).is_some_and(|origin| origin.starts_with("http://"));
    (config.stt.engine == "openai_compatible" && plain && !is_loopback_url(url))
        .then(|| "the cloud engine sends the API key without encryption".to_string())
}

/// `voicebridge check`: prints the problems, the load report counts, and
/// the report of the profile build. `left_out`: one line for each left-out
/// command.
pub fn check(config_path: &Path, left_out: bool) -> anyhow::Result<ExitCode> {
    let config = Config::load(config_path)?;
    let found = check_config(&config, env_var);
    if let Some(report) = &found.report {
        println!("{}", report_line(report));
        for line in profile_lines(&report.profile, found.profile_change.as_ref(), left_out) {
            println!("{line}");
        }
    }
    for warning in &found.warnings {
        println!("warning: {warning}");
    }
    for line in &found.info_lines {
        println!("{line}");
    }
    let problems = found.problems;
    for problem in &problems {
        println!("problem: {problem}");
    }
    if problems.is_empty() {
        println!("configuration is valid");
        Ok(ExitCode::SUCCESS)
    } else {
        println!("{} problem(s) found", problems.len());
        Ok(ExitCode::FAILURE)
    }
}

/// The `check` lines of the utterance log and the audio capture, when set.
pub(crate) fn diagnostics_lines(config: &Config) -> Vec<String> {
    let d = &config.diagnostics;
    let mut lines = Vec::new();
    if !d.log_file.as_os_str().is_empty() {
        lines.push(format!(
            "utterance log: {}",
            config.shown(&d.log_file).display()
        ));
    }
    if !d.save_audio_dir.as_os_str().is_empty() {
        lines.push(format!(
            "audio capture: {}",
            config.shown(&d.save_audio_dir).display()
        ));
    }
    lines
}

/// The session options of `run` and `replay`. Wake words apply only in
/// `always_listening` mode.
pub fn session_options(config: &Config, dry_run: bool) -> SessionOptions {
    SessionOptions {
        wake_words: match mode_of(config) {
            Mode::AlwaysListening => config.always_listening.wake_words.clone(),
            Mode::PushToTalk => Vec::new(),
        },
        exact_single_word: config.matching.exact_single_word,
        dry_run,
        hotwords: active_hotwords(config),
    }
}

/// The WAV files of `target`: the file itself, or the `*.wav` files of a
/// directory sorted by name.
pub fn wav_files(target: &Path) -> anyhow::Result<Vec<PathBuf>> {
    if target.is_file() {
        return Ok(vec![target.to_path_buf()]);
    }
    if !target.is_dir() {
        bail!("{} is not a file or a directory", target.display());
    }
    let mut files: Vec<PathBuf> = std::fs::read_dir(target)
        .with_context(|| format!("cannot read {}", target.display()))?
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.is_file() && p.extension().is_some_and(|x| x.eq_ignore_ascii_case("wav")))
        .collect();
    files.sort();
    Ok(files)
}

/// Runs each WAV file through the engine and the matcher as a dry run and
/// gives one line for each file: the file name, then the result line of
/// `run`. A file that cannot be read gives an `error:` line. Returns the
/// number of files that failed: a read error or a speech engine error.
pub fn replay_files(
    engine: Box<dyn SttEngine>,
    pipeline: Arc<Pipeline>,
    mut options: SessionOptions,
    files: &[PathBuf],
    mut emit: impl FnMut(String),
) -> usize {
    let mut failed = 0;
    // A replay never writes the request file.
    options.dry_run = true;
    let mut session = Session::new(engine, pipeline, options);
    for path in files {
        let name = path.file_name().map_or_else(
            || path.display().to_string(),
            |n| n.to_string_lossy().to_string(),
        );
        let line = match wav::read_file(path) {
            Ok(clip) => {
                let start = Instant::now();
                let outcome = session.on_utterance(&clip);
                let total = start.elapsed();
                let transcribe = session.last_transcribe_time().unwrap_or_default();
                if matches!(outcome, Outcome::EngineError { .. }) {
                    failed += 1;
                }
                outcome_line(
                    &outcome,
                    transcribe,
                    total,
                    clip.duration(),
                    levels(&clip.samples),
                    Marks {
                        glitch: false,
                        hotwords: session.last_used_hotwords(),
                    },
                )
            }
            Err(e) => {
                failed += 1;
                format!("error: {e:#}")
            }
        };
        emit(format!("{name}: {line}"));
    }
    failed
}

/// `voicebridge replay`: each WAV file through the engine and the matcher,
/// as a dry run. Exit code 1 when every file failed.
pub fn replay(config_path: &Path, target: &Path) -> anyhow::Result<ExitCode> {
    let mut config = Config::load(config_path)?;
    let mut problems: Vec<String> = config.matching_problem().into_iter().collect();
    problems.extend(engine_problems(&config, env_var));
    if !problems.is_empty() {
        for problem in &problems {
            eprintln!("problem: {problem}");
        }
        return Ok(ExitCode::FAILURE);
    }
    let files = wav_files(target)?;
    if files.is_empty() {
        eprintln!("no WAV file in {}", target.display());
        return Ok(ExitCode::FAILURE);
    }
    // The dry run never sends. As a second guard, the writer points into a
    // temporary directory and not at the request file of EDCoPilot.
    let guard = tempfile::tempdir().context("cannot make a temporary directory")?;
    config.edcopilot.request_file = guard.path().join("replay.request.txt");
    let (pipeline, report) = build_pipeline(&config)?;
    for problem in &report.problems {
        eprintln!("warning: {problem}");
    }
    if refuse_confused_hotwords(&config, &pipeline) {
        return Ok(ExitCode::FAILURE);
    }
    let engine = make_engine(&config, env_var)?;
    println!("engine: {}", engine.name());
    println!("dry run: no request file is written");
    let options = session_options(&config, true);
    let failed = replay_files(engine, Arc::new(pipeline), options, &files, |line| {
        println!("{line}")
    });
    Ok(replay_exit_code(failed, files.len()))
}

/// Exit code 1 only when every file failed.
fn replay_exit_code(failed: usize, files: usize) -> ExitCode {
    if files > 0 && failed == files {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    }
}

/// `voicebridge text`: matches the phrase. Without `send` it is a dry run.
pub fn text(config_path: &Path, phrase: &str, send: bool) -> anyhow::Result<ExitCode> {
    let config = Config::load(config_path)?;
    // Only the matching settings: `text` needs no model, key or microphone.
    if let Some(problem) = config.matching_problem() {
        eprintln!("problem: {problem}");
        return Ok(ExitCode::FAILURE);
    }
    // Without the EDCoPilot folder there is no request file: a send stops.
    if let Some(problem) = config.edcopilot_dir_problem() {
        if send {
            eprintln!("problem: {problem}");
            return Ok(ExitCode::FAILURE);
        }
        eprintln!("warning: {problem}");
    }
    let (pipeline, report) = build_pipeline(&config)?;
    // The load warnings of the commands: a text that the program does not
    // send although the list says that the command works.
    for problem in report.problems.iter().chain(&report.command_warnings) {
        eprintln!("warning: {problem}");
    }
    let mut session = text_session(&config, Arc::new(pipeline), !send);
    println!("{}", session.on_text(phrase).log_line());
    Ok(ExitCode::SUCCESS)
}

/// The session options of `voicebridge text`: no wake word and no hotwords,
/// because a typed phrase has no audio.
pub fn text_options(config: &Config, dry_run: bool) -> SessionOptions {
    SessionOptions {
        wake_words: Vec::new(),
        exact_single_word: config.matching.exact_single_word,
        dry_run,
        hotwords: Vec::new(),
    }
}

/// The session of the phrase test of the window. It has the options of the
/// live session (`session_options`) as a dry run, so that a typed phrase
/// gets the wake word rule of the saved mode. Give the phrase to
/// `Session::on_heard_text`. It has no engine.
pub fn phrase_test_session(config: &Config, pipeline: Arc<Pipeline>) -> Session {
    let engine = Box::new(FakeEngine::new(Vec::new()));
    Session::new(engine, pipeline, session_options(config, true))
}

/// The session of `voicebridge text` for one typed phrase. It has no
/// engine: `on_text` never calls the engine.
pub fn text_session(config: &Config, pipeline: Arc<Pipeline>, dry_run: bool) -> Session {
    let engine = Box::new(FakeEngine::new(Vec::new()));
    Session::new(engine, pipeline, text_options(config, dry_run))
}

/// `voicebridge devices`: prints the input devices.
pub fn devices() -> ExitCode {
    let names = list_input_devices();
    if names.is_empty() {
        println!("no input device found");
    }
    for name in names {
        println!("{name}");
    }
    ExitCode::SUCCESS
}

/// `voicebridge inputs`: prints the pressed inputs each time they change.
pub fn inputs() -> anyhow::Result<ExitCode> {
    let stop = Arc::new(AtomicBool::new(false));
    let flag = Arc::clone(&stop);
    ctrlc::set_handler(move || flag.store(true, Ordering::SeqCst))
        .context("cannot set the Ctrl+C handler")?;
    println!("press keys or joystick buttons, Ctrl+C stops");
    let mut last: Option<Vec<String>> = None;
    let mut scanner = InputScanner::new();
    while !stop.load(Ordering::SeqCst) {
        let now: Vec<String> = scanner.scan().iter().map(ToString::to_string).collect();
        if last.as_ref() != Some(&now) {
            if now.is_empty() {
                println!("(nothing pressed)");
            } else {
                println!("{}", now.join(", "));
            }
            last = Some(now);
        }
        thread::sleep(INPUTS_POLL);
    }
    Ok(ExitCode::SUCCESS)
}

/// What wakes the main thread of `run`.
enum Wake {
    CtrlC,
    Stopped(StopReason),
}

/// `voicebridge run`: the voice session.
pub fn run(config_path: &Path, dry_run: bool) -> anyhow::Result<ExitCode> {
    let config = Config::load(config_path)?;
    let (wake_tx, wake_rx) = mpsc::channel::<Wake>();
    let stopped = wake_tx.clone();
    let sink = Arc::new(ConsoleSink::new(move |reason| {
        let _ = stopped.send(Wake::Stopped(reason));
    }));
    let key_lookup: KeyLookup = Arc::new(env_var);

    // The handler is set before the start, so that a Ctrl+C during the model
    // load stops the start. The runtime uses the same flag as its stop flag,
    // and it also sets it at a stream error.
    let stop = Arc::new(AtomicBool::new(false));
    let flag = Arc::clone(&stop);
    ctrlc::set_handler(move || {
        if flag.swap(true, Ordering::SeqCst) {
            // A second Ctrl+C does not wait for the clip in flight.
            eprintln!("stopped at once");
            std::process::exit(EXIT_FORCED_STOP);
        }
        let _ = wake_tx.send(Wake::CtrlC);
    })
    .context("cannot set the Ctrl+C handler")?;

    let options = RuntimeOptions { dry_run };
    let mut runtime = match Runtime::start_with_stop(&config, options, key_lookup, sink, stop) {
        Ok(runtime) => runtime,
        // A Ctrl+C during the start: exit code 0, as for a stop after it.
        Err(error) if error.cancelled => return Ok(ExitCode::SUCCESS),
        Err(error) => {
            print_start_error(&error);
            return Ok(ExitCode::FAILURE);
        }
    };

    let wake = wake_rx.recv();
    runtime.stop();
    Ok(match wake {
        Ok(Wake::Stopped(reason)) => stop_exit_code(&reason),
        Ok(Wake::CtrlC) | Err(_) => ExitCode::SUCCESS,
    })
}

/// Prints the problems of a start that failed, on stderr.
fn print_start_error(error: &StartError) {
    let prefix = if error.step_failed {
        "error"
    } else {
        "problem"
    };
    for problem in &error.problems {
        eprintln!("{prefix}: {problem}");
    }
}

/// Exit code 1 for each stop that the user did not ask for.
fn stop_exit_code(reason: &StopReason) -> ExitCode {
    match reason {
        StopReason::Requested => ExitCode::SUCCESS,
        StopReason::DeviceError(_) | StopReason::EngineError(_) | StopReason::InputError(_) => {
            ExitCode::FAILURE
        }
    }
}

/// The talk button that the configuration names.
pub(crate) enum TalkSource<'a> {
    Joystick(&'a Joystick),
    Key(&'a str),
}

impl fmt::Display for TalkSource<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TalkSource::Joystick(j) => write!(f, "joystick \"{}\" button {}", j.device, j.button),
            TalkSource::Key(key) => {
                write!(f, "key \"{key}\"")?;
                // The resolved key, so that a wrong key shows at the start.
                match KeyboardKey::check(key) {
                    Ok(Some(vk)) => write!(f, " (vk:{vk})"),
                    _ => Ok(()),
                }
            }
        }
    }
}

/// The joystick wins over the key. `None`: no talk button is set.
pub(crate) fn talk_source(config: &Config) -> Option<TalkSource<'_>> {
    let ptt = &config.push_to_talk;
    match (&ptt.joystick, &ptt.key) {
        (Some(j), _) => Some(TalkSource::Joystick(j)),
        (None, Some(key)) => Some(TalkSource::Key(key)),
        (None, None) => None,
    }
}

/// The `check` line of the talk button. `None` in `always_listening` mode
/// or when the push-to-talk part of the configuration has a problem.
fn push_to_talk_line(config: &Config) -> Option<String> {
    if mode_of(config) != Mode::PushToTalk || !config.push_to_talk_problems().is_empty() {
        return None;
    }
    talk_source(config).map(|source| format!("push-to-talk: {source}"))
}

pub(crate) fn mode_line(config: &Config, mode: Mode) -> String {
    match (mode, talk_source(config)) {
        (Mode::AlwaysListening, _) => "always listening".to_string(),
        (Mode::PushToTalk, Some(source)) => format!("push to talk, {source}"),
        (Mode::PushToTalk, None) => "push to talk".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use edcopilot_voicebridge_stt::AudioClip;

    /// `check_config` with an empty temporary folder of the saved report:
    /// a test never reads the report folder of the user.
    fn check_config(config: &Config, env: impl Fn(&str) -> Option<String>) -> CheckReport {
        let dir = tempfile::tempdir().unwrap();
        check_config_in(config, env, Some(dir.path()))
    }

    fn cloud_config(env_name: &str) -> Config {
        let text = format!(
            "[edcopilot]\ndir = 'ed'\n[stt]\nengine = 'openai_compatible'\n\
             [stt.openai_compatible]\napi_key_env = '{env_name}'\n"
        );
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("voicebridge.toml");
        std::fs::write(&file, text).unwrap();
        Config::load(&file).unwrap()
    }

    #[test]
    fn test_engine_problems_name_the_absent_key_variable() {
        let config = cloud_config("MY_STT_KEY");
        let problems = engine_problems(&config, |_| None);
        assert_eq!(problems.len(), 1, "{problems:?}");
        assert!(problems[0].contains("MY_STT_KEY"), "{problems:?}");

        let problems = engine_problems(&config, |_| Some("  ".to_string()));
        assert_eq!(problems.len(), 1, "{problems:?}");

        let secret = "sk-secret-value-123";
        let lookup = |name: &str| (name == "MY_STT_KEY").then(|| secret.to_string());
        assert!(engine_problems(&config, lookup).is_empty());
    }

    /// The text of master for an empty name with the cloud engine. The
    /// local engine needs no name.
    #[test]
    fn test_empty_variable_name_is_a_problem_of_the_cloud_engine_only() {
        let mut config = cloud_config("");
        assert_eq!(
            engine_problems(&config, |_| None),
            ["stt.openai_compatible.api_key_env is empty, it must name an environment variable"]
        );
        config.stt.engine = "parakeet".into();
        assert!(engine_problems(&config, |_| None).is_empty());
    }

    #[test]
    fn test_engine_problems_never_repeat_a_key_like_name() {
        let config = cloud_config("sk-proj-SECRET123");
        let problems = engine_problems(&config, |_| None);
        assert_eq!(problems.len(), 1, "{problems:?}");
        assert!(!problems[0].contains("SECRET"), "{problems:?}");
        assert!(problems[0].contains("api_key_env"), "{problems:?}");
        let keyed = engine_problems_keyed(&config, |_| None);
        assert_eq!(keyed[0].0, "stt.openai_compatible.api_key_env");
        // The engine start gives the same safe text.
        let error = make_engine(&config, |_| Some("sk-real".to_string()))
            .err()
            .unwrap();
        assert!(!format!("{error:#}").contains("SECRET"), "{error:#}");
    }

    #[test]
    fn test_api_key_is_trimmed() {
        let key = api_key("MY_STT_KEY", |_| Some("  sk-value-1\r\n".to_string()));
        assert_eq!(key.as_deref(), Ok("sk-value-1"));
    }

    #[test]
    fn test_engine_starts_with_a_key_that_ends_in_a_line_break() {
        let config = cloud_config("MY_STT_KEY");
        assert!(make_engine(&config, |_| Some("sk-value-1\r\n".to_string())).is_ok());
    }

    #[test]
    fn test_engine_error_never_shows_the_key() {
        let mut config = cloud_config("MY_STT_KEY");
        config.stt.openai_compatible.base_url = String::new();
        let secret = "sk-secret-value-123";
        let error = make_engine(&config, |_| Some(secret.to_string()))
            .err()
            .expect("an empty base_url is an error");
        let text = format!("{error:#} {error:?}");
        assert!(!text.contains(secret), "{text}");
    }

    #[test]
    fn test_engine_problems_model_dir_must_be_a_directory() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("voicebridge.toml");
        std::fs::write(
            &file,
            "[edcopilot]\ndir = 'ed'\n[stt.parakeet]\nmodel_dir = 'model'\nhotwords_file = ''\n",
        )
        .unwrap();
        let config = Config::load(&file).unwrap();
        let problems = engine_problems(&config, |_| None);
        assert_eq!(problems.len(), 1, "{problems:?}");
        assert!(problems[0].contains("model_dir"), "{problems:?}");
        std::fs::create_dir(dir.path().join("model")).unwrap();
        assert!(engine_problems(&config, |_| None).is_empty());
    }

    #[test]
    fn test_engine_problems_of_the_hotwords_file() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("voicebridge.toml");
        std::fs::write(
            &file,
            "[edcopilot]\ndir = 'ed'\n[stt.parakeet]\nmodel_dir = 'model'\nhotwords_file = 'hot.txt'\n",
        )
        .unwrap();
        std::fs::create_dir(dir.path().join("model")).unwrap();
        let config = Config::load(&file).unwrap();
        // The file is absent.
        let problems = engine_problems(&config, |_| None);
        assert_eq!(problems.len(), 1, "{problems:?}");
        assert!(problems[0].contains("hot.txt"), "{problems:?}");
        // A line with a separator.
        std::fs::write(dir.path().join("hot.txt"), "Sol\nSol/Siris\n").unwrap();
        let problems = engine_problems(&config, |_| None);
        assert_eq!(problems.len(), 1, "{problems:?}");
        assert!(problems[0].contains("Sol/Siris"), "{problems:?}");
        // A good file, and no tokens.txt to check the pieces against.
        std::fs::write(dir.path().join("hot.txt"), "Sol\nZeta\n").unwrap();
        assert!(engine_problems(&config, |_| None).is_empty());
        // The model pieces cannot spell "Zeta".
        std::fs::write(
            dir.path().join("model/tokens.txt"),
            "<blk> 0\n\u{2581}S 1\no 2\nl 3\n",
        )
        .unwrap();
        let problems = engine_problems(&config, |_| None);
        assert_eq!(problems.len(), 1, "{problems:?}");
        assert!(problems[0].contains("Zeta"), "{problems:?}");
        assert!(!problems[0].contains("Sol"), "{problems:?}");
    }

    /// A config with a hotword file, and a pipeline with the vocabulary names.
    fn hotword_setup(list: &str, names: &[&str]) -> (tempfile::TempDir, Config, Pipeline) {
        use edcopilot_voicebridge_core::{AliasMap, Matcher, Writer};
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("hot.txt"), list).unwrap();
        let file = dir.path().join("voicebridge.toml");
        std::fs::write(
            &file,
            "[edcopilot]\ndir = 'ed'\n[stt.parakeet]\nmodel_dir = 'model'\nhotwords_file = 'hot.txt'\n",
        )
        .unwrap();
        let config = Config::load(&file).unwrap();
        let mut vocab = std::collections::HashMap::new();
        vocab.insert(
            "EDCoPilotSystem1".to_string(),
            names.iter().map(|n| n.to_string()).collect(),
        );
        let matcher = Matcher::new(Vec::new(), vocab, AliasMap::new(), 0.8);
        let writer = Writer::new(dir.path().join("never.txt"));
        (dir, config, Pipeline::new(matcher, writer))
    }

    #[test]
    fn test_hotword_confusions_name_both_words() {
        let (_dir, config, pipeline) = hotword_setup("Sol\nSiris\n", &["Sirius", "Sol", "Los"]);
        let problems = hotword_confusions(&config, &pipeline);
        assert_eq!(problems.len(), 1, "{problems:?}");
        assert!(problems[0].contains("\"Siris\""), "{problems:?}");
        assert!(problems[0].contains("\"Sirius\""), "{problems:?}");
        assert!(refuse_confused_hotwords(&config, &pipeline));
        let (_dir, config, pipeline) = hotword_setup("Sol\n", &["Sirius", "Sol", "Los"]);
        assert!(hotword_confusions(&config, &pipeline).is_empty());
        assert!(!refuse_confused_hotwords(&config, &pipeline));
    }

    /// A hotword equal to or near a word of a command phrase is refused.
    #[test]
    fn test_hotword_near_a_command_word_is_refused() {
        use edcopilot_voicebridge_core::{AliasMap, Command, Matcher, Writer};
        let (dir, config, _) = hotword_setup(
            "Sail
Sol
",
            &[],
        );
        let command = Command {
            phrase: Some("sail out to {TXT:System1}".to_string()),
            token_template: "Go:{TXT:System1}".to_string(),
            ..Default::default()
        };
        let matcher = Matcher::new(vec![command], Default::default(), AliasMap::new(), 0.8);
        let pipeline = Pipeline::new(matcher, Writer::new(dir.path().join("never.txt")));
        let problems = hotword_confusions(&config, &pipeline);
        assert_eq!(problems.len(), 1, "{problems:?}");
        assert!(problems[0].contains("\"Sail\""), "{problems:?}");
        assert!(
            problems[0].contains("command word \"sail\""),
            "{problems:?}"
        );
    }

    #[test]
    fn test_empty_hotword_file_gives_a_warning() {
        let (_dir, config, _) = hotword_setup("# only a comment\n", &[]);
        let warning = empty_hotwords_warning(&config).unwrap();
        assert!(warning.contains("no hotword"), "{warning}");
        let (_dir, config, _) = hotword_setup("Sol\n", &[]);
        assert_eq!(empty_hotwords_warning(&config), None);
        assert_eq!(active_hotwords(&config), ["Sol"]);
    }

    #[test]
    fn test_hotwords_line_names_the_list() {
        let h = Hotwords::parse("Sol\nSiris\n", 2.0).unwrap();
        assert_eq!(
            hotwords_line(Some(&h), Path::new("core/data/hotwords.txt")),
            "hotwords: 2 from core/data/hotwords.txt, score 2.0"
        );
        assert_eq!(hotwords_line(None, Path::new("")), "hotwords: off");
    }

    #[test]
    fn test_talk_source_prefers_the_joystick() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("voicebridge.toml");
        std::fs::write(&file, "[edcopilot]\ndir = 'ed'\n").unwrap();
        let mut config = Config::load(&file).unwrap();
        assert!(matches!(talk_source(&config), Some(TalkSource::Key("F13"))));
        config.push_to_talk.joystick = Some(crate::config::Joystick {
            device: "MOZA".to_string(),
            button: 5,
        });
        let source = talk_source(&config).unwrap();
        assert!(matches!(source, TalkSource::Joystick(_)));
        assert_eq!(source.to_string(), "joystick \"MOZA\" button 5");
        config.push_to_talk.key = None;
        config.push_to_talk.joystick = None;
        assert!(talk_source(&config).is_none());
    }

    #[test]
    fn test_mode_line_shows_the_key_as_configured() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("voicebridge.toml");
        std::fs::write(
            &file,
            "[edcopilot]\ndir = 'ed'\n[push_to_talk]\nkey = 'rcontrol'\n",
        )
        .unwrap();
        let mut config = Config::load(&file).unwrap();
        // The key as configured, and on Windows the resolved virtual key.
        let expected = if cfg!(windows) {
            "key \"rcontrol\" (vk:163)"
        } else {
            "key \"rcontrol\""
        };
        assert_eq!(
            mode_line(&config, Mode::PushToTalk),
            format!("push to talk, {expected}")
        );
        assert_eq!(
            push_to_talk_line(&config),
            Some(format!("push-to-talk: {expected}"))
        );
        assert_eq!(
            mode_line(&config, Mode::AlwaysListening),
            "always listening"
        );
        config.push_to_talk.key = Some("NoSuchKey".to_string());
        assert_eq!(push_to_talk_line(&config), None);
        config.push_to_talk.key = None;
        config.push_to_talk.joystick = Some(crate::config::Joystick {
            device: "MOZA".to_string(),
            button: 5,
        });
        assert_eq!(
            push_to_talk_line(&config).as_deref(),
            Some("push-to-talk: joystick \"MOZA\" button 5")
        );
        config.microphone.mode = "always_listening".to_string();
        assert_eq!(push_to_talk_line(&config), None);
    }

    #[test]
    fn test_outcome_line_has_both_times() {
        let outcome = Outcome::NoMatch {
            text: "hello".to_string(),
            blocked: None,
        };
        let ms = Duration::from_millis;
        let normal = Levels {
            peak_dbfs: -6.4,
            rms_dbfs: -23.6,
        };
        let line = outcome_line(
            &outcome,
            ms(120),
            ms(345),
            ms(2_140),
            normal,
            Marks::default(),
        );
        assert_eq!(
            line,
            "no match for \"hello\" (transcribe 120 ms, total 345 ms, clip 2140 ms, \
             peak -6 dBFS, rms -24 dBFS)"
        );
    }

    #[test]
    fn test_outcome_line_marks_clipping_and_very_quiet() {
        let outcome = Outcome::NoMatch {
            text: "hello".to_string(),
            blocked: None,
        };
        let ms = Duration::from_millis;
        let line = |peak_dbfs: f32| {
            let levels = Levels {
                peak_dbfs,
                rms_dbfs: -30.0,
            };
            outcome_line(&outcome, ms(1), ms(2), ms(3), levels, Marks::default())
        };
        assert!(
            line(-0.1).ends_with("peak 0 dBFS, rms -30 dBFS, clipping)"),
            "{}",
            line(-0.1)
        );
        assert!(line(0.0).ends_with(", clipping)"), "{}", line(0.0));
        assert!(!line(-0.2).contains("clipping"), "{}", line(-0.2));
        assert!(!line(-0.2).contains("very quiet"), "{}", line(-0.2));
        assert!(!line(-40.0).contains("very quiet"), "{}", line(-40.0));
        assert!(!line(-50.0).contains("very quiet"), "{}", line(-50.0));
        assert!(line(-50.1).ends_with(", very quiet)"), "{}", line(-50.1));
        assert!(line(-120.0).contains("peak -120 dBFS"), "{}", line(-120.0));
    }

    fn write_wav(path: &Path) {
        let clip = AudioClip {
            samples: vec![0.1; 1_600],
        };
        std::fs::write(path, wav::encode(&clip)).unwrap();
    }

    #[test]
    fn test_wav_files_of_a_directory_are_sorted() {
        let dir = tempfile::tempdir().unwrap();
        write_wav(&dir.path().join("b.wav"));
        write_wav(&dir.path().join("a.WAV"));
        std::fs::write(dir.path().join("notes.txt"), "x").unwrap();
        let files = wav_files(dir.path()).unwrap();
        let names: Vec<String> = files
            .iter()
            .map(|p| p.file_name().unwrap().to_string_lossy().to_string())
            .collect();
        assert_eq!(names, vec!["a.WAV", "b.wav"]);
        assert_eq!(wav_files(&files[1]).unwrap(), vec![files[1].clone()]);
        assert!(wav_files(&dir.path().join("none")).is_err());
    }

    #[test]
    fn test_replay_is_a_dry_run_with_one_line_for_each_file() {
        use edcopilot_voicebridge_core::{load_sound_alikes, AliasMap, Matcher, Writer};
        let dir = tempfile::tempdir().unwrap();
        let request = dir.path().join("EDCoPilot.request.txt");
        let a = dir.path().join("utt-1.wav");
        let b = dir.path().join("utt-2.wav");
        write_wav(&a);
        write_wav(&b);
        // A stereo file gives the existing format error.
        let stereo = dir.path().join("utt-3.wav");
        let spec = hound::WavSpec {
            channels: 2,
            sample_rate: 44_100,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };
        let mut writer = hound::WavWriter::create(&stereo, spec).unwrap();
        writer.write_sample(0i16).unwrap();
        writer.write_sample(0i16).unwrap();
        writer.finalize().unwrap();

        let core = Path::new(env!("CARGO_MANIFEST_DIR")).join("../core");
        let commands = crate::commands_source::test_support::fixture_loaded_commands();
        let matcher = Matcher::new(commands, Default::default(), AliasMap::new(), 0.8);
        let table = load_sound_alikes(&core.join("data/sound_alikes.toml")).unwrap();
        let writer = Writer::new(request.clone()).confirm_timeout(None);
        let pipeline = Arc::new(Pipeline::new(matcher, writer).with_sound_alikes(table));
        // A sound-alike match ("by" -> "buy" in the composite prefix of the
        // fixture) and an exact match.
        let engine = Box::new(FakeEngine::new(vec![
            Ok("cheapest by bid cork".into()),
            Ok("help tide log".into()),
        ]));
        let options = SessionOptions {
            wake_words: Vec::new(),
            exact_single_word: true,
            // replay_files sets the dry run itself.
            dry_run: false,
            hotwords: Vec::new(),
        };
        let mut lines = Vec::new();
        let failed = replay_files(engine, pipeline, options, &[a, b, stereo], |l| {
            lines.push(l)
        });
        assert_eq!(failed, 1);
        assert_eq!(lines.len(), 3, "{lines:?}");
        let first =
            "utt-1.wav: dry run \"cheapest by bid cork\" -> BuyTop:Cork (0.95) (sound-alike)";
        assert!(lines[0].starts_with(first), "{lines:?}");
        assert!(lines[0].contains("transcribe "), "{lines:?}");
        // 1_600 samples of 0.1: 100 ms at -20 dBFS.
        assert!(
            lines[0].contains(", clip 100 ms, peak -20 dBFS, rms -20 dBFS)"),
            "{lines:?}"
        );
        let second = "utt-2.wav: dry run \"help tide log\" -> Guide:Panel:TideLog";
        assert!(lines[1].starts_with(second), "{lines:?}");
        assert!(lines[2].starts_with("utt-3.wav: error:"), "{lines:?}");
        assert!(lines[2].contains("44100"), "{lines:?}");
        assert!(!request.exists(), "replay wrote the request file");
    }

    #[test]
    fn test_diagnostics_lines_show_only_set_parts() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("voicebridge.toml");
        std::fs::write(&file, "[edcopilot]\ndir = 'ed'\n").unwrap();
        let mut config = Config::load(&file).unwrap();
        assert!(diagnostics_lines(&config).is_empty());
        config.diagnostics.log_file = dir.path().join("u.tsv");
        config.diagnostics.save_audio_dir = dir.path().join("clips");
        let lines = diagnostics_lines(&config);
        assert_eq!(lines.len(), 2);
        assert!(lines[0].starts_with("utterance log: "));
        assert!(lines[1].starts_with("audio capture: "));
    }

    #[test]
    fn test_outcome_line_shows_a_glitch() {
        let outcome = Outcome::NoMatch {
            text: "x".into(),
            blocked: None,
        };
        let normal = Levels {
            peak_dbfs: -10.0,
            rms_dbfs: -20.0,
        };
        let ms = Duration::from_millis;
        let glitch = Marks {
            glitch: true,
            hotwords: false,
        };
        let line = outcome_line(&outcome, ms(1), ms(2), ms(3), normal, glitch);
        assert!(line.ends_with("rms -20 dBFS, glitch)"), "{line}");
        let line = outcome_line(&outcome, ms(1), ms(2), ms(3), normal, Marks::default());
        assert!(!line.contains("glitch"), "{line}");
        let both = Marks {
            glitch: true,
            hotwords: true,
        };
        let line = outcome_line(&outcome, ms(1), ms(2), ms(3), normal, both);
        assert!(line.ends_with("rms -20 dBFS, hotwords, glitch)"), "{line}");
    }

    #[test]
    fn test_check_warns_when_the_key_goes_without_encryption() {
        let mut config = cloud_config("MY_STT_KEY");
        let warning = "the cloud engine sends the API key without encryption";
        let has = |config: &Config| {
            check_config(config, |_| None)
                .warnings
                .iter()
                .any(|w| w == warning)
        };
        assert!(!has(&config), "https gives no warning");
        config.stt.openai_compatible.base_url = "http://stt.example.org/v1".into();
        assert!(has(&config));
        config.stt.openai_compatible.base_url = "http://127.0.0.1:8000/v1".into();
        assert!(!has(&config), "loopback gives no warning");
        // The local engine sends no key.
        config.stt.openai_compatible.base_url = "http://stt.example.org/v1".into();
        config.stt.engine = "parakeet".into();
        assert!(!has(&config));
    }

    #[test]
    fn test_check_gives_the_variable_name_problem_one_time() {
        let config = cloud_config("sk_not_a_name_1234567890abcdef");
        let found = check_config(&config, |_| None);
        let count = found
            .problems
            .iter()
            .filter(|p| *p == API_KEY_ENV_PROBLEM)
            .count();
        assert_eq!(count, 1, "{:?}", found.problems);
    }

    #[test]
    fn test_check_config_gives_the_data_of_check() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("voicebridge.toml");
        std::fs::write(
            &file,
            "[edcopilot]\ndir = 'ed'\n[matching]\nthreshold = 2.0\n[diagnostics]\nlog_file = 'u.tsv'\n",
        )
        .unwrap();
        let config = Config::load(&file).unwrap();
        let found = check_config(&config, |_| None);
        // The EDCoPilot profile is absent: no report, and a problem for it.
        assert!(found.report.is_none());
        assert!(
            found.problems[0].contains("matching.threshold"),
            "{found:?}"
        );
        assert!(
            found.problems.iter().any(|p| p.contains("model_dir")),
            "{found:?}"
        );
        assert!(
            found.info_lines[0].starts_with("push-to-talk: "),
            "{found:?}"
        );
        assert!(
            found.info_lines[1].starts_with("utterance log: "),
            "{found:?}"
        );
        // The load problem is kept apart too: the window shows it at the
        // field of the EDCoPilot folder.
        let load = found.load_problem.as_deref().expect("a load problem");
        assert!(
            load.starts_with("cannot read the EDCoPilot profile "),
            "{load}"
        );
        assert!(found.problems.iter().any(|p| p == load), "{found:?}");
    }

    #[test]
    fn test_check_has_no_load_problem_when_the_commands_load() {
        let dir = tempfile::tempdir().unwrap();
        let config = crate::commands_source::test_support::fixture_config(dir.path(), "");
        let found = check_config_in(&config, |_| Some("key".into()), None);
        assert!(found.report.is_some(), "{found:?}");
        assert_eq!(found.load_problem, None);
    }

    /// `voicebridge text` and the phrase test of the window use these
    /// options: no wake word and no hotwords, also in always_listening mode.
    #[test]
    fn test_text_options_have_no_wake_word_and_no_hotwords() {
        let (_dir, mut config, _) = hotword_setup("Sol\n", &[]);
        config.microphone.mode = "always_listening".into();
        config.matching.exact_single_word = false;
        let options = text_options(&config, true);
        assert!(options.wake_words.is_empty());
        assert!(options.hotwords.is_empty());
        assert!(options.dry_run);
        assert!(!options.exact_single_word);
        assert!(!text_options(&config, false).dry_run);
    }

    /// The phrase test of the window uses the options of the live session:
    /// in always_listening mode the wake word is required.
    #[test]
    fn test_phrase_test_session_has_the_wake_word_rule_of_the_live_session() {
        let (_dir, mut config, pipeline) = hotword_setup("Sol\n", &[]);
        let pipeline = Arc::new(pipeline);
        config.microphone.mode = "always_listening".into();
        config.always_listening.wake_words = vec!["copilot".into()];
        let mut session = phrase_test_session(&config, Arc::clone(&pipeline));
        assert!(matches!(
            session.on_heard_text("help chart log"),
            Outcome::NoWakeWord { .. }
        ));
        assert!(matches!(
            session.on_heard_text("copilot help chart log"),
            Outcome::NoMatch { .. }
        ));
        config.microphone.mode = "push_to_talk".into();
        let mut session = phrase_test_session(&config, pipeline);
        assert!(matches!(
            session.on_heard_text("help chart log"),
            Outcome::NoMatch { .. }
        ));
    }

    #[test]
    fn test_replay_exit_code_is_1_only_when_every_file_failed() {
        assert_eq!(replay_exit_code(2, 2), ExitCode::FAILURE);
        assert_eq!(replay_exit_code(1, 2), ExitCode::SUCCESS);
        assert_eq!(replay_exit_code(0, 2), ExitCode::SUCCESS);
    }
}
