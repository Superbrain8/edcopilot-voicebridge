//! The serializable types for the web part.
//!
//! The types of the `app` crate have no serde derives. This module converts
//! them. No type here holds an API key: the configuration gives only
//! `api_key_set`.

use serde::{Deserialize, Serialize};

use edcopilot_voicebridge_core::profile::{BuildReport, LeftOut};
use voicebridge::bind::Binding;
use voicebridge::commands_source::{left_out_line, profile_lines, ProfileChange};
use voicebridge::config::{
    is_variable_name, CommandState, Config, LoadReport, EDCOPILOT_DIR_EMPTY,
    EDCOPILOT_PROFILE_EMPTY,
};
use voicebridge::config_edit::Value;
use voicebridge::diagnostics::{Levels, Marks};
use voicebridge::list::{self, Entry};
use voicebridge::run::{report_line, CheckReport};
use voicebridge::runtime::{Mode, RuntimeEvent, StartInfo, StopReason, UtteranceReport};
use voicebridge::session::Outcome;
use voicebridge::tables::{Row, TableCheck, TableFile, TableKind, TableSave};

/// The new value of one key. JSON: `{ "kind": "integer", "value": 6 }`, or
/// `{ "kind": "absent" }` to remove the key.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
pub enum ValueDto {
    Text(String),
    Number(f64),
    Integer(i64),
    Flag(bool),
    TextList(Vec<String>),
    Absent,
}

impl From<ValueDto> for Value {
    fn from(value: ValueDto) -> Value {
        match value {
            ValueDto::Text(text) => Value::Text(text),
            ValueDto::Number(n) => Value::Number(n),
            ValueDto::Integer(n) => Value::Integer(n),
            ValueDto::Flag(flag) => Value::Flag(flag),
            ValueDto::TextList(list) => Value::TextList(list),
            ValueDto::Absent => Value::Absent,
        }
    }
}

/// One edit of the configuration, for example
/// `{ "key": "stt.parakeet.threads", "value": { "kind": "integer", "value": 6 } }`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EditDto {
    pub key: String,
    pub value: ValueDto,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SaveResultDto {
    /// With `written`: problems that the file had before and that the edits
    /// do not touch, as information. Without `written`: the problems that
    /// refused the save. Empty without `written`: nothing changed.
    pub problems: Vec<String>,
    /// True when the file was written.
    pub written: bool,
    /// The file was written while a session runs. The session keeps its old
    /// settings until it starts again.
    pub restart_needed: bool,
}

/// The result of `set_api_key` and `clear_api_key`. Never the key.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct KeyResultDto {
    /// A session that uses the cloud engine runs or starts. It keeps the
    /// key of its start until it starts again.
    pub restart_needed: bool,
}

