//! The source of the commands.
//!
//! The program reads the EDCoPilot profile (`edcopilot.profile`, by default
//! `<edcopilot.dir>\EDCoPilot-Profile.vap`) at each load of the commands
//! and builds the map with the profile builder of `core`. There is no
//! cache. A file that is not a profile stops the load: the program then
//! uses no older map.
//!
//! The report of the last session start is in `profile-report.json` in the
//! folder of the session lock. `check` compares the current profile with it
//! and shows what changed. Only a session start writes the file.

use std::fmt::Display;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use anyhow::anyhow;
use edcopilot_voicebridge_core::profile::{
    build_map, report_difference, source_sha256, BuildError, BuildReport, BuiltMap, FormatNote,
    LeftOut, LeftOutClass, ReportDifference, SourceInfo,
};
use serde::{Deserialize, Serialize};

use crate::config_edit::write_atomic_named;

/// The file name of the profile in the EDCoPilot folder.
pub const PROFILE_FILE: &str = "EDCoPilot-Profile.vap";

/// The largest profile file that the program reads (64 MiB). The tested
/// profile has less than 1 MiB.
pub const MAX_PROFILE_BYTES: u64 = 64 * 1024 * 1024;

/// The file name of the saved report in the folder of the session lock.
pub const REPORT_FILE: &str = "profile-report.json";

/// The version of the content of the report file. A file with another
/// version is ignored.
pub const REPORT_SCHEMA: u32 = 1;

/// The largest report file that the program reads (1 MiB). The report of
/// the tested profile has less than 200 KiB.
pub const MAX_REPORT_BYTES: u64 = 1024 * 1024;

/// The version of this program. The saved report holds it: another
/// version can have other builder rules and thus another result for the
/// same profile.
pub const PROGRAM_VERSION: &str = env!("CARGO_PKG_VERSION");

/// The warning when the profile changes while a session runs.
pub const PROFILE_CHANGED: &str =
    "EDCoPilot changed its profile, stop and start the session to use the new commands";

/// The `check` line for a profile with a newer format than the tested
/// format.
pub const NEWER_FORMAT: &str = "the profile has a newer format than the tested format, \
                                tell the developer when a command does not work";

/// Where the path of the profile comes from. The problem text of a
/// profile that cannot be read names the setting to change.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProfileSource {
    /// The default path `<edcopilot.dir>\EDCoPilot-Profile.vap`.
    Folder,
    /// The key `edcopilot.profile`.
    Key,
}

/// The cause of a profile that does not exist.
const NO_FILE: &str = "the file does not exist";

/// The problem of a profile that the program cannot read.
fn cannot_read(path: &Path, cause: &str, source: ProfileSource) -> anyhow::Error {
    let action = match source {
        ProfileSource::Key => "set edcopilot.profile to the profile file",
        // EDCoPilot makes the file when it starts.
        ProfileSource::Folder if cause == NO_FILE => {
            "set the EDCoPilot folder, or start EDCoPilot one time to make the file"
        }
        ProfileSource::Folder => "set the EDCoPilot folder",
    };
    anyhow!(
        "cannot read the EDCoPilot profile {}: {cause}, {action}",
        path.display()
    )
}

/// The problem of a profile that another program holds (a sharing
/// violation). The folder is correct. The program that holds the file is
/// not known: it can be EDCoPilot, VoiceAttack, or another program.
fn held(path: &Path, cause: &str) -> anyhow::Error {
    anyhow!(
        "cannot read the EDCoPilot profile {}: {cause}, another program holds the file now, \
         try again",
        path.display()
    )
}

/// The problem of a file that is not a profile that the builder accepts.
fn not_understood(path: &Path, text: impl Display) -> anyhow::Error {
    anyhow!(
        "the file {} is not an EDCoPilot profile that the program understands: {text}",
        path.display()
    )
}

/// The cause of a read error in plain words.
fn cause(error: &std::io::Error) -> String {
    match error.kind() {
        std::io::ErrorKind::NotFound => NO_FILE.to_string(),
        std::io::ErrorKind::PermissionDenied => "access to the file is denied".to_string(),
        _ => format!("the system gives the error \"{error}\""),
    }
}

/// The problem of a file that is larger than `MAX_PROFILE_BYTES`.
fn too_large(path: &Path, len: u64) -> anyhow::Error {
    not_understood(
        path,
        format!(
            "the file has {len} bytes, more than the limit of {MAX_PROFILE_BYTES} bytes, use \
             the profile file of EDCoPilot"
        ),
    )
}

/// The wait before the second read of a profile that another program
/// writes or holds at the first read.
pub const RETRY_WAIT: Duration = Duration::from_millis(300);

/// The Windows error "the file is in use by another process".
const SHARING_VIOLATION: i32 = 32;

/// A failed attempt. `again`: a second read can work (a sharing
/// violation, or a profile that is cut off).
struct Failed {
    error: anyhow::Error,
    again: bool,
}

/// The problem of a read error, and if a second read can work.
fn read_failed(path: &Path, e: &std::io::Error, source: ProfileSource) -> Failed {
    let sharing = e.raw_os_error() == Some(SHARING_VIOLATION);
    Failed {
        error: if sharing {
            held(path, &cause(e))
        } else {
            cannot_read(path, &cause(e), source)
        },
        again: sharing,
    }
}

/// Reads the profile file one time. The size check comes before the read,
/// and the read stops after the limit, also when the file grows during the
/// read. `source` selects the advice of the problem text.
pub fn read_profile(path: &Path, source: ProfileSource) -> anyhow::Result<Vec<u8>> {
    read_once(path, source).map_err(|f| f.error)
}

