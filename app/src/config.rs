//! Configuration file, validation, and the startup load report.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use anyhow::Context;
use edcopilot_voicebridge_audio::KeyboardKey;
use edcopilot_voicebridge_core::profile::{BlockingPhrase, BuildReport, LeftOut, LeftOutClass};
use edcopilot_voicebridge_core::{
    filler_name_warnings, load_alias_files, load_dynamic_vocab_checked, load_fillers_with_problems,
    load_sound_alikes_with_problems, load_synonyms_with_problems, merge_aliases,
    needs_spoken_command, removed_as_blocking, resolve_blocking_from, resolve_profile_vars,
    resolve_runtime_grammar_from, shadowed_phrases_with, slot_keys, synonym_key_warnings,
    unknown_form_warning, Command, CommandClass, Matcher, Pipeline, RemovedCommand, RuntimeGrammar,
    Writer,
};
use edcopilot_voicebridge_stt::hotwords;
use serde::Deserialize;

use crate::commands_source::{build_profile, ProfileSource, PROFILE_FILE};

const VAR_PREFIX: &str = "{TXT:EDCP_";

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    pub edcopilot: EdcopilotConfig,
    #[serde(default)]
    pub data: DataConfig,
    #[serde(default)]
    pub matching: MatchingConfig,
    #[serde(default)]
    pub microphone: MicrophoneConfig,
    #[serde(default)]
    pub push_to_talk: PushToTalkConfig,
    #[serde(default)]
    pub always_listening: AlwaysListeningConfig,
    #[serde(default)]
    pub stt: SttConfig,
    #[serde(default)]
    pub diagnostics: DiagnosticsConfig,
    /// The paths that the console shows as the file gives them. See
    /// `Config::shown`.
    #[serde(skip)]
    shown: Vec<(PathBuf, PathBuf)>,
}

/// An empty path for `request_file`, `voice_commands_ini`,
/// `dynamic_vars_dir`, or `profile` means "not set". `Config::load` fills it
/// from `dir`.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EdcopilotConfig {
    pub dir: PathBuf,
    #[serde(default)]
    pub request_file: PathBuf,
    #[serde(default)]
    pub voice_commands_ini: PathBuf,
    #[serde(default)]
    pub dynamic_vars_dir: PathBuf,
    /// The VoiceAttack profile of EDCoPilot. The default is
    /// `<dir>\EDCoPilot-Profile.vap`. The program reads the commands from
    /// it.
    #[serde(default)]
    pub profile: PathBuf,
    /// True when the file sets `profile`. False when the path comes from
    /// `dir`. The problem text of a profile that cannot be read names the
    /// key to change (see `ProfileSource`).
    #[serde(skip)]
    pub profile_from_key: bool,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct DataConfig {
    /// The key `data.command_map` of an older version, with any value. The
    /// program does not use it: it reads the commands from the EDCoPilot
    /// profile. A file with the key loads, and `OLD_MAP_KEYS` tells the
    /// user one time.
    #[serde(rename = "command_map")]
    pub old_command_map: Option<toml::Value>,
    /// The key `data.profile_vars` of an older version. As
    /// `old_command_map`.
    #[serde(rename = "profile_vars")]
    pub old_profile_vars: Option<toml::Value>,
    pub aliases_dir: PathBuf,
    /// The sound-alike table. A missing file gives an empty table.
    pub sound_alikes: PathBuf,
    /// The phrase synonyms. A missing file gives an empty table.
    pub synonyms: PathBuf,
    /// The filler words. A missing file gives an empty table.
    pub fillers: PathBuf,
}