fn path_text(path: &std::path::Path) -> String {
    path.display().to_string()
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct EdcopilotDto {
    pub dir: String,
    /// Empty: the default from `dir`.
    pub request_file: String,
    pub voice_commands_ini: String,
    pub dynamic_vars_dir: String,
    /// The value of `edcopilot.profile` as the file has it. Empty: the
    /// default from `dir`.
    pub profile: String,
    /// The profile file that the program reads: the value of the file, or
    /// the path from `dir`. Empty without both.
    pub profile_in_use: String,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct DataDto {
    pub aliases_dir: String,
    pub sound_alikes: String,
    pub synonyms: String,
    pub fillers: String,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct MatchingDto {
    pub threshold: f32,
    pub exact_single_word: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct MicrophoneDto {
    pub device: String,
    pub mode: String,
    pub vad_threshold_dbfs: f32,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct JoystickDto {
    pub device: String,
    pub button: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct PushToTalkDto {
    /// `None` and no joystick: the program uses the key F13.
    pub key: Option<String>,
    pub joystick: Option<JoystickDto>,
    pub pre_roll_ms: u32,
    pub release_tail_ms: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct AlwaysListeningDto {
    pub wake_words: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ParakeetDto {
    pub model_dir: String,
    pub threads: usize,
    pub hotwords_file: String,
    pub hotwords_score: f32,
}

/// The text when the `base_url` of the file holds a user name, a password,
/// a query or a fragment. It never repeats the value.
pub const BASE_URL_SECRET: &str = "the server address in the file holds a secret or a query, correct the file: the window does not show these parts and does not save a key for this address";

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct OpenAiCompatibleDto {
    /// As the file has it, or only the scheme, the host, the port and the
    /// path when `base_url_problem` is set.
    pub base_url: String,
    /// `BASE_URL_SECRET` when the value of the file holds a user name, a
    /// password, a query or a fragment. Else null.
    pub base_url_problem: Option<String>,
    /// The NAME of the variable. Never the key.
    pub api_key_env: String,
    pub model: String,
    pub language: String,
    pub timeout_seconds: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SttDto {
    pub engine: String,
    pub parakeet: ParakeetDto,
    pub openai_compatible: OpenAiCompatibleDto,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct DiagnosticsDto {
    pub log_file: String,
    pub save_audio_dir: String,
}

/// The texts of the start for the Status and Diagnostics screens.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct StartupNotesDto {
    /// The problem that stops the use of a configuration, for example a
    /// development build without `--config`.
    pub problem: Option<String>,
    /// The error of a failed first start, with the data folder.
    pub first_start_error: Option<String>,
    /// The notes of the update step at the start, for example the removal
    /// of the map keys of an older version.
    pub notes: Vec<String>,
    /// The errors of the update step. The configuration did not change.
    pub update_problems: Vec<String>,
    /// The data folder of the user, or null.
    pub data_dir: Option<String>,
}

/// The kind of a word table as the page names it. Serde refuses any other
/// name, so a command never gets a free text as the kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TableKindDto {
    Aliases,
    Synonyms,
    SoundAlikes,
    Fillers,
    Hotwords,
}

/// A part of the diagnostics that the Diagnostics screen switches or
/// opens. The page sends only this name, never a path. The Rust side takes
/// the path from the saved configuration.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DiagnosticsPart {
    /// The utterance log, `diagnostics.log_file`.
    Log,
    /// The audio capture, `diagnostics.save_audio_dir`.
    Audio,
    /// The data folder of the user. It has no switch, only "Open folder".
    Data,
}

/// All settings, with the values as the file has them (relative paths stay
/// relative). A key that the file does not have shows its default.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ConfigDto {
    /// The configuration file.
    pub path: String,
    /// True when the key store or the environment variable that
    /// `stt.openai_compatible.api_key_env` names has a key.
    pub api_key_set: bool,
    /// The host that the stored key was saved for, for example
    /// `https://api.openai.com`. The store gives the key only to this host.
    pub api_key_host: Option<String>,
    /// The hotword list that the hotword switch of the Setup screen writes,
    /// as an absolute path: `tables::default_hotwords_file`. The hotword
    /// table edits the same file while the hotwords are off.
    pub default_hotwords_file: String,
    /// The stamp of the file content. `save_config` gets it back.
    pub version: String,
    pub edcopilot: EdcopilotDto,
    pub data: DataDto,
    pub matching: MatchingDto,
    pub microphone: MicrophoneDto,
    pub push_to_talk: PushToTalkDto,
    pub always_listening: AlwaysListeningDto,
    pub stt: SttDto,
    pub diagnostics: DiagnosticsDto,
}

impl ConfigDto {
    /// `config` comes from `Config::load_as_written`. `profile_in_use` is
    /// `edcopilot.profile` of `Config::load`.
    pub fn new(
        path: &std::path::Path,
        config: &Config,
        api_key_set: bool,
        api_key_host: Option<String>,
        default_hotwords_file: &std::path::Path,
        profile_in_use: &std::path::Path,
        version: String,
    ) -> ConfigDto {
        let ed = &config.edcopilot;
        let data = &config.data;
        let ptt = &config.push_to_talk;
        let parakeet = &config.stt.parakeet;
        let cloud = &config.stt.openai_compatible;
        // A secret in the address never goes to the page.
        let safe_url = edcopilot_voicebridge_stt::base_url_without_secrets(&cloud.base_url);
        ConfigDto {
            path: path_text(path),
            api_key_set,
            api_key_host,
            default_hotwords_file: path_text(default_hotwords_file),
            version,
            edcopilot: EdcopilotDto {
                dir: path_text(&ed.dir),
                request_file: path_text(&ed.request_file),
                voice_commands_ini: path_text(&ed.voice_commands_ini),
                dynamic_vars_dir: path_text(&ed.dynamic_vars_dir),
                profile: path_text(&ed.profile),
                profile_in_use: path_text(profile_in_use),
            },
            data: DataDto {
                aliases_dir: path_text(&data.aliases_dir),
                sound_alikes: path_text(&data.sound_alikes),
                synonyms: path_text(&data.synonyms),
                fillers: path_text(&data.fillers),
            },
            matching: MatchingDto {
                threshold: config.matching.threshold,
                exact_single_word: config.matching.exact_single_word,
            },
            microphone: MicrophoneDto {
                device: config.microphone.device.clone(),
                mode: config.microphone.mode.clone(),
                vad_threshold_dbfs: config.microphone.vad_threshold_dbfs,
            },
            push_to_talk: PushToTalkDto {
                key: ptt.key.clone(),
                joystick: ptt.joystick.as_ref().map(|j| JoystickDto {
                    device: j.device.clone(),
                    button: j.button,
                }),
                pre_roll_ms: ptt.pre_roll_ms,
                release_tail_ms: ptt.release_tail_ms,
            },
            always_listening: AlwaysListeningDto {
                wake_words: config.always_listening.wake_words.clone(),
            },
            stt: SttDto {
                engine: config.stt.engine.clone(),
                parakeet: ParakeetDto {
                    model_dir: path_text(&parakeet.model_dir),
                    threads: parakeet.threads,
                    hotwords_file: path_text(&parakeet.hotwords_file),
                    hotwords_score: parakeet.hotwords_score,
                },
                openai_compatible: OpenAiCompatibleDto {
                    base_url: safe_url.clone().unwrap_or_else(|| cloud.base_url.clone()),
                    base_url_problem: safe_url.map(|_| BASE_URL_SECRET.to_string()),
                    // A value that is not a variable name can be a key: it is not shown.
                    api_key_env: if is_variable_name(&cloud.api_key_env) {
                        cloud.api_key_env.clone()
                    } else {
                        String::new()
                    },
                    model: cloud.model.clone(),
                    language: cloud.language.clone(),
                    timeout_seconds: cloud.timeout_seconds,
                },
            },
            diagnostics: DiagnosticsDto {
                log_file: path_text(&config.diagnostics.log_file),
                save_audio_dir: path_text(&config.diagnostics.save_audio_dir),
            },
        }
    }
}

/// The counts of the loaded data. `line` is the line of the console.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ReportDto {
    pub line: String,
    pub commands: usize,
    pub matchable: usize,
    pub unresolved_grammar: usize,
    pub composite_parts: usize,
    pub vocabulary_files: usize,
    pub vocabulary_names: usize,
    pub alias_phrases: usize,
    pub sound_alikes: usize,
    pub synonyms: usize,
    pub synonym_warnings: Vec<String>,
    pub fillers: usize,
    pub fillers_active: bool,
    pub filler_warnings: Vec<String>,
    pub problems: Vec<String>,
}

impl From<&LoadReport> for ReportDto {
    fn from(r: &LoadReport) -> ReportDto {
        ReportDto {
            line: report_line(r),
            commands: r.commands,
            matchable: r.matchable,
            unresolved_grammar: r.unresolved_grammar,
            composite_parts: r.composite_parts,
            vocabulary_files: r.vocabulary_files,
            vocabulary_names: r.vocabulary_names,
            alias_phrases: r.alias_phrases,
            sound_alikes: r.sound_alikes,
            synonyms: r.synonyms,
            synonym_warnings: r.synonym_warnings.clone(),
            fillers: r.fillers,
            fillers_active: r.fillers_active,
            filler_warnings: r.filler_warnings.clone(),
            problems: r.problems.clone(),
        }
    }
}

/// The report of the build from the EDCoPilot profile, with the texts of
/// `voicebridge check`. The screens show `lines`. The two counts are for a
/// summary of a screen.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ProfileReportDto {
    /// The commands of the map.
    pub records: usize,
    /// The number of left-out commands.
    pub left_out: usize,
    /// All lines as `check` prints them, in its order: the count line, the
    /// format note, the left-out lines, and the change line.
    pub lines: Vec<String>,
}

impl ProfileReportDto {
    pub fn new(report: &BuildReport, change: Option<&ProfileChange>) -> ProfileReportDto {
        ProfileReportDto {
            records: report.records,
            left_out: report.left_out.len(),
            lines: profile_lines(report, change, false),
        }
    }
}

/// The content of `voicebridge check`.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct CheckDto {
    /// Each problem refuses the start.
    pub problems: Vec<String>,
    /// Without the prefix `warning: `.
    pub warnings: Vec<String>,
    /// `None` when the configuration or the data did not load.
    pub report: Option<ReportDto>,
    /// The report of the build from the EDCoPilot profile. `None` when the
    /// commands did not load.
    pub profile: Option<ProfileReportDto>,
    /// The problems that the Setup screen shows at the field of the
    /// EDCoPilot folder: an empty folder, and a profile that does not load.
    /// Each one is also in `problems`.
    pub edcopilot_problems: Vec<String>,
    /// The lines of the talk button, the utterance log and the audio capture.
    pub info: Vec<String>,
}

impl CheckDto {
    pub fn from_report(found: &CheckReport) -> CheckDto {
        let profile = found
            .report
            .as_ref()
            .map(|r| ProfileReportDto::new(&r.profile, found.profile_change.as_ref()));
        let mut edcopilot_problems: Vec<String> = found
            .problems
            .iter()
            .filter(|p| *p == EDCOPILOT_DIR_EMPTY || *p == EDCOPILOT_PROFILE_EMPTY)
            .cloned()
            .collect();
        // A load error concerns the profile of the EDCoPilot folder.
        if let Some(problem) = &found.load_problem {
            if !edcopilot_problems.contains(problem) {
                edcopilot_problems.push(problem.clone());
            }
        }
        CheckDto {
            problems: found.problems.clone(),
            warnings: found.warnings.clone(),
            report: found.report.as_ref().map(ReportDto::from),
            profile,
            edcopilot_problems,
            info: found.info_lines.clone(),
        }
    }

    /// The result when the configuration file does not load.
    pub fn not_loaded(problem: String) -> CheckDto {
        CheckDto {
            problems: vec![problem],
            warnings: Vec::new(),
            report: None,
            profile: None,
            edcopilot_problems: Vec::new(),
            info: Vec::new(),
        }
    }
}

/// One left-out command of the profile. The line has the phrase and the
/// reason. The class is for a screen that groups or marks the rows.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct LeftOutRowDto {
    /// `no_write`, `multi_write`, `maintenance`, `orphan`, or
    /// `not_understood`.
    pub class: String,
    /// The line of `voicebridge check --left-out`.
    pub line: String,
}

/// The most characters of the phrase and of the reason of a left-out row
/// that go to the page. A longer text ends with `…`.
pub const MAX_LEFT_OUT_TEXT_CHARS: usize = 500;

/// The text, cut to `MAX_LEFT_OUT_TEXT_CHARS` characters with `…` at the
/// end when it is longer.
fn capped(text: &str) -> String {
    if text.chars().count() <= MAX_LEFT_OUT_TEXT_CHARS {
        return text.to_string();
    }
    let mut out: String = text.chars().take(MAX_LEFT_OUT_TEXT_CHARS - 1).collect();
    out.push('…');
    out
}

impl From<&LeftOut> for LeftOutRowDto {
    fn from(left: &LeftOut) -> LeftOutRowDto {
        // The profile is a file of another program: a phrase or a reason
        // can be very long. The line uses the cut texts too.
        let shown = LeftOut {
            phrase: capped(&left.phrase),
            class: left.class,
            tokens: Vec::new(),
            reason: capped(&left.reason),
        };
        LeftOutRowDto {
            line: left_out_line(&shown),
            class: left.class.as_str().to_string(),
        }
    }
}

/// The result of `list_left_out`: one page of the left-out commands.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct LeftOutListDto {
    pub rows: Vec<LeftOutRowDto>,
    /// The place of the first row of the page in the whole list.
    pub offset: usize,
    /// The number of all rows that the filter shows.
    pub total: usize,
    /// The id of the loaded data, as in `CommandListDto`.
    pub catalog_id: u64,
}

fn mode_name(mode: Mode) -> &'static str {
    match mode {
        Mode::PushToTalk => "push_to_talk",
        Mode::AlwaysListening => "always_listening",
    }
}

/// The last line of the console start. The window has no Ctrl+C.
const CONSOLE_ONLY_LINE: &str = "listening, Ctrl+C stops";

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct StartInfoDto {
    pub report: ReportDto,
    pub engine: String,
    /// `push_to_talk` or `always_listening`.
    pub mode: String,
    pub mode_line: String,
    /// `None`: the default input device.
    pub input_device: Option<String>,
    /// `None` for the cloud engine.
    pub threads: Option<usize>,
    pub dry_run: bool,
    /// Empty for the cloud engine.
    pub hotwords_line: String,
    pub diagnostics_lines: Vec<String>,
    /// The start lines as the console shows them.
    pub lines: Vec<String>,
}

impl From<&StartInfo> for StartInfoDto {
    fn from(info: &StartInfo) -> StartInfoDto {
        StartInfoDto {
            report: ReportDto::from(&info.report),
            engine: info.engine.clone(),
            mode: mode_name(info.mode).to_string(),
            mode_line: info.mode_line.clone(),
            input_device: info.input_device.clone(),
            threads: info.threads,
            dry_run: info.dry_run,
            hotwords_line: info.hotwords_line.clone(),
            diagnostics_lines: info.diagnostics_lines.clone(),
            lines: voicebridge::console::start_lines(info)
                .into_iter()
                .filter(|line| line != CONSOLE_ONLY_LINE)
                .collect(),
        }
    }
}