fn read_once(path: &Path, source: ProfileSource) -> Result<Vec<u8>, Failed> {
    let failed = |error| Failed {
        error,
        again: false,
    };
    let meta = std::fs::metadata(path).map_err(|e| read_failed(path, &e, source))?;
    if !meta.is_file() {
        return Err(failed(cannot_read(path, "it is not a file", source)));
    }
    if meta.len() > MAX_PROFILE_BYTES {
        return Err(failed(too_large(path, meta.len())));
    }
    let file = std::fs::File::open(path).map_err(|e| read_failed(path, &e, source))?;
    let mut bytes = Vec::with_capacity(meta.len() as usize);
    file.take(MAX_PROFILE_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| read_failed(path, &e, source))?;
    if bytes.len() as u64 > MAX_PROFILE_BYTES {
        return Err(failed(too_large(path, bytes.len() as u64)));
    }
    Ok(bytes)
}

/// Reads the profile file and builds the map. A build error is the problem
/// "not an EDCoPilot profile that the program understands". When the file
/// is in use by another program, or the profile is cut off, the program
/// reads it again one time after `RETRY_WAIT`: EDCoPilot can write the
/// file at this moment. Then the error of the second attempt counts.
/// `source` selects the advice of the problem text.
pub fn build_profile(path: &Path, source: ProfileSource) -> anyhow::Result<BuiltMap> {
    match build_once(path, source) {
        Err(Failed { again: true, .. }) => {
            std::thread::sleep(RETRY_WAIT);
            build_once(path, source).map_err(|f| f.error)
        }
        other => other.map_err(|f| f.error),
    }
}

fn build_once(path: &Path, source: ProfileSource) -> Result<BuiltMap, Failed> {
    let bytes = read_once(path, source)?;
    build_map(&bytes).map_err(|e| Failed {
        again: matches!(e, BuildError::Truncated),
        error: not_understood(path, e),
    })
}

/// The size and the time of the last change of the profile file. The watch
/// of a running session compares them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProfileStamp {
    pub len: u64,
    pub modified: Option<SystemTime>,
}

/// The stamp of the file. `None` when the file cannot be read.
pub fn profile_stamp(path: &Path) -> Option<ProfileStamp> {
    let meta = std::fs::metadata(path).ok()?;
    Some(ProfileStamp {
        len: meta.len(),
        modified: meta.modified().ok(),
    })
}

/// The number of polls after a change of the stamp with a file that the
/// watch cannot read. Then the watch warns one time.
pub const MAX_UNREAD_POLLS: u32 = 10;

/// Sees a change of the profile file while a session runs.
///
/// A change counts when the new stamp stays the same for two polls: a
/// write in parts gives one change, not one change for each part. A file
/// that cannot be read for a moment is not a change: the watch reads it
/// again at the next poll.
#[derive(Debug, Clone)]
pub struct ProfileWatch {
    path: PathBuf,
    reference: Option<ProfileStamp>,
    pending: Option<ProfileStamp>,
    /// The hash of the profile of the session. `None`: each change of the
    /// stamp counts.
    session_hash: Option<String>,
    /// The hash of the file at the last change of the stamp.
    last_hash: Option<String>,
    /// The number of failed reads after a change of the stamp. `None`: no
    /// change waits for a read.
    unread: Option<u32>,
}

impl ProfileWatch {
    /// A watch that starts with the current stamp of the file. Make it
    /// before the read of the profile, so that a change during the build
    /// also counts.
    pub fn new(path: &Path) -> ProfileWatch {
        ProfileWatch {
            path: path.to_path_buf(),
            reference: profile_stamp(path),
            pending: None,
            session_hash: None,
            last_hash: None,
            unread: None,
        }
    }

    /// The hash of the profile that the session uses (the `sha256` of its
    /// report). After a change of the stamp, the watch then reads the file
    /// and counts the change only when the content is new.
    pub fn set_session_hash(&mut self, sha256: &str) {
        self.session_hash = Some(sha256.to_string());
        self.last_hash = Some(sha256.to_string());
    }

    /// After a confirmed change of the stamp: is the content new? A file
    /// that the watch cannot read keeps the change open: the next poll
    /// reads it again.
    fn content_changed(&mut self) -> bool {
        let Some(session) = &self.session_hash else {
            return true;
        };
        // The text of the error is not used, so the source does not matter.
        match read_profile(&self.path, ProfileSource::Folder) {
            Ok(bytes) => {
                // The warning of a file that stayed unreadable was this change.
                let warned = self.unread.is_some_and(|n| n >= MAX_UNREAD_POLLS);
                self.unread = None;
                let hash = source_sha256(&bytes);
                let new = hash != *session && self.last_hash.as_ref() != Some(&hash);
                self.last_hash = Some(hash);
                new && !warned
            }
            // The file went away after the stamp: not a change.
            Err(_) if profile_stamp(&self.path).is_none() => {
                self.unread = None;
                false
            }
            Err(_) => {
                let failed = self.unread.map_or(1, |n| n.saturating_add(1));
                self.unread = Some(failed);
                failed == MAX_UNREAD_POLLS
            }
        }
    }

    /// True one time for each change of the file. With the hash of the
    /// session, a change counts only when the file has content that is
    /// not the content of the session and not the content of the last
    /// change. When the watch cannot read the file after a change of the
    /// stamp, it reads it again at each poll. A file that stays unreadable
    /// for `MAX_UNREAD_POLLS` polls counts as one change.
    pub fn poll(&mut self) -> bool {
        if self.unread.is_some() {
            return self.content_changed();
        }
        let Some(now) = profile_stamp(&self.path) else {
            return false;
        };
        if Some(now) == self.reference {
            self.pending = None;
            return false;
        }
        if self.pending == Some(now) {
            self.reference = Some(now);
            self.pending = None;
            return self.content_changed();
        }
        self.pending = Some(now);
        false
    }
}