impl Default for DataConfig {
    fn default() -> Self {
        DataConfig {
            old_command_map: None,
            old_profile_vars: None,
            aliases_dir: "core/aliases".into(),
            sound_alikes: "core/data/sound_alikes.toml".into(),
            synonyms: "core/data/synonyms.toml".into(),
            fillers: "core/data/fillers.toml".into(),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct MatchingConfig {
    pub threshold: f32,
    /// A phrase of one word needs confidence 1.0.
    pub exact_single_word: bool,
}

impl Default for MatchingConfig {
    fn default() -> Self {
        MatchingConfig {
            threshold: 0.8,
            exact_single_word: true,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct MicrophoneConfig {
    /// Empty: the default device.
    pub device: String,
    /// `push_to_talk` or `always_listening`.
    pub mode: String,
    pub vad_threshold_dbfs: f32,
}

impl Default for MicrophoneConfig {
    fn default() -> Self {
        MicrophoneConfig {
            device: String::new(),
            mode: "push_to_talk".to_string(),
            vad_threshold_dbfs: -40.0,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Joystick {
    pub device: String,
    pub button: u32,
}

/// `key` gets the default `F13` in `Config::load` when `joystick` is also unset.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PushToTalkConfig {
    pub key: Option<String>,
    pub joystick: Option<Joystick>,
    /// Audio from before the press that the recording starts with. 0 is off.
    #[serde(default = "default_pre_roll_ms")]
    pub pre_roll_ms: u32,
    /// Audio after the release that the recording keeps. 0 is off.
    #[serde(default = "default_release_tail_ms")]
    pub release_tail_ms: u32,
}

/// The largest value of `pre_roll_ms` and `release_tail_ms`.
pub const MAX_PTT_EXTRA_MS: u32 = 2_000;

fn default_pre_roll_ms() -> u32 {
    250
}

fn default_release_tail_ms() -> u32 {
    300
}

impl Default for PushToTalkConfig {
    fn default() -> Self {
        PushToTalkConfig {
            key: None,
            joystick: None,
            pre_roll_ms: default_pre_roll_ms(),
            release_tail_ms: default_release_tail_ms(),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct AlwaysListeningConfig {
    /// An empty list means no wake word.
    pub wake_words: Vec<String>,
}

impl Default for AlwaysListeningConfig {
    fn default() -> Self {
        AlwaysListeningConfig {
            wake_words: vec!["copilot".to_string(), "co pilot".to_string()],
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct SttConfig {
    /// `parakeet` or `openai_compatible`.
    pub engine: String,
    pub parakeet: ParakeetConfig,
    pub openai_compatible: OpenAiCompatibleConfig,
}

impl Default for SttConfig {
    fn default() -> Self {
        SttConfig {
            engine: "parakeet".to_string(),
            parakeet: ParakeetConfig::default(),
            openai_compatible: OpenAiCompatibleConfig::default(),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct ParakeetConfig {
    /// Required when the engine is `parakeet`. An empty path means not set.
    pub model_dir: PathBuf,
    pub threads: usize,
    /// The hotword list (one phrase on each line). An empty path switches
    /// the hotwords off, and the engine uses greedy search.
    pub hotwords_file: PathBuf,
    /// The score that the decoder adds for each hotword piece.
    pub hotwords_score: f32,
}

/// The default hotword list. Empty: the hotwords are off. The shipped list
/// is `core/data/hotwords.txt`. The default is off because a measurement
/// with real clips gave no clear gain (1 of 21 labelled clips of a user).
pub const DEFAULT_HOTWORDS_FILE: &str = "";
/// The default hotword score.
pub const DEFAULT_HOTWORDS_SCORE: f32 = 1.5;

impl Default for ParakeetConfig {
    fn default() -> Self {
        ParakeetConfig {
            model_dir: PathBuf::new(),
            threads: logical_cores().min(4),
            hotwords_file: PathBuf::from(DEFAULT_HOTWORDS_FILE),
            hotwords_score: DEFAULT_HOTWORDS_SCORE,
        }
    }
}

/// Holds the NAME of the environment variable for the key, never the key.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct OpenAiCompatibleConfig {
    pub base_url: String,
    pub api_key_env: String,
    pub model: String,
    /// Empty: the service detects the language.
    pub language: String,
    pub timeout_seconds: u64,
}

impl Default for OpenAiCompatibleConfig {
    fn default() -> Self {
        OpenAiCompatibleConfig {
            base_url: "https://api.openai.com/v1".to_string(),
            api_key_env: "OPENAI_API_KEY".to_string(),
            model: "gpt-4o-mini-transcribe".to_string(),
            language: String::new(),
            timeout_seconds: 10,
        }
    }
}

/// Data for tuning. An empty path switches the part off.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct DiagnosticsConfig {
    /// One line for each utterance is added to this file.
    pub log_file: PathBuf,
    /// Each utterance clip is saved as a WAV file in this directory.
    pub save_audio_dir: PathBuf,
}

fn logical_cores() -> usize {
    std::thread::available_parallelism().map_or(1, |n| n.get())
}

fn absolute(base: &Path, path: &Path) -> PathBuf {
    if path.as_os_str().is_empty() || path.is_absolute() {
        path.to_path_buf()
    } else {
        base.join(path)
    }
}

/// The folder of the configuration file as an absolute path. A relative
/// path (`--config voicebridge.toml`) starts at the current folder. The path
/// is not made canonical, so it gets no `\\?\` prefix.
pub fn config_folder(path: &Path) -> PathBuf {
    let given = path.parent().unwrap_or_else(|| Path::new(""));
    std::path::absolute(path)
        .ok()
        .and_then(|file| file.parent().map(Path::to_path_buf))
        .unwrap_or_else(|| given.to_path_buf())
}

impl Config {
    /// Read and parse the file. Relative paths start at the folder of the
    /// file, as an absolute path: each derived path is absolute.
    pub fn load(path: &Path) -> anyhow::Result<Config> {
        let text = std::fs::read_to_string(path)
            .with_context(|| format!("cannot read configuration file {}", path.display()))?;
        Config::parse_file(&text, path)
    }

    /// Parse the text of the file `path`, as `load` does.
    pub fn parse_file(text: &str, path: &Path) -> anyhow::Result<Config> {
        let given = path.parent().unwrap_or_else(|| Path::new(""));
        let name = path.display().to_string();
        let mut config = Config::parse(text, &config_folder(path), &name)?;
        // The console shows a relative path of the file as before: relative
        // to the folder in the configuration path.
        let mut shown = Config::from_toml(text, &name)?;
        shown.resolve_paths(given);
        let pairs = [
            (&config.diagnostics.log_file, shown.diagnostics.log_file),
            (
                &config.diagnostics.save_audio_dir,
                shown.diagnostics.save_audio_dir,
            ),
            (
                &config.stt.parakeet.hotwords_file,
                shown.stt.parakeet.hotwords_file,
            ),
        ];
        config.shown = pairs
            .into_iter()
            .filter(|(full, shown)| *full != shown)
            .map(|(full, shown)| (full.clone(), shown))
            .collect();
        Ok(config)
    }

    /// The path as the console shows it: a path of the file that was
    /// relative stays relative to the folder in the configuration path, as
    /// it was before the folder was made absolute. A path that changed
    /// after the load is shown as it is.
    pub fn shown<'a>(&'a self, path: &'a Path) -> &'a Path {
        self.shown
            .iter()
            .find(|(full, _)| full == path)
            .map_or(path, |(_, shown)| shown.as_path())
    }

    /// Read and parse the file, with the values as the file has them. The
    /// paths stay relative, the paths that default from `edcopilot.dir` stay
    /// empty, and the default talk key is not set. The user interface shows
    /// these values. Use `load` for a session.
    pub fn load_as_written(path: &Path) -> anyhow::Result<Config> {
        Config::load_as_written_text(path).map(|(config, _)| config)
    }

    /// `load_as_written`, and the text of the file.
    pub fn load_as_written_text(path: &Path) -> anyhow::Result<(Config, String)> {
        let text = std::fs::read_to_string(path)
            .with_context(|| format!("cannot read configuration file {}", path.display()))?;
        let config = Config::from_toml(&text, &path.display().to_string())?;
        Ok((config, text))
    }

    /// `name` is the file name for error texts.
    pub(crate) fn parse(text: &str, base: &Path, name: &str) -> anyhow::Result<Config> {
        let mut config = Config::from_toml(text, name)?;
        config.resolve_paths(base);
        Ok(config)
    }

    /// The configuration in `text`, with no change. The error text has no
    /// line and no value of the file (see `toml_error`).
    fn from_toml(text: &str, name: &str) -> anyhow::Result<Config> {
        toml::from_str(text).map_err(|e| {
            anyhow::anyhow!(
                "invalid configuration file {name}: {}",
                crate::toml_error::de_error_text(&e, text)
            )
        })
    }

    fn resolve_paths(&mut self, base: &Path) {
        let ed = &mut self.edcopilot;
        ed.dir = absolute(base, &ed.dir);
        let empty = |p: &Path| p.as_os_str().is_empty();
        // Without a folder, a path that defaults from it stays empty. Else
        // it would be a path in the current folder.
        let no_dir = empty(&ed.dir);
        let derived = |path: &Path, default: &dyn Fn(&Path) -> PathBuf| {
            if !empty(path) {
                absolute(base, path)
            } else if no_dir {
                PathBuf::new()
            } else {
                default(&ed.dir)
            }
        };
        let request_file = derived(&ed.request_file, &|d| d.join("EDCoPilot.request.txt"));
        let voice_commands_ini = derived(&ed.voice_commands_ini, &|d| {
            d.join("working").join("EDCoPilot.VoiceCommands.ini")
        });
        let dynamic_vars_dir = derived(&ed.dynamic_vars_dir, &|d| d.join("Dynamic var files"));
        ed.profile_from_key = !empty(&ed.profile);
        let profile = derived(&ed.profile, &|d| d.join(PROFILE_FILE));
        ed.request_file = request_file;
        ed.voice_commands_ini = voice_commands_ini;
        ed.dynamic_vars_dir = dynamic_vars_dir;
        ed.profile = profile;
        self.data.aliases_dir = absolute(base, &self.data.aliases_dir);
        self.data.sound_alikes = absolute(base, &self.data.sound_alikes);
        self.data.synonyms = absolute(base, &self.data.synonyms);
        self.data.fillers = absolute(base, &self.data.fillers);
        self.stt.parakeet.model_dir = absolute(base, &self.stt.parakeet.model_dir);
        self.stt.parakeet.hotwords_file = absolute(base, &self.stt.parakeet.hotwords_file);
        let diagnostics = &mut self.diagnostics;
        diagnostics.log_file = absolute(base, &diagnostics.log_file);
        diagnostics.save_audio_dir = absolute(base, &diagnostics.save_audio_dir);
        if self.push_to_talk.key.is_none() && self.push_to_talk.joystick.is_none() {
            self.push_to_talk.key = Some("F13".to_string());
        }
    }

    /// The problem when `edcopilot.dir` is empty and a path that defaults
    /// from it is empty too. Then the program has no request file, no voice
    /// commands file, no vocabulary folder, or no profile, and it reads
    /// nothing in their place. A configuration that sets the four paths by
    /// hand needs no folder. When only the profile path is empty, the text
    /// names `edcopilot.profile`.
    pub fn edcopilot_dir_problem(&self) -> Option<&'static str> {
        let ed = &self.edcopilot;
        let empty = |p: &Path| p.as_os_str().is_empty();
        if !empty(&ed.dir) {
            return None;
        }
        if empty(&ed.request_file) || empty(&ed.voice_commands_ini) || empty(&ed.dynamic_vars_dir) {
            return Some(EDCOPILOT_DIR_EMPTY);
        }
        empty(&ed.profile).then_some(EDCOPILOT_PROFILE_EMPTY)
    }

    /// True when the file has `data.command_map` or `data.profile_vars`, the
    /// keys of an older version. The program does not use them.
    pub fn has_old_map_keys(&self) -> bool {
        self.data.old_command_map.is_some() || self.data.old_profile_vars.is_some()
    }

    /// `OLD_MAP_KEYS` when the file has a key of `has_old_map_keys`.
    pub fn old_map_keys_warning(&self) -> Option<String> {
        self.has_old_map_keys().then(|| OLD_MAP_KEYS.to_string())
    }

    /// The problem of `matching.threshold`, if any. `validate` includes it.
    pub fn matching_problem(&self) -> Option<String> {
        let threshold = self.matching.threshold;
        (!(0.0..=1.0).contains(&threshold))
            .then(|| format!("matching.threshold is {threshold}, it must be from 0.0 to 1.0"))
    }

    /// One text for each problem. An empty list means the configuration is valid.
    pub fn validate(&self) -> Vec<String> {
        self.validate_keyed()
            .into_iter()
            .map(|(_, text)| text)
            .collect()
    }

    /// `validate` with the key of each problem, for example
    /// `("matching.threshold", "matching.threshold is 7, ...")`. A problem of
    /// a whole table has the name of the table as its key. The texts and
    /// their order are the texts of `validate`.
    pub fn validate_keyed(&self) -> Vec<(&'static str, String)> {
        let mut problems: Vec<(&'static str, String)> = Vec::new();
        // A new configuration of the user interface has no folder when
        // C:\EDCoPilot does not exist.
        if let Some(problem) = self.edcopilot_dir_problem() {
            problems.push(("edcopilot.dir", problem.to_string()));
        }
        let request = &self.edcopilot.request_file;
        if !request.as_os_str().is_empty() && !request.is_absolute() {
            problems.push(("edcopilot.request_file", request_file_problem(request)));
        }
        problems.extend(
            self.matching_problem()
                .map(|text| ("matching.threshold", text)),
        );
        match self.microphone.mode.as_str() {
            "push_to_talk" => problems.extend(self.push_to_talk_keyed()),
            "always_listening" => problems.extend(self.push_to_talk_timing_keyed()),
            other => problems.push((
                "microphone.mode",
                format!(
                    "microphone.mode is \"{other}\", it must be \"push_to_talk\" or \"always_listening\""
                ),
            )),
        }
        match self.stt.engine.as_str() {
            "parakeet" => self.validate_parakeet(&mut problems),
            "openai_compatible" => {}
            other => problems.push((
                "stt.engine",
                format!(
                    "stt.engine is \"{other}\", it must be \"parakeet\" or \"openai_compatible\""
                ),
            )),
        }
        // Only the always-listening mode uses the voice threshold.
        let vad = self.microphone.vad_threshold_dbfs;
        if self.microphone.mode == "always_listening" && !vad.is_finite() {
            problems.push((
                "microphone.vad_threshold_dbfs",
                format!("microphone.vad_threshold_dbfs is {vad}, it must be a finite number"),
            ));
        }
        problems.extend(self.cloud_keyed());
        problems
    }

    /// The problems of `[stt.openai_compatible]`. A value that can be a
    /// secret is a problem for both engines, because it sits in the file: a
    /// key in `api_key_env` and a user name or a password in `base_url`. The
    /// other checks apply only when the cloud engine is selected. An empty
    /// `api_key_env` is not a problem here: the cloud engine reports the
    /// absent key at its start.
    fn cloud_keyed(&self) -> Vec<(&'static str, String)> {
        let cloud = &self.stt.openai_compatible;
        let selected = self.stt.engine == "openai_compatible";
        let mut problems = Vec::new();
        // The text never repeats the value.
        if !cloud.api_key_env.is_empty() && !is_variable_name(&cloud.api_key_env) {
            problems.push((
                "stt.openai_compatible.api_key_env",
                API_KEY_ENV_PROBLEM.to_string(),
            ));
        }
        // The text shows the scheme and the host only, never user data.
        let url_problem = if selected {
            edcopilot_voicebridge_stt::base_url_problem(&cloud.base_url)
        } else {
            edcopilot_voicebridge_stt::base_url_user_info_problem(&cloud.base_url)
        };
        if let Some(problem) = url_problem {
            problems.push((
                "stt.openai_compatible.base_url",
                format!("stt.openai_compatible.base_url {problem}"),
            ));
        }
        let timeout = cloud.timeout_seconds;
        if selected && !(1..=MAX_TIMEOUT_SECONDS).contains(&timeout) {
            problems.push((
                "stt.openai_compatible.timeout_seconds",
                format!(
                    "stt.openai_compatible.timeout_seconds is {timeout}, it must be from 1 to {MAX_TIMEOUT_SECONDS}"
                ),
            ));
        }
        problems
    }

    /// The problems of `[push_to_talk]`, also in `always_listening` mode.
    /// `validate` includes them in `push_to_talk` mode.
    pub fn push_to_talk_problems(&self) -> Vec<String> {
        self.push_to_talk_keyed()
            .into_iter()
            .map(|(_, text)| text)
            .collect()
    }

    fn push_to_talk_keyed(&self) -> Vec<(&'static str, String)> {
        let mut problems = Vec::new();
        let ptt = &self.push_to_talk;
        match (&ptt.key, &ptt.joystick) {
            (Some(_), Some(_)) => problems.push((
                "push_to_talk",
                "push_to_talk has both key and joystick, set only one of them".to_string(),
            )),
            (None, None) => problems.push((
                "push_to_talk",
                "push_to_talk needs a key or a joystick in push_to_talk mode".to_string(),
            )),
            (Some(key), None) => {
                if let Err(error) = KeyboardKey::check(key) {
                    problems.push((
                        "push_to_talk.key",
                        format!("push_to_talk.key \"{key}\" is not valid: {error}"),
                    ));
                }
            }
            (None, Some(_)) => {}
        }
        problems.extend(self.push_to_talk_timing_keyed());
        problems
    }

    /// The range problems of `pre_roll_ms` and `release_tail_ms`. `validate`
    /// includes them in both modes.
    pub fn push_to_talk_timing_problems(&self) -> Vec<String> {
        self.push_to_talk_timing_keyed()
            .into_iter()
            .map(|(_, text)| text)
            .collect()
    }

    fn push_to_talk_timing_keyed(&self) -> Vec<(&'static str, String)> {
        let ptt = &self.push_to_talk;
        [
            ("push_to_talk.pre_roll_ms", ptt.pre_roll_ms),
            ("push_to_talk.release_tail_ms", ptt.release_tail_ms),
        ]
        .into_iter()
        .filter(|&(_, value)| value > MAX_PTT_EXTRA_MS)
        .map(|(name, value)| {
            (
                name,
                format!("{name} is {value}, it must be from 0 to {MAX_PTT_EXTRA_MS}"),
            )
        })
        .collect()
    }

    /// The problems of `[diagnostics]`. Only `check` shows them: `run` does
    /// not stop for them, it switches the part off.
    pub fn diagnostics_problems(&self) -> Vec<String> {
        let log = &self.diagnostics.log_file;
        if log.as_os_str().is_empty() {
            return Vec::new();
        }
        if log.is_dir() {
            return vec![format!(
                "diagnostics.log_file is a directory, it must be a file: {}",
                log.display()
            )];
        }
        match log.parent() {
            Some(parent) if !parent.as_os_str().is_empty() && !parent.is_dir() => vec![format!(
                "the directory of diagnostics.log_file does not exist: {}",
                parent.display()
            )],
            _ => Vec::new(),
        }
    }

    /// The problem of `hotwords_score` when the hotwords are on. A value that
    /// is not a finite number is out of the range. When the hotwords are off,
    /// the score is not used and is not checked.
    fn hotwords_score_problem(&self) -> Option<(&'static str, String)> {
        let score = self.stt.parakeet.hotwords_score;
        let in_use = !self.stt.parakeet.hotwords_file.as_os_str().is_empty();
        let bad = in_use && !(hotwords::MIN_SCORE..=hotwords::MAX_SCORE).contains(&score);
        bad.then(|| {
            (
                "stt.parakeet.hotwords_score",
                format!(
                    "stt.parakeet.hotwords_score is {score}, it must be from {} to {}",
                    hotwords::MIN_SCORE,
                    hotwords::MAX_SCORE
                ),
            )
        })
    }

    fn validate_parakeet(&self, problems: &mut Vec<(&'static str, String)>) {
        if self.stt.parakeet.model_dir.as_os_str().is_empty() {
            problems.push((
                "stt.parakeet.model_dir",
                "stt.parakeet.model_dir is empty, the parakeet engine needs it".into(),
            ));
        }
        let threads = self.stt.parakeet.threads;
        let cores = logical_cores();
        if threads == 0 || threads > cores {
            problems.push((
                "stt.parakeet.threads",
                format!(
                    "stt.parakeet.threads is {threads}, it must be from 1 to {cores} (logical cores)"
                ),
            ));
        }
        problems.extend(self.hotwords_score_problem());
    }
}

/// The problem of a configuration without the EDCoPilot folder. See
/// `Config::edcopilot_dir_problem`.
pub const EDCOPILOT_DIR_EMPTY: &str = "edcopilot.dir is empty, set the EDCoPilot folder";

/// The problem of a configuration without the EDCoPilot folder that sets
/// the three other paths by hand, but not the profile.
pub const EDCOPILOT_PROFILE_EMPTY: &str =
    "edcopilot.dir and edcopilot.profile are empty, set the EDCoPilot folder or edcopilot.profile";

/// The warning for a configuration with `data.command_map` or
/// `data.profile_vars`. `check`, the session start, and the window show it
/// one time.
pub const OLD_MAP_KEYS: &str = "data.command_map and data.profile_vars are not used any more, \
                                the program reads the EDCoPilot profile";

/// The problem of a request file that is not an absolute path. The writer
/// refuses such a file, so each match would give a send error.
pub fn request_file_problem(path: &Path) -> String {
    format!(
        "the request file is not an absolute path: {}, give the configuration file with its full path",
        path.display()
    )
}

/// The largest value of `stt.openai_compatible.timeout_seconds`.
pub const MAX_TIMEOUT_SECONDS: u64 = 600;

/// The problem text for an `api_key_env` that is not the name of a variable.
/// It never repeats the value, because the value can be a key.
pub const API_KEY_ENV_PROBLEM: &str = "stt.openai_compatible.api_key_env must be the name of an environment variable (letters, digits and _), not the key itself";

/// The longest name of an environment variable that the program accepts.
pub const MAX_VARIABLE_NAME: usize = 64;

/// The starts of API keys that use only letters, digits and `_`: Groq
/// (`gsk_`), ElevenLabs and Stripe (`sk_`), Stripe (`pk_`, `rk_`), and xAI
/// (`xai_`). The check ignores the case.
const KEY_PREFIXES: [&str; 5] = ["sk_", "gsk_", "pk_", "rk_", "xai_"];

/// The shortest run without `_` that is a key when it mixes letters and
/// digits. A real name is words that `_` joins.
const RANDOM_RUN: usize = 20;

/// True for the name of an environment variable: a letter or `_` first,
/// then letters, digits and `_`, at most 64 characters. ASCII only. A text
/// that looks like an API key is not a name: it starts with a known key
/// prefix, or it has a run of 20 or more characters without `_` that mixes
/// letters and digits.
pub fn is_variable_name(name: &str) -> bool {
    let mut chars = name.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    let shape = name.len() <= MAX_VARIABLE_NAME
        && (first.is_ascii_alphabetic() || first == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_');
    if !shape {
        return false;
    }
    let lower = name.to_ascii_lowercase();
    let prefixed = KEY_PREFIXES.iter().any(|p| lower.starts_with(p));
    let random = name.split('_').any(|run| {
        run.len() >= RANDOM_RUN
            && run.chars().any(|c| c.is_ascii_alphabetic())
            && run.chars().any(|c| c.is_ascii_digit())
    });
    !prefixed && !random
}

/// The URL functions of the cloud engine. They use the URL parser of the
/// HTTP client, so a check sees the host that the request goes to.
pub use edcopilot_voicebridge_stt::{is_loopback_url, url_origin};

/// The name for a text: the name itself when it is a variable name, else a
/// fixed text. A value that is not a name can be a key.
pub fn safe_variable_name(name: &str) -> &str {
    if is_variable_name(name) {
        name
    } else {
        "(not a valid variable name)"
    }
}

/// What `build_pipeline` loaded. `problems` holds one line for each missing path.
#[derive(Debug, Clone, Default)]
pub struct LoadReport {
    pub commands: usize,
    /// Commands that have no unresolved `{TXT:EDCP_` and can fire.
    pub matchable: usize,
    /// Commands whose phrase still has `{TXT:EDCP_` after resolution.
    pub unresolved_grammar: usize,
    /// Prefixes and suffixes of composite commands. They fire only together.
    pub composite_parts: usize,
    pub vocabulary_files: usize,
    pub vocabulary_names: usize,
    pub alias_phrases: usize,
    /// Entries of the sound-alike table.
    pub sound_alikes: usize,
    /// Entries of the synonym table.
    pub synonyms: usize,
    /// A synonym key that is also in a command phrase. Not a problem: the
    /// synonym then applies only when the text as heard matches nothing.
    pub synonym_warnings: Vec<String>,
    /// Entries of the filler table.
    pub fillers: usize,
    /// False in `always_listening` mode: the pipeline then has no filler
    /// step.
    pub fillers_active: bool,
    /// A filler word that is a word of a name. Not a problem: the word then
    /// stays where the whole name is in the text.
    pub filler_warnings: Vec<String>,
    /// An alias key that no command takes, with its file. Not a problem:
    /// the phrases then never give a token, which is a miss and not a
    /// wrong match. The phrases of a `{CMD}` key block a text that they
    /// name (see `MatchResult::Blocked`).
    pub alias_warnings: Vec<String>,
    /// A vocabulary file with a form that the program does not know. Its
    /// slot gets no names: a miss, not a wrong token.
    pub vocabulary_warnings: Vec<String>,
    /// The slots (`System12`, `Group3`, ...) whose files the vocabulary
    /// check covers: each slot that a command uses.
    pub checked_slots: BTreeSet<String>,
    /// The report of the build from the EDCoPilot profile.
    pub profile: BuildReport,
    /// The warnings of the load of the commands (`LoadedCommands::warnings`).
    pub command_warnings: Vec<String>,
    pub problems: Vec<String>,
}

impl LoadReport {
    /// The warnings of the load in the order of `voicebridge check`: the
    /// synonyms, the fillers, the aliases, the vocabulary, and the commands.
    pub fn warnings(&self) -> impl Iterator<Item = &String> {
        self.synonym_warnings
            .iter()
            .chain(&self.filler_warnings)
            .chain(&self.alias_warnings)
            .chain(&self.vocabulary_warnings)
            .chain(&self.command_warnings)
    }
}

/// If a command can fire, and if not, why not.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommandState {
    /// The matcher can match the phrase and make the token.
    Works,
    /// The command has no spoken phrase.
    NoPhrase,
    /// The phrase still has `{TXT:EDCP_` after resolution.
    UnresolvedGrammar,
    /// The token needs a value that the phrase does not give.
    NeedsValue,
    /// A prefix or a suffix of a composite command. It fires only when a
    /// prefix and a suffix of its group are spoken together.
    CompositePart,
}

/// The state of one command after `load_commands`.
pub fn command_state(command: &Command) -> CommandState {
    match command.phrase.as_deref() {
        None => CommandState::NoPhrase,
        Some(phrase) if phrase.contains(VAR_PREFIX) => CommandState::UnresolvedGrammar,
        Some(_) if command.is_composite_part() => CommandState::CompositePart,
        Some(_)
            if command.class == CommandClass::DynamicValue && command.number_vars.is_empty() =>
        {
            CommandState::NeedsValue
        }
        Some(_) => CommandState::Works,
    }
}

/// What `load_commands` gives.
#[derive(Debug, Clone)]
pub struct LoadedCommands {
    pub commands: Vec<Command>,
    /// The report of the build from the EDCoPilot profile. The commands
    /// that the check of the runtime grammar takes out are in its left-out
    /// list.
    pub profile: BuildReport,
    /// The warnings of the load: `OLD_MAP_KEYS` when the file has a key of
    /// an older version, then the lines of `shadow_warnings`.
    pub warnings: Vec<String>,
    /// The phrases of the left-out commands that VoiceAttack recognises by
    /// voice, with the runtime grammar and the profile variables: those of
    /// the build and those of the commands that the number check takes
    /// out. The matcher keeps them as paths without a token. They are not
    /// commands: the command list does not show them.
    pub blocking: Vec<BlockingPhrase>,
    /// The blocking phrases that do not resolve or do not parse. They are
    /// not in `blocking`.
    pub blocking_skipped: usize,
}

/// Load the commands and put the runtime grammar and the profile variables
/// into the phrases.
///
/// The program reads the EDCoPilot profile (`edcopilot.profile`) and
/// builds the commands: the report of the build comes with them. A profile
/// that cannot be read and a file that is not a profile are errors. The
/// keys `data.command_map` and `data.profile_vars` of an older version
/// change nothing: they give the warning `OLD_MAP_KEYS`.
///
/// A number command whose phrase with the runtime grammar and the profile
/// variables has number text outside its ranges leaves the list (see
/// `resolve_runtime_grammar`). It goes to the left-out list of the report,
/// and its phrase becomes a blocking phrase.
///
/// The blocking phrases of the build get the same runtime grammar and
/// profile variables (`resolve_blocking`).
pub fn load_commands(config: &Config) -> anyhow::Result<LoadedCommands> {
    let profile = &config.edcopilot.profile;
    // An empty path comes from an empty `edcopilot.dir`. It is never read,
    // so no file of the current folder is used.
    if profile.as_os_str().is_empty() {
        anyhow::bail!(config
            .edcopilot_dir_problem()
            .unwrap_or(EDCOPILOT_DIR_EMPTY));
    }
    let source = if config.edcopilot.profile_from_key {
        ProfileSource::Key
    } else {
        ProfileSource::Folder
    };
    let built = build_profile(profile, source)?;
    let (commands, profile_vars, mut report, mut blocking) = (
        built.commands,
        built.profile_vars,
        built.report,
        built.blocking,
    );
    let ini = &config.edcopilot.voice_commands_ini;
    // An empty path is never read. `Config::validate` reports the folder.
    // The number check runs also without the file, and it sees the grammar
    // of the profile variables.
    // The file is read one time for the commands and the blocking phrases.
    let grammar = (!ini.as_os_str().is_empty())
        .then(|| RuntimeGrammar::read(ini))
        .flatten();
    let phrases: Vec<String> = commands.iter().filter_map(|c| c.phrase.clone()).collect();
    let out = resolve_runtime_grammar_from(commands, grammar.as_ref(), &profile_vars);
    add_removed(&mut report, phrases, &out.removed);
    blocking.extend(removed_as_blocking(&out.removed));
    let blocking = resolve_blocking_from(blocking, grammar.as_ref(), &profile_vars);
    let commands = resolve_profile_vars(out.commands, &profile_vars);
    let mut warnings: Vec<String> = config.old_map_keys_warning().into_iter().collect();
    warnings.extend(shadow_warnings(&commands, &blocking.phrases));
    warnings.extend(blocking_skipped_warning(blocking.skipped));
    Ok(LoadedCommands {
        commands,
        profile: report,
        warnings,
        blocking: blocking.phrases,
        blocking_skipped: blocking.skipped,
    })
}

/// The warning when `skipped` blocking phrases do not resolve or do not
/// parse: the text of these left-out commands is not blocked, so a similar
/// command can match it.
pub fn blocking_skipped_warning(skipped: usize) -> Option<String> {
    (skipped > 0).then(|| {
        format!(
            "{skipped} left-out commands have a phrase that the program cannot read, their \
             text is not blocked"
        )
    })
}

/// The largest number of lines of `shadow_warnings` with a command.
pub const MAX_SHADOW_WARNINGS: usize = 10;

/// One line for each working command with a text that is also a text of a
/// command that the program cannot send (`shadowed_phrases_with`): a
/// command that needs a value, or a left-out command (`blocking`). The
/// matcher sends no command for that text, although the command list says
/// that the command works. At most `MAX_SHADOW_WARNINGS` lines, then one
/// line with the number of the other commands.
pub fn shadow_warnings(commands: &[Command], blocking: &[BlockingPhrase]) -> Vec<String> {
    let found = shadowed_phrases_with(commands, blocking);
    let mut lines: Vec<String> = found
        .iter()
        .take(MAX_SHADOW_WARNINGS)
        .map(|s| {
            format!(
                "the phrase \"{}\" of the command \"{}\" is also a phrase of a command that \
                 the program cannot send, the program does not send a command for it",
                s.text, s.phrase
            )
        })
        .collect();
    if found.len() > MAX_SHADOW_WARNINGS {
        lines.push(format!(
            "... and {} more",
            found.len() - MAX_SHADOW_WARNINGS
        ));
    }
    lines
}

/// Puts the commands that the check of the runtime grammar took out into
/// the left-out list of the report. `phrases`: the phrases of the built
/// records before the resolution.
fn add_removed(report: &mut BuildReport, phrases: Vec<String>, removed: &[RemovedCommand]) {
    if removed.is_empty() {
        return;
    }
    let mut left: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    for p in phrases {
        *left.entry(p).or_default() += 1;
    }
    for r in removed {
        report.records -= 1;
        if let Some(n) = left.get_mut(&r.phrase) {
            *n -= 1;
            if *n == 0 {
                report.mapped_phrases.remove(&r.phrase);
            }
        }
        report.left_out.push(LeftOut {
            phrase: r.phrase.clone(),
            class: LeftOutClass::NotUnderstood,
            tokens: vec![r.token_template.clone()],
            reason: r.reason.clone(),
        });
    }
}

/// Load the data files and build the pipeline. A profile that cannot be
/// read or built is an error, as each error of `load_commands`.
pub fn build_pipeline(config: &Config) -> anyhow::Result<(Pipeline, LoadReport)> {
    let loaded = load_commands(config)?;
    Ok(build_pipeline_with_commands(config, loaded))
}

/// Loads the vocabulary of `dir`. The files of `slots` must have the known
/// form (see `load_dynamic_vocab_checked`): a file with another form gives
/// no names and one warning.
pub fn load_vocabulary(
    dir: &Path,
    slots: &BTreeSet<String>,
) -> (std::collections::HashMap<String, Vec<String>>, Vec<String>) {
    let (vocab, unknown) = load_dynamic_vocab_checked(dir, slots);
    (vocab, form_warnings(&unknown))
}

/// The most vocabulary files of unknown form that get one line each.
const MAX_FORM_LINES: usize = 5;

/// The warnings for the vocabulary files of unknown form: one line for
/// each file, or one line for all files when there are more than
/// `MAX_FORM_LINES`. That line names the first files.
fn form_warnings(unknown: &[String]) -> Vec<String> {
    if unknown.len() <= MAX_FORM_LINES {
        return unknown.iter().map(|f| unknown_form_warning(f)).collect();
    }
    vec![format!(
        "{} vocabulary files have a form that the program does not know, their names are \
         not used: {}, ...",
        unknown.len(),
        unknown[..MAX_FORM_LINES].join(", ")
    )]
}

/// `build_pipeline` with the result of `load_commands`. The other data
/// files load here. A missing file gives a problem line, not an error. The
/// report gets the profile report and the warnings of the load.
pub fn build_pipeline_with_commands(
    config: &Config,
    loaded: LoadedCommands,
) -> (Pipeline, LoadReport) {
    let LoadedCommands {
        commands,
        profile,
        warnings,
        blocking,
        blocking_skipped: _,
    } = loaded;
    let mut report = LoadReport {
        profile,
        command_warnings: warnings,
        ..LoadReport::default()
    };
    let ed = &config.edcopilot;
    // An empty path comes from an empty `edcopilot.dir`, which is one
    // problem of `Config::validate`. It gives no problem of its own, and it
    // is never read.
    let set = |p: &Path| !p.as_os_str().is_empty();

    if set(&ed.voice_commands_ini) && !ed.voice_commands_ini.is_file() {
        report.problems.push(format!(
            "voice commands file not found: {}",
            ed.voice_commands_ini.display()
        ));
    }
    report.commands = commands.len();
    let count = |state| {
        commands
            .iter()
            .filter(|c| command_state(c) == state)
            .count()
    };
    report.unresolved_grammar = count(CommandState::UnresolvedGrammar);
    report.matchable = count(CommandState::Works);
    report.composite_parts = count(CommandState::CompositePart);

    if set(&ed.dynamic_vars_dir) && !ed.dynamic_vars_dir.is_dir() {
        report.problems.push(format!(
            "vocabulary directory not found: {}",
            ed.dynamic_vars_dir.display()
        ));
    }
    // The commands of the profile assume a form of the slot files.
    report.checked_slots = slot_keys(commands.iter().filter_map(|c| c.phrase.as_deref()));
    let vocab = if set(&ed.dynamic_vars_dir) {
        let (vocab, warnings) = load_vocabulary(&ed.dynamic_vars_dir, &report.checked_slots);
        report.vocabulary_warnings = warnings;
        vocab
    } else {
        Default::default()
    };
    report.vocabulary_files = vocab.len();
    report.vocabulary_names = vocab.values().map(Vec::len).sum();

    if !config.data.aliases_dir.is_dir() {
        report.problems.push(format!(
            "aliases directory not found: {}",
            config.data.aliases_dir.display()
        ));
    }
    let alias_files = load_alias_files(&config.data.aliases_dir);
    let aliases = merge_aliases(&alias_files);
    report.alias_phrases = aliases.values().map(Vec::len).sum();

    // A missing or invalid file gives an empty table and a problem line.
    let (sound_alikes, problems) = load_sound_alikes_with_problems(&config.data.sound_alikes);
    report.problems.extend(problems);
    report.sound_alikes = sound_alikes.len();

    let (synonyms, problems) = load_synonyms_with_problems(&config.data.synonyms);
    report.problems.extend(problems);
    report.synonyms = synonyms.len();
    report.synonym_warnings = synonym_key_warnings(&synonyms, &commands);

    let (fillers, problems) = load_fillers_with_problems(&config.data.fillers);
    report.problems.extend(problems);
    report.fillers = fillers.len();
    // Filler words make the matcher more permissive. In always-listening
    // mode the speech is not always for the program, so they are off.
    report.fillers_active = config.microphone.mode != "always_listening";

    let matcher =
        Matcher::new(commands, vocab, aliases, config.matching.threshold).with_blocking(&blocking);
    for (path, file) in &alias_files {
        let mut keys: Vec<&String> = file
            .keys()
            .filter(|key| matcher.ignored_alias_keys().contains(key))
            .collect();
        keys.sort();
        report.alias_warnings.extend(keys.into_iter().map(|key| {
            // The command of a {CMD} token works: only the alias path
            // cannot build the token. The alias phrase stays: a text that
            // names it blocks, so it gives no command. The phrases of the
            // other keys are not in the matcher.
            let reason = if needs_spoken_command(key) {
                "needs the English command text, an alias cannot give it, \
                 the program sends no command for its phrases"
            } else {
                "matches no command that works, the program ignores its phrases"
            };
            format!(
                "alias file {}: the token \"{key}\" {reason}",
                path.display()
            )
        }));
    }
    let names = matcher.names();
    report.filler_warnings = filler_name_warnings(&fillers, names.iter().map(String::as_str));
    let writer = Writer::new(ed.request_file.clone());
    let mut pipeline = Pipeline::new(matcher, writer)
        .with_sound_alikes(sound_alikes)
        .with_synonyms(synonyms);
    if report.fillers_active {
        pipeline = pipeline.with_fillers(fillers);
    }
    (pipeline, report)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands_source::test_support::FIXTURE_SHADOW;
    use std::fs;
    use std::path::{Path, PathBuf};

    fn minimal() -> Config {
        Config::parse(
            "[edcopilot]\ndir = \"C:\\\\EDCoPilot\"\n",
            Path::new("."),
            "t.toml",
        )
        .unwrap()
    }

    #[test]
    fn test_example_file_loads_and_is_valid() {
        let text = include_str!("../voicebridge.example.toml");
        let mut config = Config::parse(text, Path::new("."), "example").unwrap();
        config.stt.parakeet.model_dir = PathBuf::from("models/parakeet");
        let cores = std::thread::available_parallelism().map_or(1, |n| n.get());
        if cores < 4 {
            config.stt.parakeet.threads = 1;
        }
        assert_eq!(config.validate(), Vec::<String>::new());
    }

    #[test]
    fn test_minimal_file_gets_defaults() {
        let config = minimal();
        assert_eq!(config.matching.threshold, 0.8);
        assert_eq!(config.microphone.mode, "push_to_talk");
        assert_eq!(config.push_to_talk.key.as_deref(), Some("F13"));
    }

    #[test]
    fn test_pre_roll_and_tail_defaults_and_values() {
        let config = minimal();
        assert_eq!(config.push_to_talk.pre_roll_ms, 250);
        assert_eq!(config.push_to_talk.release_tail_ms, 300);
        let text =
            "[edcopilot]\ndir = 'x'\n[push_to_talk]\npre_roll_ms = 0\nrelease_tail_ms = 2000\n";
        let config = Config::parse(text, Path::new("."), "t.toml").unwrap();
        assert_eq!(config.push_to_talk.pre_roll_ms, 0);
        assert_eq!(config.push_to_talk.release_tail_ms, 2000);
        assert_eq!(config.push_to_talk.key.as_deref(), Some("F13"));
        assert!(config.push_to_talk_problems().is_empty());
    }

    #[test]
    fn test_validate_pre_roll_and_tail_range() {
        let mut config = minimal();
        config.stt.parakeet.model_dir = PathBuf::from("m");
        config.stt.parakeet.threads = 1;
        config.push_to_talk.pre_roll_ms = 2_001;
        let problems = config.validate();
        assert_eq!(problems.len(), 1, "{problems:?}");
        assert!(
            problems[0].contains("push_to_talk.pre_roll_ms"),
            "{problems:?}"
        );
        assert!(problems[0].contains("2001"), "{problems:?}");
        config.push_to_talk.pre_roll_ms = 2_000;
        config.push_to_talk.release_tail_ms = 5_000;
        let problems = config.validate();
        assert_eq!(problems.len(), 1, "{problems:?}");
        assert!(
            problems[0].contains("push_to_talk.release_tail_ms"),
            "{problems:?}"
        );
        config.push_to_talk.release_tail_ms = 0;
        assert!(config.validate().is_empty());
        // A negative value does not load.
        let text = "[edcopilot]\ndir = 'x'\n[push_to_talk]\npre_roll_ms = -5\n";
        assert!(Config::parse(text, Path::new("."), "t.toml").is_err());
    }

    #[test]
    fn test_validate_pre_roll_and_tail_range_in_always_listening_mode() {
        let mut config = minimal();
        config.stt.parakeet.model_dir = PathBuf::from("m");
        config.stt.parakeet.threads = 1;
        config.microphone.mode = "always_listening".to_string();
        assert!(config.validate().is_empty());
        config.push_to_talk.pre_roll_ms = 5_000;
        config.push_to_talk.release_tail_ms = 2_001;
        let problems = config.validate();
        assert_eq!(problems.len(), 2, "{problems:?}");
        assert!(
            problems[0].contains("push_to_talk.pre_roll_ms"),
            "{problems:?}"
        );
        assert!(
            problems[1].contains("push_to_talk.release_tail_ms"),
            "{problems:?}"
        );
        // In push_to_talk mode each problem shows one time.
        config.microphone.mode = "push_to_talk".to_string();
        assert_eq!(config.validate().len(), 2, "{:?}", config.validate());
    }

    #[test]
    fn test_unknown_key_is_an_error() {
        let text = "[edcopilot]\ndir = \"x\"\nbogus_key = 1\n";
        let error = Config::parse(text, Path::new("."), "t.toml").unwrap_err();
        let text = format!("{error:#}");
        assert!(text.contains("bogus_key"), "{text}");
        assert!(text.contains("t.toml"), "{text}");
    }

    #[test]
    fn test_validate_reports_each_problem() {
        let mut config = minimal();
        config.matching.threshold = 1.5;
        config.push_to_talk.key = None;
        config.stt.parakeet.threads = 0;
        assert_eq!(config.validate().len(), 4, "{:?}", config.validate());
    }

    #[test]
    fn test_validate_unknown_mode_and_engine() {
        let mut config = minimal();
        config.microphone.mode = "shout".to_string();
        config.stt.engine = "magic".to_string();
        let problems = config.validate();
        assert_eq!(problems.len(), 2, "{problems:?}");
        assert!(problems[0].contains("shout"));
        assert!(problems[1].contains("magic"));
    }

    #[test]
    fn test_validate_key_and_joystick() {
        let mut config = minimal();
        config.stt.parakeet.model_dir = PathBuf::from("m");
        config.push_to_talk.joystick = Some(Joystick {
            device: "MOZA".to_string(),
            button: 5,
        });
        assert_eq!(config.validate().len(), 1);
        config.push_to_talk.key = Some("NotAKey".to_string());
        config.push_to_talk.joystick = None;
        let problems = config.validate();
        assert_eq!(problems.len(), 1);
        assert!(problems[0].contains("NotAKey"));
    }

    #[test]
    fn test_validate_key_forms() {
        let mut config = minimal();
        config.stt.parakeet.model_dir = PathBuf::from("m");
        config.stt.parakeet.threads = 1;
        // No character here: its check depends on the host keyboard layout.
        let mut good = vec!["F13", "rcontrol"];
        if cfg!(windows) {
            good.extend(["vk:226", "vk:0xE2", "Mouse4", "Mouse5"]);
        }
        for good in good {
            config.push_to_talk.key = Some(good.to_string());
            assert_eq!(config.validate(), Vec::<String>::new(), "{good}");
        }
        for bad in ["Mouse1", "vk:0", "vk:255", "NoSuchKey"] {
            config.push_to_talk.key = Some(bad.to_string());
            let problems = config.validate();
            assert_eq!(problems.len(), 1, "{bad}: {problems:?}");
            assert!(problems[0].contains(bad), "{problems:?}");
        }
    }

    #[test]
    fn test_validate_thread_count() {
        let mut config = minimal();
        config.stt.parakeet.model_dir = PathBuf::from("m");
        config.stt.parakeet.threads = 0;
        assert_eq!(config.validate().len(), 1);
        config.stt.parakeet.threads = 100_000;
        assert_eq!(config.validate().len(), 1);
        config.stt.parakeet.threads = 1;
        assert!(config.validate().is_empty());
    }

    #[test]
    fn test_paths_default_from_the_edcopilot_dir() {
        let config = minimal();
        let request = config.edcopilot.request_file.to_string_lossy().to_string();
        let ini = config
            .edcopilot
            .voice_commands_ini
            .to_string_lossy()
            .replace('\\', "/");
        let vars = config
            .edcopilot
            .dynamic_vars_dir
            .to_string_lossy()
            .to_string();
        assert!(request.ends_with("EDCoPilot.request.txt"));
        assert!(ini.ends_with("working/EDCoPilot.VoiceCommands.ini"));
        assert!(vars.ends_with("Dynamic var files"));
    }

    #[test]
    fn test_empty_edcopilot_dir_is_one_problem_that_names_the_key() {
        let dir = tempfile::tempdir().unwrap();
        let core = Path::new(env!("CARGO_MANIFEST_DIR")).join("../core");
        let mut config = Config::parse(
            "[edcopilot]\ndir = \"\"\n[stt]\nengine = \"openai_compatible\"\n",
            dir.path(),
            "t.toml",
        )
        .unwrap();
        // The profile path is set: the commands load. The other paths that
        // default from the folder stay empty.
        let ed = dir.path().join("ed");
        edcopilot_voicebridge_core::profile::fixture::write_fixture_edcopilot(&ed);
        config.edcopilot.profile = ed.join(crate::commands_source::PROFILE_FILE);
        config.data.aliases_dir = core.join("aliases");
        config.data.sound_alikes = core.join("data/sound_alikes.toml");
        config.data.synonyms = core.join("data/synonyms.toml");
        config.data.fillers = core.join("data/fillers.toml");
        assert_eq!(
            config.validate_keyed(),
            [(
                "edcopilot.dir",
                "edcopilot.dir is empty, set the EDCoPilot folder".to_string()
            )]
        );
        // The paths that default from the folder stay empty: the program
        // never reads or writes a path in the current folder.
        let ed = &config.edcopilot;
        assert!(ed.request_file.as_os_str().is_empty());
        assert!(ed.voice_commands_ini.as_os_str().is_empty());
        assert!(ed.dynamic_vars_dir.as_os_str().is_empty());
        assert!(config.edcopilot_dir_problem().is_some());
        // The files that default from the folder give no second problem.
        let (_pipeline, report) = build_pipeline(&config).unwrap();
        assert!(report.problems.is_empty(), "{:?}", report.problems);
        assert_eq!(report.vocabulary_files, 0);
    }

    /// The program reads the profile: the path `edcopilot.profile` counts
    /// in the rule of the folder.
    #[test]
    fn test_empty_edcopilot_dir_with_three_paths_and_no_profile_names_the_profile() {
        let dir = tempfile::tempdir().unwrap();
        let three = "[edcopilot]\ndir = \"\"\nrequest_file = \"req.txt\"\n\
                     voice_commands_ini = \"vc.ini\"\ndynamic_vars_dir = \"vars\"\n";
        let engine = "[stt]\nengine = \"openai_compatible\"\n";
        let config = Config::parse(&format!("{three}{engine}"), dir.path(), "t.toml").unwrap();
        assert!(config.edcopilot.profile.as_os_str().is_empty());
        assert!(config.edcopilot_dir_problem().is_some());
        let expected = "edcopilot.dir and edcopilot.profile are empty, set the EDCoPilot \
                        folder or edcopilot.profile";
        assert_eq!(
            config.validate_keyed(),
            [("edcopilot.dir", expected.to_string())]
        );
        // The load gives the same text, so `check` shows one problem.
        let error = load_commands(&config).expect_err("no profile");
        assert_eq!(format!("{error:#}"), expected);
        let found = crate::run::check_config_in(&config, |_| Some("k".into()), None);
        assert_eq!(found.problems, [expected], "{found:?}");
        // The old key `data.command_map` changes nothing: the program
        // always reads the profile.
        let with_old_key = format!(
            "{three}[data]\ncommand_map = '{}'\n{engine}",
            dir.path().join("map.json").display()
        );
        let config = Config::parse(&with_old_key, dir.path(), "t.toml").unwrap();
        assert!(config.edcopilot_dir_problem().is_some());
        assert_eq!(
            config.validate_keyed(),
            [("edcopilot.dir", expected.to_string())]
        );
    }

    #[test]
    fn test_empty_edcopilot_dir_with_the_four_paths_set_is_valid() {
        let dir = tempfile::tempdir().unwrap();
        let config = Config::parse(
            "[edcopilot]\ndir = \"\"\nrequest_file = \"req.txt\"\n\
             voice_commands_ini = \"vc.ini\"\ndynamic_vars_dir = \"vars\"\n\
             profile = \"p.vap\"\n\
             [stt]\nengine = \"openai_compatible\"\n",
            dir.path(),
            "t.toml",
        )
        .unwrap();
        assert!(config.edcopilot_dir_problem().is_none());
        assert!(config.validate().is_empty(), "{:?}", config.validate());
        assert_eq!(config.edcopilot.request_file, dir.path().join("req.txt"));
        // One of the three paths empty: the folder is missing.
        let config = Config::parse(
            "[edcopilot]\ndir = \"\"\nrequest_file = \"req.txt\"\n\
             voice_commands_ini = \"vc.ini\"\n[stt]\nengine = \"openai_compatible\"\n",
            dir.path(),
            "t.toml",
        )
        .unwrap();
        assert!(config.edcopilot_dir_problem().is_some());
        assert!(config.edcopilot.dynamic_vars_dir.as_os_str().is_empty());
        assert_eq!(config.validate().len(), 1);
    }

    #[test]
    fn test_relative_paths_resolve_against_the_config_directory() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("voicebridge.toml");
        fs::write(
            &file,
            "[edcopilot]\ndir = \"ed\"\n[data]\nsynonyms = \"syn.toml\"\n\
             [stt.parakeet]\nmodel_dir = \"model\"\n",
        )
        .unwrap();
        let config = Config::load(&file).unwrap();
        assert_eq!(config.edcopilot.dir, dir.path().join("ed"));
        assert_eq!(config.data.synonyms, dir.path().join("syn.toml"));
        assert_eq!(config.stt.parakeet.model_dir, dir.path().join("model"));
        assert!(config.edcopilot.request_file.starts_with(dir.path()));
    }

    /// A temporary folder below the target folder, and the path of a file
    /// in it relative to the current folder. Unit tests run in the folder
    /// of the crate.
    fn relative_temp_file(name: &str) -> (tempfile::TempDir, PathBuf) {
        let target = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("target");
        fs::create_dir_all(&target).unwrap();
        let dir = tempfile::Builder::new()
            .prefix("vb-relative-")
            .tempdir_in(&target)
            .unwrap();
        let folder = dir.path().file_name().unwrap();
        let relative = Path::new("..").join("target").join(folder).join(name);
        assert!(relative.is_relative());
        (dir, relative)
    }

    /// `--config voicebridge.toml` gives a relative path. Each derived
    /// path must still be absolute: the writer refuses a relative request
    /// file.
    #[test]
    fn test_a_relative_config_path_gives_an_absolute_request_file() {
        let (dir, file) = relative_temp_file("voicebridge.toml");
        fs::write(
            &file,
            "[edcopilot]\ndir = \"ed\"\n[diagnostics]\nlog_file = \"logs/u.tsv\"\n",
        )
        .unwrap();
        let config = Config::load(&file).unwrap();
        let folder = std::path::absolute(dir.path()).unwrap();
        let request = &config.edcopilot.request_file;
        assert!(request.is_absolute(), "{}", request.display());
        assert_eq!(request, &folder.join("ed").join("EDCoPilot.request.txt"));
        assert!(!request.to_string_lossy().starts_with(r"\\?\"));
        assert!(config.diagnostics.log_file.is_absolute());
        assert!(config.validate().iter().all(|p| !p.contains("request")));
    }

    /// The console shows a relative diagnostics path as before, also when
    /// the program uses the absolute path.
    #[test]
    fn test_the_console_shows_relative_paths_as_the_file_gives_them() {
        let (_dir, file) = relative_temp_file("voicebridge.toml");
        fs::write(
            &file,
            "[edcopilot]\ndir = \"ed\"\n[diagnostics]\nlog_file = \"logs/u.tsv\"\n\
             save_audio_dir = \"logs/audio\"\n",
        )
        .unwrap();
        let mut config = Config::load(&file).unwrap();
        let given = file.parent().unwrap();
        assert_eq!(
            crate::run::diagnostics_lines(&config),
            [
                format!("utterance log: {}", given.join("logs/u.tsv").display()),
                format!("audio capture: {}", given.join("logs/audio").display()),
            ]
        );
        // A path that changed after the load is shown as it is.
        let other = std::path::absolute("other.tsv").unwrap();
        config.diagnostics.log_file = other.clone();
        assert_eq!(config.shown(&other), other.as_path());
    }

    #[test]
    fn test_a_request_file_that_is_not_absolute_is_a_problem() {
        let config = Config::parse(
            "[edcopilot]\ndir = \"ed\"\n[stt]\nengine = \"openai_compatible\"\n",
            Path::new("."),
            "t.toml",
        )
        .unwrap();
        assert!(config.edcopilot.request_file.is_relative());
        let keyed = config.validate_keyed();
        assert!(
            keyed
                .iter()
                .any(|(key, text)| *key == "edcopilot.request_file"
                    && text == &request_file_problem(&config.edcopilot.request_file)),
            "{keyed:?}"
        );
    }

    #[test]
    fn test_diagnostics_paths_resolve_and_default_to_off() {
        let config = minimal();
        assert!(config.diagnostics.log_file.as_os_str().is_empty());
        assert!(config.diagnostics.save_audio_dir.as_os_str().is_empty());
        assert!(config.diagnostics_problems().is_empty());

        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("voicebridge.toml");
        fs::write(
            &file,
            "[edcopilot]
dir = 'ed'
[diagnostics]
log_file = 'logs/u.tsv'
             save_audio_dir = 'clips'
",
        )
        .unwrap();
        let config = Config::load(&file).unwrap();
        assert_eq!(config.diagnostics.log_file, dir.path().join("logs/u.tsv"));
        assert_eq!(config.diagnostics.save_audio_dir, dir.path().join("clips"));
        let problems = config.diagnostics_problems();
        assert_eq!(problems.len(), 1, "{problems:?}");
        assert!(problems[0].contains("logs"), "{problems:?}");
        fs::create_dir(dir.path().join("logs")).unwrap();
        assert!(config.diagnostics_problems().is_empty());
        // A directory is not a log file.
        fs::create_dir(dir.path().join("logs/u.tsv")).unwrap();
        let problems = config.diagnostics_problems();
        assert_eq!(problems.len(), 1, "{problems:?}");
        assert!(problems[0].contains("is a directory"), "{problems:?}");
    }

    #[test]
    fn test_hotword_defaults_resolve_against_the_config_directory() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("voicebridge.toml");
        fs::write(&file, "[edcopilot]\ndir = 'ed'\n").unwrap();
        let config = Config::load(&file).unwrap();
        let p = &config.stt.parakeet;
        assert_eq!(p.hotwords_score, DEFAULT_HOTWORDS_SCORE);
        // The hotwords are off by default.
        assert!(p.hotwords_file.as_os_str().is_empty());
        fs::write(
            &file,
            "[edcopilot]\ndir = 'ed'\n[stt.parakeet]\nhotwords_file = 'my/hot.txt'\nhotwords_score = 1.5\n",
        )
        .unwrap();
        let config = Config::load(&file).unwrap();
        let p = &config.stt.parakeet;
        assert_eq!(p.hotwords_file, dir.path().join("my/hot.txt"));
        assert_eq!(p.hotwords_score, 1.5);
        // An empty value switches the hotwords off.
        fs::write(
            &file,
            "[edcopilot]\ndir = 'ed'\n[stt.parakeet]\nhotwords_file = ''\n",
        )
        .unwrap();
        let config = Config::load(&file).unwrap();
        assert!(config.stt.parakeet.hotwords_file.as_os_str().is_empty());
    }

    #[test]
    fn test_unknown_parakeet_key_is_an_error() {
        let text = "[edcopilot]\ndir = 'x'\n[stt.parakeet]\nhotword_file = 'a'\n";
        let error = Config::parse(text, Path::new("."), "t.toml").unwrap_err();
        assert!(format!("{error:#}").contains("hotword_file"), "{error:#}");
    }

    #[test]
    fn test_validate_hotword_score_range() {
        let mut config = minimal();
        config.stt.parakeet.model_dir = PathBuf::from("m");
        config.stt.parakeet.threads = 1;
        config.stt.parakeet.hotwords_file = PathBuf::from("hot.txt");
        for good in [0.1, 2.0, 4.0] {
            config.stt.parakeet.hotwords_score = good;
            assert_eq!(config.validate(), Vec::<String>::new(), "{good}");
        }
        for bad in [0.0, 4.5, f32::NAN] {
            config.stt.parakeet.hotwords_score = bad;
            let problems = config.validate();
            assert_eq!(problems.len(), 1, "{bad}: {problems:?}");
            assert!(
                problems[0].contains("stt.parakeet.hotwords_score"),
                "{problems:?}"
            );
        }
        // The score does not matter when the hotwords are off.
        config.stt.parakeet.hotwords_file = PathBuf::new();
        assert!(config.validate().is_empty());
    }

    /// The shipped list loads, and each line is a phrase.
    #[test]
    fn test_shipped_hotword_list_loads() {
        let list = Path::new(env!("CARGO_MANIFEST_DIR")).join("../core/data/hotwords.txt");
        let h = edcopilot_voicebridge_stt::Hotwords::load(&list, DEFAULT_HOTWORDS_SCORE).unwrap();
        assert!(!h.is_empty());
    }

    #[test]
    fn test_unknown_diagnostics_key_is_an_error() {
        let text = "[edcopilot]
dir = 'x'
[diagnostics]
log = 'a'
";
        assert!(Config::parse(text, Path::new("."), "t.toml").is_err());
    }

    /// Keys that use only letters, digits and `_` are not variable names.
    #[test]
    fn test_a_key_with_only_name_characters_is_not_a_variable_name() {
        let alnum = |n: usize| {
            "aB3dE5gH7jK9mN1pQ2sT4vW6xY8z0Ac"
                .chars()
                .cycle()
                .take(n)
                .collect::<String>()
        };
        let keys = [
            format!("gsk_{}", alnum(52)),
            format!("sk_{}", alnum(48)),
            format!("GSK_{}", "ABCDEF".repeat(3)),
            "pk_live_abc".to_string(),
            "rk_test_abc".to_string(),
            "xai_abc".to_string(),
            "3f9a0c4e1b7d2a6f8e0c3b5d7a9f1e2c".to_string(),
            "9f3a1c7e5b2d8f4a6c0e9b1d3f5a7c9e2b4d6f8a".to_string(),
            alnum(40),
            format!("MY_{}", alnum(20)),
            "A".repeat(100),
        ];
        for key in &keys {
            assert!(!is_variable_name(key), "{key}");
            let mut config = minimal();
            config.stt.parakeet.model_dir = PathBuf::from("m");
            config.stt.parakeet.threads = 1;
            config.stt.openai_compatible.api_key_env = key.clone();
            // The problem comes for both engines and never repeats the value.
            for engine in ["parakeet", "openai_compatible"] {
                config.stt.engine = engine.to_string();
                let problems = config.validate();
                assert!(
                    problems.iter().any(|p| p == API_KEY_ENV_PROBLEM),
                    "{engine} {problems:?}"
                );
                assert!(!problems.iter().any(|p| p.contains(key.as_str())));
            }
            assert_eq!(safe_variable_name(key), "(not a valid variable name)");
        }
        for name in [
            "OPENAI_API_KEY",
            "GROQ_API_KEY",
            "VOICEBRIDGE_UI_TEST_KEY",
            "MY_KEY_2",
            "API_KEY_OPENAI",
            "VOICEBRIDGE_UI_TEST_NO_SUCH_KEY",
            "ELEVENLABS_API_KEY",
            "SKY_KEY",
        ] {
            assert!(is_variable_name(name), "{name}");
        }
    }

    #[test]
    fn test_api_key_env_must_be_a_variable_name() {
        for good in ["OPENAI_API_KEY", "_X", "a1", &"A".repeat(64)] {
            assert!(is_variable_name(good), "{good}");
        }
        let long = "A".repeat(65);
        for bad in [
            "",
            "sk-proj-abc123",
            "1ABC",
            "MY KEY",
            "KEY\n",
            long.as_str(),
            "Schl\u{fc}ssel",
        ] {
            assert!(!is_variable_name(bad), "{bad}");
        }
        let mut config = minimal();
        config.stt.parakeet.model_dir = PathBuf::from("m");
        config.stt.parakeet.threads = 1;
        assert!(config.validate().is_empty());
        let secret = "sk-proj-SECRET123";
        config.stt.openai_compatible.api_key_env = secret.to_string();
        let problems = config.validate();
        assert_eq!(problems, vec![API_KEY_ENV_PROBLEM.to_string()]);
        assert!(!format!("{problems:?}").contains("SECRET"));
        // Also for the local engine: the file must never hold a key.
        assert_eq!(
            config.validate_keyed()[0].0,
            "stt.openai_compatible.api_key_env"
        );
    }

    #[test]
    fn test_url_origin_and_loopback() {
        let cases = [
            ("https://api.openai.com/v1", Some("https://api.openai.com")),
            (
                "HTTPS://API.OpenAI.com:443/v1/",
                Some("https://api.openai.com"),
            ),
            (
                "https://api.openai.com:8443",
                Some("https://api.openai.com:8443"),
            ),
            (
                "http://user:pw@Example.org:80/x?y#z",
                Some("http://example.org"),
            ),
            ("http://[::1]:8080/v1", Some("http://[::1]:8080")),
            ("http://localhost:11434/v1", Some("http://localhost:11434")),
            ("ftp://example.org", None),
            ("https://", None),
            ("api.openai.com/v1", None),
            ("", None),
            ("https://host:notaport/", None),
            // The rules of the HTTP client: a backslash is a slash, tabs and
            // line breaks go away, the host is lower case and in punycode.
            (
                "https://good.example\\@evil.example/v1",
                Some("https://good.example"),
            ),
            ("https://good.ex\tample/v1", Some("https://good.example")),
            (
                "https://b\u{fc}cher.example",
                Some("https://xn--bcher-kva.example"),
            ),
            ("https://%65vil.example/v1", Some("https://evil.example")),
            ("https://good.example./v1", Some("https://good.example.")),
        ];
        for (url, expected) in cases {
            assert_eq!(url_origin(url).as_deref(), expected, "{url}");
        }
        for url in [
            "http://localhost:8000",
            "http://127.0.0.1/v1",
            "http://127.1.2.3:9",
            "http://[::1]/",
        ] {
            assert!(is_loopback_url(url), "{url}");
        }
        for url in [
            "http://example.org",
            "http://192.168.1.2",
            "http://localhost.evil.org",
        ] {
            assert!(!is_loopback_url(url), "{url}");
        }
    }

    /// Each hostile `base_url` is refused, and the problem text never shows
    /// the user information of the input.
    #[test]
    fn test_validate_refuses_a_hostile_base_url() {
        let mut config = minimal();
        // The cloud engine checks the whole URL. The local engine checks
        // only user data (see the test of the empty cloud fields).
        config.stt.engine = "openai_compatible".to_string();
        let refused = [
            "https://good.example\\@evil.example/v1",
            "https://good.example@evil.example/v1",
            "https://SECRETUSER:SECRETPW@evil.example/v1",
            "https://SECRETUSER@evil.example/v1",
            "https://good.example\t/v1",
            "https://good.ex\nample/v1",
            "https://good.example /v1",
            " https://good.example/v1",
            "ftp://good.example/v1",
            "file:///C:/x",
            "https://",
            "not a url",
            "api.openai.com/v1",
            "",
        ];
        for url in refused {
            config.stt.openai_compatible.base_url = url.to_string();
            let keyed = config.validate_keyed();
            assert_eq!(keyed.len(), 1, "{url:?}: {keyed:?}");
            assert_eq!(keyed[0].0, "stt.openai_compatible.base_url", "{url:?}");
            assert!(!keyed[0].1.contains("SECRET"), "{url:?}: {}", keyed[0].1);
            assert!(!keyed[0].1.contains("good.example@"), "{}", keyed[0].1);
        }
        for url in [
            "https://api.openai.com/v1",
            "https://API.OpenAI.com:443/v1/",
            "http://localhost:11434/v1",
            "http://[::1]:8080/v1",
            "https://b\u{fc}cher.example/v1",
        ] {
            config.stt.openai_compatible.base_url = url.to_string();
            assert!(
                config.validate().is_empty(),
                "{url}: {:?}",
                config.validate()
            );
        }
    }

    /// A local configuration with empty cloud fields was valid on master.
    #[test]
    fn test_empty_cloud_fields_do_not_block_the_local_engine() {
        let text = "[edcopilot]\ndir = 'ed'\n[stt]\nengine = 'parakeet'\n\
                    [stt.parakeet]\nmodel_dir = 'm'\nthreads = 1\n\
                    [stt.openai_compatible]\nbase_url = ''\napi_key_env = ''\nmodel = ''\n\
                    language = ''\ntimeout_seconds = 0\n";
        // An absolute folder: a relative request file is a problem of its own.
        let base = Path::new(env!("CARGO_MANIFEST_DIR"));
        let config = Config::parse(text, base, "t.toml").unwrap();
        assert_eq!(config.validate(), Vec::<String>::new());
        // Odd values of the cloud engine do not block the local engine.
        let mut config = config;
        for url in [
            "not a url",
            "ftp://x.example",
            "https://a.example\\b",
            "https://",
        ] {
            config.stt.openai_compatible.base_url = url.to_string();
            assert_eq!(config.validate(), Vec::<String>::new(), "{url}");
        }
        config.stt.openai_compatible.timeout_seconds = 9_999;
        assert_eq!(config.validate(), Vec::<String>::new());
        // A secret in the file is a problem for both engines.
        config.stt.openai_compatible.base_url = "https://SECRETUSER:SECRETPW@x.example/v1".into();
        let keyed = config.validate_keyed();
        assert_eq!(keyed.len(), 1, "{keyed:?}");
        assert_eq!(keyed[0].0, "stt.openai_compatible.base_url");
        assert!(!keyed[0].1.contains("SECRET"), "{}", keyed[0].1);
        config.stt.openai_compatible.base_url = String::new();
        config.stt.openai_compatible.api_key_env = "sk-proj-SECRET".into();
        assert_eq!(config.validate(), vec![API_KEY_ENV_PROBLEM.to_string()]);
    }

    /// The cloud engine checks its own fields. An empty name is a problem of
    /// the engine (the key is absent), not of `validate`.
    #[test]
    fn test_cloud_engine_checks_its_fields() {
        let mut config = minimal();
        config.stt.engine = "openai_compatible".into();
        assert_eq!(config.validate(), Vec::<String>::new());
        config.stt.openai_compatible.api_key_env = String::new();
        assert_eq!(config.validate(), Vec::<String>::new());
        let cases: [(&str, &str); 3] = [
            ("stt.openai_compatible.base_url", "base_url"),
            ("stt.openai_compatible.base_url", "base_url"),
            ("stt.openai_compatible.timeout_seconds", "timeout"),
        ];
        config.stt.openai_compatible.base_url = String::new();
        assert_eq!(config.validate_keyed()[0].0, cases[0].0);
        config.stt.openai_compatible.base_url = "not a url".into();
        assert_eq!(config.validate_keyed()[0].0, cases[1].0);
        config.stt.openai_compatible.base_url = "https://api.openai.com/v1".into();
        config.stt.openai_compatible.timeout_seconds = 0;
        assert_eq!(config.validate_keyed()[0].0, cases[2].0);
    }

    /// The example file with an empty variable name, for each engine.
    #[test]
    fn test_example_with_an_empty_variable_name_for_each_engine() {
        let example = include_str!("../voicebridge.example.toml")
            .replace("api_key_env = \"OPENAI_API_KEY\"", "api_key_env = \"\"");
        assert!(example.contains("api_key_env = \"\""));
        for engine in ["parakeet", "openai_compatible"] {
            let text = example.replace("engine = \"parakeet\"", &format!("engine = \"{engine}\""));
            let mut config = Config::parse(&text, Path::new("."), "example").unwrap();
            config.stt.parakeet.model_dir = PathBuf::from("models/parakeet");
            config.stt.parakeet.threads = 1;
            assert_eq!(config.validate(), Vec::<String>::new(), "{engine}");
        }
    }

    #[test]
    fn test_validate_refuses_numbers_that_are_not_finite_and_the_timeout_range() {
        let mut config = minimal();
        config.stt.parakeet.model_dir = PathBuf::from("m");
        config.stt.parakeet.threads = 1;
        config.microphone.mode = "always_listening".to_string();
        config.microphone.vad_threshold_dbfs = f32::NAN;
        config.stt.parakeet.hotwords_file = PathBuf::from("hot.txt");
        config.stt.parakeet.hotwords_score = f32::INFINITY;
        // The local engine does not use the timeout.
        config.stt.openai_compatible.timeout_seconds = 0;
        let keyed = config.validate_keyed();
        let keys: Vec<&str> = keyed.iter().map(|(k, _)| *k).collect();
        assert_eq!(
            keys,
            [
                "stt.parakeet.hotwords_score",
                "microphone.vad_threshold_dbfs"
            ],
            "{keyed:?}"
        );
        // The push-to-talk mode does not use the voice threshold.
        config.microphone.mode = "push_to_talk".to_string();
        assert_eq!(config.validate_keyed().len(), 1);
        config.stt.parakeet.hotwords_score = 1.5;
        config.stt.engine = "openai_compatible".to_string();
        let keyed = config.validate_keyed();
        assert_eq!(keyed.len(), 1, "{keyed:?}");
        assert_eq!(keyed[0].0, "stt.openai_compatible.timeout_seconds");
        assert!(keyed[0].1.contains("from 1 to 600"), "{keyed:?}");
        for (seconds, ok) in [(1, true), (600, true), (601, false)] {
            config.stt.openai_compatible.timeout_seconds = seconds;
            assert_eq!(config.validate().is_empty(), ok, "{seconds}");
        }
    }

    #[test]
    fn test_validate_keyed_names_the_key_of_each_problem() {
        let mut config = minimal();
        config.matching.threshold = 1.5;
        config.push_to_talk.key = Some("NoSuchKey".to_string());
        config.push_to_talk.pre_roll_ms = 9_999;
        config.stt.parakeet.threads = 0;
        let keyed = config.validate_keyed();
        let keys: Vec<&str> = keyed.iter().map(|(k, _)| *k).collect();
        assert_eq!(
            keys,
            [
                "matching.threshold",
                "push_to_talk.key",
                "push_to_talk.pre_roll_ms",
                "stt.parakeet.model_dir",
                "stt.parakeet.threads"
            ]
        );
        // The texts are the texts of `validate`, in the same order.
        let texts: Vec<String> = keyed.into_iter().map(|(_, t)| t).collect();
        assert_eq!(texts, config.validate());
        assert!(texts.iter().all(|t| t.starts_with("matching.")
            || t.starts_with("push_to_talk.")
            || t.starts_with("stt.")));
    }

    #[test]
    fn test_load_as_written_keeps_the_values_of_the_file() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("voicebridge.toml");
        fs::write(
            &file,
            "[edcopilot]\ndir = 'ed'\n[data]\nsynonyms = 'my.toml'\n",
        )
        .unwrap();
        let config = Config::load_as_written(&file).unwrap();
        assert_eq!(config.edcopilot.dir, Path::new("ed"));
        assert!(config.edcopilot.request_file.as_os_str().is_empty());
        assert_eq!(config.data.synonyms, Path::new("my.toml"));
        assert_eq!(config.push_to_talk.key, None);
        fs::write(&file, "[edcopilot\n").unwrap();
        let error = Config::load_as_written(&file).unwrap_err();
        assert!(
            format!("{error:#}").contains("voicebridge.toml"),
            "{error:#}"
        );
    }

    #[test]
    fn test_load_names_the_file() {
        let error = Config::load(Path::new("no/such/voicebridge.toml")).unwrap_err();
        assert!(format!("{error:#}").contains("voicebridge.toml"));
    }

    #[test]
    fn test_load_report_counts_and_problems() {
        let dir = tempfile::tempdir().unwrap();
        // The invented EDCoPilot folder of the fixture (profile mode).
        let ed = dir.path().join("ed");
        edcopilot_voicebridge_core::profile::fixture::write_fixture_edcopilot(&ed);
        let ini = ed.join("working").join("EDCoPilot.VoiceCommands.ini");
        let no_aliases = dir.path().join("no-aliases");
        fs::create_dir_all(&no_aliases).unwrap();

        let core = Path::new(env!("CARGO_MANIFEST_DIR")).join("../core");
        let mut config = Config::parse(
            &format!("[edcopilot]\ndir = {:?}\n", ed.to_string_lossy()),
            dir.path(),
            "t.toml",
        )
        .unwrap();
        config.data.aliases_dir = no_aliases.clone();
        config.data.sound_alikes = core.join("data/sound_alikes.toml");
        config.data.synonyms = core.join("data/synonyms.toml");
        config.data.fillers = core.join("data/fillers.toml");

        let (_pipeline, report) = build_pipeline(&config).unwrap();
        // The counts of the shipped word tables.
        assert_eq!(report.sound_alikes, 13);
        assert_eq!(report.synonyms, 6);
        assert!(report.fillers > 0);
        assert!(report.fillers_active);
        assert!(
            report.synonym_warnings.is_empty(),
            "{:?}",
            report.synonym_warnings
        );
        // The number check of the runtime grammar takes out fixture
        // records. They are left-out commands of the profile report, not
        // warnings.
        let counts = edcopilot_voicebridge_core::profile::fixture::fixture_counts();
        assert_eq!(report.command_warnings, [FIXTURE_SHADOW]);
        assert_eq!(report.commands, counts.loaded);
        let profile = &report.profile;
        assert_eq!(profile.records, counts.loaded);
        // The prefixes and suffixes of the market group. The INI of the
        // fixture resolves the runtime grammar of each suffix.
        assert_eq!(report.composite_parts, counts.composite_parts);
        // The vocabulary files of the fixture. "Not set" gives no name.
        assert_eq!(report.vocabulary_files, counts.vocabulary_files);
        assert_eq!(report.vocabulary_names, counts.vocabulary_names);
        assert!(report.matchable > 0);
        // `{TXT:EDCP_GATECHIME}` is absent from the INI.
        assert_eq!(report.unresolved_grammar, 1);
        assert!(report.matchable + report.unresolved_grammar <= report.commands);
        assert!(report.problems.is_empty(), "{:?}", report.problems);
        assert!(
            report.alias_warnings.is_empty(),
            "{:?}",
            report.alias_warnings
        );

        // An alias key that no command takes gives a warning with the file.
        let aliases = dir.path().join("aliases");
        fs::create_dir_all(&aliases).unwrap();
        let file = aliases.join("xx.toml");
        fs::write(
            &file,
            "'OpenTideLogPanel' = [\"gezeitenbuch\"]\n'NoSuchToken' = [\"gibt es nicht\"]\n\
             'HarbourInfo:{TXT:SystemName}:{CMD}' = [\"erzähl mir von {TXT:System1}\"]\n",
        )
        .unwrap();
        config.data.aliases_dir = aliases;
        let (_pipeline, report) = build_pipeline(&config).unwrap();
        // A key with {CMD}: the command works, only the alias cannot build
        // the token.
        assert_eq!(
            report.alias_warnings,
            [
                format!(
                    "alias file {}: the token \"HarbourInfo:{{TXT:SystemName}}:{{CMD}}\" needs the English command text, an alias cannot give it, the program sends no command for its phrases",
                    file.display()
                ),
                format!(
                    "alias file {}: the token \"NoSuchToken\" matches no command that works, the program ignores its phrases",
                    file.display()
                ),
            ]
        );
        config.data.aliases_dir = no_aliases;

        fs::remove_file(&ini).unwrap();
        let (_pipeline, report) = build_pipeline(&config).unwrap();
        assert_eq!(report.problems.len(), 1);
        assert!(report.problems[0].contains(&ini.to_string_lossy().to_string()));

        // No profile: the load is an error.
        fs::remove_file(ed.join(crate::commands_source::PROFILE_FILE)).unwrap();
        assert!(build_pipeline(&config).is_err());
    }

    /// A missing sound-alike file is a problem line and an empty table.
    #[test]
    fn test_missing_sound_alike_file_is_a_problem() {
        let dir = tempfile::tempdir().unwrap();
        let mut config = crate::commands_source::test_support::fixture_config(dir.path(), "");
        config.data.sound_alikes = dir.path().join("missing.toml");
        let (_pipeline, report) = build_pipeline(&config).unwrap();
        assert_eq!(report.sound_alikes, 0);
        let missing = dir
            .path()
            .join("missing.toml")
            .to_string_lossy()
            .to_string();
        assert!(
            report.problems.iter().any(|p| p.contains(&missing)),
            "{:?}",
            report.problems
        );
    }

    #[test]
    fn test_default_sound_alike_path_resolves_against_the_config_directory() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("voicebridge.toml");
        fs::write(
            &file,
            "[edcopilot]
dir = \"ed\"
",
        )
        .unwrap();
        let config = Config::load(&file).unwrap();
        assert_eq!(
            config.data.sound_alikes,
            dir.path().join("core/data/sound_alikes.toml")
        );
        assert_eq!(
            config.data.fillers,
            dir.path().join("core/data/fillers.toml")
        );
    }

    /// A config for `build_pipeline` with the invented EDCoPilot folder of
    /// the fixture and the shipped word tables.
    fn fixture_map_config(dir: &Path) -> Config {
        crate::commands_source::test_support::fixture_config(dir, "")
    }

    /// A text that matches only through the filler step: the fixture
    /// command `[nearest berth;] [size;] [1..4;] [old; new; spare;] steam
    /// crane` with "for a" next to the number.
    const FILLER_TEXT: &str = "nearest berth for a size 3 steam crane";

    /// A missing filler file is a problem line and an empty table, as for
    /// the other tables. The pipeline still loads.
    #[test]
    fn test_missing_filler_file_is_a_problem() {
        let dir = tempfile::tempdir().unwrap();
        let mut config = fixture_map_config(dir.path());
        config.data.fillers = dir.path().join("missing.toml");
        let (pipeline, report) = build_pipeline(&config).unwrap();
        assert_eq!(report.fillers, 0);
        let missing = dir
            .path()
            .join("missing.toml")
            .to_string_lossy()
            .to_string();
        assert!(
            report.problems.iter().any(|p| p.contains(&missing)),
            "{:?}",
            report.problems
        );
        assert_eq!(pipeline.match_text(FILLER_TEXT), None);
    }

    /// In always-listening mode the pipeline has no filler step.
    #[test]
    fn test_fillers_are_off_in_always_listening_mode() {
        let dir = tempfile::tempdir().unwrap();
        let mut config = fixture_map_config(dir.path());
        let text = FILLER_TEXT;
        let (pipeline, report) = build_pipeline(&config).unwrap();
        assert!(report.fillers_active);
        assert!(pipeline.match_text(text).is_some_and(|m| m.via_filler));
        config.microphone.mode = "always_listening".to_string();
        let (pipeline, report) = build_pipeline(&config).unwrap();
        assert!(report.fillers > 0);
        assert!(!report.fillers_active);
        assert_eq!(pipeline.match_text(text), None);
    }
    /// The vocabulary folder of a fixture configuration.
    fn vocab_dir(config: &Config) -> PathBuf {
        config.edcopilot.dynamic_vars_dir.clone()
    }

    /// Gives the system files 1 to `n` of the fixture a form that the
    /// program does not know: a list with one name and no separator.
    fn break_system_files(config: &Config, n: usize) {
        for k in 1..=n {
            let file = vocab_dir(config).join(format!("EDCoPilotSystem{k}.txt"));
            fs::write(file, format!("[Harbour Light {k}]")).unwrap();
        }
    }

    /// The `check` warnings about the form of the vocabulary files.
    fn form_warnings(config: &Config) -> Vec<String> {
        let found = crate::run::check_config_in(config, |_| Some("k".into()), None);
        found
            .warnings
            .into_iter()
            .filter(|w| w.contains("form that the program does not know"))
            .collect()
    }

    #[test]
    fn test_check_warns_one_time_for_a_vocabulary_file_of_unknown_form() {
        let dir = tempfile::tempdir().unwrap();
        let config = crate::commands_source::test_support::fixture_config(dir.path(), "");
        assert_eq!(form_warnings(&config), Vec::<String>::new());
        break_system_files(&config, 1);
        assert_eq!(
            form_warnings(&config),
            [
                "the vocabulary file EDCoPilotSystem1.txt has a form that the program does not \
              know, its names are not used"
            ]
        );
    }

    #[test]
    fn test_up_to_five_files_of_unknown_form_give_one_line_each_and_more_give_one_line() {
        let dir = tempfile::tempdir().unwrap();
        let config = crate::commands_source::test_support::fixture_config(dir.path(), "");
        break_system_files(&config, 5);
        let five: Vec<String> = (1..=5)
            .map(|k| {
                format!(
                    "the vocabulary file EDCoPilotSystem{k}.txt has a form that the program \
                     does not know, its names are not used"
                )
            })
            .collect();
        assert_eq!(form_warnings(&config), five);
        break_system_files(&config, 6);
        assert_eq!(
            form_warnings(&config),
            [
                "6 vocabulary files have a form that the program does not know, their names \
              are not used: EDCoPilotSystem1.txt, EDCoPilotSystem2.txt, EDCoPilotSystem3.txt, \
              EDCoPilotSystem4.txt, EDCoPilotSystem5.txt, ..."
            ]
        );
    }

    /// The keys of an older version, as an older first start or the old
    /// example wrote them.
    const OLD_KEYS: &str = "command_map = 'C:\\Program\\resources\\data\\command_map_v3.json'\n\
                            profile_vars = 'C:\\Program\\resources\\data\\profile_vars.json'\n";

    /// A fixture configuration with `data_extra` in `[data]`, in its own
    /// folder.
    fn config_with(data_extra: &str) -> (tempfile::TempDir, Config) {
        let dir = tempfile::tempdir().unwrap();
        let config =
            crate::commands_source::test_support::fixture_config_with(dir.path(), data_extra, "");
        (dir, config)
    }

    /// A left-out command whose phrase the program cannot read (here: a
    /// runtime grammar variable that the voice commands file does not have)
    /// does not block its text. The load says so in one warning.
    #[test]
    fn test_skipped_blocking_phrases_give_one_warning() {
        use edcopilot_voicebridge_core::profile::{fixture, test_support::sound};
        let (_dir, config) = config_with("");
        // An invented command that writes no request: a left-out command.
        let mut b = fixture::fixture_builder();
        b.category("Logbook")
            .full("{TXT:EDCP_INVENTEDGAP} the invented bell", vec![sound()]);
        fs::write(&config.edcopilot.profile, b.build()).unwrap();
        let loaded = load_commands(&config).unwrap();
        assert_eq!(loaded.blocking_skipped, 1);
        let line = format!(
            "{} left-out commands have a phrase that the program cannot read, their text is not \
             blocked",
            loaded.blocking_skipped
        );
        assert_eq!(
            loaded.warnings.iter().filter(|w| **w == line).count(),
            1,
            "{:?}",
            loaded.warnings
        );
        // The check shows it as a warning.
        let found = crate::run::check_config_in(&config, |_| Some("key".into()), None);
        assert!(found.warnings.contains(&line), "{found:?}");
        // No line without a skipped phrase.
        let (_other, config) = config_with("");
        let loaded = load_commands(&config).unwrap();
        assert_eq!(loaded.blocking_skipped, 0);
        assert!(!loaded.warnings.iter().any(|w| w.contains("cannot read")));
    }

    #[test]
    fn test_shadowed_phrases_give_one_warning_each_and_at_most_ten() {
        let (_dir, config) = config_with("");
        assert_eq!(load_commands(&config).unwrap().warnings, [FIXTURE_SHADOW]);
        let (_, report) = build_pipeline(&config).unwrap();
        assert_eq!(report.command_warnings, [FIXTURE_SHADOW]);
        // 12 working commands with a text of a value command.
        let command = |phrase: String, class: CommandClass| Command {
            phrase: Some(phrase),
            token_template: "Rope={TXT:Length}".into(),
            class,
            ..Default::default()
        };
        let mut commands = Vec::new();
        for n in 1..=12 {
            commands.push(command(
                format!("coil rope {n}"),
                CommandClass::DynamicValue,
            ));
            commands.push(command(
                format!("[coil rope {n}; stow rope {n}]"),
                CommandClass::Fixed,
            ));
        }
        let lines = shadow_warnings(&commands, &[]);
        assert_eq!(lines.len(), MAX_SHADOW_WARNINGS + 1);
        assert_eq!(
            lines[0],
            "the phrase \"coil rope 1\" of the command \"[coil rope 1; stow rope 1]\" is also a \
             phrase of a command that the program cannot send, the program does not send a \
             command for it"
        );
        assert_eq!(lines[MAX_SHADOW_WARNINGS], "... and 2 more");
        assert_eq!(
            shadow_warnings(&commands[..20], &[]).len(),
            MAX_SHADOW_WARNINGS
        );
        assert!(shadow_warnings(&commands[..1], &[]).is_empty());
        // A left-out command shadows with the same text.
        let left_out = [BlockingPhrase {
            phrase: "stow rope 3".to_string(),
            class: LeftOutClass::NoWrite,
            command_type: edcopilot_voicebridge_core::CommandType::Full,
            composite_group: None,
        }];
        let fixed: Vec<Command> = commands
            .iter()
            .filter(|c| c.class == CommandClass::Fixed)
            .cloned()
            .collect();
        assert_eq!(
            shadow_warnings(&fixed, &left_out),
            ["the phrase \"stow rope 3\" of the command \"[coil rope 3; stow rope 3]\" is also a \
              phrase of a command that the program cannot send, the program does not send a \
              command for it"]
        );
    }

    /// The load gives the blocking phrases of the left-out commands, and
    /// the pipeline keeps them: the text of a left-out command gives no
    /// command, also through a sound-alike variant or a fuzzy word. The
    /// commands do not have them.
    #[test]
    fn test_the_pipeline_blocks_the_text_of_a_left_out_command() {
        use edcopilot_voicebridge_core::{BlockReason, MatchResult};
        let (_dir, config) = config_with("");
        let counts = edcopilot_voicebridge_core::profile::fixture::fixture_counts();
        let loaded = load_commands(&config).unwrap();
        assert_eq!(loaded.blocking.len(), counts.blocking_loaded);
        assert_eq!(loaded.blocking_skipped, counts.blocking_skipped);
        assert_eq!(loaded.commands.len(), counts.loaded);
        let phrases: BTreeSet<&str> = loaded.blocking.iter().map(|b| b.phrase.as_str()).collect();
        assert!(loaded
            .commands
            .iter()
            .all(|c| !phrases.contains(c.phrase.as_deref().unwrap_or(""))));
        let (pipeline, _) = build_pipeline(&config).unwrap();
        for text in ["lock the wind speed", "test the lamp", "ring all bells"] {
            assert_eq!(
                pipeline.match_result(text),
                MatchResult::Blocked(BlockReason::NoToken),
                "{text}"
            );
        }
        for (text, token) in [
            ("log the wind speed", "LogWind:Speed"),
            ("test the lamps", "Lamp:TestAll"),
        ] {
            assert_eq!(
                pipeline.match_text(text).map(|m| m.token).as_deref(),
                Some(token),
                "{text}"
            );
        }
    }

    #[test]
    fn test_old_map_keys_are_ignored_with_one_warning() {
        assert_eq!(
            OLD_MAP_KEYS,
            "data.command_map and data.profile_vars are not used any more, the program reads \
             the EDCoPilot profile"
        );
        // Each form of the old keys: both, one of them, empty values, a map
        // file of the user, and values that are not a text (a number, an
        // inline table, an array, a bool). The last form is a sub-table
        // `[data.command_map]` after the other tables: the second text goes
        // after `[data]`.
        let forms = [
            (OLD_KEYS, ""),
            ("command_map = 'mine.json'\n", ""),
            ("profile_vars = ''\n", ""),
            ("command_map = ''\nprofile_vars = ''\n", ""),
            ("command_map = 7\n", ""),
            ("command_map = { path = 'mine.json', version = 3 }\n", ""),
            ("profile_vars = ['a.json', 'b.json']\n", ""),
            ("command_map = true\n", ""),
            ("", "[data.command_map]\npath = 'mine.json'\nversion = 3\n"),
        ];
        for (data_keys, tables) in forms {
            let keys = format!("{data_keys}{tables}");
            let dir = tempfile::tempdir().unwrap();
            let config = crate::commands_source::test_support::fixture_config_with(
                dir.path(),
                data_keys,
                tables,
            );
            assert!(config.has_old_map_keys(), "{keys}");
            let loaded = load_commands(&config).unwrap();
            assert_eq!(loaded.warnings, [OLD_MAP_KEYS, FIXTURE_SHADOW], "{keys}");
            let found = crate::run::check_config_in(&config, |_| Some("k".into()), None);
            assert_eq!(
                found.warnings.iter().filter(|w| *w == OLD_MAP_KEYS).count(),
                1,
                "{keys}: {:?}",
                found.warnings
            );
            assert!(found.problems.is_empty(), "{keys}: {:?}", found.problems);
        }
        // Without the keys: no warning of the keys.
        let (_dir, config) = config_with("");
        assert!(!config.has_old_map_keys());
        assert_eq!(load_commands(&config).unwrap().warnings, [FIXTURE_SHADOW]);
    }

    /// A configuration with the old keys loads the same commands, the same
    /// report and the same problems as one without them. Only the warning
    /// is new. The keys name no file that exists: nothing reads them.
    #[test]
    fn test_a_configuration_with_the_old_keys_behaves_as_one_without() {
        let (_a, with) = config_with(OLD_KEYS);
        let (_b, without) = config_with("");
        let (loaded_with, loaded_without) = (
            load_commands(&with).unwrap(),
            load_commands(&without).unwrap(),
        );
        assert_eq!(loaded_with.commands, loaded_without.commands);
        assert_eq!(loaded_with.profile, loaded_without.profile);
        let (_, mut report_with) = build_pipeline(&with).unwrap();
        let (_, report_without) = build_pipeline(&without).unwrap();
        assert_eq!(report_with.command_warnings, [OLD_MAP_KEYS, FIXTURE_SHADOW]);
        report_with.command_warnings.retain(|w| w != OLD_MAP_KEYS);
        assert_eq!(format!("{report_with:?}"), format!("{report_without:?}"));
        // The form check of the vocabulary runs as without the keys.
        break_system_files(&with, 1);
        assert_eq!(form_warnings(&with).len(), 1);
        // A profile that does not load is a problem, also with the keys,
        // and the check still gives the warning one time.
        fs::remove_file(&with.edcopilot.profile).unwrap();
        assert!(load_commands(&with).is_err());
        let found = crate::run::check_config_in(&with, |_| Some("k".into()), None);
        assert!(found.load_problem.is_some());
        assert_eq!(
            found.warnings.iter().filter(|w| *w == OLD_MAP_KEYS).count(),
            1,
            "{:?}",
            found.warnings
        );
    }
}