/// The result of one utterance or one typed phrase.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct OutcomeDto {
    /// `sent`, `dry_run`, `no_match`, `no_wake_word`, `rejected`,
    /// `not_consumed`, `engine_error`, or `send_error`.
    pub kind: String,
    /// The heard text. `None` for `engine_error`.
    pub text: Option<String>,
    pub token: Option<String>,
    pub confidence: Option<f32>,
    /// The reason of the result. For `rejected`: why the program rejected
    /// the match. For `engine_error` and `send_error`: the error text. For
    /// `not_consumed`: `withdrawn` or `could not be withdrawn`. For a
    /// `no_match` that a path without a token blocked: the text of
    /// `session::NO_TOKEN_TEXT`. `None` for each other result.
    pub reason: Option<String>,
    /// For a `no_match` that a candidate reached but the program blocked:
    /// `tie`, `required_words`, `no_token`, `forms` or `dropped_word`.
    /// `None` for each other result.
    pub blocked: Option<String>,
    /// In this order, when set: `sound_alike`, `synonym`, `filler`,
    /// `hotwords`, `glitch`, `clipping`, `very_quiet`.
    pub marks: Vec<String>,
}

impl OutcomeDto {
    /// The outcome with the marks of the outcome only. A typed phrase has no
    /// audio, so it has no other marks.
    pub fn from_outcome(outcome: &Outcome) -> OutcomeDto {
        OutcomeDto::with_marks(outcome, None)
    }

    /// The outcome with the marks of the outcome and of the clip.
    pub fn with_marks(outcome: &Outcome, clip: Option<(Levels, Marks)>) -> OutcomeDto {
        let some = |s: &String| Some(s.clone());
        let (kind, text, token, confidence, reason, flags) = match outcome {
            Outcome::Sent {
                text,
                token,
                confidence,
                sound_alike,
                synonym,
                filler,
            } => (
                "sent",
                some(text),
                some(token),
                Some(*confidence),
                None,
                (*sound_alike, *synonym, *filler),
            ),
            Outcome::DryRun {
                text,
                token,
                confidence,
                sound_alike,
                synonym,
                filler,
            } => (
                "dry_run",
                some(text),
                some(token),
                Some(*confidence),
                None,
                (*sound_alike, *synonym, *filler),
            ),
            Outcome::NoMatch { text, blocked } => (
                "no_match",
                some(text),
                None,
                None,
                blocked
                    .and_then(voicebridge::session::blocked_text)
                    .map(str::to_string),
                Default::default(),
            ),
            Outcome::NoWakeWord { text } => (
                "no_wake_word",
                some(text),
                None,
                None,
                None,
                Default::default(),
            ),
            Outcome::Rejected {
                text,
                token,
                confidence,
                sound_alike,
                synonym,
                filler,
                reason,
            } => (
                "rejected",
                some(text),
                some(token),
                Some(*confidence),
                some(reason),
                (*sound_alike, *synonym, *filler),
            ),
            Outcome::NotConsumed {
                text,
                token,
                withdrawn,
            } => (
                "not_consumed",
                some(text),
                some(token),
                None,
                Some(
                    if *withdrawn {
                        "withdrawn"
                    } else {
                        "could not be withdrawn"
                    }
                    .to_string(),
                ),
                Default::default(),
            ),
            Outcome::EngineError { message } => (
                "engine_error",
                None,
                None,
                None,
                some(message),
                Default::default(),
            ),
            Outcome::SendError {
                text,
                token,
                message,
            } => (
                "send_error",
                some(text),
                some(token),
                None,
                some(message),
                Default::default(),
            ),
        };
        let (sound_alike, synonym, filler): (bool, bool, bool) = flags;
        let (hotwords, glitch, clipping, very_quiet) = match clip {
            Some((levels, marks)) => (
                marks.hotwords,
                marks.glitch,
                levels.clipping(),
                levels.very_quiet(),
            ),
            None => (false, false, false, false),
        };
        let marks = [
            (sound_alike, "sound_alike"),
            (synonym, "synonym"),
            (filler, "filler"),
            (hotwords, "hotwords"),
            (glitch, "glitch"),
            (clipping, "clipping"),
            (very_quiet, "very_quiet"),
        ]
        .into_iter()
        .filter(|(on, _)| *on)
        .map(|(_, name)| name.to_string())
        .collect();
        let blocked = match outcome {
            Outcome::NoMatch {
                blocked: Some(reason),
                ..
            } => Some(reason.name().to_string()),
            _ => None,
        };
        OutcomeDto {
            kind: kind.to_string(),
            text,
            token,
            confidence,
            reason,
            blocked,
            marks,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct UtteranceDto {
    pub outcome: OutcomeDto,
    pub transcribe_ms: u64,
    pub total_ms: u64,
    pub clip_ms: u64,
    pub peak_dbfs: f32,
    pub rms_dbfs: f32,
    /// The name of the saved clip, when the audio capture is on.
    pub audio_file: Option<String>,
}

fn millis(d: std::time::Duration) -> u64 {
    u64::try_from(d.as_millis()).unwrap_or(u64::MAX)
}

impl From<&UtteranceReport> for UtteranceDto {
    fn from(r: &UtteranceReport) -> UtteranceDto {
        UtteranceDto {
            outcome: OutcomeDto::with_marks(&r.outcome, Some((r.levels, r.marks))),
            transcribe_ms: millis(r.transcribe),
            total_ms: millis(r.total),
            clip_ms: millis(r.clip),
            peak_dbfs: r.levels.peak_dbfs,
            rms_dbfs: r.levels.rms_dbfs,
            audio_file: r.audio_file.clone(),
        }
    }
}

/// Why the session stopped. `message` is `None` for `requested`.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct StopReasonDto {
    /// `requested`, `device_error`, `engine_error`, or `input_error`.
    pub kind: String,
    pub message: Option<String>,
}

impl From<&StopReason> for StopReasonDto {
    fn from(reason: &StopReason) -> StopReasonDto {
        let (kind, message) = match reason {
            StopReason::Requested => ("requested", None),
            StopReason::DeviceError(m) => ("device_error", Some(m.clone())),
            StopReason::EngineError(m) => ("engine_error", Some(m.clone())),
            StopReason::InputError(m) => ("input_error", Some(m.clone())),
        };
        StopReasonDto {
            kind: kind.to_string(),
            message,
        }
    }
}

/// One event of the session. The Tauri event `session://event` carries it.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum EventDto {
    /// `text` has no prefix `warning: `.
    Warning {
        text: String,
    },
    Started {
        info: StartInfoDto,
    },
    /// `down` is true when the talk button went down.
    Talk {
        down: bool,
    },
    /// The fields of `UtteranceDto` are at the top level of the event.
    Utterance(UtteranceDto),
    Dropped {
        reason: String,
    },
    VocabularyReloaded {
        files: usize,
        names: usize,
    },
    Stopped {
        reason: StopReasonDto,
        glitches: u64,
    },
}

impl From<&RuntimeEvent> for EventDto {
    fn from(event: &RuntimeEvent) -> EventDto {
        match event {
            RuntimeEvent::Warning(line) => EventDto::Warning {
                text: line.strip_prefix("warning: ").unwrap_or(line).to_string(),
            },
            RuntimeEvent::Started(info) => EventDto::Started {
                info: info.as_ref().into(),
            },
            RuntimeEvent::Talk(down) => EventDto::Talk { down: *down },
            RuntimeEvent::Utterance(report) => EventDto::Utterance(report.into()),
            RuntimeEvent::Dropped { reason } => EventDto::Dropped {
                reason: reason.clone(),
            },
            RuntimeEvent::VocabularyReloaded { files, names } => EventDto::VocabularyReloaded {
                files: *files,
                names: *names,
            },
            RuntimeEvent::Stopped { reason, glitches } => EventDto::Stopped {
                reason: reason.into(),
                glitches: *glitches,
            },
        }
    }
}

/// The state of the session for `session_status`.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct StatusDto {
    pub running: bool,
    /// True while a start is in work (the model loads). `running` is false
    /// then.
    pub starting: bool,
    pub dry_run: bool,
    /// The start lines of the running session, or of the last session.
    pub start_info: Option<StartInfoDto>,
    /// Why the last session stopped. `None` while a session runs and before
    /// the first session.
    pub stop_reason: Option<StopReasonDto>,
}

/// A captured talk button. The web part sets it with `save_config`.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct BindingDto {
    /// `key` or `joystick`.
    pub kind: String,
    /// For example `key "F13"` or `joystick "MOZA" button 5`.
    pub label: String,
    /// The value for `push_to_talk.key`.
    pub key: Option<String>,
    /// The values for `push_to_talk.joystick.device` and `.button`.
    pub joystick: Option<JoystickDto>,
}

impl From<&Binding> for BindingDto {
    fn from(binding: &Binding) -> BindingDto {
        let label = binding.to_string();
        match binding {
            Binding::Key(spec) => BindingDto {
                kind: "key".to_string(),
                label,
                key: Some(spec.clone()),
                joystick: None,
            },
            Binding::Joystick { device, button } => BindingDto {
                kind: "joystick".to_string(),
                label,
                key: None,
                joystick: Some(JoystickDto {
                    device: device.clone(),
                    button: *button,
                }),
            },
        }
    }
}