/// The content of the report file. It holds what `report_difference`
/// needs: the phrases, the left-out commands with class and reason, the
/// counts, and the hash of the source.
#[derive(Debug, Serialize, Deserialize)]
struct SavedReport {
    schema: u32,
    sha256: String,
    voiceattack_version: String,
    commands_in_profile: usize,
    records: usize,
    mapped_phrases: Vec<String>,
    left_out: Vec<SavedLeftOut>,
    /// Added after schema 1 was in use. A file without it reads as empty.
    #[serde(default)]
    unknown_settings: Vec<SavedSetting>,
    /// The version of the program that wrote the file. Added after schema
    /// 1 was in use. A file without it reads as an empty text.
    #[serde(default)]
    program_version: String,
    /// The records with a phrase variable that the program does not know.
    /// Added after schema 1 was in use. A file without it reads as 0.
    #[serde(default)]
    unknown_phrase_vars: usize,
    /// The records that never give a token. Added after schema 1 was in
    /// use. A file without it reads as 0.
    #[serde(default)]
    cannot_fire: usize,
    /// The path of the profile file. Added after schema 1 was in use. A
    /// file without it reads as an empty text: it is then the report of no
    /// known profile, and `profile_change` gives no line.
    #[serde(default)]
    profile_path: String,
}

/// The number of actions with an unknown setting in one field.
#[derive(Debug, Serialize, Deserialize)]
struct SavedSetting {
    action_type: i32,
    field: usize,
    actions: usize,
}

#[derive(Debug, Serialize, Deserialize)]
struct SavedLeftOut {
    phrase: String,
    class: String,
    reason: String,
}

/// The classes of the left-out commands.
const CLASSES: [LeftOutClass; 5] = [
    LeftOutClass::NoWrite,
    LeftOutClass::MultiWrite,
    LeftOutClass::Maintenance,
    LeftOutClass::Orphan,
    LeftOutClass::NotUnderstood,
];

/// The path of the report file in `dir`.
pub fn report_path(dir: &Path) -> PathBuf {
    dir.join(REPORT_FILE)
}

/// Writes the report of a session start into `dir` (atomic write). The
/// folder must exist: the session lock makes it. `profile` is the path of
/// the profile file of the report.
pub fn save_report(dir: &Path, report: &BuildReport, profile: &Path) -> anyhow::Result<()> {
    let saved = SavedReport {
        schema: REPORT_SCHEMA,
        sha256: report.source.sha256.clone(),
        voiceattack_version: report.source.voiceattack_version.clone(),
        commands_in_profile: report.commands_in_profile,
        records: report.records,
        mapped_phrases: report.mapped_phrases.iter().cloned().collect(),
        left_out: report
            .left_out
            .iter()
            .map(|l| SavedLeftOut {
                phrase: l.phrase.clone(),
                class: l.class.as_str().to_string(),
                reason: l.reason.clone(),
            })
            .collect(),
        unknown_settings: report
            .unknown_settings
            .iter()
            .map(|(&(action_type, field), &actions)| SavedSetting {
                action_type,
                field,
                actions,
            })
            .collect(),
        program_version: PROGRAM_VERSION.to_string(),
        unknown_phrase_vars: report.unknown_phrase_vars,
        cannot_fire: report.cannot_fire,
        profile_path: profile.to_string_lossy().into_owned(),
    };
    let text = serde_json::to_string_pretty(&saved)?;
    write_atomic_named(&report_path(dir), &text, "profile report")
}

/// The saved report of the last session start.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SavedProfileReport {
    pub report: BuildReport,
    /// The version of the program that wrote it. Empty for a file of an
    /// older program.
    pub program_version: String,
    /// The path of the profile file. Empty for a file of an older program.
    pub profile_path: String,
}

/// The report of the last session start in `dir`. `None` when the file is
/// absent, larger than `MAX_REPORT_BYTES`, does not parse, or has another
/// schema version: then there is no difference to show, and it is not an
/// error.
pub fn read_saved_report(dir: &Path) -> Option<SavedProfileReport> {
    let path = report_path(dir);
    if std::fs::metadata(&path).ok()?.len() > MAX_REPORT_BYTES {
        return None;
    }
    let text = std::fs::read_to_string(path).ok()?;
    let saved: SavedReport = serde_json::from_str(&text).ok()?;
    if saved.schema != REPORT_SCHEMA {
        return None;
    }
    let mut left_out = Vec::with_capacity(saved.left_out.len());
    for l in saved.left_out {
        let class = CLASSES.into_iter().find(|c| c.as_str() == l.class)?;
        left_out.push(LeftOut {
            phrase: l.phrase,
            class,
            tokens: Vec::new(),
            reason: l.reason,
        });
    }
    let report = BuildReport {
        source: SourceInfo {
            sha256: saved.sha256,
            compressed_len: 0,
            inflated_len: 0,
            voiceattack_version: saved.voiceattack_version,
            actions: 0,
            request_writes: 0,
        },
        format: FormatNote::Tested,
        commands_in_profile: saved.commands_in_profile,
        records: saved.records,
        left_out,
        mapped_phrases: saved.mapped_phrases.into_iter().collect(),
        unknown_settings: saved
            .unknown_settings
            .into_iter()
            .map(|s| ((s.action_type, s.field), s.actions))
            .collect(),
        unknown_phrase_vars: saved.unknown_phrase_vars,
        cannot_fire: saved.cannot_fire,
    };
    Some(SavedProfileReport {
        report,
        program_version: saved.program_version,
        profile_path: saved.profile_path,
    })
}

/// A difference to the saved report.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProfileChange {
    pub difference: ReportDifference,
    /// The profile is the same file, and the program version is another
    /// version: the builder of this version reads the file in another way.
    pub same_profile: bool,
}

impl ProfileChange {
    /// The line of `check` and of the session start.
    pub fn line(&self) -> String {
        let d = &self.difference;
        let counts = format!(
            "{} commands added, {} removed, {} now left out, {} now in the map",
            d.added.len(),
            d.removed.len(),
            d.moved_to_left_out.len(),
            d.moved_to_map.len()
        );
        if self.same_profile {
            format!("this program version reads the EDCoPilot profile in another way: {counts}")
        } else {
            format!("the EDCoPilot profile changed: {counts}")
        }
    }
}

/// True when two profile paths name the same file. Windows compares paths
/// without case, and with `/` as `\`.
fn same_path(a: &str, b: &Path) -> bool {
    let b = b.to_string_lossy();
    if cfg!(windows) {
        let plain = |p: &str| p.replace('/', "\\").to_lowercase();
        plain(a) == plain(&b)
    } else {
        a == b
    }
}