/// One step of the model download. The Tauri event `model://event`
/// carries it.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ModelEventDto {
    /// `done` of `total` bytes of the archive are on the disk.
    Progress {
        done: u64,
        total: u64,
    },
    /// The program checks the hash of the archive.
    Verifying,
    Unpacking,
    /// The model is in `model_dir`. `config_error` is `None` when the
    /// configuration has the new model folder. `restart_needed` is true when
    /// the file got a new model folder while a session runs: that session
    /// keeps the model of its start.
    Done {
        model_dir: String,
        config_error: Option<String>,
        restart_needed: bool,
    },
    Failed {
        message: String,
    },
    Cancelled,
}

/// One line of the command list, as `voicebridge commands` prints it.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct CommandRowDto {
    /// Empty: the command has no category.
    pub category: String,
    pub phrase: String,
    pub token: String,
    pub works: bool,
    /// Why the command cannot fire, for example "does not work: no phrase".
    /// `None` when it works.
    pub reason: Option<String>,
    /// True for a phrase from an alias file.
    pub alias: bool,
}

impl From<&Entry> for CommandRowDto {
    fn from(entry: &Entry) -> CommandRowDto {
        CommandRowDto {
            category: entry.category.clone(),
            phrase: entry.phrase.clone(),
            token: entry.token.clone(),
            works: entry.state == CommandState::Works,
            reason: list::reason(entry.state).map(str::to_string),
            alias: entry.alias,
        }
    }
}

/// One composite group with the lines that the console prints for it.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct CompositeGroupDto {
    /// For example `Composite group "trade lookup": say one prefix and
    /// one suffix as one sentence`.
    pub heading: String,
    /// The lines of the rules, the prefixes and the suffixes, without the
    /// indent.
    pub lines: Vec<String>,
}

impl CompositeGroupDto {
    /// The group from the lines of `list::composite_lines`. `None` when the
    /// filter shows no line of the group.
    pub fn from_lines(lines: Vec<String>) -> Option<CompositeGroupDto> {
        let mut lines = lines.into_iter();
        let heading = lines.next()?;
        let heading = heading.trim_start().trim_start_matches("== ").to_string();
        Some(CompositeGroupDto {
            heading,
            lines: lines.map(|l| l.trim().to_string()).collect(),
        })
    }
}

/// The result of `list_commands`.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct CommandListDto {
    /// One page of the commands and the aliases that the filter shows,
    /// sorted by category and phrase.
    pub rows: Vec<CommandRowDto>,
    /// The place of the first row of the page in the whole list.
    pub offset: usize,
    /// The number of all rows that the filter shows.
    pub total: usize,
    /// The id of the loaded data. A rebuild after a change of a file gives
    /// a new id. Pages with different ids do not fit together.
    pub catalog_id: u64,
    pub composite: Vec<CompositeGroupDto>,
    /// The last line of the console list, for example `12 shown. 2319 of
    /// 2826 phrases work (commands and aliases). ...`.
    pub summary: String,
    /// The problems of the command data: the matching setting, the files of
    /// EDCoPilot, and the data files.
    pub problems: Vec<String>,
    /// The warnings of the synonyms, the filler words, the aliases, the
    /// vocabulary files, and the load of the commands.
    pub warnings: Vec<String>,
}

/// The result of `test_phrase`. The phrase test is always a dry run.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct PhraseTestDto {
    pub outcome: OutcomeDto,
    /// The result line, as the console writes it, for example
    /// `dry run "help chart log" -> Guide:Panel:ChartLog (1.00)`.
    pub line: String,
    /// The load problems that `voicebridge text` prints as warnings.
    pub warnings: Vec<String>,
    /// The wake words that the phrase needs: the saved wake words in
    /// always_listening mode, else empty.
    pub wake_words: Vec<String>,
}

/// The model that the download gets, for the Setup screen.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ModelInfoDto {
    pub name: String,
    pub archive_bytes: u64,
    pub unpacked_bytes: u64,
    /// The folder of the downloaded models. `None` when the program cannot
    /// find the per-user data folder.
    pub models_dir: Option<String>,
    /// True while a download runs.
    pub running: bool,
    /// The last step of the running download: `progress`, `verifying`, or
    /// `unpacking`. `None` when no download runs, or before its first step.
    pub step: Option<ModelEventDto>,
}

/// One row of a word table. `spoken` and `meant` have the meaning of
/// `voicebridge::tables::Row` for the table.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RowDto {
    pub spoken: String,
    pub meant: Vec<String>,
}

impl From<&Row> for RowDto {
    fn from(row: &Row) -> Self {
        RowDto {
            spoken: row.spoken.clone(),
            meant: row.meant.clone(),
        }
    }
}

impl From<RowDto> for Row {
    fn from(row: RowDto) -> Self {
        Row {
            spoken: row.spoken,
            meant: row.meant,
        }
    }
}

/// A word table as the file has it: the result of `read_table`.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct TableDto {
    /// `aliases`, `synonyms`, `sound_alikes`, `fillers`, or `hotwords`.
    pub kind: String,
    /// The language of an alias file. `None` for the other tables.
    pub language: Option<String>,
    /// The file of the table, for the text of the screen.
    pub path: String,
    /// False when the file does not exist. A save makes it.
    pub exists: bool,
    pub rows: Vec<RowDto>,
    /// The stamp of the file content. `save_table` needs it.
    pub version: String,
    /// True when the configuration has no hotword file.
    pub hotwords_off: bool,
    /// The languages of the alias folder, for each table.
    pub languages: Vec<String>,
}

impl TableDto {
    pub fn new(kind: &TableKind, file: &TableFile) -> Self {
        TableDto {
            kind: kind_name(kind).to_string(),
            // The language as the file name has it.
            language: match kind {
                TableKind::Aliases { language } => {
                    Some(file.language.clone().unwrap_or_else(|| language.clone()))
                }
                _ => None,
            },
            path: file.path.display().to_string(),
            exists: file.exists,
            rows: file.rows.iter().map(RowDto::from).collect(),
            version: file.version.clone(),
            hotwords_off: file.hotwords_off,
            languages: file.languages.clone(),
        }
    }
}

/// The name of a table kind on the wire.
pub fn kind_name(kind: &TableKind) -> &'static str {
    match kind {
        TableKind::Aliases { .. } => "aliases",
        TableKind::Synonyms => "synonyms",
        TableKind::SoundAlikes => "sound_alikes",
        TableKind::Fillers => "fillers",
        TableKind::Hotwords => "hotwords",
    }
}

/// The result of `check_table`: the lines of `voicebridge check` for the
/// rows.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct TableCheckDto {
    pub problems: Vec<String>,
    pub warnings: Vec<String>,
}

impl From<&TableCheck> for TableCheckDto {
    fn from(check: &TableCheck) -> Self {
        TableCheckDto {
            problems: check.problems.clone(),
            warnings: check.warnings.clone(),
        }
    }
}

/// The result of `save_table`.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct TableSaveDto {
    /// False when the save was refused or when the text did not change.
    pub written: bool,
    /// The problems of the rows. They refused the save.
    pub problems: Vec<String>,
    /// All warnings of the rows.
    pub warnings: Vec<String>,
    /// The warnings that the page did not confirm. They refused the save.
    pub unconfirmed: Vec<String>,
    /// The stamp of the file after the call.
    pub version: String,
    /// The file was written while a session runs. That session keeps its
    /// old tables.
    pub restart_needed: bool,
}

impl TableSaveDto {
    pub fn new(save: &TableSave, restart_needed: bool) -> Self {
        TableSaveDto {
            written: save.written,
            problems: save.check.problems.clone(),
            warnings: save.check.warnings.clone(),
            unconfirmed: save.unconfirmed.clone(),
            version: save.version.clone(),
            restart_needed,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use edcopilot_voicebridge_core::profile::{
        FormatNote, LeftOutClass, ReportDifference, SourceInfo,
    };
    use edcopilot_voicebridge_core::BlockReason;
    use serde_json::json;
    use std::time::Duration;
    use voicebridge::commands_source::NEWER_FORMAT;

    /// An invented profile report: two commands in the map, three left
    /// out, and a change since the last session start.
    fn profile_report_sample() -> (BuildReport, ProfileChange) {
        let left = |phrase: &str, class, reason: &str| LeftOut {
            phrase: phrase.into(),
            class,
            tokens: Vec::new(),
            reason: reason.into(),
        };
        let unknown = "an action type that the program does not know: 99";
        let report = BuildReport {
            source: SourceInfo {
                sha256: "00".into(),
                compressed_len: 0,
                inflated_len: 0,
                voiceattack_version: "1.0".into(),
                actions: 0,
                request_writes: 0,
            },
            format: FormatNote::Tested,
            commands_in_profile: 5,
            records: 2,
            left_out: vec![
                left(
                    "polish the brass",
                    LeftOutClass::NoWrite,
                    "the command writes no request",
                ),
                left("hoist the lantern", LeftOutClass::NotUnderstood, unknown),
                left("lower the lantern", LeftOutClass::NotUnderstood, unknown),
            ],
            mapped_phrases: ["ring the bell", "sail for home"]
                .into_iter()
                .map(String::from)
                .collect(),
            unknown_settings: Default::default(),
            ..BuildReport::default()
        };
        let change = ProfileChange {
            difference: ReportDifference {
                added: vec!["ring the bell".into()],
                removed: Vec::new(),
                moved_to_left_out: vec!["lower the lantern".into()],
                moved_to_map: Vec::new(),
            },
            same_profile: false,
        };
        (report, change)
    }

    /// The problem of an absent profile, with an invented folder.
    const NO_PROFILE: &str = "cannot read the EDCoPilot profile D:\\ED\\EDCoPilot-Profile.vap: \
                              the file does not exist, set the EDCoPilot folder, or start \
                              EDCoPilot one time to make the file";

    #[test]
    fn test_profile_report_dto_has_the_lines_of_check() {
        let (report, change) = profile_report_sample();
        let dto = ProfileReportDto::new(&report, Some(&change));
        assert_eq!(dto.records, 2);
        assert_eq!(dto.left_out, 3);
        assert_eq!(
            dto.lines,
            [
                "commands from the EDCoPilot profile: 2 commands, 3 left out",
                "left out, 2 commands: an action type that the program does not know: 99, \
                 for example \"hoist the lantern\"",
                "the EDCoPilot profile changed: 1 commands added, 0 removed, 1 now left out, \
                 0 now in the map",
            ]
        );
        assert_eq!(dto.lines, profile_lines(&report, Some(&change), false));
        let newer = BuildReport {
            format: FormatNote::NewerThanTested(Default::default()),
            ..report.clone()
        };
        let dto = ProfileReportDto::new(&newer, None);
        assert_eq!(dto.lines[1], NEWER_FORMAT);
        assert_eq!(dto.lines.len(), 3);
        // One row for each left-out command, with the line of
        // `check --left-out`.
        let row = LeftOutRowDto::from(&report.left_out[0]);
        assert_eq!(
            serde_json::to_value(&row).unwrap(),
            json!({"class": "no_write",
                   "line": "left out: \"polish the brass\" (no_write): the command writes no request"})
        );
    }

    /// A very long phrase or reason is cut for the page, also in the line.
    #[test]
    fn test_a_left_out_row_caps_the_phrase_and_the_reason() {
        let long = LeftOut {
            phrase: "ring ".repeat(400),
            class: LeftOutClass::NotUnderstood,
            tokens: vec!["Bell".into()],
            reason: "ä".repeat(MAX_LEFT_OUT_TEXT_CHARS + 1),
        };
        let row = LeftOutRowDto::from(&long);
        let (phrase, reason) = row
            .line
            .strip_prefix("left out: \"")
            .and_then(|rest| rest.split_once("\" (not_understood): "))
            .expect("the form of the line");
        for text in [phrase, reason] {
            assert_eq!(text.chars().count(), MAX_LEFT_OUT_TEXT_CHARS);
            assert!(text.ends_with('…'));
        }
        assert!(phrase.starts_with("ring ring "));
        // A text at the limit stays as it is.
        let exact = LeftOut {
            phrase: "a".repeat(MAX_LEFT_OUT_TEXT_CHARS),
            reason: "short".into(),
            ..long
        };
        let row = LeftOutRowDto::from(&exact);
        assert_eq!(
            row.line,
            format!("left out: \"{}\" (not_understood): short", exact.phrase)
        );
        assert_eq!(MAX_LEFT_OUT_TEXT_CHARS, 500);
    }

    #[test]
    fn test_edit_dto_round_trip() {
        let cases = [
            (
                json!({"key": "edcopilot.dir", "value": {"kind": "text", "value": "D:\\ED"}}),
                ValueDto::Text("D:\\ED".into()),
            ),
            (
                json!({"key": "matching.threshold", "value": {"kind": "number", "value": 0.85}}),
                ValueDto::Number(0.85),
            ),
            (
                json!({"key": "stt.parakeet.threads", "value": {"kind": "integer", "value": 6}}),
                ValueDto::Integer(6),
            ),
            (
                json!({"key": "matching.exact_single_word", "value": {"kind": "flag", "value": false}}),
                ValueDto::Flag(false),
            ),
            (
                json!({"key": "always_listening.wake_words",
                       "value": {"kind": "text_list", "value": ["copilot", "co pilot"]}}),
                ValueDto::TextList(vec!["copilot".into(), "co pilot".into()]),
            ),
            (
                json!({"key": "push_to_talk.joystick", "value": {"kind": "absent"}}),
                ValueDto::Absent,
            ),
        ];
        for (value, expected) in cases {
            let edit: EditDto = serde_json::from_value(value.clone()).unwrap();
            assert_eq!(edit.value, expected);
            assert_eq!(serde_json::to_value(&edit).unwrap(), value);
        }
        // An integer in a number field is a number.
        let edit: EditDto = serde_json::from_value(
            json!({"key": "matching.threshold", "value": {"kind": "number", "value": 1}}),
        )
        .unwrap();
        assert_eq!(Value::from(edit.value), Value::Number(1.0));
        // An unknown kind is an error.
        let bad = json!({"key": "x", "value": {"kind": "table", "value": {}}});
        assert!(serde_json::from_value::<EditDto>(bad).is_err());
    }

    fn start_info() -> StartInfo {
        StartInfo {
            report: LoadReport {
                commands: 2988,
                matchable: 2000,
                ..LoadReport::default()
            },
            engine: "parakeet".to_string(),
            mode: Mode::PushToTalk,
            mode_line: "push to talk, key \"F13\"".to_string(),
            input_device: None,
            threads: Some(4),
            dry_run: true,
            hotwords_line: "hotwords: off".to_string(),
            diagnostics_lines: vec![],
        }
    }

    fn json_of(event: RuntimeEvent) -> serde_json::Value {
        serde_json::to_value(EventDto::from(&event)).unwrap()
    }

    #[test]
    fn test_event_dto_shapes() {
        assert_eq!(
            json_of(RuntimeEvent::Warning(
                "warning: vocabulary directory not found: x".into()
            )),
            json!({"type": "warning", "text": "vocabulary directory not found: x"})
        );
        assert_eq!(
            json_of(RuntimeEvent::Warning("worker busy".into())),
            json!({"type": "warning", "text": "worker busy"})
        );
        assert_eq!(
            json_of(RuntimeEvent::Talk(true)),
            json!({"type": "talk", "down": true})
        );
        assert_eq!(
            json_of(RuntimeEvent::Dropped {
                reason: "waited 5000 ms".into()
            }),
            json!({"type": "dropped", "reason": "waited 5000 ms"})
        );
        assert_eq!(
            json_of(RuntimeEvent::VocabularyReloaded {
                files: 3,
                names: 12
            }),
            json!({"type": "vocabulary_reloaded", "files": 3, "names": 12})
        );
        assert_eq!(
            json_of(RuntimeEvent::Stopped {
                reason: StopReason::Requested,
                glitches: 0
            }),
            json!({"type": "stopped", "reason": {"kind": "requested", "message": null}, "glitches": 0})
        );
        assert_eq!(
            json_of(RuntimeEvent::Stopped {
                reason: StopReason::DeviceError("device removed".into()),
                glitches: 2
            }),
            json!({"type": "stopped",
                   "reason": {"kind": "device_error", "message": "device removed"},
                   "glitches": 2})
        );

        let started = json_of(RuntimeEvent::Started(Box::new(start_info())));
        assert_eq!(started["type"], "started");
        let info = &started["info"];
        assert_eq!(info["engine"], "parakeet");
        assert_eq!(info["mode"], "push_to_talk");
        assert_eq!(info["mode_line"], "push to talk, key \"F13\"");
        assert_eq!(info["input_device"], json!(null));
        assert_eq!(info["threads"], 4);
        assert_eq!(info["dry_run"], true);
        assert_eq!(info["hotwords_line"], "hotwords: off");
        assert_eq!(info["diagnostics_lines"], json!([]));
        assert_eq!(info["report"]["commands"], 2988);
        assert_eq!(info["report"]["matchable"], 2000);
        assert!(info["report"]["line"]
            .as_str()
            .unwrap()
            .starts_with("loaded 2988 commands"));
        let lines = info["lines"].as_array().unwrap();
        assert_eq!(lines.last().unwrap(), "dry run: no request file is written");
        let keys: Vec<&String> = info.as_object().unwrap().keys().collect();
        assert_eq!(keys.len(), 10, "{keys:?}");

        let utterance = json_of(RuntimeEvent::Utterance(UtteranceReport {
            outcome: Outcome::DryRun {
                text: "cheapest by bid cork".into(),
                token: "BuyTop:Cork".into(),
                confidence: 0.95,
                sound_alike: true,
                synonym: false,
                filler: true,
            },
            transcribe: Duration::from_millis(120),
            total: Duration::from_millis(345),
            clip: Duration::from_millis(2140),
            levels: Levels {
                peak_dbfs: -0.05,
                rms_dbfs: -20.0,
            },
            marks: Marks {
                glitch: true,
                hotwords: true,
            },
            audio_file: Some("utt-1.wav".into()),
        }));
        assert_eq!(
            utterance,
            json!({
                "type": "utterance",
                "outcome": {
                    "kind": "dry_run",
                    "text": "cheapest by bid cork",
                    "token": "BuyTop:Cork",
                    "confidence": 0.95f32,
                    "reason": null,
                    "blocked": null,
                    "marks": ["sound_alike", "filler", "hotwords", "glitch", "clipping"]
                },
                "transcribe_ms": 120,
                "total_ms": 345,
                "clip_ms": 2140,
                "peak_dbfs": -0.05f32,
                "rms_dbfs": -20.0,
                "audio_file": "utt-1.wav"
            })
        );
    }

    #[test]
    fn test_outcome_dto_kinds() {
        let rejected = OutcomeDto::from_outcome(&Outcome::Rejected {
            text: "a".into(),
            token: "T".into(),
            confidence: 0.5,
            sound_alike: false,
            synonym: true,
            filler: false,
            reason: "below the threshold".into(),
        });
        assert_eq!(
            serde_json::to_value(&rejected).unwrap(),
            json!({"kind": "rejected", "text": "a", "token": "T", "confidence": 0.5,
                   "reason": "below the threshold", "blocked": null, "marks": ["synonym"]})
        );
        // A text that names a command that the program cannot send.
        let no_token = OutcomeDto::from_outcome(&Outcome::NoMatch {
            text: "lock the lantern".into(),
            blocked: Some(BlockReason::NoToken),
        });
        assert_eq!(
            serde_json::to_value(&no_token).unwrap(),
            json!({"kind": "no_match", "text": "lock the lantern", "token": null,
                   "confidence": null,
                   "reason": "blocked: the text is a phrase of a command that the program cannot send",
                   "blocked": "no_token", "marks": []})
        );
        // The older causes keep the plain text: no reason.
        for (reason, name) in [
            (BlockReason::Tie, "tie"),
            (BlockReason::RequiredWords, "required_words"),
            (BlockReason::Forms, "forms"),
            (BlockReason::DroppedWord, "dropped_word"),
        ] {
            let dto = OutcomeDto::from_outcome(&Outcome::NoMatch {
                text: "x".into(),
                blocked: Some(reason),
            });
            assert_eq!(dto.reason, None, "{name}");
            assert_eq!(dto.blocked.as_deref(), Some(name));
        }
        let quiet = OutcomeDto::with_marks(
            &Outcome::NoMatch {
                text: "x".into(),
                blocked: None,
            },
            Some((
                Levels {
                    peak_dbfs: -60.0,
                    rms_dbfs: -70.0,
                },
                Marks::default(),
            )),
        );
        assert_eq!(quiet.kind, "no_match");
        assert_eq!(quiet.marks, ["very_quiet"]);
        let cases = [
            (
                Outcome::NoWakeWord { text: "x".into() },
                "no_wake_word",
                None,
            ),
            (
                Outcome::NotConsumed {
                    text: "x".into(),
                    token: "T".into(),
                    withdrawn: true,
                },
                "not_consumed",
                Some("withdrawn"),
            ),
            (
                Outcome::EngineError {
                    message: "timeout".into(),
                },
                "engine_error",
                Some("timeout"),
            ),
            (
                Outcome::SendError {
                    text: "x".into(),
                    token: "T".into(),
                    message: "denied".into(),
                },
                "send_error",
                Some("denied"),
            ),
            (
                Outcome::Sent {
                    text: "x".into(),
                    token: "T".into(),
                    confidence: 1.0,
                    sound_alike: false,
                    synonym: false,
                    filler: false,
                },
                "sent",
                None,
            ),
        ];
        for (outcome, kind, reason) in cases {
            let dto = OutcomeDto::from_outcome(&outcome);
            assert_eq!(dto.kind, kind);
            assert_eq!(dto.reason.as_deref(), reason);
            assert!(dto.marks.is_empty());
        }
    }

    #[test]
    fn test_config_dto_shape() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("voicebridge.toml");
        std::fs::write(&file, include_str!("../../app/voicebridge.example.toml")).unwrap();
        let config = Config::load_as_written(&file).unwrap();
        let dto = ConfigDto::new(
            std::path::Path::new("voicebridge.toml"),
            &config,
            true,
            Some("https://api.openai.com".into()),
            std::path::Path::new("D:\\vb\\hotwords.txt"),
            std::path::Path::new("C:\\EDCoPilot\\EDCoPilot-Profile.vap"),
            "0123456789abcdef-42".into(),
        );
        assert_eq!(
            serde_json::to_value(&dto).unwrap(),
            json!({
                "path": "voicebridge.toml",
                "api_key_set": true,
                "api_key_host": "https://api.openai.com",
                "default_hotwords_file": "D:\\vb\\hotwords.txt",
                "version": "0123456789abcdef-42",
                "edcopilot": {"dir": "C:\\EDCoPilot", "request_file": "",
                              "voice_commands_ini": "", "dynamic_vars_dir": "",
                              "profile": "",
                              "profile_in_use": "C:\\EDCoPilot\\EDCoPilot-Profile.vap"},
                "data": {"aliases_dir": "core/aliases",
                         "sound_alikes": "core/data/sound_alikes.toml",
                         "synonyms": "core/data/synonyms.toml",
                         "fillers": "core/data/fillers.toml"},
                "matching": {"threshold": 0.8f32, "exact_single_word": true},
                "microphone": {"device": "", "mode": "push_to_talk", "vad_threshold_dbfs": -40.0},
                "push_to_talk": {"key": "F13", "joystick": null,
                                 "pre_roll_ms": 250, "release_tail_ms": 300},
                "always_listening": {"wake_words": ["copilot", "co pilot"]},
                "stt": {"engine": "parakeet",
                        "parakeet": {"model_dir": "", "threads": 4,
                                     "hotwords_file": "", "hotwords_score": 1.5},
                        "openai_compatible": {"base_url": "https://api.openai.com/v1",
                                              "base_url_problem": null,
                                              "api_key_env": "OPENAI_API_KEY",
                                              "model": "gpt-4o-mini-transcribe",
                                              "language": "", "timeout_seconds": 10}},
                "diagnostics": {"log_file": "", "save_audio_dir": ""}
            })
        );
    }

    #[test]
    fn test_status_check_and_save_dto_shapes() {
        let status = StatusDto {
            running: false,
            starting: false,
            dry_run: true,
            start_info: None,
            stop_reason: Some(StopReasonDto::from(&StopReason::DeviceError("gone".into()))),
        };
        assert_eq!(
            serde_json::to_value(&status).unwrap(),
            json!({"running": false, "starting": false, "dry_run": true, "start_info": null,
                   "stop_reason": {"kind": "device_error", "message": "gone"}})
        );
        let started = StatusDto {
            running: true,
            starting: false,
            dry_run: true,
            start_info: Some(StartInfoDto::from(&start_info())),
            stop_reason: None,
        };
        let value = serde_json::to_value(&started).unwrap();
        assert_eq!(value["start_info"]["engine"], "parakeet");
        assert_eq!(value["stop_reason"], json!(null));

        let (profile, change) = profile_report_sample();
        let check = CheckDto {
            problems: vec!["p".into()],
            warnings: vec!["w".into()],
            report: Some(ReportDto::from(&LoadReport::default())),
            profile: Some(ProfileReportDto::new(&profile, Some(&change))),
            edcopilot_problems: Vec::new(),
            info: vec!["push-to-talk: key \"F13\"".into()],
        };
        let value = serde_json::to_value(&check).unwrap();
        assert_eq!(value["problems"], json!(["p"]));
        assert_eq!(value["warnings"], json!(["w"]));
        assert_eq!(value["info"], json!(["push-to-talk: key \"F13\""]));
        let report = value["report"].as_object().unwrap();
        let mut keys: Vec<&str> = report.keys().map(String::as_str).collect();
        keys.sort_unstable();
        assert_eq!(
            keys,
            [
                "alias_phrases",
                "commands",
                "composite_parts",
                "filler_warnings",
                "fillers",
                "fillers_active",
                "line",
                "matchable",
                "problems",
                "sound_alikes",
                "synonym_warnings",
                "synonyms",
                "unresolved_grammar",
                "vocabulary_files",
                "vocabulary_names"
            ]
        );
        assert_eq!(value["profile"]["records"], 2);
        assert_eq!(value["edcopilot_problems"], json!([]));
        assert_eq!(
            serde_json::to_value(CheckDto::not_loaded("bad".into())).unwrap(),
            json!({"problems": ["bad"], "warnings": [], "report": null, "profile": null,
                   "edcopilot_problems": [], "info": []})
        );

        let save = SaveResultDto {
            problems: vec!["stt.parakeet.model_dir is empty, the parakeet engine needs it".into()],
            written: true,
            restart_needed: false,
        };
        assert_eq!(
            serde_json::to_value(&save).unwrap(),
            json!({"problems": ["stt.parakeet.model_dir is empty, the parakeet engine needs it"],
                   "written": true, "restart_needed": false})
        );
    }

    /// The JSON as the window gets it: the text of `serde_json`, read again.
    /// An `f32` such as 0.95 is then 0.95, as in the fixture file.
    fn wire<T: Serialize>(value: &T) -> serde_json::Value {
        serde_json::from_str(&serde_json::to_string(value).unwrap()).unwrap()
    }

    /// The web part tests its store with the samples of this file. This
    /// test makes sure that each sample is the JSON of a real DTO.
    #[test]
    fn test_fixture_samples_are_the_json_of_the_dtos() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("web/src/lib/fixtures/dto-samples.json");
        let text = std::fs::read_to_string(&path).unwrap();
        let fixture: serde_json::Value = serde_json::from_str(&text).unwrap();

        let utterance = RuntimeEvent::Utterance(UtteranceReport {
            outcome: Outcome::DryRun {
                text: "cheapest by bid cork".into(),
                token: "BuyTop:Cork".into(),
                confidence: 0.95,
                sound_alike: true,
                synonym: false,
                filler: true,
            },
            transcribe: Duration::from_millis(120),
            total: Duration::from_millis(345),
            clip: Duration::from_millis(2140),
            levels: Levels {
                peak_dbfs: -0.05,
                rms_dbfs: -20.0,
            },
            marks: Marks {
                glitch: true,
                hotwords: true,
            },
            audio_file: Some("utt-1.wav".into()),
        });
        let events = [
            (
                "warning_with_prefix_removed",
                RuntimeEvent::Warning("warning: vocabulary directory not found: x".into()),
            ),
            ("warning", RuntimeEvent::Warning("worker busy".into())),
            ("talk_down", RuntimeEvent::Talk(true)),
            ("talk_up", RuntimeEvent::Talk(false)),
            (
                "dropped",
                RuntimeEvent::Dropped {
                    reason: "waited 5000 ms".into(),
                },
            ),
            (
                "vocabulary_reloaded",
                RuntimeEvent::VocabularyReloaded {
                    files: 3,
                    names: 12,
                },
            ),
            (
                "stopped_requested",
                RuntimeEvent::Stopped {
                    reason: StopReason::Requested,
                    glitches: 0,
                },
            ),
            (
                "stopped_device_error",
                RuntimeEvent::Stopped {
                    reason: StopReason::DeviceError("device removed".into()),
                    glitches: 2,
                },
            ),
            ("started", RuntimeEvent::Started(Box::new(start_info()))),
            ("utterance_dry_run", utterance),
        ];
        let samples = fixture["events"].as_object().unwrap();
        assert_eq!(samples.len(), events.len(), "a sample without a check");
        for (name, event) in &events {
            assert_eq!(
                samples[*name],
                wire(&EventDto::from(event)),
                "event sample {name}"
            );
        }

        let rejected = OutcomeDto::from_outcome(&Outcome::Rejected {
            text: "a".into(),
            token: "T".into(),
            confidence: 0.5,
            sound_alike: false,
            synonym: true,
            filler: false,
            reason: "below the threshold".into(),
        });
        assert_eq!(fixture["outcomes"]["rejected"], wire(&rejected));

        let status = |running, starting, dry_run, stop_reason: Option<StopReason>| StatusDto {
            running,
            starting,
            dry_run,
            start_info: None,
            stop_reason: stop_reason.as_ref().map(StopReasonDto::from),
        };
        let statuses = [
            ("never_started", status(false, false, false, None)),
            ("starting", status(false, true, false, None)),
            (
                "stopped_after_device_error",
                status(
                    false,
                    false,
                    true,
                    Some(StopReason::DeviceError("gone".into())),
                ),
            ),
        ];
        assert_eq!(fixture["status"].as_object().unwrap().len(), statuses.len());
        for (name, dto) in &statuses {
            assert_eq!(fixture["status"][*name], wire(dto), "status sample {name}");
        }

        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("voicebridge.toml");
        std::fs::write(&file, include_str!("../../app/voicebridge.example.toml")).unwrap();
        let config = Config::load_as_written(&file).unwrap();
        let config = ConfigDto::new(
            std::path::Path::new("voicebridge.toml"),
            &config,
            true,
            Some("https://api.openai.com".into()),
            std::path::Path::new("C:\\Users\\x\\AppData\\Roaming\\VoiceBridge\\hotwords.txt"),
            std::path::Path::new("C:\\EDCoPilot\\EDCoPilot-Profile.vap"),
            "0123456789abcdef-42".into(),
        );
        assert_eq!(fixture["config"], wire(&config));

        let (profile, change) = profile_report_sample();
        let check = CheckDto {
            problems: vec!["p".into()],
            warnings: vec!["w".into()],
            report: Some(ReportDto::from(&LoadReport::default())),
            profile: Some(ProfileReportDto::new(&profile, Some(&change))),
            edcopilot_problems: Vec::new(),
            info: vec!["push-to-talk: key \"F13\"".into()],
        };
        assert_eq!(fixture["check"]["loaded"], wire(&check));
        assert_eq!(
            fixture["check"]["not_loaded"],
            wire(&CheckDto::not_loaded("bad".into()))
        );
        let no_profile = CheckDto {
            problems: vec![NO_PROFILE.into()],
            warnings: Vec::new(),
            report: None,
            profile: None,
            edcopilot_problems: vec![NO_PROFILE.into()],
            info: Vec::new(),
        };
        assert_eq!(fixture["check"]["no_profile"], wire(&no_profile));
        assert_eq!(fixture["check"].as_object().unwrap().len(), 3);
        let left_out = LeftOutListDto {
            rows: profile.left_out.iter().map(LeftOutRowDto::from).collect(),
            offset: 0,
            total: 3,
            catalog_id: 7,
        };
        assert_eq!(fixture["left_out_list"], wire(&left_out));

        let save = |problem: &str, written| SaveResultDto {
            problems: vec![problem.to_string()],
            written,
            restart_needed: false,
        };
        assert_eq!(
            fixture["save"]["written_with_old_problem"],
            wire(&save(
                "stt.parakeet.model_dir is empty, the parakeet engine needs it",
                true
            ))
        );
        for (name, restart_needed) in [("restart_needed", true), ("no_restart", false)] {
            assert_eq!(
                fixture["key_result"][name],
                wire(&KeyResultDto { restart_needed }),
                "key result sample {name}"
            );
        }
        assert_eq!(fixture["key_result"].as_object().unwrap().len(), 2);
        assert_eq!(
            fixture["save"]["refused"],
            wire(&save(
                "matching.threshold must be between 0.0 and 1.0",
                false
            ))
        );

        assert_eq!(
            fixture["binding"]["key"],
            wire(&BindingDto::from(&Binding::Key("F13".into())))
        );
        assert_eq!(
            fixture["binding"]["joystick"],
            wire(&BindingDto::from(&Binding::Joystick {
                device: "MOZA".into(),
                button: 5,
            }))
        );

        let model_events = [
            (
                "progress",
                ModelEventDto::Progress {
                    done: 1_048_576,
                    total: 487_211_443,
                },
            ),
            ("verifying", ModelEventDto::Verifying),
            ("unpacking", ModelEventDto::Unpacking),
            (
                "done",
                ModelEventDto::Done {
                    model_dir: "C:\\Users\\x\\AppData\\Roaming\\VoiceBridge\\models\\m".into(),
                    config_error: None,
                    restart_needed: true,
                },
            ),
            (
                "done_not_saved",
                ModelEventDto::Done {
                    model_dir: "D:\\models\\m".into(),
                    config_error: Some("the file does not load".into()),
                    restart_needed: false,
                },
            ),
            (
                "failed",
                ModelEventDto::Failed {
                    message: "the downloaded file is damaged".into(),
                },
            ),
            ("cancelled", ModelEventDto::Cancelled),
        ];
        let samples = fixture["model_events"].as_object().unwrap();
        assert_eq!(samples.len(), model_events.len());
        for (name, dto) in &model_events {
            assert_eq!(samples[*name], wire(dto), "model event sample {name}");
        }
        let info = ModelInfoDto {
            name: "sherpa-onnx-nemo-parakeet-tdt-0.6b-v3-int8".into(),
            archive_bytes: 487_211_443,
            unpacked_bytes: 671_341_917,
            models_dir: Some("C:\\Users\\x\\AppData\\Roaming\\VoiceBridge\\models".into()),
            running: false,
            step: None,
        };
        assert_eq!(fixture["model_info"]["idle"], wire(&info));
        let running = ModelInfoDto {
            running: true,
            step: Some(ModelEventDto::Unpacking),
            ..info
        };
        assert_eq!(fixture["model_info"]["unpacking"], wire(&running));
        assert_eq!(fixture["model_info"].as_object().unwrap().len(), 2);

        assert_eq!(fixture["command_list"], wire(&command_list_sample()));
        let phrase_tests = phrase_test_samples();
        let samples = fixture["phrase_test"].as_object().unwrap();
        assert_eq!(samples.len(), phrase_tests.len());
        for (name, dto) in &phrase_tests {
            assert_eq!(samples[*name], wire(dto), "phrase test sample {name}");
        }

        let (tables, check, saves) = table_samples();
        let samples = fixture["table"].as_object().unwrap();
        assert_eq!(samples.len(), tables.len());
        for (name, dto) in &tables {
            assert_eq!(samples[*name], wire(dto), "table sample {name}");
        }
        assert_eq!(fixture["table_check"], wire(&check));
        let samples = fixture["table_save"].as_object().unwrap();
        assert_eq!(samples.len(), saves.len());
        for (name, dto) in &saves {
            assert_eq!(samples[*name], wire(dto), "table save sample {name}");
        }

        let startup = [
            (
                "updated",
                StartupNotesDto {
                    notes: vec![crate::paths::MAP_KEYS_REMOVED.into()],
                    data_dir: Some("C:\\Users\\x\\AppData\\Roaming\\VoiceBridge".into()),
                    ..StartupNotesDto::default()
                },
            ),
            (
                "dev_build",
                StartupNotesDto {
                    problem: Some(crate::paths::DEV_BUILD.into()),
                    ..StartupNotesDto::default()
                },
            ),
        ];
        let samples = fixture["startup"].as_object().unwrap();
        assert_eq!(samples.len(), startup.len());
        for (name, dto) in &startup {
            assert_eq!(samples[*name], wire(dto), "startup sample {name}");
        }
    }