/// What changed from the saved report in `dir` to `report` of the profile
/// file `profile`. `None` without a folder, without a usable saved report,
/// for a saved report of another profile file (two configurations with
/// different profiles share the report file), for the same profile and the
/// same program version, or when nothing changed.
pub fn profile_change(
    report: &BuildReport,
    dir: Option<&Path>,
    profile: &Path,
) -> Option<ProfileChange> {
    let old = read_saved_report(dir?)?;
    if !same_path(&old.profile_path, profile) {
        return None;
    }
    let same_profile = old.report.source.sha256 == report.source.sha256;
    if same_profile && old.program_version == PROGRAM_VERSION {
        return None;
    }
    let difference = report_difference(&old.report, report);
    (!difference.is_empty()).then_some(ProfileChange {
        difference,
        same_profile,
    })
}

/// The line of `check --left-out` for one left-out command.
pub fn left_out_line(left_out: &LeftOut) -> String {
    format!(
        "left out: \"{}\" ({}): {}",
        left_out.phrase,
        left_out.class.as_str(),
        left_out.reason
    )
}

/// The `check` lines of the profile build. `every_left_out`: one line for
/// each left-out command, in place of one line for each reason of the
/// commands that the builder does not understand.
pub fn profile_lines(
    report: &BuildReport,
    change: Option<&ProfileChange>,
    every_left_out: bool,
) -> Vec<String> {
    let mut lines = vec![format!(
        "commands from the EDCoPilot profile: {} commands, {} left out",
        report.records,
        report.left_out.len()
    )];
    if report.format != FormatNote::Tested {
        lines.push(NEWER_FORMAT.to_string());
    }
    if report.unknown_phrase_vars > 0 {
        lines.push(format!(
            "{} commands have a phrase variable that the program does not know",
            report.unknown_phrase_vars
        ));
    }
    if every_left_out {
        lines.extend(report.left_out.iter().map(left_out_line));
    } else {
        for (reason, n, phrase) in report.class_reasons(LeftOutClass::NotUnderstood) {
            lines.push(format!(
                "left out, {n} commands: {reason}, for example \"{phrase}\""
            ));
        }
    }
    if let Some(change) = change {
        lines.push(change.line());
    }
    lines
}

#[cfg(test)]
pub(crate) mod test_support {
    //! A configuration with the invented EDCoPilot folder of the fixture.

    use std::path::{Path, PathBuf};

    use edcopilot_voicebridge_core::profile::fixture::write_fixture_edcopilot;

    use crate::config::Config;

    /// The warning of the invented pair of the fixture: a working phrase
    /// that a value command also has.
    pub const FIXTURE_SHADOW: &str = "the phrase \"set lamp colour\" of the command \"[paint the lamp; set lamp colour]\" is also a phrase of a command that the program cannot send, the program does not send a command for it";

    /// The `core` folder of the repository.
    pub fn core_dir() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../core")
    }

    /// Writes the fixture folder into `<dir>\ed` and a configuration into
    /// `<dir>\voicebridge.toml`. `extra` goes at the end.
    pub fn fixture_config(dir: &Path, extra: &str) -> Config {
        fixture_config_with(dir, "", extra)
    }

    /// The fixture commands as the program loads them from the fixture
    /// folder (profile mode): the build, the runtime grammar of the fixture
    /// INI with the number check, and the profile variables.
    pub fn fixture_loaded_commands() -> Vec<edcopilot_voicebridge_core::Command> {
        let dir = tempfile::tempdir().unwrap();
        crate::config::load_commands(&fixture_config(dir.path(), ""))
            .unwrap()
            .commands
    }

    /// `fixture_config` with `data_extra` in the table `[data]`.
    pub fn fixture_config_with(dir: &Path, data_extra: &str, extra: &str) -> Config {
        let ed = dir.join("ed");
        write_fixture_edcopilot(&ed);
        let core = core_dir();
        let text = format!(
            "[edcopilot]\ndir = '{}'\n\
             [data]\n{data_extra}aliases_dir = '{}'\nsound_alikes = '{}'\nsynonyms = '{}'\nfillers = '{}'\n\
             [stt]\nengine = 'openai_compatible'\n\
             [stt.openai_compatible]\napi_key_env = 'VB_TEST_KEY'\n\
             {extra}",
            ed.display(),
            dir.join("no-aliases").display(),
            core.join("data/sound_alikes.toml").display(),
            core.join("data/synonyms.toml").display(),
            core.join("data/fillers.toml").display(),
        );
        std::fs::create_dir_all(dir.join("no-aliases")).unwrap();
        let file = dir.join("voicebridge.toml");
        std::fs::write(&file, text).unwrap();
        Config::load(&file).unwrap()
    }
}

#[cfg(test)]
mod tests {
    use super::test_support::fixture_config;
    use super::*;
    use crate::config::load_commands;
    use edcopilot_voicebridge_core::profile::fixture::fixture_profile;
    use edcopilot_voicebridge_core::profile::source_sha256;
    use edcopilot_voicebridge_core::{resolve_profile_vars, resolve_runtime_grammar};

    /// The profile path of the saved reports of the tests.
    const TEST_PROFILE: &str = "D:\\ED\\EDCoPilot-Profile.vap";