    type TableSamples = (
        Vec<(&'static str, TableDto)>,
        TableCheckDto,
        Vec<(&'static str, TableSaveDto)>,
    );

    /// A synonym table, a new alias file, the hotwords while they are off,
    /// a check with a problem and a warning, and three saves.
    fn table_samples() -> TableSamples {
        let row = |spoken: &str, meant: &[&str]| Row {
            spoken: spoken.into(),
            meant: meant.iter().map(|m| m.to_string()).collect(),
        };
        let file = |path: &str, rows: Vec<Row>, hotwords_off, languages: &[&str]| TableFile {
            path: path.into(),
            exists: !rows.is_empty(),
            version: if rows.is_empty() {
                "absent".into()
            } else {
                "00ab-12".into()
            },
            rows,
            hotwords_off,
            languages: languages.iter().map(|l| l.to_string()).collect(),
            language: None,
        };
        let synonyms = file(
            "D:\\vb\\core\\data\\synonyms.toml",
            vec![
                row("navigate to", &["sail out to", "sail out for"]),
                row("<b>x</b>", &["{TXT:EDCP_X}"]),
            ],
            true,
            &[],
        );
        let french = file("D:\\vb\\core\\aliases\\fr.toml", vec![], true, &["de"]);
        let hotwords = file(
            "D:\\vb\\core\\data\\hotwords.txt",
            vec![row("Sol", &[]), row("Varnick", &[])],
            true,
            &[],
        );
        let tables = vec![
            ("synonyms", TableDto::new(&TableKind::Synonyms, &synonyms)),
            (
                "new_language",
                TableDto::new(
                    &TableKind::Aliases {
                        language: "fr".into(),
                    },
                    &french,
                ),
            ),
            (
                "hotwords_off",
                TableDto::new(&TableKind::Hotwords, &hotwords),
            ),
        ];
        let check = TableCheckDto::from(&TableCheck {
            problems: vec!["stt.parakeet.hotwords_file: the hotword \"Siris\" is near the hotword \"Sirius\" (word similarity 0.83)".into()],
            warnings: vec!["synonym \"sail out\" is also in the phrase \"sail out to <System>\" -> SailTo: the synonym applies only when the text as heard matches no command".into()],
        });
        let save = |written, problems: &[&str], unconfirmed: &[&str], restart_needed| {
            let texts = |list: &[&str]| list.iter().map(|t| t.to_string()).collect::<Vec<_>>();
            TableSaveDto::new(
                &TableSave {
                    written,
                    check: TableCheck {
                        problems: texts(problems),
                        warnings: texts(unconfirmed),
                    },
                    unconfirmed: texts(unconfirmed),
                    version: "00cd-14".into(),
                },
                restart_needed,
            )
        };
        let saves = vec![
            ("written_while_running", save(true, &[], &[], true)),
            (
                "refused_problem",
                save(false, &["row 1: the first field is empty"], &[], false),
            ),
            (
                "refused_unconfirmed",
                save(
                    false,
                    &[],
                    &["synonym \"sail out\" is also in a phrase"],
                    false,
                ),
            ),
        ];
        (tables, check, saves)
    }

    /// A list with a row that works, an alias, a row that does not work,
    /// untrusted text, and one composite group.
    fn command_list_sample() -> CommandListDto {
        let entry = |phrase: &str, token: &str, category: &str, state, alias| Entry {
            phrase: phrase.into(),
            token: token.into(),
            category: category.into(),
            state,
            alias,
        };
        let rows = [
            entry(
                "[Let's row home; sail out for home]",
                "SailOutForHome",
                "Harbour traffic",
                CommandState::Works,
                false,
            ),
            entry(
                "heimweg",
                "SailOutForHome",
                "Harbour traffic",
                CommandState::Works,
                true,
            ),
            entry(
                "{TXT:EDCP_X} <b>&amp;</b>",
                "Say=\"x\"",
                "",
                CommandState::UnresolvedGrammar,
                false,
            ),
        ];
        let lines = vec![
            "\n== Composite group \"trade lookup\": say one prefix and one suffix as one sentence"
                .to_string(),
            "  prefix: [best;] local [sell; buy]  (sets offer_scope = Local:)".to_string(),
            "  suffix: [price for;] cork  ->  {TXT:TradeSide}Cork".to_string(),
        ];
        CommandListDto {
            rows: rows.iter().map(CommandRowDto::from).collect(),
            offset: 0,
            total: 3,
            catalog_id: 7,
            composite: CompositeGroupDto::from_lines(lines).into_iter().collect(),
            summary: "3 shown. 2 of 3 phrases work (commands and aliases). \
                      1 composite groups. 0 synonyms. 0 filler words."
                .into(),
            problems: vec!["vocabulary directory not found: D:\\ED\\Dynamic var files".into()],
            warnings: vec!["synonym \"navigate\" is a word of a command phrase".into()],
        }
    }

    fn phrase_test_samples() -> Vec<(&'static str, PhraseTestDto)> {
        let test = |outcome: Outcome, warnings: Vec<String>| PhraseTestDto {
            line: outcome.log_line(),
            outcome: OutcomeDto::from_outcome(&outcome),
            warnings,
            wake_words: Vec::new(),
        };
        vec![
            (
                "dry_run",
                test(
                    Outcome::DryRun {
                        text: "help chart log".into(),
                        token: "Guide:Panel:ChartLog".into(),
                        confidence: 1.0,
                        sound_alike: false,
                        synonym: false,
                        filler: false,
                    },
                    vec![],
                ),
            ),
            (
                "rejected",
                test(
                    Outcome::Rejected {
                        text: "belays".into(),
                        token: "Belay".into(),
                        confidence: 0.83,
                        sound_alike: false,
                        synonym: false,
                        filler: false,
                        reason: "a phrase of one word needs an exact match".into(),
                    },
                    vec!["vocabulary directory not found: D:\\ED\\Dynamic var files".into()],
                ),
            ),
            (
                "no_match",
                test(
                    Outcome::NoMatch {
                        text: "<script>x</script>".into(),
                        blocked: None,
                    },
                    vec![],
                ),
            ),
            (
                "no_match_blocked",
                test(
                    Outcome::NoMatch {
                        text: "lock the lantern".into(),
                        blocked: Some(BlockReason::NoToken),
                    },
                    vec![],
                ),
            ),
            (
                "no_match_tie",
                test(
                    Outcome::NoMatch {
                        text: "close the gate".into(),
                        blocked: Some(BlockReason::Tie),
                    },
                    vec![],
                ),
            ),
            (
                "no_wake_word",
                PhraseTestDto {
                    wake_words: vec!["copilot".into(), "co pilot".into()],
                    ..test(
                        Outcome::NoWakeWord {
                            text: "help chart log".into(),
                        },
                        vec![],
                    )
                },
            ),
        ]
    }

    #[test]
    fn test_command_row_and_phrase_test_shapes() {
        let list = serde_json::to_value(command_list_sample()).unwrap();
        assert_eq!(
            list["rows"][2],
            json!({"category": "", "phrase": "{TXT:EDCP_X} <b>&amp;</b>", "token": "Say=\"x\"",
                   "works": false,
                   "reason": "does not work: grammar not found in the voice commands file",
                   "alias": false})
        );
        assert_eq!(list["rows"][1]["alias"], true);
        assert_eq!(list["rows"][0]["reason"], json!(null));
        assert_eq!(
            list["composite"][0],
            json!({"heading": "Composite group \"trade lookup\": say one prefix and one suffix as one sentence",
                   "lines": ["prefix: [best;] local [sell; buy]  (sets offer_scope = Local:)",
                             "suffix: [price for;] cork  ->  {TXT:TradeSide}Cork"]})
        );
        assert_eq!(CompositeGroupDto::from_lines(Vec::new()), None);
        let rejected = serde_json::to_value(&phrase_test_samples()[1].1).unwrap();
        assert_eq!(
            rejected["line"],
            "rejected \"belays\" -> Belay (0.83): a phrase of one word needs an exact match"
        );
        assert_eq!(rejected["outcome"]["kind"], "rejected");
    }

    #[test]
    fn test_binding_dto_shapes() {
        let key = BindingDto::from(&Binding::Key("F13".into()));
        assert_eq!(
            serde_json::to_value(&key).unwrap(),
            json!({"kind": "key", "label": "key \"F13\"", "key": "F13", "joystick": null})
        );
        let joystick = BindingDto::from(&Binding::Joystick {
            device: "MOZA".into(),
            button: 5,
        });
        assert_eq!(
            serde_json::to_value(&joystick).unwrap(),
            json!({"kind": "joystick", "label": "joystick \"MOZA\" button 5", "key": null,
                   "joystick": {"device": "MOZA", "button": 5}})
        );
    }
}