    #[test]
    fn test_load_commands_builds_from_the_profile_of_the_folder() {
        let dir = tempfile::tempdir().unwrap();
        let config = fixture_config(dir.path(), "");
        assert_eq!(
            config.edcopilot.profile,
            dir.path().join("ed").join(PROFILE_FILE)
        );
        let loaded = load_commands(&config).unwrap();
        let report = loaded.profile;
        assert_eq!(loaded.commands.len(), report.records);
        assert_eq!(loaded.warnings, [test_support::FIXTURE_SHADOW]);
        // The same steps as the core test of the fixture folder.
        let built = build_map(&fixture_profile()).unwrap();
        let ini = dir.path().join("ed/working/EDCoPilot.VoiceCommands.ini");
        let expected = resolve_profile_vars(
            resolve_runtime_grammar(built.commands, &ini).commands,
            &built.profile_vars,
        );
        assert_eq!(loaded.commands, expected);
        // The command that the check of the runtime grammar takes out is a
        // left-out command of the report.
        let mut report_without = report.clone();
        let removed = report_without.left_out.pop().unwrap();
        assert_eq!(removed.phrase, "{TXT:EDCP_WICKLAMP}");
        assert_eq!(removed.class, LeftOutClass::NotUnderstood);
        assert_eq!(
            removed.reason,
            edcopilot_voicebridge_core::NUMBER_TEXT_REASON
        );
        assert_eq!(removed.tokens, ["WickLamp:{TXT:WickNo}"]);
        assert!(!report.mapped_phrases.contains(&removed.phrase));
        assert_eq!(report.records + 1, built.report.records);
        assert_eq!(report_without.left_out, built.report.left_out);
    }

    #[test]
    fn test_absent_profile_is_one_problem_that_names_the_file() {
        let dir = tempfile::tempdir().unwrap();
        let config = fixture_config(dir.path(), "");
        let profile = dir.path().join("ed").join(PROFILE_FILE);
        std::fs::remove_file(&profile).unwrap();
        let found = crate::run::check_config_in(&config, |_| Some("key".into()), None);
        assert!(found.report.is_none());
        let expected = format!(
            "cannot read the EDCoPilot profile {}: the file does not exist, set the EDCoPilot \
             folder, or start EDCoPilot one time to make the file",
            profile.display()
        );
        assert_eq!(found.problems, [expected], "{found:?}");
    }

    #[test]
    fn test_an_absent_profile_of_the_key_names_the_key() {
        let dir = tempfile::tempdir().unwrap();
        let ed = dir.path().join("ed");
        std::fs::create_dir_all(&ed).unwrap();
        let file = dir.path().join("voicebridge.toml");
        let given = dir.path().join("copy.vap");
        std::fs::write(
            &file,
            format!(
                "[edcopilot]\ndir = '{}'\nprofile = '{}'\n",
                ed.display(),
                given.display()
            ),
        )
        .unwrap();
        let config = crate::config::Config::load(&file).unwrap();
        assert!(config.edcopilot.profile_from_key);
        let error = format!("{:#}", load_commands(&config).unwrap_err());
        assert_eq!(
            error,
            format!(
                "cannot read the EDCoPilot profile {}: the file does not exist, set \
                 edcopilot.profile to the profile file",
                given.display()
            )
        );
        // A folder in place of the file: the same advice.
        std::fs::create_dir(&given).unwrap();
        let error = format!("{:#}", load_commands(&config).unwrap_err());
        assert!(
            error.ends_with(": it is not a file, set edcopilot.profile to the profile file"),
            "{error}"
        );
        // The default path of the folder.
        let config = fixture_config(dir.path(), "");
        assert!(!config.edcopilot.profile_from_key);
    }

    #[test]
    fn test_a_profile_that_is_a_folder_keeps_the_advice_of_the_folder() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(PROFILE_FILE);
        std::fs::create_dir(&path).unwrap();
        let error = format!(
            "{:#}",
            read_profile(&path, ProfileSource::Folder).unwrap_err()
        );
        assert!(
            error.ends_with(": it is not a file, set the EDCoPilot folder"),
            "{error}"
        );
    }

    #[test]
    fn test_a_profile_above_the_size_limit_is_refused_with_its_size() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(PROFILE_FILE);
        let file = std::fs::File::create(&path).unwrap();
        file.set_len(MAX_PROFILE_BYTES + 1).unwrap();
        drop(file);
        let error = format!(
            "{:#}",
            read_profile(&path, ProfileSource::Folder).unwrap_err()
        );
        assert_eq!(
            error,
            format!(
                "the file {} is not an EDCoPilot profile that the program understands: \
                 the file has 67108865 bytes, more than the limit of 67108864 bytes, use the \
                 profile file of EDCoPilot",
                path.display()
            )
        );
        // A file at the limit is read, and the builder gives its own error.
        let file = std::fs::OpenOptions::new().write(true).open(&path).unwrap();
        file.set_len(16).unwrap();
        drop(file);
        assert_eq!(
            read_profile(&path, ProfileSource::Folder).unwrap().len(),
            16
        );
        let error = format!(
            "{:#}",
            build_profile(&path, ProfileSource::Folder).unwrap_err()
        );
        assert!(
            error.starts_with(&format!(
                "the file {} is not an EDCoPilot profile that the program understands: ",
                path.display()
            )),
            "{error}"
        );
    }

    /// The fixture with one more command.
    fn changed_profile() -> Vec<u8> {
        let mut b = edcopilot_voicebridge_core::profile::fixture::fixture_builder();
        b.category("Logbook")
            .fixed("open the invented extra room", "OpenRoom:Extra");
        b.build()
    }

    #[test]
    fn test_the_saved_report_gives_the_difference() {
        let dir = tempfile::tempdir().unwrap();
        let old = build_map(&fixture_profile()).unwrap().report;
        save_report(dir.path(), &old, Path::new(TEST_PROFILE)).unwrap();
        let saved = read_saved_report(dir.path())
            .expect("the report parses")
            .report;
        assert_eq!(saved.mapped_phrases, old.mapped_phrases);
        assert_eq!(saved.unknown_settings, old.unknown_settings);
        assert_eq!(saved.left_out.len(), old.left_out.len());
        // The same profile: no difference.
        assert_eq!(
            profile_change(&old, Some(dir.path()), Path::new(TEST_PROFILE)),
            None
        );
        assert_eq!(profile_change(&old, None, Path::new(TEST_PROFILE)), None);
        let new = build_map(&changed_profile()).unwrap().report;
        let change =
            profile_change(&new, Some(dir.path()), Path::new(TEST_PROFILE)).expect("a difference");
        assert!(!change.same_profile);
        assert_eq!(change.difference.added, ["open the invented extra room"]);
        assert!(change.difference.removed.is_empty());
        let lines = profile_lines(&new, Some(&change), false);
        assert_eq!(
            lines.last().unwrap(),
            "the EDCoPilot profile changed: 1 commands added, 0 removed, 0 now left out, 0 now in the map"
        );
    }

    /// Two configurations with different profiles share the report file. A
    /// saved report of another profile file gives no line.
    #[test]
    fn test_a_saved_report_of_another_profile_file_gives_no_line() {
        let dir = tempfile::tempdir().unwrap();
        let old = build_map(&fixture_profile()).unwrap().report;
        let new = build_map(&changed_profile()).unwrap().report;
        save_report(dir.path(), &old, Path::new(TEST_PROFILE)).unwrap();
        let other = Path::new("D:\\Copy\\EDCoPilot-Profile.vap");
        assert_eq!(profile_change(&new, Some(dir.path()), other), None);
        // The same file with other case and other separators.
        let same = Path::new("d:/ed/edcopilot-profile.VAP");
        assert_eq!(
            profile_change(&new, Some(dir.path()), same).is_some(),
            cfg!(windows)
        );
        // A report of an older program has no path: no line.
        let path = report_path(dir.path());
        let text = std::fs::read_to_string(&path).unwrap();
        let mut json: serde_json::Value = serde_json::from_str(&text).unwrap();
        json.as_object_mut().unwrap().remove("profile_path");
        std::fs::write(&path, json.to_string()).unwrap();
        assert_eq!(read_saved_report(dir.path()).unwrap().profile_path, "");
        assert_eq!(
            profile_change(&new, Some(dir.path()), Path::new(TEST_PROFILE)),
            None
        );
    }

    /// A file of schema 1 without the unknown settings still reads.
    #[test]
    fn test_a_saved_report_without_the_unknown_settings_reads() {
        let dir = tempfile::tempdir().unwrap();
        let mut old = build_map(&fixture_profile()).unwrap().report;
        old.unknown_settings.insert((21, 4), 3);
        save_report(dir.path(), &old, Path::new(TEST_PROFILE)).unwrap();
        let text = std::fs::read_to_string(report_path(dir.path())).unwrap();
        let mut json: serde_json::Value = serde_json::from_str(&text).unwrap();
        assert_eq!(json["unknown_settings"][0]["actions"], 3);
        json.as_object_mut().unwrap().remove("unknown_settings");
        std::fs::write(report_path(dir.path()), json.to_string()).unwrap();
        let saved = read_saved_report(dir.path())
            .expect("the report parses")
            .report;
        assert!(saved.unknown_settings.is_empty());
        assert_eq!(saved.mapped_phrases, old.mapped_phrases);
    }

    /// The count of the records with an unknown phrase variable: a line
    /// only when it is not zero. The saved report keeps it, and a saved
    /// report without it still reads.
    #[test]
    fn test_unknown_phrase_variables_give_a_line_and_are_saved() {
        let dir = tempfile::tempdir().unwrap();
        let mut report = build_map(&fixture_profile()).unwrap().report;
        let line = "3 commands have a phrase variable that the program does not know";
        assert!(!profile_lines(&report, None, false).contains(&line.to_string()));
        report.unknown_phrase_vars = 3;
        assert!(profile_lines(&report, None, false).contains(&line.to_string()));
        save_report(dir.path(), &report, Path::new(TEST_PROFILE)).unwrap();
        let saved = read_saved_report(dir.path()).unwrap().report;
        assert_eq!(saved.unknown_phrase_vars, 3);
        let text = std::fs::read_to_string(report_path(dir.path())).unwrap();
        let mut json: serde_json::Value = serde_json::from_str(&text).unwrap();
        json.as_object_mut().unwrap().remove("unknown_phrase_vars");
        std::fs::write(report_path(dir.path()), json.to_string()).unwrap();
        let saved = read_saved_report(dir.path()).unwrap().report;
        assert_eq!(saved.unknown_phrase_vars, 0);
    }

    #[test]
    fn test_a_report_file_that_does_not_parse_or_has_another_schema_is_ignored() {
        let dir = tempfile::tempdir().unwrap();
        let new = build_map(&changed_profile()).unwrap().report;
        std::fs::write(report_path(dir.path()), "not json").unwrap();
        assert!(read_saved_report(dir.path()).is_none());
        assert_eq!(
            profile_change(&new, Some(dir.path()), Path::new(TEST_PROFILE)),
            None
        );
        let old = build_map(&fixture_profile()).unwrap().report;
        save_report(dir.path(), &old, Path::new(TEST_PROFILE)).unwrap();
        let text = std::fs::read_to_string(report_path(dir.path()))
            .unwrap()
            .replace("\"schema\": 1", "\"schema\": 2");
        std::fs::write(report_path(dir.path()), text).unwrap();
        assert!(read_saved_report(dir.path()).is_none());
        assert_eq!(
            profile_change(&new, Some(dir.path()), Path::new(TEST_PROFILE)),
            None
        );
    }

    #[test]
    fn test_the_lines_group_the_commands_that_are_not_understood() {
        let report = build_map(&fixture_profile()).unwrap().report;
        let lines = profile_lines(&report, None, false);
        assert_eq!(
            lines[0],
            format!(
                "commands from the EDCoPilot profile: {} commands, {} left out",
                report.records,
                report.left_out.len()
            )
        );
        let groups: Vec<&String> = lines
            .iter()
            .filter(|l| l.starts_with("left out, "))
            .collect();
        let not_understood = report.left_out_count(LeftOutClass::NotUnderstood);
        let sum: usize = groups
            .iter()
            .map(|l| {
                l["left out, ".len()..]
                    .split(' ')
                    .next()
                    .unwrap()
                    .parse::<usize>()
                    .unwrap()
            })
            .sum();
        assert_eq!(sum, not_understood);
        let every = profile_lines(&report, None, true);
        assert_eq!(every.len(), 1 + report.left_out.len());
        assert!(
            every
                .iter()
                .any(|l| l.starts_with("left out: \"ring all bells\" (multi_write): ")),
            "{every:?}"
        );
    }

    #[test]
    fn test_profile_watch_gives_one_signal_for_each_change() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(PROFILE_FILE);
        std::fs::write(&path, b"one").unwrap();
        let mut watch = ProfileWatch::new(&path);
        assert!(!watch.poll());
        std::fs::write(&path, b"second").unwrap();
        // The first poll sees the change, the second poll confirms it.
        assert!(!watch.poll());
        assert!(watch.poll());
        assert!(!watch.poll());
        assert!(!watch.poll());
        // An absent file is not a change.
        std::fs::remove_file(&path).unwrap();
        assert!(!watch.poll());
        assert!(!watch.poll());
        std::fs::write(&path, b"third one").unwrap();
        assert!(!watch.poll());
        assert!(watch.poll());
    }

    /// Sets the time of the last change of `path`.
    fn touch_at(path: &Path, secs: u64) {
        let file = std::fs::OpenOptions::new().write(true).open(path).unwrap();
        file.set_modified(SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(secs))
            .unwrap();
    }

    /// The watch reads the file after a change of the stamp and compares
    /// its hash with the hash of the profile of the session.
    #[test]
    fn test_profile_watch_warns_only_when_the_content_differs() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(PROFILE_FILE);
        std::fs::write(&path, b"one").unwrap();
        touch_at(&path, 1_000_000);
        let mut watch = ProfileWatch::new(&path);
        watch.set_session_hash(&source_sha256(b"one"));
        // The same content written again: a new time, no warning.
        std::fs::write(&path, b"one").unwrap();
        touch_at(&path, 1_000_100);
        for _ in 0..4 {
            assert!(!watch.poll());
        }
        // New content of the same size: one warning.
        std::fs::write(&path, b"two").unwrap();
        touch_at(&path, 1_000_200);
        assert!(!watch.poll());
        assert!(watch.poll());
        assert!(!watch.poll());
        // The new content written again: no second warning.
        std::fs::write(&path, b"two").unwrap();
        touch_at(&path, 1_000_300);
        for _ in 0..4 {
            assert!(!watch.poll());
        }
        // Back to the content of the session: no warning.
        std::fs::write(&path, b"one").unwrap();
        touch_at(&path, 1_000_400);
        for _ in 0..4 {
            assert!(!watch.poll());
        }
        // A change again: one warning.
        std::fs::write(&path, b"two").unwrap();
        touch_at(&path, 1_000_500);
        assert!(!watch.poll());
        assert!(watch.poll());
        // An absent file is not a change.
        std::fs::remove_file(&path).unwrap();
        for _ in 0..4 {
            assert!(!watch.poll());
        }
    }

    /// A file that the watch cannot read after a change of the stamp is
    /// read again at the next polls. Only a read with new content warns. A
    /// file that stays unreadable warns one time.
    #[cfg(windows)]
    #[test]
    fn test_profile_watch_reads_a_held_file_again() {
        use std::os::windows::fs::OpenOptionsExt;
        let hold = |path: &Path| {
            std::fs::OpenOptions::new()
                .read(true)
                .share_mode(0)
                .open(path)
                .unwrap()
        };
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(PROFILE_FILE);
        std::fs::write(&path, b"one").unwrap();
        touch_at(&path, 1_000_000);
        let mut watch = ProfileWatch::new(&path);
        watch.set_session_hash(&source_sha256(b"one"));

        // The same content, held at the confirmation: no warning.
        touch_at(&path, 1_000_100);
        assert!(!watch.poll());
        let held = hold(&path);
        for _ in 0..3 {
            assert!(!watch.poll());
        }
        drop(held);
        for _ in 0..4 {
            assert!(!watch.poll());
        }

        // New content, held at the confirmation: the warning comes after
        // the read.
        std::fs::write(&path, b"two").unwrap();
        touch_at(&path, 1_000_200);
        assert!(!watch.poll());
        let held = hold(&path);
        for _ in 0..3 {
            assert!(!watch.poll());
        }
        drop(held);
        assert!(watch.poll());
        for _ in 0..4 {
            assert!(!watch.poll());
        }

        // New content that stays held: one warning after MAX_UNREAD_POLLS
        // polls, and no second warning when the read works again.
        std::fs::write(&path, b"three").unwrap();
        touch_at(&path, 1_000_300);
        assert!(!watch.poll());
        let held = hold(&path);
        let warnings = (0..MAX_UNREAD_POLLS * 2)
            .map(|_| watch.poll())
            .collect::<Vec<_>>();
        assert_eq!(warnings.iter().filter(|w| **w).count(), 1);
        assert!(warnings[MAX_UNREAD_POLLS as usize - 1]);
        drop(held);
        for _ in 0..4 {
            assert!(!watch.poll());
        }
    }

    /// The saved report holds the program version. With the same profile
    /// and another version, a different result gives its own line.
    #[test]
    fn test_another_program_version_with_another_result_gives_its_own_line() {
        let dir = tempfile::tempdir().unwrap();
        let report = build_map(&fixture_profile()).unwrap().report;
        save_report(dir.path(), &report, Path::new(TEST_PROFILE)).unwrap();
        let saved = read_saved_report(dir.path()).unwrap();
        assert_eq!(saved.program_version, PROGRAM_VERSION);
        let path = report_path(dir.path());
        let original: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        let edit = |version: Option<&str>, drop_phrase: bool| {
            let mut json = original.clone();
            match version {
                Some(v) => json["program_version"] = v.into(),
                None => {
                    json.as_object_mut().unwrap().remove("program_version");
                }
            }
            if drop_phrase {
                json["mapped_phrases"].as_array_mut().unwrap().remove(0);
            }
            std::fs::write(&path, json.to_string()).unwrap();
        };
        // The same version and the same profile: no compare.
        edit(Some(PROGRAM_VERSION), true);
        assert_eq!(
            profile_change(&report, Some(dir.path()), Path::new(TEST_PROFILE)),
            None
        );
        // Another version with the same result: no line.
        edit(Some("0.0.1"), false);
        assert_eq!(
            profile_change(&report, Some(dir.path()), Path::new(TEST_PROFILE)),
            None
        );
        // Another version with another result.
        edit(Some("0.0.1"), true);
        let change = profile_change(&report, Some(dir.path()), Path::new(TEST_PROFILE))
            .expect("a difference");
        assert!(change.same_profile);
        assert_eq!(change.difference.added.len(), 1);
        assert_eq!(
            change.line(),
            "this program version reads the EDCoPilot profile in another way: \
             1 commands added, 0 removed, 0 now left out, 0 now in the map"
        );
        let lines = profile_lines(&report, Some(&change), false);
        assert_eq!(lines.last().unwrap(), &change.line());
        // A file of an older program has no version: it reads, and the
        // version counts as another version.
        edit(None, true);
        let saved = read_saved_report(dir.path()).expect("the report parses");
        assert_eq!(saved.program_version, "");
        assert!(
            profile_change(&report, Some(dir.path()), Path::new(TEST_PROFILE))
                .is_some_and(|c| c.same_profile)
        );
        edit(None, false);
        assert_eq!(
            profile_change(&report, Some(dir.path()), Path::new(TEST_PROFILE)),
            None
        );
    }

    #[test]
    fn test_a_saved_report_above_its_size_limit_is_ignored() {
        let dir = tempfile::tempdir().unwrap();
        let report = build_map(&fixture_profile()).unwrap().report;
        save_report(dir.path(), &report, Path::new(TEST_PROFILE)).unwrap();
        let path = report_path(dir.path());
        let text = std::fs::read_to_string(&path).unwrap();
        // White space after the JSON text: the file still parses.
        let padded = |len: u64| {
            let mut t = text.clone();
            t.push_str(&" ".repeat(len as usize - text.len()));
            std::fs::write(&path, t).unwrap();
        };
        padded(MAX_REPORT_BYTES);
        assert!(read_saved_report(dir.path()).is_some());
        padded(MAX_REPORT_BYTES + 1);
        assert!(read_saved_report(dir.path()).is_none());
        assert_eq!(MAX_REPORT_BYTES, 1024 * 1024);
    }
    /// A profile that is cut off at the first read (EDCoPilot writes it at
    /// this moment) builds at the second read, 300 ms later.
    #[test]
    fn test_a_cut_profile_is_read_again_one_time() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(PROFILE_FILE);
        let full = fixture_profile();
        std::fs::write(&path, &full[..full.len() / 2]).unwrap();
        let writer = {
            let path = path.clone();
            std::thread::spawn(move || {
                std::thread::sleep(std::time::Duration::from_millis(100));
                std::fs::write(&path, fixture_profile()).unwrap();
            })
        };
        let built = build_profile(&path, ProfileSource::Folder).expect("the second read builds");
        writer.join().unwrap();
        assert_eq!(built.report.source.sha256, source_sha256(&full));
        // A file that stays cut gives the error of the builder.
        std::fs::write(&path, &full[..full.len() / 2]).unwrap();
        let started = std::time::Instant::now();
        let error = format!(
            "{:#}",
            build_profile(&path, ProfileSource::Folder).unwrap_err()
        );
        assert!(error.contains("cut off"), "{error}");
        assert!(
            error.ends_with(", try again, or start EDCoPilot one time to make the file again"),
            "{error}"
        );
        assert!(started.elapsed() >= RETRY_WAIT);
        // A file that is not a profile: no second read.
        std::fs::write(&path, "a text file").unwrap();
        let started = std::time::Instant::now();
        assert!(build_profile(&path, ProfileSource::Folder).is_err());
        assert!(started.elapsed() < RETRY_WAIT);
    }

    /// A profile that another program holds open with no sharing (os error
    /// 32) is read again one time.
    #[cfg(windows)]
    #[test]
    fn test_a_profile_that_another_program_holds_is_read_again_one_time() {
        use std::os::windows::fs::OpenOptionsExt;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(PROFILE_FILE);
        std::fs::write(&path, fixture_profile()).unwrap();
        let held = std::fs::OpenOptions::new()
            .read(true)
            .share_mode(0)
            .open(&path)
            .unwrap();
        let error = std::fs::File::open(&path).unwrap_err();
        assert_eq!(error.raw_os_error(), Some(32));
        let release = std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(100));
            drop(held);
        });
        assert!(build_profile(&path, ProfileSource::Folder).is_ok());
        release.join().unwrap();
    }

    /// A profile that another program still holds after the second read:
    /// the advice says to try again, not to set the folder.
    #[cfg(windows)]
    #[test]
    fn test_a_profile_that_stays_held_gives_the_advice_to_try_again() {
        use std::os::windows::fs::OpenOptionsExt;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(PROFILE_FILE);
        std::fs::write(&path, fixture_profile()).unwrap();
        let held = std::fs::OpenOptions::new()
            .read(true)
            .share_mode(0)
            .open(&path)
            .unwrap();
        let error = format!(
            "{:#}",
            build_profile(&path, ProfileSource::Folder).unwrap_err()
        );
        drop(held);
        let start = format!("cannot read the EDCoPilot profile {}: ", path.display());
        assert!(error.starts_with(&start), "{error}");
        assert!(
            error.ends_with(", another program holds the file now, try again"),
            "{error}"
        );
        assert!(!error.contains("set the EDCoPilot folder"), "{error}");
        // Another cause gives the advice of the folder.
        std::fs::remove_file(&path).unwrap();
        let error = format!(
            "{:#}",
            build_profile(&path, ProfileSource::Folder).unwrap_err()
        );
        assert!(
            error.ends_with(
                ": the file does not exist, set the EDCoPilot folder, or start EDCoPilot one \
                 time to make the file"
            ),
            "{error}"
        );
    }
}
